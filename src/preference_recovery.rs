//! Preserve unreadable preferences until a verified original backup exists.
use crate::{
    Dudel, egui,
    preferences::{self, Preferences},
};
use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
};
pub(crate) const CAPTURE_SOURCE: &str = "{invalid saved settings";
pub(crate) struct CaptureStorage;
impl eframe::Storage for CaptureStorage {
    fn get_string(&self, key: &str) -> Option<String> {
        (key == preferences::KEY).then(|| CAPTURE_SOURCE.into())
    }
    fn set_string(&mut self, _key: &str, _value: String) {}
    fn flush(&mut self) {}
}

pub(crate) struct Recovery {
    container: Option<PathBuf>,
    reason: String,
    open: bool,
    backup: Option<PathBuf>,
    feedback: Option<String>,
    /// A file chosen in a dialog, used on the next frame, which has storage.
    chosen: Option<Chosen>,
}

enum Chosen {
    Backup(PathBuf),
    Restore(PathBuf),
}

fn short_error(mut error: String) -> String {
    if let Some((limit, _)) = error.char_indices().nth(512) {
        error.truncate(limit);
        error.push_str("...");
    }
    error
}

fn original(storage: Option<&dyn eframe::Storage>) -> Result<String, String> {
    storage
        .and_then(|s| s.get_string(preferences::KEY))
        .ok_or_else(|| "The original settings are not available in this session".into())
}
fn write_new(path: &Path, source: &str) -> Result<(), String> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut file = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
    file.write_all(source.as_bytes())
        .map_err(|e| e.to_string())?;
    file.as_file().sync_all().map_err(|e| e.to_string())?;
    file.persist_noclobber(path).map_err(|e| {
        if e.error.kind() == std::io::ErrorKind::AlreadyExists {
            "Choose a new backup filename; existing files are preserved".to_owned()
        } else {
            format!("Backup could not be saved: {}", e.error)
        }
    })?;
    Ok(())
}
fn equal_file(path: &Path, source: &str) -> Result<(), String> {
    let mut file =
        std::fs::File::open(path).map_err(|e| format!("Backup could not be read: {e}"))?;
    let mismatch = || {
        "The backup no longer matches the original settings. Export a new backup first".to_owned()
    };
    if file.metadata().map_err(|e| e.to_string())?.len() != source.len() as u64 {
        return Err(mismatch());
    }
    let mut buffer = [0; 64 * 1024];
    for bytes in source.as_bytes().chunks(buffer.len()) {
        file.read_exact(&mut buffer[..bytes.len()])
            .map_err(|e| e.to_string())?;
        if bytes != &buffer[..bytes.len()] {
            return Err(mismatch());
        }
    }
    if file.read(&mut buffer[..1]).map_err(|e| e.to_string())? != 0 {
        return Err(mismatch());
    }
    Ok(())
}

impl Dudel {
    pub(crate) fn protect_settings_container(&mut self, path: PathBuf, reason: String) {
        self.preference_recovery = Some(Recovery {
            container: Some(path),
            reason: short_error(reason),
            open: true,
            backup: None,
            feedback: None,
            chosen: None,
        });
    }
    pub(crate) fn restore_preferences_source(&mut self, source: &str) {
        match Preferences::decode(source) {
            Ok(p) => {
                self.apply_preferences(p);
                self.preference_recovery = None;
            }
            Err(reason) => {
                let reason = short_error(reason);
                self.status = format!("Settings recovery needed: {reason}");
                self.preference_recovery = Some(Recovery {
                    container: None,
                    reason,
                    open: true,
                    backup: None,
                    feedback: None,
                    chosen: None,
                });
            }
        }
    }
    fn verify_preferences_backup(
        &self,
        storage: Option<&dyn eframe::Storage>,
    ) -> Result<(), String> {
        let path = self
            .preference_recovery
            .as_ref()
            .and_then(|r| r.backup.as_ref())
            .ok_or("Export the original settings before replacing them")?;
        if let Some(original) = &self.preference_recovery.as_ref().unwrap().container {
            if crate::settings_store::equal_files(path, original)? {
                Ok(())
            } else {
                Err(
                    "The backup no longer matches the original settings. Export a new backup first"
                        .into(),
                )
            }
        } else {
            equal_file(path, &original(storage)?)
        }
    }
    pub(crate) fn backup_preferences_to(
        &mut self,
        storage: Option<&dyn eframe::Storage>,
        path: &Path,
    ) -> Result<(), String> {
        if self.preference_recovery.is_none() {
            return Err("No settings recovery is pending".into());
        }
        if let Some(original) = &self.preference_recovery.as_ref().unwrap().container {
            let parent = path.parent().unwrap_or(Path::new("."));
            let mut source = std::fs::File::open(original).map_err(|e| e.to_string())?;
            let mut pending = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
            std::io::copy(&mut source, &mut pending).map_err(|e| e.to_string())?;
            pending.as_file().sync_all().map_err(|e| e.to_string())?;
            pending
                .persist_noclobber(path)
                .map_err(|e| e.error.to_string())?;
            if !crate::settings_store::equal_files(path, original)? {
                return Err(
                    "Original settings changed while backing up; export a new backup".into(),
                );
            }
        } else {
            let source = original(storage)?;
            write_new(path, &source)?;
            equal_file(path, &source)?;
        }
        let recovery = self.preference_recovery.as_mut().unwrap();
        recovery.backup = Some(path.to_owned());
        recovery.feedback =
            Some("Original settings backed up. Choose which settings to use below.".into());
        Ok(())
    }
    pub(crate) fn use_current_preferences(
        &mut self,
        storage: Option<&dyn eframe::Storage>,
    ) -> Result<(), String> {
        self.verify_preferences_backup(storage)?;
        // Validate the same snapshot the save callback will write before lifting
        // the guard. Invalid session state must never replace protected data.
        Preferences::decode(&self.encode_preferences()?)?;
        self.preference_recovery = None;
        self.status = "Current settings will be saved; original backup preserved".into();
        Ok(())
    }
    pub(crate) fn restore_preferences_file(
        &mut self,
        storage: Option<&dyn eframe::Storage>,
        path: &Path,
    ) -> Result<(), String> {
        let source = crate::read_bounded(path, 128 * 1024 * 1024)?;
        let settings = Preferences::decode(&source).or_else(|_| {
            crate::settings_store::preferences_from_container(source.as_bytes())
                .and_then(|json| Preferences::decode(&json))
        })?;
        self.verify_preferences_backup(storage)?;
        self.close_input_gestures();
        self.apply_preferences(settings);
        self.brush_search.clear();
        self.recent_brushes.clear();
        self.brush_favorites_only = false;
        self.scratchpad.reset_stroke();
        self.preference_recovery = None;
        self.status = "Settings restored; original backup preserved".into();
        Ok(())
    }
    pub(crate) fn preference_recovery_banner(&mut self, ctx: &egui::Context) {
        let Some(recovery) = &mut self.preference_recovery else {
            return;
        };
        egui::TopBottomPanel::top("settings recovery banner").show(ctx, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    egui::RichText::new("Settings saving is paused")
                        .color(ui.visuals().warn_fg_color),
                );
                if ui.button("Review settings recovery").clicked() {
                    recovery.open = true;
                }
            });
        });
    }
    pub(crate) fn preference_recovery_ui(
        &mut self,
        ctx: &egui::Context,
        storage: Option<&dyn eframe::Storage>,
    ) {
        let Some(recovery) = self.preference_recovery.as_mut() else {
            return;
        };
        let result = match recovery.chosen.take() {
            Some(Chosen::Backup(path)) => Some(self.backup_preferences_to(storage, &path)),
            Some(Chosen::Restore(path)) => Some(self.restore_preferences_file(storage, &path)),
            None => None,
        };
        if let Some(Err(error)) = result
            && let Some(recovery) = &mut self.preference_recovery
        {
            recovery.feedback = Some(short_error(error));
        }
        let Some(recovery) = self.preference_recovery.as_ref() else {
            return;
        };
        if !recovery.open {
            return;
        }
        let reason = recovery.reason.clone();
        let backup = recovery.backup.clone();
        let feedback = recovery.feedback.clone();
        let container = recovery.container.is_some();
        let available = container || storage.is_some();
        let mut open = true;
        let mut action = 0;
        let maximum =
            (ctx.content_rect().size() - egui::vec2(32.0, 64.0)).max(egui::vec2(240.0, 160.0));
        egui::Window::new("Settings recovery").id(egui::Id::new("settings recovery"))
            .open(&mut open).collapsible(false).resizable(true)
            .default_size(egui::vec2(480.0,320.0).min(maximum))
            .min_size(egui::vec2(280.0,180.0).min(maximum)).max_size(maximum).vscroll(true)
            .show(ctx,|ui|{
                ui.label("Saved settings could not be restored. Settings saving is paused to preserve the original. Export a backup before choosing replacement settings.");
                ui.label("Project saving is available.");
                egui::CollapsingHeader::new("Details").show(ui,|ui|{ui.label(&reason);});
                if ui.add_enabled(available,egui::Button::new("Export original settings..."))
                    .on_hover_text("Save an exact copy to a new file before replacing saved settings").clicked(){action=1;}
                if let Some(path)=&backup {
                    ui.add(egui::Label::new("Original backup saved").truncate()).on_hover_text(path.display().to_string());
                } else {ui.small("Export a backup first to enable the choices below.");}
                ui.horizontal_wrapped(|ui|{
                    if ui.add_enabled(backup.is_some() && available,egui::Button::new("Use current settings"))
                        .on_hover_text("Enable saving this session's brushes, colors and workspace").clicked(){action=2;}
                    if ui.add_enabled(backup.is_some() && available,egui::Button::new("Restore settings file..."))
                        .on_hover_text("Replace this session's brushes, colors and workspace from a validated settings file").clicked(){action=3;}
                });
                if let Some(message)=&feedback {ui.label(message);}
                ui.separator();
                if ui.button("Keep original settings").clicked(){action=4;}
            });
        if action == 4 {
            open = false;
        }
        if let Some(recovery) = &mut self.preference_recovery {
            recovery.open = open;
        }
        let choose = |app: &mut Dudel, chosen| {
            if let Some(recovery) = &mut app.preference_recovery {
                recovery.chosen = Some(chosen);
            }
        };
        let result = match action {
            1 => {
                let dialog = rfd::AsyncFileDialog::new()
                    .add_filter("Dudel settings", &["json", "ron"])
                    .set_file_name(if container {
                        "Dudel-original-settings.ron"
                    } else {
                        "Dudel-original-settings.json"
                    });
                self.ask_save_file(dialog, move |app, path| choose(app, Chosen::Backup(path)));
                None
            }
            2 => Some(self.use_current_preferences(storage)),
            3 => {
                let dialog =
                    rfd::AsyncFileDialog::new().add_filter("Dudel settings", &["json", "ron"]);
                self.ask_open_file(dialog, move |app, path| choose(app, Chosen::Restore(path)));
                None
            }
            _ => None,
        };
        if let Some(Err(error)) = result
            && let Some(recovery) = &mut self.preference_recovery
        {
            recovery.feedback = Some(short_error(error));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::Storage;
    #[derive(Default)]
    struct Store(std::collections::HashMap<String, String>);
    impl Storage for Store {
        fn get_string(&self, key: &str) -> Option<String> {
            self.0.get(key).cloned()
        }
        fn set_string(&mut self, key: &str, value: String) {
            self.0.insert(key.into(), value);
        }
        fn flush(&mut self) {}
    }
    fn failed(source: &str) -> (Dudel, Store) {
        let mut app = Dudel::default();
        app.restore_preferences_source(source);
        assert!(app.preference_recovery.is_some());
        let mut store = Store::default();
        store.set_string(preferences::KEY, source.into());
        store.set_string("other-key", "preserve other framework data".into());
        (app, store)
    }
    #[test]
    fn failed_restore_and_dismissal_preserve_original_through_repeated_settings_saves() {
        let good = Dudel::default().encode_preferences().unwrap();
        let mut future: serde_json::Value = serde_json::from_str(&good).unwrap();
        future["version"] = serde_json::json!(999);
        let mut invalid: serde_json::Value = serde_json::from_str(&good).unwrap();
        invalid["brushes"][0]["name"] = serde_json::json!("A".repeat(205));
        let mut unknown: serde_json::Value = serde_json::from_str(&good).unwrap();
        unknown["current_brush"]["tip"] = serde_json::json!("🖌".repeat(10000));
        for source in [
            "{invalid settings".to_owned(),
            future.to_string(),
            invalid.to_string(),
            unknown.to_string(),
        ] {
            let (mut app, mut store) = failed(&source);
            let doc = app.doc.clone();
            assert!(
                app.preference_recovery
                    .as_ref()
                    .unwrap()
                    .reason
                    .chars()
                    .count()
                    <= 515
            );
            app.preference_recovery.as_mut().unwrap().open = false;
            app.brush.name = "Working session ink".into();
            app.color = [12, 34, 56];
            for _ in 0..4 {
                eframe::App::save(&mut app, &mut store);
            }
            assert_eq!(store.get_string(preferences::KEY).unwrap(), source);
            assert_eq!(
                store.get_string("other-key").unwrap(),
                "preserve other framework data"
            );
            assert!(!eframe::App::persist_egui_memory(&app));
            assert!(crate::document::same_snapshot(&doc, &app.doc));
            assert!(app.use_current_preferences(Some(&store)).is_err());
        }
    }
    #[test]
    fn backup_is_exact_non_overwriting_and_bound_to_current_original_bytes() {
        let source = format!("{{broken settings {}", "é".repeat(100000));
        let (mut app, mut store) = failed(&source);
        let directory = tempfile::tempdir().unwrap();
        let backup = directory.path().join("original.json");
        std::fs::write(&backup, b"existing sentinel").unwrap();
        assert!(app.backup_preferences_to(Some(&store), &backup).is_err());
        assert_eq!(std::fs::read(&backup).unwrap(), b"existing sentinel");
        assert!(app.preference_recovery.as_ref().unwrap().backup.is_none());
        assert!(
            app.backup_preferences_to(None, &directory.path().join("missing-source.json"))
                .is_err()
        );
        let backup = directory.path().join("new-original.json");
        app.backup_preferences_to(Some(&store), &backup).unwrap();
        assert_eq!(std::fs::read(&backup).unwrap(), source.as_bytes());
        eframe::App::save(&mut app, &mut store);
        assert_eq!(store.get_string(preferences::KEY).unwrap(), source);
        let mut changed = source.as_bytes().to_vec();
        changed[0] = b'!';
        std::fs::write(&backup, &changed).unwrap();
        assert!(app.use_current_preferences(Some(&store)).is_err());
        std::fs::write(&backup, &source).unwrap();
        std::fs::remove_file(&backup).unwrap();
        assert!(app.use_current_preferences(Some(&store)).is_err());
        std::fs::write(&backup, &source).unwrap();
        store.set_string(preferences::KEY, "different newer original".into());
        assert!(app.use_current_preferences(Some(&store)).is_err());
        assert!(app.preference_recovery.is_some());
        let fresh = directory.path().join("newer-original.json");
        app.backup_preferences_to(Some(&store), &fresh).unwrap();
        let saved_name = app.brushes[0].name.clone();
        app.brushes[0].name.clear();
        assert!(app.use_current_preferences(Some(&store)).is_err());
        assert!(app.preference_recovery.is_some());
        assert_eq!(
            store.get_string(preferences::KEY).unwrap(),
            "different newer original"
        );
        app.brushes[0].name = saved_name;
        app.brush.name = "Session ink".into();
        app.use_current_preferences(Some(&store)).unwrap();
        assert!(eframe::App::persist_egui_memory(&app));
        eframe::App::save(&mut app, &mut store);
        assert_eq!(
            Preferences::decode(&store.get_string(preferences::KEY).unwrap())
                .unwrap()
                .current_brush
                .name,
            "Session ink"
        );
        assert_eq!(
            std::fs::read_to_string(fresh).unwrap(),
            "different newer original"
        );
    }
    #[test]
    fn validated_settings_restore_is_atomic_and_requires_an_intact_original_backup() {
        let (mut app, mut store) = failed("{unreadable original");
        let doc = app.doc.clone();
        let brush = app.brush.clone();
        let directory = tempfile::tempdir().unwrap();
        let candidate = directory.path().join("candidate.json");
        let mut replacement = Dudel::default();
        replacement.brush.name = "Recovered custom ink".into();
        replacement.brush.stabilization = 17.0;
        replacement.color = [93, 71, 22];
        std::fs::write(&candidate, replacement.encode_preferences().unwrap()).unwrap();
        assert!(
            app.restore_preferences_file(Some(&store), &candidate)
                .is_err()
        );
        assert!(app.brush == brush && app.preference_recovery.is_some());
        let backup = directory.path().join("original.json");
        app.backup_preferences_to(Some(&store), &backup).unwrap();
        let invalid = directory.path().join("invalid.json");
        std::fs::write(&invalid, b"{bad replacement").unwrap();
        assert!(
            app.restore_preferences_file(Some(&store), &invalid)
                .is_err()
        );
        assert!(app.brush == brush && app.preference_recovery.is_some());
        app.restore_preferences_file(Some(&store), &candidate)
            .unwrap();
        assert!(app.preference_recovery.is_none());
        assert_eq!(app.brush.name, "Recovered custom ink");
        assert_eq!(app.brush.stabilization, 17.0);
        assert_eq!(app.color, [93, 71, 22]);
        assert!(crate::document::same_snapshot(&doc, &app.doc));
        assert!(!app.history.can_undo());
        assert_eq!(
            store.get_string(preferences::KEY).unwrap(),
            "{unreadable original"
        );
        eframe::App::save(&mut app, &mut store);
        assert_eq!(
            Preferences::decode(&store.get_string(preferences::KEY).unwrap())
                .unwrap()
                .current_brush
                .name,
            "Recovered custom ink"
        );
        assert_eq!(
            std::fs::read_to_string(backup).unwrap(),
            "{unreadable original"
        );
    }
    #[test]
    fn project_save_remains_available_while_settings_are_protected() {
        let (mut app, mut store) = failed("{unreadable original");
        app.doc.layers[0]
            .pixels
            .writer()
            .rgba_mut(0)
            .copy_from_slice(&[7, 41, 193, 255]);
        app.changed();
        let directory = tempfile::tempdir().unwrap();
        let project = directory.path().join("artwork.dudel");
        app.path = Some(project.clone());
        app.save(false);
        let ctx = egui::Context::default();
        let start = std::time::Instant::now();
        while app.save_job.is_some() {
            let _ = ctx.run(Default::default(), |ctx| app.poll_save(ctx));
            assert!(
                start.elapsed() < std::time::Duration::from_secs(10),
                "Project save did not finish"
            );
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        assert!(!app.modified && app.preference_recovery.is_some());
        assert_eq!(
            crate::storage::load_project(&project).unwrap().layers[0]
                .pixels
                .rgba(0),
            [7, 41, 193, 255]
        );
        eframe::App::save(&mut app, &mut store);
        assert_eq!(
            store.get_string(preferences::KEY).unwrap(),
            "{unreadable original"
        );
    }
    #[test]
    fn recovery_dialog_can_be_dismissed_reopened_and_accept_a_verified_current_session() {
        struct Ui {
            ctx: egui::Context,
            app: Dudel,
            store: Store,
            output: egui::FullOutput,
            time: f64,
        }
        impl Ui {
            fn frame(&mut self, events: Vec<egui::Event>) {
                self.time += 0.05;
                self.output = self.ctx.run(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(640.0, 480.0),
                        )),
                        time: Some(self.time),
                        events,
                        ..Default::default()
                    },
                    |ctx| {
                        crate::workspace::theme(ctx);
                        self.app.preference_recovery_banner(ctx);
                        egui::CentralPanel::default().show(ctx, |_| {});
                        self.app.preference_recovery_ui(ctx, Some(&self.store));
                    },
                );
            }
            fn settle(&mut self) {
                for _ in 0..4 {
                    self.frame(vec![]);
                }
            }
            fn text(&self, name: &str) -> egui::Pos2 {
                fn find(s: &egui::Shape, name: &str, clip: egui::Rect) -> Option<egui::Pos2> {
                    match s {
                        egui::Shape::Text(t) if t.galley.text() == name => {
                            let r =
                                egui::Rect::from_min_size(t.pos, t.galley.size()).intersect(clip);
                            r.is_positive().then(|| r.center())
                        }
                        egui::Shape::Vec(shapes) => shapes.iter().find_map(|s| find(s, name, clip)),
                        _ => None,
                    }
                }
                self.output
                    .shapes
                    .iter()
                    .rev()
                    .find_map(|s| find(&s.shape, name, s.clip_rect))
                    .unwrap_or_else(|| panic!("Recovery control missing: {name}"))
            }
            fn click(&mut self, name: &str) {
                let pos = self.text(name);
                for pressed in [true, false] {
                    self.frame(vec![
                        egui::Event::PointerMoved(pos),
                        egui::Event::PointerButton {
                            pos,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: Default::default(),
                        },
                    ]);
                }
                self.settle();
            }
        }
        let (app, store) = failed("{protected original");
        let mut ui = Ui {
            ctx: egui::Context::default(),
            app,
            store,
            output: Default::default(),
            time: 0.0,
        };
        ui.settle();
        ui.click("Keep original settings");
        assert!(!ui.app.preference_recovery.as_ref().unwrap().open);
        eframe::App::save(&mut ui.app, &mut ui.store);
        assert_eq!(
            ui.store.get_string(preferences::KEY).unwrap(),
            "{protected original"
        );
        ui.click("Review settings recovery");
        assert!(ui.app.preference_recovery.as_ref().unwrap().open);
        let directory = tempfile::tempdir().unwrap();
        ui.app
            .backup_preferences_to(Some(&ui.store), &directory.path().join("backup.json"))
            .unwrap();
        ui.settle();
        ui.click("Use current settings");
        assert!(ui.app.preference_recovery.is_none());
        eframe::App::save(&mut ui.app, &mut ui.store);
        assert!(Preferences::decode(&ui.store.get_string(preferences::KEY).unwrap()).is_ok());
    }
}
