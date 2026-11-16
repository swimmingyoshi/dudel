//! App side of `svg_export`: the parts that need the window, the app state or
//! app-only modules. The engine part is `dudel_core::svg_export`, re-exported
//! here so `crate::svg_export::…` paths keep working.
use crate::Dudel;
pub(crate) use dudel_core::svg_export::*;

impl Dudel {
    pub(crate) fn export_svg(&mut self) {
        let name = self
            .path
            .as_deref()
            .and_then(|p| p.file_stem())
            .map_or("Painting".into(), |s| s.to_string_lossy().into_owned());
        let dialog = self.dialog_in(
            crate::locations::Kind::Exports,
            rfd::AsyncFileDialog::new()
                .add_filter("SVG", &["svg"])
                .set_file_name(format!("{name}.svg")),
        );
        self.ask_save_file(dialog, move |app, path| app.export_svg_to(&path));
    }
    pub(crate) fn export_svg_to(&mut self, path: &std::path::Path) {
        self.flush_render_queue();
        let (svg, lines) = encode(&self.doc);
        self.status = match crate::storage::atomic_write(path, svg.as_bytes()) {
            Ok(()) => format!("Exported {lines} vector line(s) to SVG; raster layers are left out"),
            Err(error) => error,
        };
    }
}
