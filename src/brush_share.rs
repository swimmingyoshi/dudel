//! Sharing brushes: export one brush as a `.dudelbrush` file with its author,
//! license and a description, and review any brush file before it joins the
//! library. A `.dudelbrush` is a one-brush pack, so it carries tips, grain and
//! a brush graph like any pack and older Dudel refuses packs it cannot read.
use crate::{
    Dudel,
    brush_pack::{BrushPack, PackInfo},
    egui, storage,
};

/// Common licenses offered when exporting; any text is allowed.
const LICENSES: [&str; 4] = [
    "All rights reserved",
    "CC0 1.0 (public domain)",
    "CC BY 4.0",
    "CC BY-SA 4.0",
];
/// Largest brush file read: tips and grain are at most 128 × 128 each.
const FILE_LIMIT: u64 = 16 * 1024 * 1024;

/// The Export brush form.
pub(crate) struct ExportForm {
    info: PackInfo,
}

/// A brush file read and checked, waiting for Add to library or Cancel.
pub(crate) struct PendingImport {
    pack: BrushPack,
    file: String,
    preview: Option<egui::TextureHandle>,
}

/// A file name from the brush name: letters, digits, spaces, - and _.
fn file_name(name: &str) -> String {
    let clean: String = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || matches!(c, ' ' | '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .collect();
    let clean = clean.trim();
    format!(
        "{}.dudelbrush",
        if clean.is_empty() { "brush" } else { clean }
    )
}

impl Dudel {
    /// Preset > Export brush…: ask for author, license and description first.
    pub(crate) fn open_brush_export(&mut self) {
        if let Err(error) = self.brush.validate() {
            self.status = format!("Correct the brush before exporting: {error}");
            return;
        }
        self.brush_export = Some(ExportForm {
            info: PackInfo {
                author: self.brush_author.clone(),
                license: self.brush_license.clone(),
                description: String::new(),
            },
        });
    }

    /// Screenshot scene: the import review for a shared smudge brush.
    pub(crate) fn brush_import_capture_fixture(&mut self) {
        let brush = crate::workspace::default_brushes()
            .into_iter()
            .find(|b| b.name == "Smudge")
            .unwrap_or_default();
        self.brush_import = Some(PendingImport {
            pack: BrushPack::single(
                crate::brush::Brush {
                    name: "Soft blender".into(),
                    ..brush
                },
                PackInfo {
                    author: "Grace Hopper".into(),
                    license: "CC BY 4.0".into(),
                    description:
                        "Pulls neighbouring colors together for soft skin and sky gradients.".into(),
                },
            ),
            file: "Soft blender.dudelbrush".into(),
            preview: None,
        });
    }

    /// Library > Import brush…: any brush file, reviewed before it is added.
    pub(crate) fn brush_import_file(&mut self) {
        let dialog = self.dialog_in(
            crate::locations::Kind::Brushes,
            rfd::AsyncFileDialog::new()
                .add_filter("Dudel brushes", &["dudelbrush", "dudelbrushes", "json"]),
        );
        self.ask_open_file(dialog, Self::import_brush_pack);
    }

    /// Read a brush file and show what it holds before adding it.
    pub(crate) fn import_brush_pack(&mut self, path: std::path::PathBuf) {
        let result = crate::read_bounded(&path, FILE_LIMIT).and_then(|s| BrushPack::read(&s));
        match result {
            Ok(pack) => {
                self.brush_import = Some(PendingImport {
                    pack,
                    file: path
                        .file_name()
                        .map_or_else(String::new, |n| n.to_string_lossy().into_owned()),
                    preview: None,
                });
            }
            Err(error) => self.status = format!("Brush file: {error}"),
        }
    }

    /// Add the reviewed brushes to the library and select the first.
    pub(crate) fn accept_brush_import(&mut self) {
        let Some(pending) = self.brush_import.take() else {
            return;
        };
        let count = pending.pack.brushes.len();
        let author = pending
            .pack
            .info
            .as_ref()
            .map(|i| i.author.trim().to_owned())
            .filter(|a| !a.is_empty());
        match pending.pack.append_to(&mut self.brushes) {
            Ok(first) => {
                self.selected_brush = first;
                self.brush = self.brushes[first].clone();
                self.brush_search.clear();
                let what = if count == 1 {
                    format!("Added “{}”", self.brush.name)
                } else {
                    format!("Added {count} brushes")
                };
                self.status = match author {
                    Some(author) => format!("{what} by {author}"),
                    None => what,
                };
            }
            Err(error) => self.status = format!("Brush file: {error}"),
        }
    }

    /// Write the brush with the form's details to `path`.
    fn write_brush_file(&mut self, info: PackInfo, path: std::path::PathBuf) {
        let pack = BrushPack::single(self.brush.clone(), info);
        let result = pack
            .validate()
            .and_then(|()| serde_json::to_vec(&pack).map_err(|e| e.to_string()))
            .and_then(|bytes| storage::atomic_write(&path, &bytes));
        self.status = match result {
            Ok(()) => format!("Exported “{}”", self.brush.name),
            Err(error) => format!("Brush export: {error}"),
        };
    }

    /// The Export brush and Import brush dialogs.
    pub(crate) fn brush_share_ui(&mut self, ctx: &egui::Context) {
        self.brush_export_ui(ctx);
        self.brush_import_ui(ctx);
    }

    fn brush_export_ui(&mut self, ctx: &egui::Context) {
        let Some(form) = &mut self.brush_export else {
            return;
        };
        let mut cancel = false;
        let mut export = false;
        let response = egui::Modal::new(egui::Id::new("export brush")).show(ctx, |ui| {
            ui.set_width(380.0);
            cancel |= crate::dialogs::dialog_title(ui, "Export brush");
            ui.label(egui::RichText::new(&self.brush.name).strong());
            ui.add_space(4.0);
            let field = |ui: &mut egui::Ui, label: &str, text: &mut String, hint: &str| {
                ui.label(label);
                ui.add(
                    egui::TextEdit::singleline(text)
                        .hint_text(hint)
                        .char_limit(100)
                        .desired_width(f32::INFINITY),
                );
            };
            field(ui, "Author", &mut form.info.author, "Your name");
            ui.horizontal(|ui| {
                ui.label("License");
                ui.menu_button("Common licenses", |ui| {
                    for license in LICENSES {
                        if ui.button(license).clicked() {
                            form.info.license = license.into();
                            ui.close();
                        }
                    }
                });
            });
            ui.add(
                egui::TextEdit::singleline(&mut form.info.license)
                    .hint_text("How others may use it")
                    .char_limit(100)
                    .desired_width(f32::INFINITY),
            );
            ui.label("Description");
            ui.add(
                egui::TextEdit::multiline(&mut form.info.description)
                    .hint_text("What it is good for")
                    .char_limit(1000)
                    .desired_rows(3)
                    .desired_width(f32::INFINITY),
            );
            if self.brush.graph.is_some() {
                ui.small("Includes its brush graph; older Dudel versions refuse the file.");
            }
            ui.small("Tip and paper-grain images are included in the file.");
            crate::dialogs::dialog_buttons(ui, |ui| {
                export = ui
                    .add(crate::workspace::primary_button(ui, "Export…"))
                    .clicked();
                cancel |= ui.button("Cancel").clicked();
            });
        });
        if export {
            let form = self.brush_export.take().unwrap();
            if let Err(error) = form.info.validate() {
                self.status = error;
                self.brush_export = Some(form);
                return;
            }
            self.brush_author = form.info.author.trim().to_owned();
            self.brush_license = form.info.license.trim().to_owned();
            let dialog = self.dialog_in(
                crate::locations::Kind::Brushes,
                rfd::AsyncFileDialog::new()
                    .add_filter("Dudel brush", &["dudelbrush"])
                    .set_file_name(file_name(&self.brush.name)),
            );
            let info = form.info;
            self.ask_save_file(dialog, move |app, path| app.write_brush_file(info, path));
        } else if cancel || response.should_close() {
            self.brush_export = None;
        }
    }

    fn brush_import_ui(&mut self, ctx: &egui::Context) {
        let Some(pending) = &mut self.brush_import else {
            return;
        };
        let Some(first) = pending.pack.brushes.first() else {
            self.brush_import = None;
            return;
        };
        let preview = pending
            .preview
            .get_or_insert_with(|| {
                let pixels = crate::brush_previews::render(first);
                ctx.load_texture(
                    "brush import preview",
                    egui::ColorImage::from_rgba_unmultiplied(
                        [crate::brush_previews::WIDTH, crate::brush_previews::HEIGHT],
                        &pixels,
                    ),
                    egui::TextureOptions::LINEAR,
                )
            })
            .clone();
        let count = pending.pack.brushes.len();
        let mut add = false;
        let mut cancel = false;
        let response = egui::Modal::new(egui::Id::new("import brush")).show(ctx, |ui| {
            ui.set_width(380.0);
            cancel |= crate::dialogs::dialog_title(
                ui,
                if count == 1 {
                    "Import brush"
                } else {
                    "Import brushes"
                },
            );
            ui.weak(&pending.file);
            let size = egui::vec2(
                crate::brush_previews::WIDTH as f32,
                crate::brush_previews::HEIGHT as f32,
            ) * 1.6;
            let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
            ui.painter()
                .rect_filled(rect, 4.0, ui.visuals().extreme_bg_color);
            ui.painter().image(
                preview.id(),
                rect,
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                egui::Color32::WHITE,
            );
            ui.label(egui::RichText::new(&first.name).strong());
            if count > 1 {
                ui.label(format!("and {} more", count - 1));
            }
            if let Some(info) = &pending.pack.info {
                if !info.author.trim().is_empty() {
                    ui.label(format!("By {}", info.author));
                }
                if !info.license.trim().is_empty() {
                    ui.label(format!("License: {}", info.license));
                }
                if !info.description.trim().is_empty() {
                    ui.label(&info.description);
                }
            }
            let mut notes = Vec::new();
            if pending.pack.brushes.iter().any(|b| b.graph.is_some()) {
                notes.push("Uses a brush graph");
            }
            if pending.pack.brushes.iter().any(|b| !b.blend.is_paint()) {
                notes.push("Smudge or blur brush");
            }
            if !notes.is_empty() {
                ui.small(notes.join(" · "));
            }
            crate::dialogs::dialog_buttons(ui, |ui| {
                add = ui
                    .add(crate::workspace::primary_button(ui, "Add to library"))
                    .clicked();
                cancel |= ui.button("Cancel").clicked();
            });
        });
        if add {
            self.accept_brush_import();
        } else if cancel || response.should_close() {
            self.brush_import = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::brush::Brush;

    fn graph_brush() -> Brush {
        let mut brush = Brush {
            name: "Speedy ink: v2/final".into(),
            ..Default::default()
        };
        brush.graph = Some(crate::brush_graph::BrushGraph::from_brush(&brush));
        brush.graph_layout = crate::brush_graph::auto_layout(brush.graph.as_ref().unwrap());
        brush
    }

    #[test]
    fn dudelbrush_files_round_trip_with_details_and_review_before_adding() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = Dudel {
            brush: graph_brush(),
            ..Default::default()
        };
        assert_eq!(
            file_name(&app.brush.name),
            "Speedy ink_ v2_final.dudelbrush"
        );
        let path = dir.path().join("speedy.dudelbrush");
        let info = PackInfo {
            author: "Ada".into(),
            license: "CC BY 4.0".into(),
            description: "Thins out when you draw fast.\nTry it on lineart.".into(),
        };
        app.write_brush_file(info.clone(), path.clone());
        assert!(app.status.starts_with("Exported"), "{}", app.status);
        // The file is a version 3 one-brush pack with the details.
        let pack: BrushPack =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!((pack.version, pack.brushes.len()), (3, 1));
        assert_eq!(pack.info.as_ref(), Some(&info));
        // Importing shows it first; Cancel adds nothing, Add adds it as saved.
        let mut other = Dudel::default();
        let before = other.brushes.len();
        other.import_brush_pack(path.clone());
        assert!(other.brush_import.is_some());
        other.brush_import = None;
        assert_eq!(other.brushes.len(), before);
        other.import_brush_pack(path);
        other.accept_brush_import();
        assert_eq!(other.brushes.len(), before + 1);
        assert!(other.brush == app.brush);
        assert!(other.status.ends_with("by Ada"), "{}", other.status);
    }

    #[test]
    fn legacy_json_brushes_import_and_bad_files_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        let legacy = dir.path().join("old.json");
        std::fs::write(&legacy, serde_json::to_vec(&Brush::default()).unwrap()).unwrap();
        let mut app = Dudel::default();
        app.import_brush_pack(legacy);
        let pending = app.brush_import.as_ref().expect("legacy brush reviewed");
        assert!(pending.pack.info.is_none() && pending.pack.brushes.len() == 1);
        // Malformed, newer and over-long files never reach the dialog.
        let check = |name: &str, contents: String, words: &str| {
            let path = dir.path().join(name);
            std::fs::write(&path, contents).unwrap();
            let mut app = Dudel::default();
            app.import_brush_pack(path);
            assert!(app.brush_import.is_none(), "{name}");
            assert!(app.status.contains(words), "{name}: {}", app.status);
        };
        check("broken.dudelbrush", "{not json".into(), "Brush file");
        let mut newer = serde_json::to_value(BrushPack::new(vec![Brush::default()])).unwrap();
        newer["version"] = 4.into();
        check("newer.dudelbrush", newer.to_string(), "newer Dudel");
        let mut long =
            serde_json::to_value(BrushPack::single(Brush::default(), PackInfo::default())).unwrap();
        long["info"] = serde_json::json!({"author": "x".repeat(101)});
        check("long.dudelbrush", long.to_string(), "too long");
    }
}
