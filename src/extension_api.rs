//! API 2 contributions. Packages own their commands, brushes, tool presets and
//! panels. All actions enter the same guarded application paths as native UI.
use crate::{Dudel, egui, tools::Tool};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Contributions {
    #[serde(default)]
    pub commands: Vec<Command>,
    #[serde(default)]
    pub panels: Vec<Panel>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Command {
    pub id: String,
    pub label: String,
    pub action: Action,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Action {
    Brush { brush: Box<crate::brush::Brush> },
    Tool { tool: ToolPreset },
    Filter { source: String },
    WasmFilter { wasm: Vec<u8> },
    Host { command: HostCommand },
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ToolPreset {
    Brush {
        brush: Box<crate::brush::Brush>,
        erase: bool,
    },
    Shape {
        options: crate::lasso_paint::Options,
    },
    Fill {
        tolerance: u8,
    },
    Selection {
        shape: SelectionShape,
    },
    Pointer,
    Picker,
    Move,
}
#[derive(Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SelectionShape {
    Rectangle,
    Lasso,
    Polygon,
    Wand,
}
#[derive(Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HostCommand {
    Undo,
    Redo,
    NewLayer,
    Deselect,
    Transform,
    FitCanvas,
    FlipView,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Panel {
    pub id: String,
    pub title: String,
    pub widgets: Vec<Widget>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Widget {
    Label { text: String },
    Button { command: String },
    BrushSize,
    Opacity,
    Color,
    Snapping,
    GuideLock,
    Separator,
}
pub(crate) fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 80
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}
fn text(value: &str, max: usize) -> Result<(), String> {
    if value.trim().is_empty() || value.len() > max || value.chars().any(char::is_control) {
        Err("Invalid contribution label".into())
    } else {
        Ok(())
    }
}
impl Contributions {
    pub(crate) fn validate(&self, capabilities: &[String]) -> Result<(), String> {
        let caps: BTreeSet<&str> = capabilities.iter().map(String::as_str).collect();
        if caps.len() != capabilities.len()
            || caps.iter().any(|c| {
                !["brushes", "tools", "commands", "pixels.filter", "ui.panels"].contains(c)
            })
        {
            return Err("Unknown or duplicate API 2 capability".into());
        }
        if self.commands.len() > 64
            || self.panels.len() > 8
            || (self.commands.is_empty() && self.panels.is_empty())
        {
            return Err("A plugin needs contributions (maximum 64 commands and 8 panels)".into());
        }
        let mut ids = BTreeSet::new();
        for command in &self.commands {
            if !valid_id(&command.id) || !ids.insert(&command.id) {
                return Err("Invalid or duplicate command ID".into());
            }
            text(&command.label, 100)?;
            let required = match &command.action {
                Action::Brush { brush } => {
                    brush.validate()?;
                    "brushes"
                }
                Action::Tool { tool } => {
                    if let ToolPreset::Brush { brush, .. } = tool {
                        brush.validate()?;
                    }
                    "tools"
                }
                Action::Filter { source } => {
                    validate_filter(source)?;
                    "pixels.filter"
                }
                Action::WasmFilter { wasm } => {
                    crate::wasm_filters::validate(wasm)?;
                    "pixels.filter"
                }
                Action::Host { .. } => "commands",
            };
            if !caps.contains(required) {
                return Err(format!(
                    "Command {} requires capability {required}",
                    command.id
                ));
            }
        }
        let mut panel_ids = BTreeSet::new();
        for panel in &self.panels {
            if !caps.contains("ui.panels")
                || !valid_id(&panel.id)
                || !panel_ids.insert(&panel.id)
                || panel.widgets.len() > 64
            {
                return Err(
                    "Invalid panel ID, widget count or missing ui.panels capability".into(),
                );
            }
            text(&panel.title, 100)?;
            for widget in &panel.widgets {
                match widget {
                    Widget::Label { text: label } => text(label, 1000)?,
                    Widget::Button { command } if !ids.contains(command) => {
                        return Err(format!("Panel references unknown command {command}"));
                    }
                    Widget::BrushSize | Widget::Opacity | Widget::Color
                        if !caps.contains("brushes") =>
                    {
                        return Err("Brush controls require the brushes capability".into());
                    }
                    Widget::Snapping | Widget::GuideLock if !caps.contains("tools") => {
                        return Err("Guide controls require the tools capability".into());
                    }
                    _ => {}
                }
            }
        }
        Ok(())
    }
}
pub(crate) fn validate_filter(source: &str) -> Result<(), String> {
    if source.len() > 32768 {
        return Err("Extension script exceeds 32 KiB".into());
    }
    let mut engine = rhai::Engine::new_raw();
    engine.set_max_expr_depths(32, 16);
    let ast = engine
        .compile(source)
        .map_err(|e| format!("Invalid script: {e}"))?;
    if !ast
        .iter_functions()
        .any(|f| f.name == "filter" && f.params.len() == 4)
    {
        return Err("Extension must define filter(r, g, b, a)".into());
    }
    Ok(())
}
impl Dudel {
    pub(crate) fn run_extension_command(&mut self, package: &str, command: &str) {
        let Some(action) = self
            .extensions
            .iter()
            .find(|e| e.enabled && e.package.id == package)
            .and_then(|e| {
                e.package
                    .contributes
                    .commands
                    .iter()
                    .find(|c| c.id == command)
            })
            .map(|c| c.action.clone())
        else {
            self.status = "Extension command is unavailable or disabled".into();
            return;
        };
        // Resolve again at activation, so disabled/replaced packages cannot keep
        // live callbacks or mutate a stale document through a retained panel.
        if self.unsaved_action.is_some() || self.pending_action.is_some() || self.load_job.is_some()
        {
            self.status = "Finish the current canvas action before running an extension".into();
            return;
        }
        match action {
            Action::Brush { brush } => self.use_extension_brush(*brush, false),
            Action::Tool { tool } => match tool {
                ToolPreset::Brush { brush, erase } => self.use_extension_brush(*brush, erase),
                ToolPreset::Shape { options } => {
                    self.select_tool(Tool::Lasso);
                    self.lasso = options;
                }
                ToolPreset::Fill { tolerance } => {
                    self.select_tool(Tool::Fill);
                    self.fill_tolerance = tolerance;
                }
                ToolPreset::Selection { shape } => {
                    self.select_tool(Tool::Select);
                    self.selection_drag = None;
                    self.selection_shape = match shape {
                        SelectionShape::Rectangle => crate::selection_ui::Shape::Rectangle,
                        SelectionShape::Lasso => crate::selection_ui::Shape::Lasso,
                        SelectionShape::Polygon => crate::selection_ui::Shape::Polygon,
                        SelectionShape::Wand => crate::selection_ui::Shape::Wand,
                    };
                }
                ToolPreset::Pointer => self.select_tool(Tool::Pointer),
                ToolPreset::Picker => self.select_tool(Tool::Picker),
                ToolPreset::Move => self.select_tool(Tool::Move),
            },
            Action::Filter { source } => {
                self.start_filter(crate::plugins::FilterProgram::Rhai(source))
            }
            Action::WasmFilter { wasm } => {
                self.start_filter(crate::plugins::FilterProgram::Wasm(wasm))
            }
            Action::Host { command } => match command {
                HostCommand::Undo => self.undo(),
                HostCommand::Redo => self.redo(),
                HostCommand::NewLayer => self.new_layer(),
                HostCommand::Deselect => self.set_selection(None),
                HostCommand::Transform => {
                    self.affine_settings.mirror = None;
                    self.toggle_free_transform();
                }
                HostCommand::FitCanvas => self.fit_view(),
                HostCommand::FlipView => {
                    self.mirror = !self.mirror;
                }
            },
        }
    }
    fn use_extension_brush(&mut self, brush: crate::brush::Brush, erase: bool) {
        self.select_tool(Tool::Brush);
        self.unswap_eraser();
        self.brush_group = brush.group();
        self.brush = brush;
        self.scratchpad.reset_stroke();
        if erase {
            self.eraser_brush = self.brush.clone();
            self.eraser_own = true;
            self.select_tool(Tool::Eraser);
        }
        self.status = "Extension brush selected; Save as new preset keeps a library copy".into();
    }
    pub(crate) fn extension_contributions_ui(
        &mut self,
        ui: &mut egui::Ui,
        package_id: Option<&str>,
    ) {
        let packages: Vec<_> = self
            .extensions
            .iter()
            .filter(|e| {
                e.enabled && e.package.api == 2 && package_id.is_none_or(|id| e.package.id == id)
            })
            .map(|e| {
                (
                    e.package.id.clone(),
                    e.package.name.clone(),
                    e.package.contributes.clone(),
                )
            })
            .collect();
        for (id, name, contributions) in packages {
            ui.push_id(&id, |ui| {
                if package_id.is_none() {
                    ui.strong(name);
                }
                ui.horizontal_wrapped(|ui| {
                    for command in contributions.commands {
                        if ui.button(&command.label).clicked() {
                            self.run_extension_command(&id, &command.id);
                        }
                    }
                    for panel in contributions.panels {
                        if ui.button(format!("{}…", panel.title)).clicked() {
                            self.extension_panels.insert(format!("{id}/{}", panel.id));
                        }
                    }
                });
            });
        }
    }
    pub(crate) fn extension_tools_ui(&mut self, ui: &mut egui::Ui, side: f32) {
        let commands: Vec<_> = self
            .extensions
            .iter()
            .filter(|e| e.enabled)
            .flat_map(|e| {
                e.package
                    .contributes
                    .commands
                    .iter()
                    .filter(|c| matches!(c.action, Action::Brush { .. } | Action::Tool { .. }))
                    .map(|c| (e.package.id.clone(), c.clone()))
            })
            .collect();
        for (package, command) in commands {
            ui.push_id((&package, &command.id), |ui| {
                let initials: String = command.label.chars().take(2).collect();
                if ui
                    .add_sized([side, side], egui::Button::new(initials))
                    .on_hover_text(format!("{} · {package}", command.label))
                    .clicked()
                {
                    self.run_extension_command(&package, &command.id);
                }
            });
        }
    }
    pub(crate) fn extension_panels_ui(&mut self, ctx: &egui::Context) {
        let panels: Vec<_> = self
            .extensions
            .iter()
            .filter(|e| e.enabled)
            .flat_map(|e| {
                e.package.contributes.panels.iter().map(|p| {
                    (
                        e.package.id.clone(),
                        p.clone(),
                        e.package.contributes.commands.clone(),
                    )
                })
            })
            .collect();
        let available: BTreeSet<_> = panels
            .iter()
            .map(|(id, p, _)| format!("{id}/{}", p.id))
            .collect();
        self.extension_panels.retain(|key| available.contains(key));
        for (id, panel, commands) in panels {
            let key = format!("{id}/{}", panel.id);
            if !self.extension_panels.contains(&key) {
                continue;
            }
            let mut open = true;
            egui::Window::new(&panel.title)
                .id(egui::Id::new(("extension panel", &key)))
                .open(&mut open)
                .default_width(280.0)
                .vscroll(true)
                .show(ctx, |ui| {
                    ui.weak(&id);
                    for (index, widget) in panel.widgets.iter().enumerate() {
                        ui.push_id(index, |ui| match widget {
                            Widget::Label { text } => {
                                ui.label(text);
                            }
                            Widget::Button { command } => {
                                if let Some(c) = commands.iter().find(|c| &c.id == command)
                                    && ui.button(&c.label).clicked()
                                {
                                    self.run_extension_command(&id, command);
                                }
                            }
                            Widget::BrushSize => {
                                ui.add(
                                    egui::Slider::new(&mut self.brush.size, 1.0..=500.0)
                                        .text("Size")
                                        .suffix(" px"),
                                );
                            }
                            Widget::Opacity => {
                                ui.add(
                                    egui::Slider::new(&mut self.brush.opacity, 0.0..=1.0)
                                        .text("Opacity"),
                                );
                            }
                            Widget::Color => {
                                if self.input_stroke_target().is_raster() {
                                    if ui.color_edit_button_srgb(&mut self.color).changed() {
                                        self.hsv = crate::color_wheel::rgb_to_hsv(self.color);
                                    }
                                } else {
                                    ui.add(
                                        egui::Slider::new(&mut self.mask_colors[0], 0..=255)
                                            .text("Mask coverage"),
                                    );
                                }
                            }
                            Widget::Snapping => {
                                ui.checkbox(&mut self.snapping, "Snap to guides");
                            }
                            Widget::GuideLock => {
                                ui.checkbox(&mut self.guides_locked, "Lock guides");
                            }
                            Widget::Separator => {
                                ui.separator();
                            }
                        });
                    }
                });
            if !open {
                self.extension_panels.remove(&key);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn package() -> crate::extensions::Package {
        serde_json::from_str(include_str!("../plugins/example-studio-kit.dudelplugin")).unwrap()
    }
    #[test]
    fn contributions_require_capabilities_unique_ids_and_resolved_panel_buttons() {
        package().validate().unwrap();
        for case in 0..5 {
            let mut p = package();
            match case {
                0 => p.capabilities.retain(|c| c != "tools"),
                1 => p
                    .contributes
                    .commands
                    .push(p.contributes.commands[0].clone()),
                2 => p.contributes.panels[0].widgets.push(Widget::Button {
                    command: "missing".into(),
                }),
                3 => p.capabilities.push("filesystem".into()),
                _ => p.contributes.commands[0].id = "../escape".into(),
            }
            assert!(p.validate().is_err(), "case {case}");
        }
    }
    #[test]
    fn brushes_tools_commands_and_disable_use_native_state_and_undo() {
        let mut app = Dudel {
            doc: crate::document::Document::new(16, 16),
            ..Default::default()
        };
        app.extensions.push(crate::extensions::Installed {
            package: package(),
            enabled: true,
        });
        let library_size = app.brushes.len();
        app.run_extension_command("dudel-studio-kit", "ink");
        assert_eq!(app.brush.name, "Studio ink");
        assert_eq!(app.brushes.len(), library_size);
        assert!(app.tool == Tool::Brush);
        app.run_extension_command("dudel-studio-kit", "circle");
        assert!(app.tool == Tool::Lasso);
        assert!(app.lasso.constrain && app.lasso.from_center);
        assert_eq!(app.lasso.shape, crate::lasso_paint::Shape::Ellipse);
        app.run_extension_command("dudel-studio-kit", "new-layer");
        app.flush_render_queue();
        assert_eq!(app.doc.layers.len(), 2);
        app.undo();
        app.flush_render_queue();
        assert_eq!(app.doc.layers.len(), 1);
        app.extensions[0].enabled = false;
        app.run_extension_command("dudel-studio-kit", "new-layer");
        app.flush_render_queue();
        assert_eq!(app.doc.layers.len(), 1);
        app.extension_panels
            .insert("dudel-studio-kit/drawing".into());
        let ctx = egui::Context::default();
        let _ = ctx.run(Default::default(), |ctx| app.extension_panels_ui(ctx));
        assert!(app.extension_panels.is_empty());
    }
}
