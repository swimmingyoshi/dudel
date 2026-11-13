//! Canvas paper: a surface with tooth that every brush paints on. A paper
//! is a height field computed from the pixel position (no stored image):
//! paint lands on the raised parts first and reaches the valleys only when a
//! dab delivers enough. Light or soft strokes break up into grain; firm
//! strokes cover. This is stage one of the canvas material plan: surface
//! profiles, with no fluid simulation.
use crate::{Dudel, egui};
use serde::{Deserialize, Serialize};

/// The structure of the surface.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Pattern {
    /// Irregular bumps, like watercolor paper.
    #[default]
    Grain,
    /// Crossing threads, like canvas.
    Weave,
    /// Long fibers lying one way, like washi.
    Fibers,
    /// Sparse sharp peaks, like pastel paper.
    Tooth,
}
impl Pattern {
    pub const ALL: [Pattern; 4] = [Self::Grain, Self::Weave, Self::Fibers, Self::Tooth];
    pub fn name(self) -> &'static str {
        match self {
            Self::Grain => "Grain",
            Self::Weave => "Weave",
            Self::Fibers => "Fibers",
            Self::Tooth => "Tooth",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Paper {
    pub pattern: Pattern,
    /// How deep the valleys are: 0 smooth, 1 very rough.
    pub roughness: f32,
    /// Size of one bump, thread or fiber width, in pixels.
    pub scale: f32,
    /// Which way fibers and threads run, in degrees.
    #[serde(default)]
    pub direction: f32,
    /// How readily the surface draws paint into its valleys: 0 keeps paint
    /// on the peaks, 1 lets even light strokes sink in.
    #[serde(default)]
    pub absorbency: f32,
}

impl Paper {
    /// Built-in papers, by name.
    pub fn presets() -> [(&'static str, Paper); 6] {
        let paper = |pattern, roughness, scale, direction, absorbency| Paper {
            pattern,
            roughness,
            scale,
            direction,
            absorbency,
        };
        [
            ("Hot-pressed", paper(Pattern::Grain, 0.2, 3.0, 0.0, 0.4)),
            ("Cold-pressed", paper(Pattern::Grain, 0.5, 6.0, 0.0, 0.5)),
            ("Rough", paper(Pattern::Grain, 0.85, 10.0, 0.0, 0.6)),
            ("Canvas", paper(Pattern::Weave, 0.55, 5.0, 0.0, 0.1)),
            ("Washi", paper(Pattern::Fibers, 0.45, 4.0, 20.0, 0.9)),
            ("Pastel paper", paper(Pattern::Tooth, 0.6, 2.5, 0.0, 0.2)),
        ]
    }
    pub fn validate(&self) -> Result<(), String> {
        let ok = self.roughness.is_finite()
            && (0.0..=1.0).contains(&self.roughness)
            && self.scale.is_finite()
            && (1.0..=128.0).contains(&self.scale)
            && self.direction.is_finite()
            && (-180.0..=180.0).contains(&self.direction)
            && self.absorbency.is_finite()
            && (0.0..=1.0).contains(&self.absorbency);
        if ok {
            Ok(())
        } else {
            Err("Paper setting outside supported range".into())
        }
    }
    /// The preset this paper matches, if any.
    pub fn preset_name(&self) -> Option<&'static str> {
        Self::presets()
            .into_iter()
            .find(|(_, paper)| paper == self)
            .map(|(name, _)| name)
    }
    /// Surface height at a pixel: 0 the deepest valley, 1 a peak.
    pub fn height(&self, x: usize, y: usize) -> f32 {
        self.surface().height(x, y)
    }
    /// The paper ready to be asked about many pixels: its direction worked
    /// out once instead of at every pixel.
    pub fn surface(&self) -> Surface {
        let (sin, cos) = self.direction.to_radians().sin_cos();
        Surface {
            paper: *self,
            sin,
            cos,
        }
    }
    /// Granulation: how much more (above 1) or less paint settles at a
    /// pixel of this `height`, valleys collecting it.
    pub fn pool_at(&self, height: f32, granulation: f32) -> f32 {
        (1.0 + granulation * self.roughness.max(0.3) * (0.5 - height) * 3.0).clamp(0.15, 2.0)
    }
    /// The share of a dab's paint that lands at a pixel of this `height`.
    /// `delivery` is how strongly the dab paints there (0 to 1, its alpha
    /// over the brush's opacity); `response` is the brush's Paper texture
    /// setting.
    pub fn hold_at(&self, height: f32, delivery: f32, response: f32) -> f32 {
        let depth = (1.0 - height) * self.roughness * response;
        if depth <= 0.0 {
            return 1.0;
        }
        // Paint reaches as deep as the dab pushes it, plus what the paper
        // draws in; a soft threshold keeps the grain from aliasing.
        let reach = delivery + self.absorbency * 0.35;
        ((reach - depth) / 0.2 + 0.5).clamp(0.0, 1.0)
    }
}

/// A paper with its direction worked out (`Paper::surface`).
#[derive(Clone, Copy, Debug)]
pub struct Surface {
    pub paper: Paper,
    sin: f32,
    cos: f32,
}

impl Surface {
    /// How high the paper stands at a pixel: 0 a valley, 1 a peak.
    pub fn height(&self, x: usize, y: usize) -> f32 {
        let Self { paper, sin, cos } = *self;
        let (x, y) = (x as f32 + 0.5, y as f32 + 0.5);
        // Along and across the paper's direction, in bumps.
        let u = (x * cos + y * sin) / paper.scale;
        let v = (-x * sin + y * cos) / paper.scale;
        match paper.pattern {
            Pattern::Grain => 0.65 * noise(u, v, 1) + 0.35 * noise(u * 2.3, v * 2.3, 2),
            Pattern::Weave => {
                // Threads alternate over and under; each bulges mid-crossing.
                let (cu, cv) = (u.floor() as i64, v.floor() as i64);
                let (fu, fv) = (u - u.floor(), v - v.floor());
                let bulge = |t: f32| (t * std::f32::consts::PI).sin();
                let thread = if (cu + cv).rem_euclid(2) == 0 {
                    bulge(fv) * (0.6 + 0.4 * bulge(fu))
                } else {
                    bulge(fu) * (0.6 + 0.4 * bulge(fv))
                };
                0.85 * thread + 0.15 * noise(u * 3.0, v * 3.0, 3)
            }
            Pattern::Fibers => 0.7 * noise(u / 9.0, v, 4) + 0.3 * noise(u / 3.0, v * 2.0, 5),
            Pattern::Tooth => {
                // Fine, high-contrast relief: sharp peaks and pits.
                let n = 0.6 * noise(u, v, 6) + 0.4 * noise(u * 2.7, v * 2.7, 7);
                (n - 0.5) * 2.4 + 0.5
            }
        }
        .clamp(0.0, 1.0)
    }
}

/// Smooth value noise in 0..=1.
fn noise(x: f32, y: f32, seed: u32) -> f32 {
    let lattice = |ix: i64, iy: i64| {
        let mut h = (ix as u32).wrapping_mul(0x9E37_79B1)
            ^ (iy as u32).wrapping_mul(0x85EB_CA77)
            ^ seed.wrapping_mul(0xC2B2_AE3D);
        h ^= h >> 16;
        h = h.wrapping_mul(0x7FEB_352D);
        h ^= h >> 15;
        h = h.wrapping_mul(0x846C_A68B);
        h ^= h >> 16;
        (h & 0xFFFF) as f32 / 65535.0
    };
    let (ix, iy) = (x.floor(), y.floor());
    let ease = |t: f32| t * t * (3.0 - 2.0 * t);
    let (tx, ty) = (ease(x - ix), ease(y - iy));
    let (ix, iy) = (ix as i64, iy as i64);
    let top = lattice(ix, iy) + (lattice(ix + 1, iy) - lattice(ix, iy)) * tx;
    let bottom = lattice(ix, iy + 1) + (lattice(ix + 1, iy + 1) - lattice(ix, iy + 1)) * tx;
    top + (bottom - top) * ty
}

/// The Canvas > Paper dialog's state.
pub(crate) struct PaperForm {
    paper: Option<Paper>,
    preview: Option<(Option<Paper>, egui::TextureHandle)>,
}

const PREVIEW: [usize; 2] = [320, 110];

/// The surface lit from the upper left, with a soft and a firm stroke.
fn preview_image(paper: Option<Paper>) -> egui::ColorImage {
    let [w, h] = PREVIEW;
    let mut doc = crate::document::Document::new(w, h);
    doc.paper = paper;
    let mut rgba = vec![0u8; w * h * 4];
    for y in 0..h {
        for x in 0..w {
            let shade = paper.map_or(244.0, |p| {
                let slope = p.height(x, y) - p.height(x + 1, y + 1);
                244.0 + slope * p.roughness * 70.0 - p.roughness * 6.0
            });
            let v = shade.clamp(200.0, 255.0) as u8;
            rgba[(y * w + x) * 4..][..4].copy_from_slice(&[v, v, v.saturating_sub(4), 255]);
        }
    }
    for (row, brush) in [
        crate::brush::Brush {
            size: 26.0,
            hardness: 0.3,
            opacity: 0.9,
            flow: 0.35,
            spacing: 0.08,
            pressure_size: false,
            pressure_opacity: true,
            ..Default::default()
        },
        crate::brush::Brush {
            size: 14.0,
            hardness: 0.9,
            spacing: 0.06,
            ..Default::default()
        },
    ]
    .into_iter()
    .enumerate()
    {
        let settings = crate::stroke_buffer::Settings {
            target: crate::paint_target::Target::Raster,
            brush: brush.clone(),
            color: [38, 44, 59],
            secondary: [38, 44, 59],
            erase: false,
            layer: 0,
            symmetry: Default::default(),
        };
        let mut stroke = crate::stroke::Stroke::default();
        let mut buffer = crate::stroke_buffer::StrokeBuffer::default();
        for i in 0..=60 {
            let t = i as f32 / 60.0;
            let sample = crate::stroke::Sample {
                x: 20.0 + t * (w as f32 - 40.0),
                y: 34.0 + row as f32 * 44.0 + (t * std::f32::consts::TAU).sin() * 8.0,
                pressure: 0.15 + 0.85 * t,
                pressure_known: true,
                ..Default::default()
            };
            let dabs = if i == 60 {
                stroke.finish(sample, &brush)
            } else {
                stroke.sample(sample, &brush)
            };
            for dab in dabs {
                buffer.paint(&mut doc, dab, &settings);
            }
        }
    }
    for (i, p) in doc.layers[0]
        .pixels
        .region(w, [0, 0, w, h])
        .chunks_exact(4)
        .enumerate()
    {
        let a = f32::from(p[3]) / 255.0;
        for c in 0..3 {
            let under = f32::from(rgba[i * 4 + c]);
            rgba[i * 4 + c] = (f32::from(p[c]) * a + under * (1.0 - a)).round() as u8;
        }
    }
    egui::ColorImage::from_rgba_unmultiplied(PREVIEW, &rgba)
}

impl Dudel {
    /// Canvas > Paper…
    pub(crate) fn open_paper_dialog(&mut self) {
        self.paper_form = Some(PaperForm {
            paper: self.doc.paper,
            preview: None,
        });
    }

    pub(crate) fn paper_dialog(&mut self, ctx: &egui::Context) {
        let Some(form) = &mut self.paper_form else {
            return;
        };
        let (mut apply, mut cancel) = (false, false);
        let response = egui::Modal::new(egui::Id::new("paper dialog")).show(ctx, |ui| {
            ui.set_width(PREVIEW[0] as f32 + 16.0);
            cancel |= crate::dialogs::dialog_title(ui, "Paper");
            if form
                .preview
                .as_ref()
                .is_none_or(|(shown, _)| *shown != form.paper)
            {
                let image = preview_image(form.paper);
                match &mut form.preview {
                    Some((shown, texture)) => {
                        texture.set(image, egui::TextureOptions::NEAREST);
                        *shown = form.paper;
                    }
                    None => {
                        form.preview = Some((
                            form.paper,
                            ctx.load_texture("paper preview", image, egui::TextureOptions::NEAREST),
                        ));
                    }
                }
            }
            if let Some((_, texture)) = &form.preview {
                ui.image((
                    texture.id(),
                    egui::vec2(PREVIEW[0] as f32, PREVIEW[1] as f32),
                ));
            }
            ui.horizontal_wrapped(|ui| {
                if ui
                    .selectable_label(form.paper.is_none(), "Smooth")
                    .on_hover_text("No paper texture: brushes paint as on a plain canvas")
                    .clicked()
                {
                    form.paper = None;
                }
                for (name, preset) in Paper::presets() {
                    if ui
                        .selectable_label(form.paper == Some(preset), name)
                        .clicked()
                    {
                        form.paper = Some(preset);
                    }
                }
            });
            if let Some(paper) = &mut form.paper {
                ui.horizontal_wrapped(|ui| {
                    ui.label("Surface");
                    for pattern in Pattern::ALL {
                        ui.selectable_value(&mut paper.pattern, pattern, pattern.name());
                    }
                });
                crate::workspace::labeled_slider(
                    ui,
                    "Roughness",
                    egui::Slider::new(&mut paper.roughness, 0.0..=1.0),
                )
                .on_hover_text(
                    "How deep the valleys are: rougher paper breaks light strokes up more",
                );
                crate::workspace::labeled_slider(
                    ui,
                    "Grain size",
                    egui::Slider::new(&mut paper.scale, 1.0..=64.0)
                        .logarithmic(true)
                        .suffix(" px"),
                );
                crate::workspace::labeled_slider(
                    ui,
                    "Absorbency",
                    egui::Slider::new(&mut paper.absorbency, 0.0..=1.0),
                )
                .on_hover_text(
                    "Absorbent paper draws paint into its valleys, so strokes cover more evenly",
                );
                if matches!(paper.pattern, Pattern::Weave | Pattern::Fibers) {
                    crate::workspace::labeled_slider(
                        ui,
                        "Direction",
                        egui::Slider::new(&mut paper.direction, -90.0..=90.0)
                            .suffix("°")
                            .integer(),
                    );
                }
            }
            ui.small(
                "Paper changes how brushes lay paint down from now on; paint already on the \
                 canvas stays as it is. Each brush's Paper texture setting (Brush settings › \
                 Texture) says how much it feels the paper. The paper is saved with the project.",
            );
            crate::dialogs::dialog_buttons(ui, |ui| {
                apply = ui
                    .add(crate::workspace::primary_button(ui, "Apply"))
                    .clicked();
                cancel |= ui.button("Cancel").clicked();
            });
        });
        if apply {
            let paper = self.paper_form.take().and_then(|form| form.paper);
            self.dispatch_document_edit(
                crate::document_edits::Edit::Paper(paper),
                "Paper changed",
                "The canvas already has this paper",
            );
        } else if cancel || response.should_close() {
            self.paper_form = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;

    fn coverage(paper: Option<Paper>, brush: &crate::brush::Brush) -> (usize, usize) {
        let mut doc = Document::new(160, 60);
        doc.paper = paper;
        let settings = crate::stroke_buffer::Settings {
            target: crate::paint_target::Target::Raster,
            brush: brush.clone(),
            color: [0, 0, 0],
            secondary: [0, 0, 0],
            erase: false,
            layer: 0,
            symmetry: Default::default(),
        };
        let mut stroke = crate::stroke::Stroke::default();
        let mut buffer = crate::stroke_buffer::StrokeBuffer::default();
        for i in 0..=20 {
            let sample = crate::stroke::Sample {
                x: 20.0 + i as f32 * 6.0,
                y: 30.0,
                pressure: 1.0,
                pressure_known: true,
                ..Default::default()
            };
            let dabs = if i == 20 {
                stroke.finish(sample, brush)
            } else {
                stroke.sample(sample, brush)
            };
            for dab in dabs {
                buffer.paint(&mut doc, dab, &settings);
            }
        }
        buffer.finish_tail(&mut doc, &settings);
        // Pixels in the stroke's core band that got paint, and that are solid.
        let mut painted = 0;
        let mut solid = 0;
        for y in 26..34 {
            for x in 30..130 {
                let a = doc.layers[0].pixels.rgba(y * 160 + x)[3];
                painted += usize::from(a > 0);
                solid += usize::from(a > 200);
            }
        }
        (painted, solid)
    }

    #[test]
    fn heights_are_repeatable_in_range_and_differ_by_pattern() {
        for (name, paper) in Paper::presets() {
            paper.validate().unwrap();
            assert_eq!(paper.preset_name(), Some(name));
            let heights: Vec<f32> = (0..400)
                .map(|i| paper.height(i % 20 * 3, i / 20 * 3))
                .collect();
            assert!(heights.iter().all(|h| (0.0..=1.0).contains(h)), "{name}");
            let (low, high) = heights
                .iter()
                .fold((1.0f32, 0.0f32), |(l, h), v| (l.min(*v), h.max(*v)));
            assert!(high - low > 0.4, "{name} has relief: {low}..{high}");
            assert_eq!(paper.height(7, 9), paper.height(7, 9));
        }
        let bad = Paper {
            scale: 0.0,
            ..Paper::presets()[0].1
        };
        assert!(bad.validate().is_err());
    }

    #[test]
    fn rough_paper_breaks_up_soft_strokes_and_firm_strokes_cover() {
        let rough = Some(Paper::presets()[2].1);
        let soft = crate::brush::Brush {
            size: 16.0,
            hardness: 0.8,
            flow: 0.3,
            spacing: 0.1,
            pressure_size: false,
            ..Default::default()
        };
        let firm = crate::brush::Brush {
            flow: 1.0,
            ..soft.clone()
        };
        let (plain_painted, _) = coverage(None, &soft);
        let (rough_painted, _) = coverage(rough, &soft);
        assert_eq!(plain_painted, 800, "the plain stroke covers its core");
        assert!(
            rough_painted < plain_painted * 9 / 10 && rough_painted > plain_painted / 10,
            "{rough_painted} of {plain_painted}"
        );
        // A firm stroke fills the valleys.
        let (_, firm_solid) = coverage(rough, &firm);
        assert!(firm_solid > 780, "{firm_solid}");
        // A brush that ignores paper paints as on a plain canvas.
        let ignoring = crate::brush::Brush {
            paper_response: 0.0,
            ..soft.clone()
        };
        assert_eq!(coverage(rough, &ignoring), coverage(None, &soft));
        // Absorbent paper lets the same soft stroke sink in further.
        let thirsty = rough.map(|p| Paper {
            absorbency: 1.0,
            ..p
        });
        assert!(coverage(thirsty, &soft).0 > rough_painted);
    }

    #[test]
    fn paper_is_an_undoable_edit_saved_with_the_project() {
        let paper = Paper::presets()[1].1;
        let source = Document::new(8, 8);
        let (changed, outcome) = crate::document_edits::Edit::Paper(Some(paper))
            .prepare(&source)
            .unwrap();
        assert!(outcome.changed && changed.paper == Some(paper));
        let (_, again) = crate::document_edits::Edit::Paper(Some(paper))
            .prepare(&changed)
            .unwrap();
        assert!(!again.changed);
        assert!(!crate::document::same_snapshot(&source, &changed));
        let bad = Paper {
            roughness: 2.0,
            ..paper
        };
        assert!(
            crate::document_edits::Edit::Paper(Some(bad))
                .prepare(&source)
                .is_err()
        );
        // Project file and package both keep it.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("paper.dudel");
        crate::storage::save_project(&path, &changed).unwrap();
        assert_eq!(
            crate::storage::load_project(&path).unwrap().paper,
            Some(paper)
        );
        crate::storage::save_project(&path, &source).unwrap();
        assert_eq!(crate::storage::load_project(&path).unwrap().paper, None);
    }
}

/// `DUDEL_PAPER_SHEET=out.png cargo test paper_sheet -- --ignored` draws a
/// soft and a firm stroke on smooth canvas and on every built-in paper.
#[cfg(test)]
#[test]
#[ignore = "writes a picture for review"]
fn paper_sheet() {
    let Ok(path) = std::env::var("DUDEL_PAPER_SHEET") else {
        return;
    };
    let papers: Vec<Option<Paper>> = std::iter::once(None)
        .chain(Paper::presets().into_iter().map(|(_, p)| Some(p)))
        .collect();
    let [w, h] = PREVIEW;
    let mut sheet = Vec::with_capacity(w * h * 4 * papers.len());
    for paper in &papers {
        sheet.extend(preview_image(*paper).as_raw());
    }
    image::save_buffer(
        path,
        &sheet,
        w as u32,
        (h * papers.len()) as u32,
        image::ExtendedColorType::Rgba8,
    )
    .unwrap();
}
