//! A stable, scrollable home for settings instead of nested transient menus.
use crate::{Dudel, egui};
impl Dudel {
    pub(crate) fn settings_ui(&mut self, ctx: &egui::Context) {
        let mut open = self.settings_open;
        egui::Window::new("Settings").open(&mut open).collapsible(false)
            .default_width(480.0).default_height(360.0).vscroll(true)
            .show(ctx, |ui| {
                ui.horizontal_wrapped(|ui| {
                    for (i, name) in ["Workspace", "Drawing", "Display", "Storage"].iter().enumerate() {
                        ui.selectable_value(&mut self.settings_section, i, *name);
                    }
                });
                ui.add_space(8.0);
                ui.separator();
                ui.add_space(8.0);
                match self.settings_section {
                    0 => {
                        ui.heading("Make room for your work");
                        ui.label("Choose what stays visible while you draw.");
                        ui.add_space(8.0);
                        ui.checkbox(&mut self.show_panels, "Show workspace panels");
                        if ui.button("Restore automatic panel layout").clicked() {
                            self.tool_panel_override = None; self.inspector_override = None; self.show_panels = true;
                        }
                        ui.horizontal(|ui| {
                            ui.label("Open project limit");
                            let minimum = self.tabs.len().max(1);
                            ui.add(egui::DragValue::new(&mut self.tabs.max).range(minimum..=crate::tabs::LIMIT));
                        });
                        ui.small("Each open project uses memory, including parked tabs.");
                        ui.separator();
                        ui.menu_button("Choose or customize a theme", |ui| self.theme_menu(ui));
                        if ui.button("Manage extensions…").clicked() { self.extension_manager = true; }
                    }
                    1 => {
                        ui.heading("Drawing and pen input");
                        ui.label("Brush-specific pressure and stabilization settings live in Brush Studio.");
                        ui.add_space(8.0);
                        ui.checkbox(&mut self.input.native_requested, "Use native pen input");
                        ui.small(&self.input.diagnostics.status);
                        ui.checkbox(&mut self.timelapse.prefs.record_new, "Record timelapses for new canvases");
                        ui.checkbox(&mut self.guides_locked, "Lock guide handles");
                        ui.checkbox(&mut self.snapping, "Snap strokes to guides");
                    }
                    2 => {
                        ui.heading("Display and performance");
                        ui.label(format!("Running now: {}", self.display_active));
                        ui.add_space(8.0);
                        if ui.button("Graphics API and latency…").clicked() { self.display_window = true; }
                        ui.small("Graphics changes take effect after restarting Dudel.");
                        ui.checkbox(&mut self.performance.open, "Show performance monitor");
                    }
                    _ => {
                        ui.heading("Files and storage");
                        ui.label("Keep projects, exports and recovery files where you expect them.");
                        ui.add_space(8.0);
                        if ui.button("Storage & folders…").clicked() { self.storage_window = true; }
                        #[cfg(windows)] {
                            ui.separator();
                            ui.strong("File Explorer thumbnails");
                            ui.small("Show artwork previews for .dudel files. Enable again after moving Dudel's folder.");
                            ui.horizontal_wrapped(|ui| {
                                for (enable, label) in [(true, "Enable previews"), (false, "Remove previews")] {
                                    if ui.button(label).clicked() { self.status = crate::file_preview::set_explorer_thumbnails(enable).unwrap_or_else(|error| format!("File Explorer previews: {error}")); }
                                }
                            });
                        }
                    }
                }
            });
        self.settings_open = open;
    }
}
