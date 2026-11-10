//! Named workspaces: the panel arrangement saved under a name, to switch
//! between setups (sketching with the library open, painting with big color,
//! a bare canvas for a small screen). A workspace remembers which panels show,
//! their widths and heights, and the Color or Navigator tab.
use crate::{Dudel, egui, preferences::LayoutPrefs};
use serde::{Deserialize, Serialize};

pub(crate) const MAX: usize = 16;
const NAME_LIMIT: usize = 64;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Workspace {
    pub name: String,
    #[serde(default = "shown")]
    pub panels: bool,
    #[serde(default)]
    pub tool_panel: Option<bool>,
    #[serde(default)]
    pub inspector: Option<bool>,
    /// Tool settings and right panel widths; None keeps the automatic width.
    #[serde(default)]
    pub widths: [Option<f32>; 2],
    #[serde(default)]
    pub layout: LayoutPrefs,
}
fn shown() -> bool {
    true
}

impl Workspace {
    fn valid(&self) -> bool {
        let size =
            |v: Option<f32>| v.is_none_or(|v| v.is_finite() && (0.0..=10_000.0).contains(&v));
        !self.name.trim().is_empty()
            && self.name.chars().count() <= NAME_LIMIT
            && !self.name.chars().any(char::is_control)
            && self.widths.into_iter().all(size)
            && [
                self.layout.library_height,
                self.layout.color_height,
                self.layout.navigator_height,
                self.layout.layers_height,
            ]
            .into_iter()
            .all(size)
    }
}

/// Keep well-formed workspaces with distinct names, at most `MAX`.
pub(crate) fn sanitize(workspaces: &mut Vec<Workspace>) {
    let mut seen = std::collections::HashSet::new();
    workspaces.retain(|w| w.valid() && seen.insert(w.name.trim().to_lowercase()));
    workspaces.truncate(MAX);
}

impl Dudel {
    /// The current arrangement under `name`.
    pub(crate) fn current_workspace(&self, name: &str) -> Workspace {
        Workspace {
            name: name.trim().to_owned(),
            panels: self.show_panels,
            tool_panel: self.tool_panel_override,
            inspector: self.inspector_override,
            widths: self.sidebar_widths,
            layout: LayoutPrefs {
                library_height: self.library_height,
                color_height: self.color_height,
                navigator_height: self.navigator_height,
                layers_height: self.layers_height,
                navigator_tab: self.inspector_navigator,
            },
        }
    }

    /// Save the current arrangement, replacing a workspace of the same name.
    pub(crate) fn save_workspace(&mut self, name: &str) -> Result<(), String> {
        let workspace = self.current_workspace(name);
        if !workspace.valid() {
            return Err(format!("Name the workspace (1–{NAME_LIMIT} characters)"));
        }
        let key = workspace.name.to_lowercase();
        if let Some(existing) = self
            .workspaces
            .iter_mut()
            .find(|w| w.name.to_lowercase() == key)
        {
            *existing = workspace;
        } else if self.workspaces.len() >= MAX {
            return Err(format!("Up to {MAX} workspaces; delete one first"));
        } else {
            self.workspaces.push(workspace);
        }
        self.status = format!("Saved workspace “{}”", name.trim());
        Ok(())
    }

    pub(crate) fn apply_workspace(&mut self, index: usize) {
        let Some(w) = self.workspaces.get(index).cloned() else {
            return;
        };
        self.show_panels = w.panels;
        self.tool_panel_override = w.tool_panel;
        self.inspector_override = w.inspector;
        self.sidebar_widths = w.widths;
        self.library_height = w.layout.library_height;
        self.color_height = w.layout.color_height;
        self.navigator_height = w.layout.navigator_height;
        self.layers_height = w.layout.layers_height;
        self.inspector_navigator = w.layout.navigator_tab;
        self.status = format!("Workspace “{}”", w.name);
    }

    /// View > Workspaces.
    pub(crate) fn workspaces_menu(&mut self, ui: &mut egui::Ui) {
        if self.workspaces.is_empty() {
            ui.weak("No saved workspaces yet");
        }
        let mut apply = None;
        for (index, workspace) in self.workspaces.iter().enumerate() {
            if ui.button(&workspace.name).clicked() {
                apply = Some(index);
                ui.close();
            }
        }
        if let Some(index) = apply {
            self.apply_workspace(index);
        }
        ui.separator();
        ui.label("Save the current layout as");
        let mut save = false;
        ui.horizontal(|ui| {
            let field = ui.add(
                egui::TextEdit::singleline(&mut self.workspace_name)
                    .hint_text("Name")
                    .char_limit(NAME_LIMIT)
                    .desired_width(150.0),
            );
            save |= field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            save |= ui
                .add_enabled(
                    !self.workspace_name.trim().is_empty(),
                    egui::Button::new("Save"),
                )
                .on_hover_text("A workspace of the same name is replaced")
                .clicked();
        });
        if save {
            let name = std::mem::take(&mut self.workspace_name);
            if let Err(error) = self.save_workspace(&name) {
                self.status = error;
                self.workspace_name = name;
            } else {
                ui.close();
            }
        }
        if !self.workspaces.is_empty() {
            ui.menu_button("Delete", |ui| {
                let mut remove = None;
                for (index, workspace) in self.workspaces.iter().enumerate() {
                    if ui.button(&workspace.name).clicked() {
                        remove = Some(index);
                        ui.close();
                    }
                }
                if let Some(index) = remove {
                    let removed = self.workspaces.remove(index);
                    self.status = format!("Deleted workspace “{}”", removed.name);
                }
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspaces_save_switch_replace_and_survive_preferences() {
        let mut app = Dudel {
            sidebar_widths: [Some(300.0), Some(340.0)],
            color_height: Some(260.0),
            inspector_navigator: true,
            ..Default::default()
        };
        app.save_workspace("Painting").unwrap();
        app.show_panels = true;
        app.tool_panel_override = Some(false);
        app.inspector_override = Some(false);
        app.save_workspace(" Bare canvas ").unwrap();
        assert_eq!(app.workspaces[1].name, "Bare canvas");
        // Switching restores every part of the arrangement.
        app.apply_workspace(0);
        assert_eq!(app.sidebar_widths, [Some(300.0), Some(340.0)]);
        assert_eq!(app.color_height, Some(260.0));
        assert!(app.inspector_navigator);
        assert_ne!(app.tool_panel_override, Some(false));
        app.apply_workspace(1);
        assert_eq!(app.tool_panel_override, Some(false));
        // Same name (any case) replaces; empty names are refused.
        app.sidebar_widths[0] = Some(280.0);
        app.save_workspace("painting").unwrap();
        assert_eq!(app.workspaces.len(), 2);
        assert_eq!(app.workspaces[0].widths[0], Some(280.0));
        assert!(app.save_workspace("  ").is_err());
        // Preferences keep them; damaged or duplicate entries are dropped.
        let mut prefs =
            crate::preferences::Preferences::decode(&app.encode_preferences().unwrap()).unwrap();
        assert!(prefs.workspaces == app.workspaces);
        let mut bad = prefs.workspaces[0].clone();
        bad.widths[0] = Some(-5.0);
        prefs.workspaces.push(bad);
        prefs.workspaces.push(prefs.workspaces[1].clone());
        let json = serde_json::to_string(&prefs).unwrap();
        let loaded = crate::preferences::Preferences::decode(&json).unwrap();
        assert!(loaded.workspaces == app.workspaces);
    }

    #[test]
    fn the_workspace_list_is_bounded() {
        let mut app = Dudel::default();
        for i in 0..MAX {
            app.save_workspace(&format!("Layout {i}")).unwrap();
        }
        assert!(app.save_workspace("One more").is_err());
        assert!(app.save_workspace("Layout 3").is_ok());
    }
}
