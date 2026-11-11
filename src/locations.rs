//! Where Dudel keeps user files: one library folder (Documents/Dudel by
//! default) with a subfolder per kind, each of which can point elsewhere.
//! App data (settings, recovery, extensions) stays in the platform data
//! folder and is shown read-only.
use crate::{Dudel, egui};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Projects,
    Exports,
    Packages,
    Themes,
    Brushes,
    Palettes,
    Timelapses,
}
const KINDS: usize = 7;
impl Kind {
    pub const ALL: [Kind; KINDS] = [
        Kind::Projects,
        Kind::Exports,
        Kind::Packages,
        Kind::Themes,
        Kind::Brushes,
        Kind::Palettes,
        Kind::Timelapses,
    ];
    pub fn name(self) -> &'static str {
        match self {
            Self::Projects => "Projects",
            Self::Exports => "Exports",
            Self::Packages => "Packages",
            Self::Themes => "Themes",
            Self::Brushes => "Brushes",
            Self::Palettes => "Palettes",
            Self::Timelapses => "Timelapses",
        }
    }
    pub fn hint(self) -> &'static str {
        match self {
            Self::Projects => ".dudel projects; Open and Save start here",
            Self::Exports => "PNG exports of finished artwork",
            Self::Packages => ".zip project packages (layers as PNG plus the project)",
            Self::Themes => ".dudeltheme files",
            Self::Brushes => ".dudelbrush files and .dudelbrushes packs",
            Self::Palettes => ".dudelpalette files",
            Self::Timelapses => "Timelapse recordings, one folder of PNG frames per canvas",
        }
    }
    fn index(self) -> usize {
        Self::ALL.iter().position(|k| *k == self).unwrap()
    }
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
pub struct Locations {
    /// Parent of every default subfolder; None uses Documents/Dudel.
    #[serde(default)]
    pub library: Option<PathBuf>,
    /// Per-kind overrides by `Kind::index`; None uses `library/<Kind>`.
    #[serde(default, deserialize_with = "kind_paths")]
    pub custom: [Option<PathBuf>; KINDS],
    /// Save dialogs for an already saved project start in its own folder.
    #[serde(default = "yes")]
    pub beside_project: bool,
}
fn yes() -> bool {
    true
}
/// Settings written before a kind existed list fewer folders.
fn kind_paths<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<[Option<PathBuf>; KINDS], D::Error> {
    let mut paths = Vec::<Option<PathBuf>>::deserialize(deserializer)?.into_iter();
    Ok(std::array::from_fn(|_| paths.next().flatten()))
}
impl Default for Locations {
    fn default() -> Self {
        Self {
            library: None,
            custom: Default::default(),
            beside_project: true,
        }
    }
}
impl Locations {
    pub fn library(&self) -> PathBuf {
        self.library.clone().unwrap_or_else(default_library)
    }
    pub fn get(&self, kind: Kind) -> PathBuf {
        self.custom[kind.index()]
            .clone()
            .unwrap_or_else(|| self.library().join(kind.name()))
    }
    pub fn is_custom(&self, kind: Kind) -> bool {
        self.custom[kind.index()].is_some()
    }
    pub fn set(&mut self, kind: Kind, path: Option<PathBuf>) {
        self.custom[kind.index()] = path;
    }
    /// The folder for `kind`, created if missing; None when it cannot be made.
    pub fn ensure(&self, kind: Kind) -> Option<PathBuf> {
        let path = self.get(kind);
        std::fs::create_dir_all(&path).ok()?;
        Some(path)
    }
    /// Drop paths that are not absolute, so a hand-edited settings file
    /// cannot point dialogs at a folder relative to the working directory.
    pub fn sanitize(&mut self) {
        let absolute = |p: &Option<PathBuf>| p.as_ref().is_some_and(|p| p.is_absolute());
        if !absolute(&self.library) {
            self.library = None;
        }
        for custom in &mut self.custom {
            if !absolute(custom) {
                *custom = None;
            }
        }
    }
}

fn default_library() -> PathBuf {
    directories::UserDirs::new()
        .and_then(|dirs| {
            dirs.document_dir()
                .map(Path::to_path_buf)
                .or_else(|| Some(dirs.home_dir().to_path_buf()))
        })
        .unwrap_or_else(std::env::temp_dir)
        .join("Dudel")
}

/// Settings, recovery snapshots and extensions.
pub fn app_data() -> Option<PathBuf> {
    directories::ProjectDirs::from("org", "Dudel", "Dudel").map(|d| d.data_local_dir().to_owned())
}

/// Total bytes of regular files under `path`, bounded so a huge tree
/// cannot stall the caller.
pub fn folder_size(path: &Path) -> u64 {
    let mut total = 0;
    let mut stack = vec![path.to_owned()];
    let mut visited = 0;
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            visited += 1;
            if visited > 20_000 {
                return total;
            }
            match entry.file_type() {
                Ok(kind) if kind.is_dir() => stack.push(entry.path()),
                Ok(kind) if kind.is_file() => {
                    total += entry.metadata().map_or(0, |m| m.len());
                }
                _ => {}
            }
        }
    }
    total
}

pub fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

/// Show a folder in the system file manager.
pub fn reveal(path: &Path) -> Result<(), String> {
    #[cfg(windows)]
    let program = "explorer";
    #[cfg(target_os = "macos")]
    let program = "open";
    #[cfg(all(unix, not(target_os = "macos")))]
    let program = "xdg-open";
    std::process::Command::new(program)
        .arg(path)
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("Could not open {}: {e}", path.display()))
}

impl Dudel {
    /// Start `dialog` in the folder for `kind`. Project dialogs start beside
    /// the open project when that option is on.
    pub(crate) fn dialog_in(
        &self,
        kind: Kind,
        dialog: rfd::AsyncFileDialog,
    ) -> rfd::AsyncFileDialog {
        if kind == Kind::Projects
            && self.locations.beside_project
            && let Some(folder) = self.path.as_deref().and_then(Path::parent)
            && folder.is_dir()
        {
            return dialog.set_directory(folder);
        }
        // Tests never create folders in the user's documents.
        if cfg!(test) {
            return dialog;
        }
        match self.locations.ensure(kind) {
            Some(folder) => dialog.set_directory(folder),
            None => dialog,
        }
    }
    pub(crate) fn storage_window(&mut self, ctx: &egui::Context) {
        if !self.storage_window {
            return;
        }
        let mut open = true;
        egui::Window::new("Storage & folders")
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_width(560.0)
            .show(ctx, |ui| {
                ui.label(
                    egui::RichText::new(
                        "File dialogs start in these folders. Folders are created when first used.",
                    )
                    .weak(),
                );
                ui.add_space(4.0);
                let library = self.locations.library();
                egui::Grid::new("storage library")
                    .num_columns(2)
                    .spacing([10.0, 6.0])
                    .show(ui, |ui| {
                        ui.strong("Dudel folder");
                        ui.horizontal(|ui| {
                            path_label(ui, &library);
                            if ui.button("Change…").clicked() {
                                let dialog = rfd::AsyncFileDialog::new()
                                    .set_title("Choose the Dudel folder")
                                    .set_directory(&library);
                                self.ask_folder(dialog, |app, folder| {
                                    app.locations.library = Some(folder);
                                    app.status = "Dudel folder changed".into();
                                });
                            }
                            if ui.button("Open").clicked() {
                                self.reveal_folder(&library);
                            }
                            if self.locations.library.is_some()
                                && ui.button("Default").on_hover_text(default_library().display().to_string()).clicked()
                            {
                                self.locations.library = None;
                            }
                        });
                        ui.end_row();
                    });
                ui.separator();
                egui::Grid::new("storage kinds")
                    .num_columns(2)
                    .spacing([10.0, 6.0])
                    .striped(true)
                    .show(ui, |ui| {
                        for kind in Kind::ALL {
                            ui.label(kind.name()).on_hover_text(kind.hint());
                            ui.horizontal(|ui| {
                                let path = self.locations.get(kind);
                                path_label(ui, &path);
                                if ui.button("Change…").clicked() {
                                    let dialog = rfd::AsyncFileDialog::new()
                                        .set_title(format!("Choose the {} folder", kind.name()));
                                    let dialog = if path.is_dir() {
                                        dialog.set_directory(&path)
                                    } else {
                                        dialog
                                    };
                                    self.ask_folder(dialog, move |app, folder| {
                                        app.locations.set(kind, Some(folder));
                                        app.status = format!("{} folder changed", kind.name());
                                    });
                                }
                                if ui.button("Open").clicked() {
                                    match self.locations.ensure(kind) {
                                        Some(folder) => self.reveal_folder(&folder),
                                        None => {
                                            self.status =
                                                format!("Could not create {}", path.display())
                                        }
                                    }
                                }
                                if self.locations.is_custom(kind)
                                    && ui.button("Default").on_hover_text("Use the subfolder of the Dudel folder").clicked()
                                {
                                    self.locations.set(kind, None);
                                }
                            });
                            ui.end_row();
                        }
                    });
                ui.add_space(4.0);
                ui.checkbox(
                    &mut self.locations.beside_project,
                    "Save and open beside the current project",
                )
                .on_hover_text("When a saved project is open, project dialogs start in its folder instead of Projects");
                ui.separator();
                ui.strong("App data");
                ui.label(
                    egui::RichText::new(
                        "Settings, crash recovery snapshots and installed extensions. Managed by Dudel.",
                    )
                    .weak(),
                );
                if let Some(data) = app_data() {
                    ui.horizontal(|ui| {
                        path_label(ui, &data);
                        if ui.button("Open").clicked() {
                            self.reveal_folder(&data);
                        }
                    });
                    // Sizes are measured on request; walking folders each frame would stall.
                    let id = ui.make_persistent_id("storage sizes");
                    let sizes: Option<Vec<(String, u64)>> = ui.ctx().data(|d| d.get_temp(id));
                    match sizes {
                        Some(sizes) => {
                            for (name, bytes) in sizes {
                                ui.label(format!("{name}: {}", human_bytes(bytes)));
                            }
                        }
                        None => {
                            if ui.button("Measure disk use").clicked() {
                                let sizes: Vec<(String, u64)> = ["recovery", "extensions"]
                                    .into_iter()
                                    .map(|name| (name.to_owned(), folder_size(&data.join(name))))
                                    .chain(std::iter::once((
                                        "Dudel folder".to_owned(),
                                        folder_size(&library),
                                    )))
                                    .collect();
                                ui.ctx().data_mut(|d| d.insert_temp(id, sizes));
                            }
                        }
                    }
                }
            });
        if !open {
            self.storage_window = false;
        }
    }
    fn reveal_folder(&mut self, path: &Path) {
        if let Err(error) = reveal(path) {
            self.status = error;
        }
    }
}

fn path_label(ui: &mut egui::Ui, path: &Path) {
    let text = path.display().to_string();
    ui.add(egui::Label::new(egui::RichText::new(&text).monospace().small()).truncate())
        .on_hover_text(text);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn kinds_default_under_the_library_and_overrides_survive_a_round_trip() {
        let root = tempfile::tempdir().unwrap();
        let mut locations = Locations {
            library: Some(root.path().to_owned()),
            ..Default::default()
        };
        assert_eq!(locations.get(Kind::Themes), root.path().join("Themes"));
        let custom = root.path().join("elsewhere");
        locations.set(Kind::Exports, Some(custom.clone()));
        assert_eq!(locations.get(Kind::Exports), custom);
        assert_eq!(locations.ensure(Kind::Exports), Some(custom.clone()));
        assert!(custom.is_dir());
        let json = serde_json::to_string(&locations).unwrap();
        let mut restored: Locations = serde_json::from_str(&json).unwrap();
        restored.sanitize();
        assert_eq!(restored, locations);
        // Older settings without this section get defaults.
        let legacy: Locations = serde_json::from_str("{}").unwrap();
        assert_eq!(legacy, Locations::default());
        let six: Locations =
            serde_json::from_str(r#"{"custom":[null,"/art",null,null,null,null]}"#).unwrap();
        assert_eq!(six.get(Kind::Exports), PathBuf::from("/art"));
        assert!(!six.is_custom(Kind::Timelapses));
        assert!(legacy.beside_project);
        // Relative paths are never trusted.
        let mut relative = locations.clone();
        relative.library = Some("relative".into());
        relative.set(Kind::Themes, Some("x".into()));
        relative.sanitize();
        assert!(relative.library.is_none() && !relative.is_custom(Kind::Themes));
        std::fs::write(custom.join("a.png"), [0u8; 1500]).unwrap();
        assert_eq!(folder_size(root.path()), 1500);
        assert_eq!(human_bytes(1536), "1.5 KB");
    }
}
