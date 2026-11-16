//! App side of `import_report`: the parts that need the window, the app state or
//! app-only modules. The engine part is `dudel_core::import_report`, re-exported
//! here so `crate::import_report::…` paths keep working.
pub(crate) use dudel_core::import_report::*;

impl crate::Dudel {
    pub(crate) fn import_report_ui(&mut self, ctx: &crate::egui::Context) {
        let report = &mut self.import_report;
        if !report.open || report.is_empty() {
            return;
        }
        let mut open = true;
        crate::egui::Window::new("Import details")
            .open(&mut open)
            .default_width(440.0)
            .vscroll(true)
            .show(ctx, |ui| {
                ui.label("Your source file has not been changed. Save the imported artwork as a new .dudel project.");
                if ui
                    .button("Copy all")
                    .on_hover_text("Copy every message as text (or select text and press Ctrl+C)")
                    .clicked()
                {
                    ui.ctx().copy_text(report.text());
                }
                ui.separator();
                if report.notes.is_empty() {
                    ui.label("Nothing was left out.");
                }
                for note in &report.notes {
                    ui.label(note);
                }
                if report.details.is_empty() {
                    return;
                }
                if report.notes.is_empty() {
                    ui.add_space(4.0);
                    for detail in &report.details {
                        ui.weak(detail);
                    }
                } else {
                    crate::egui::CollapsingHeader::new(format!(
                        "Other details ({})",
                        report.details.len()
                    ))
                    .show(ui, |ui| {
                        for detail in &report.details {
                            ui.weak(detail);
                        }
                    });
                }
            });
        report.open = open;
    }
    /// A status-bar button that reopens the last import's messages.
    pub(crate) fn import_report_button(&mut self, ui: &mut crate::egui::Ui) {
        let report = &mut self.import_report;
        if report.open || report.is_empty() {
            return;
        }
        let label = if report.notes.is_empty() {
            "Import details".to_string()
        } else {
            format!("Import details ({})", report.notes.len())
        };
        if ui
            .add(crate::egui::Button::new(label).small())
            .on_hover_text("What the import kept, left out or changed")
            .clicked()
        {
            report.open = true;
        }
    }
}
