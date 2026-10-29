//! User-visible conversion notes stay separate from editable document content.
pub(crate) struct Imported {
    pub document: crate::document::Document,
    pub notes: Vec<String>,
}

pub(crate) fn verify(
    path: &std::path::Path,
    output: Option<&std::path::Path>,
    reference: Option<&std::path::Path>,
    preview: bool,
) -> Result<(), String> {
    let psd = path
        .extension()
        .and_then(|s| s.to_str())
        .is_some_and(|s| s.eq_ignore_ascii_case("psd"));
    let imported = if preview {
        crate::clip_metadata::import_preview(path, &|| false)?
    } else if psd {
        crate::psd_import::import(path, &|| false)?
    } else {
        crate::clip_metadata::import_document(path, &|| false)?
    };
    imported.document.validate()?;
    #[derive(serde::Serialize)]
    struct Report {
        canvas: [usize; 2],
        layers: usize,
        folders: usize,
        masks: usize,
        notes: Vec<String>,
        rasters: Vec<crate::clip_raster::Statistics>,
        /// Dudel's rendering of the import against the PSD's saved composite.
        #[serde(skip_serializing_if = "Option::is_none")]
        saved_composite: Option<Difference>,
        /// Against a reference image given on the command line.
        #[serde(skip_serializing_if = "Option::is_none")]
        reference: Option<Difference>,
    }
    let doc = imported.document;
    let rendered = doc.composite_transparent();
    // Files saved without a flattened image have nothing to compare against.
    let saved_composite = if psd {
        crate::psd_import::saved_composite(path).ok().map(|saved| {
            let saved: Vec<u8> = (0..doc.width * doc.height)
                .flat_map(|i| saved.document.layers[0].pixels.rgba(i))
                .collect();
            difference(&rendered, &saved)
        })
    } else {
        None
    };
    let reference = match reference {
        Some(reference) => {
            let image = image::open(reference)
                .map_err(|e| format!("Reference image: {e}"))?
                .into_rgba8();
            if image.dimensions() != (doc.width as u32, doc.height as u32) {
                return Err("Reference image size differs from the import".into());
            }
            Some(difference(&rendered, image.as_raw()))
        }
        None => None,
    };
    let report = Report {
        canvas: [doc.width, doc.height],
        layers: doc.layers.len(),
        folders: doc.layers.iter().filter(|l| l.folder).count(),
        masks: doc.layers.iter().filter(|l| l.mask.is_some()).count(),
        notes: imported.notes,
        rasters: doc
            .layers
            .iter()
            .filter(|l| !l.folder)
            .map(|l| crate::clip_raster::statistics(&l.pixels))
            .collect(),
        saved_composite,
        reference,
    };
    crate::clip_container::write_report(&report, output)
}

/// How far a render is from a reference, on what is visible: colors are
/// weighted by alpha, so color stored under transparent pixels is ignored.
#[derive(serde::Serialize)]
struct Difference {
    total_pixels: usize,
    exact_pixels: usize,
    /// Pixels where any alpha-weighted channel differs by more than 2.
    differing_pixels: usize,
    max_channel_difference: u8,
    mean_absolute_channel_difference: f64,
}
fn difference(rendered: &[u8], reference: &[u8]) -> Difference {
    let visible = |p: &[u8]| -> [u8; 4] {
        let a = u32::from(p[3]);
        let c = |v: u8| ((u32::from(v) * a + 127) / 255) as u8;
        [c(p[0]), c(p[1]), c(p[2]), p[3]]
    };
    let mut result = Difference {
        total_pixels: rendered.len() / 4,
        exact_pixels: 0,
        differing_pixels: 0,
        max_channel_difference: 0,
        mean_absolute_channel_difference: 0.0,
    };
    let mut sum = 0u64;
    for (a, b) in rendered.chunks_exact(4).zip(reference.chunks_exact(4)) {
        let (a, b) = (visible(a), visible(b));
        let channels = [0, 1, 2, 3].map(|c| a[c].abs_diff(b[c]));
        let worst = channels.into_iter().max().unwrap_or(0);
        sum += channels.into_iter().map(u64::from).sum::<u64>();
        result.exact_pixels += usize::from(worst == 0);
        result.differing_pixels += usize::from(worst > 2);
        result.max_channel_difference = result.max_channel_difference.max(worst);
    }
    result.mean_absolute_channel_difference = sum as f64 / rendered.len().max(1) as f64;
    result
}

impl crate::Dudel {
    pub(crate) fn import_report_ui(&mut self, ctx: &crate::egui::Context) {
        if self.import_notes.is_empty() {
            return;
        }
        let mut open = true;
        crate::egui::Window::new("Import details")
            .open(&mut open).default_width(440.0).vscroll(true)
            .show(ctx, |ui| {
                ui.label("Your source file has not been changed. Save the imported artwork as a new .dudel project.");
                ui.separator();
                for note in &self.import_notes { ui.label(note); }
            });
        if !open {
            self.import_notes.clear();
        }
    }
}
