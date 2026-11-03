use crate::{Dudel, read_bounded, storage};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
const PACKAGE_LIMIT: u64 = 5 * 1024 * 1024;

/// The SDK and installer use the same authoritative validation path.
pub(crate) fn check_package(path: &Path) -> Result<(), String> {
    let source = read_bounded(path, PACKAGE_LIMIT)?;
    let package: Package = serde_json::from_str(&source).map_err(|e| e.to_string())?;
    package.validate()
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Package {
    pub format: String,
    pub version: u32,
    pub api: u32,
    pub id: String,
    pub name: String,
    pub author: String,
    pub release: String,
    pub description: String,
    pub capabilities: Vec<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub source: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub wasm: Vec<u8>,
    #[serde(default)]
    pub contributes: crate::extension_api::Contributions,
}
impl Package {
    pub fn validate(&self) -> Result<(), String> {
        let modern = self.format == "dudel-extension" && self.version == 2 && self.api == 2;
        if !modern
            && (!matches!(
                self.format.as_str(),
                "dudel-rhai-extension" | "dudel-wasm-extension"
            ) || self.version != 1
                || self.api != 1)
        {
            return Err(
                "Incompatible extension format/API (supported: legacy filters 1, contributions 2)"
                    .into(),
            );
        }
        if self.id.is_empty()
            || self.id.len() > 80
            || !self
                .id
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        {
            return Err("Extension ID must contain lowercase letters, digits or hyphens".into());
        }
        for (value, max) in [
            (&self.name, 100),
            (&self.author, 100),
            (&self.release, 40),
            (&self.description, 1000),
        ] {
            if value.trim().is_empty() || value.len() > max || value.chars().any(char::is_control) {
                return Err("Invalid extension metadata".into());
            }
        }
        if modern {
            if !self.source.is_empty() || !self.wasm.is_empty() {
                return Err("API 2 programs belong in contributed commands".into());
            }
            return self.contributes.validate(&self.capabilities);
        }
        if !self.contributes.commands.is_empty() || !self.contributes.panels.is_empty() {
            return Err("Contributions require API 2".into());
        }
        if self.capabilities != ["pixels.filter"] {
            return Err("Only the pixels.filter capability is supported".into());
        }
        if self.format == "dudel-wasm-extension" {
            if !self.source.is_empty() {
                return Err("WASM packages cannot contain Rhai source".into());
            }
            return crate::wasm_filters::validate(&self.wasm);
        }
        if !self.wasm.is_empty() {
            return Err("Rhai packages cannot contain a WASM module".into());
        }
        if self.source.len() > 32768 {
            return Err("Extension script exceeds 32 KiB".into());
        }
        let mut engine = rhai::Engine::new_raw();
        engine.set_max_expr_depths(32, 16);
        let ast = engine
            .compile(&self.source)
            .map_err(|e| format!("Invalid script: {e}"))?;
        if !ast
            .iter_functions()
            .any(|function| function.name == "filter" && function.params.len() == 4)
        {
            return Err("Extension must define filter(r, g, b, a)".into());
        }
        Ok(())
    }
    pub(crate) fn program(&self) -> crate::plugins::FilterProgram {
        if self.format == "dudel-wasm-extension" {
            crate::plugins::FilterProgram::Wasm(self.wasm.clone())
        } else {
            crate::plugins::FilterProgram::Rhai(self.source.clone())
        }
    }
}
/// What a declared capability lets a package do, as the install prompt and
/// the manager describe it.
pub(crate) fn capability_text(capability: &str) -> &'static str {
    match capability {
        "brushes" => "Add brushes",
        "tools" => "Add presets of Dudel's tools",
        "tools.script" => {
            "Add tools that run their own code while you draw, to decide where the brush goes"
        }
        "commands" => "Add commands for undo, redo, new layer, selections, transform and the view",
        "pixels.filter" => "Change the active layer's pixels when you run one of its filters",
        "ui.panels" => "Show panels with its buttons and brush, color and guide controls",
        _ => "Unknown capability",
    }
}

/// A capability's short name for the manager's one-line summary.
fn capability_name(capability: &str) -> &str {
    match capability {
        "brushes" => "brushes",
        "tools" => "tool presets",
        "tools.script" => "scripted tools",
        "commands" => "commands",
        "pixels.filter" => "layer filters",
        "ui.panels" => "panels",
        other => other,
    }
}

/// The install prompt: who made the package and what it may do. A
/// replacement marks capabilities the installed release did not have.
fn install_prompt(package: &Package, installed: Option<&Package>) -> String {
    let mut text = match installed {
        Some(old) => format!(
            "Replace ‘{}’ {} with release {} by {}?",
            old.name, old.release, package.release, package.author
        ),
        None => format!(
            "Install ‘{}’ {} by {}?",
            package.name, package.release, package.author
        ),
    };
    text.push_str(&format!(
        "

{}

It can:",
        package.description
    ));
    for capability in &package.capabilities {
        let new = installed.is_some_and(|old| !old.capabilities.contains(capability));
        text.push_str(&format!(
            "
• {}{}",
            capability_text(capability),
            if new { " (new)" } else { "" }
        ));
    }
    text.push_str(
        "

Extensions run inside Dudel without access to your files or the network.",
    );
    text
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Installed {
    pub package: Package,
    pub enabled: bool,
}

fn directory() -> Result<PathBuf, String> {
    let dirs = directories::ProjectDirs::from("org", "Dudel", "Dudel")
        .ok_or("Local application directory unavailable")?;
    let path = dirs.data_local_dir().join("extensions");
    std::fs::create_dir_all(&path).map_err(|e| e.to_string())?;
    Ok(path)
}
fn save(root: &Path, extension: &Installed) -> Result<(), String> {
    extension.package.validate()?;
    let data = serde_json::to_vec(extension).map_err(|e| e.to_string())?;
    storage::atomic_write(
        &root.join(format!("{}.dudelinstalled", extension.package.id)),
        &data,
    )
}
impl Dudel {
    pub(crate) fn load_extensions(&mut self) {
        self.extensions.clear();
        self.extension_errors.clear();
        let result: Result<_, String> = (|| {
            let root = directory()?;
            let entries = std::fs::read_dir(root).map_err(|e| e.to_string())?;
            for entry in entries
                .filter_map(Result::ok)
                .filter(|e| {
                    e.path()
                        .extension()
                        .is_some_and(|ext| ext == "dudelinstalled")
                })
                .take(65)
            {
                if self.extensions.len() >= 64 {
                    self.extension_errors
                        .push("Extension limit reached (64)".into());
                    break;
                }
                let path = entry.path();
                let decoded = (|| {
                    let source = read_bounded(&path, PACKAGE_LIMIT)?;
                    let installed: Installed =
                        serde_json::from_str(&source).map_err(|e| e.to_string())?;
                    installed.package.validate()?;
                    if path.file_stem().and_then(|s| s.to_str())
                        != Some(installed.package.id.as_str())
                    {
                        return Err("Installed ID differs from filename".into());
                    }
                    Ok::<_, String>(installed)
                })();
                match decoded {
                    Ok(extension) => self.extensions.push(extension),
                    Err(error) => self
                        .extension_errors
                        .push(format!("{}: {error}", path.display())),
                }
            }
            self.extensions
                .sort_by(|a, b| a.package.name.cmp(&b.package.name));
            Ok::<_, String>(())
        })();
        if let Err(error) = result {
            self.extension_errors.push(error);
        }
    }
    fn install_extension(&mut self) {
        let dialog = rfd::AsyncFileDialog::new().add_filter("Dudel extension", &["dudelplugin"]);
        self.ask_open_file(dialog, Self::install_extension_from);
    }
    fn install_extension_from(&mut self, path: PathBuf) {
        let result: Result<_, String> = (|| {
            let source = read_bounded(&path, PACKAGE_LIMIT)?;
            let package: Package = serde_json::from_str(&source).map_err(|e| e.to_string())?;
            package.validate()?;
            let existing = self.extensions.iter().find(|e| e.package.id == package.id);
            if existing.is_none() && self.extensions.len() >= 64 {
                return Err("Extension limit reached (64)".into());
            }
            let text = install_prompt(&package, existing.map(|e| &e.package));
            Ok((package, existing.map(|extension| extension.enabled), text))
        })();
        match result {
            Ok((package, enabled, text)) => {
                let (title, action) = match enabled {
                    Some(_) => ("Replace installed extension", "Replace"),
                    None => ("Install extension", "Install"),
                };
                self.confirm(title, text, action, move |app| {
                    app.save_extension(package, enabled.unwrap_or(true))
                });
            }
            Err(error) => self.status = format!("Extension installation: {error}"),
        }
    }
    fn save_extension(&mut self, package: Package, enabled: bool) {
        match directory().and_then(|root| save(&root, &Installed { package, enabled })) {
            Ok(()) => {
                self.load_extensions();
                self.status = "Extension installed".into();
            }
            Err(error) => self.status = format!("Extension installation: {error}"),
        }
    }
    pub(crate) fn extension_manager_ui(&mut self, ctx: &eframe::egui::Context) {
        let mut open = self.extension_manager;
        eframe::egui::Window::new("Extensions").open(&mut open).default_width(480.0).default_height(480.0).vscroll(true).show(ctx, |ui| {
            ui.heading("Your extensions");
            ui.label("Brushes, tool presets, commands, panels and filters. Enabled extensions contribute tools to the rail; their panels open here or from the Filter menu.");
            ui.horizontal(|ui| { if ui.button("Install…").clicked() { self.install_extension(); }
if ui.button("Refresh").clicked() { self.load_extensions(); } });
            if self.extensions.is_empty() { ui.label("No installed extensions. Install a .dudelplugin package."); }
            let mut run = None;
            let mut uninstall = None;
            for index in 0..self.extensions.len() {
                ui.separator();
                let package_id = self.extensions[index].package.id.clone();
                ui.push_id(&package_id, |ui| {
                let extension = &self.extensions[index];
                ui.strong(&extension.package.name);
                ui.label(format!("{} · {}", extension.package.release, extension.package.author));
                ui.label(&extension.package.description);
                let short: Vec<_> = extension.package.capabilities.iter().map(|c| capability_name(c)).collect();
                let full: Vec<_> = extension.package.capabilities.iter().map(|c| format!("• {}", capability_text(c))).collect();
                ui.weak(format!("API {} · can use {}", extension.package.api, short.join(", "))).on_hover_text(full.join("\n"));
                let mut enabled = extension.enabled;
                ui.horizontal_wrapped(|ui| {
                    if ui.checkbox(&mut enabled, "Enabled").changed() {
                        let mut updated = self.extensions[index].clone(); updated.enabled = enabled;
                        match directory().and_then(|root| save(&root, &updated)) { Ok(()) => self.extensions[index] = updated, Err(error) => self.status = error }
                    }
                    if self.extensions[index].package.api == 1 && ui.add_enabled(enabled && self.filter_job.is_none(), eframe::egui::Button::new("Run filter")).clicked() { run = Some(self.extensions[index].package.program()); }
                    if ui.button("Export package...").clicked() {
                        let package = self.extensions[index].package.clone();
                        let dialog = rfd::AsyncFileDialog::new().add_filter("Dudel extension", &["dudelplugin"]).set_file_name(format!("{}-{}.dudelplugin", package.id, package.release));
                        self.ask_save_file(dialog, move |app, path| {
                            let result = serde_json::to_vec_pretty(&package).map_err(|e| e.to_string()).and_then(|bytes| crate::storage::atomic_write(&path,&bytes));
                            app.status = match result { Ok(()) => "Shareable plugin package exported".into(), Err(e) => format!("Plugin export: {e}") };
                        });
                    }
                    if ui.button("Uninstall…").clicked() { uninstall = Some(index); }
                });
                self.extension_contributions_ui(ui, Some(&package_id));
                });
            }
            if let Some(program) = run { self.start_filter(program); }
            if let Some(index) = uninstall {
                let package = &self.extensions[index].package;
                let (id, text) = (package.id.clone(), format!("Remove ‘{}’ from this installation? Artwork is unaffected.", package.name));
                self.confirm("Uninstall extension", text, "Uninstall", move |app| {
                    let result = directory().and_then(|root| std::fs::remove_file(root.join(format!("{id}.dudelinstalled"))).map_err(|e| e.to_string()));
                    match result { Ok(()) => { app.cancel_filter(); app.load_extensions(); app.status = "Extension uninstalled".into(); }, Err(error) => app.status = format!("Uninstall: {error}") }
                });
            }
            for error in &self.extension_errors { ui.colored_label(eframe::egui::Color32::from_rgb(235,170,79), error); }
        });
        self.extension_manager = open;
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn wasm_packages_install_restore_and_dispatch_to_the_wasm_host() {
        let package: Package =
            serde_json::from_str(include_str!("../plugins/example-wasm-invert.dudelplugin"))
                .unwrap();
        package.validate().unwrap();
        let root = tempfile::tempdir().unwrap();
        save(
            root.path(),
            &Installed {
                package: package.clone(),
                enabled: true,
            },
        )
        .unwrap();
        let restored: Installed = serde_json::from_slice(
            &std::fs::read(root.path().join("dudel-wasm-invert.dudelinstalled")).unwrap(),
        )
        .unwrap();
        assert!(restored.enabled);
        let crate::plugins::FilterProgram::Wasm(bytes) = restored.package.program() else {
            panic!("Wrong runtime");
        };
        let mut doc = crate::document::Document::new(1, 1);
        doc.layers[0].pixels = vec![10, 30, 200, 128].into();
        crate::wasm_filters::apply(
            &mut doc.layers[0],
            &bytes,
            None,
            1,
            &crate::plugins::FilterControl::default(),
        )
        .unwrap();
        assert_eq!(doc.layers[0].pixels, vec![245, 225, 55, 128]);
        let mut bad = package;
        bad.source = "fn filter(r,g,b,a) { [r,g,b,a] }".into();
        assert!(bad.validate().is_err());
        bad.source.clear();
        bad.wasm = wat::parse_str("(module)").unwrap();
        assert!(bad.validate().is_err());
    }
    #[test]
    fn sdk_example_validates_and_runs_through_the_filter_host() {
        let package: Package =
            serde_json::from_str(include_str!("../plugins/example-invert.dudelplugin")).unwrap();
        package.validate().unwrap();
        let mut document = crate::document::Document::new(1, 1);
        document.layers[0].pixels = vec![10, 30, 200, 128].into();
        crate::plugins::apply_filter(&mut document.layers[0], &package.source).unwrap();
        assert_eq!(document.layers[0].pixels, vec![245, 225, 55, 128]);
    }
    #[test]
    fn installing_asks_first_and_lists_what_the_package_can_do() {
        let path =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("plugins/example-studio-kit.dudelplugin");
        let mut app = Dudel::default();
        app.install_extension_from(path.clone());
        assert!(
            app.dialogs.open(),
            "nothing is installed before the owner agrees"
        );
        assert!(app.extensions.is_empty());
        let package: Package =
            serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        let text = install_prompt(&package, None);
        assert!(text.starts_with("Install ‘"));
        for capability in &package.capabilities {
            assert!(text.contains(capability_text(capability)), "{capability}");
        }
        assert!(!text.contains("(new)"));
        let mut old = package.clone();
        old.capabilities.retain(|c| c != "ui.panels");
        let text = install_prompt(&package, Some(&old));
        assert!(text.starts_with("Replace ‘"));
        assert!(text.contains(&format!("{} (new)", capability_text("ui.panels"))));
        assert_eq!(text.matches("(new)").count(), 1);
    }
    fn package() -> Package {
        Package {
            format: "dudel-rhai-extension".into(),
            version: 1,
            api: 1,
            id: "sample-invert".into(),
            name: "Sample invert".into(),
            author: "Dudel".into(),
            release: "1.0.0".into(),
            description: "Invert active-layer colors".into(),
            capabilities: vec!["pixels.filter".into()],
            source: include_str!("../plugins/invert.rhai").into(),
            wasm: vec![],
            contributes: Default::default(),
        }
    }
    #[test]
    fn packages_validate_compatibility_and_install_atomically() {
        let root = tempfile::tempdir().unwrap();
        let mut installed = Installed {
            package: package(),
            enabled: false,
        };
        save(root.path(), &installed).unwrap();
        let path = root.path().join("sample-invert.dudelinstalled");
        let original = std::fs::read(&path).unwrap();
        let decoded: Installed = serde_json::from_slice(&original).unwrap();
        assert!(!decoded.enabled);
        decoded.package.validate().unwrap();
        installed.package.api = 2;
        assert!(save(root.path(), &installed).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), original);
        installed.package = package();
        installed.package.id = "../escape".into();
        assert!(installed.package.validate().is_err());
        installed.package = package();
        installed.package.capabilities.push("filesystem".into());
        assert!(installed.package.validate().is_err());
        installed.package = package();
        installed.package.source = "fn other() { 0 }".into();
        assert!(installed.package.validate().is_err());
    }
}
