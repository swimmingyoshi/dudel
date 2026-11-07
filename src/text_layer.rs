//! Text layers: the layer keeps its words and layout, and its pixels are that
//! text drawn once. The pixels stay an ordinary raster, so compositing, effects,
//! masks and export need nothing new, and a project looks the same on a computer
//! without its font: opening never redraws text.
use crate::{
    affine::Matrix,
    document::Document,
    pixels::Pixels,
    text_render::{Align, Block, Measure},
};

/// Why a text layer's pixels can't be edited directly.
pub(crate) const BLOCKER: &str =
    "This is a text layer — edit its text, or rasterize it (Layer menu) to edit its pixels";

/// Why part of a text layer can't be moved or transformed.
pub(crate) const DESELECT: &str = "Deselect first: text moves and transforms as a whole";

/// Longest text in bytes. A thousand layers of it still fit project metadata.
pub(crate) const MAX_TEXT: usize = 8192;
const MAX_FONT_NAME: usize = 256;
/// Largest em square in pixels. Turned text rasterizes a glyph's whole width
/// at once, so this also bounds that buffer.
const MAX_SIZE: f64 = 8192.0;
const MAX_POSITION: f64 = 1e7;

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub(crate) struct TextLayer {
    pub(crate) string: String,
    /// The font's full name, as `text_render::find_font` takes it.
    pub(crate) font: String,
    /// The em square in canvas pixels, so that changing the document's DPI
    /// leaves text alone.
    pub(crate) size: f64,
    pub(crate) color: [u8; 3],
    pub(crate) align: Align,
    pub(crate) vertical: bool,
    /// The box width lines wrap at, or None for lines that end only at breaks.
    pub(crate) wrap: Option<f64>,
    /// The height the box was drawn at. Text past it makes the box taller.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) box_height: Option<f64>,
    /// A multiple of the font's own line height.
    pub(crate) line_spacing: f64,
    /// Canvas pixels: where the first line starts, and the unit vectors along
    /// the lines and down across them.
    pub(crate) start: [f64; 2],
    pub(crate) along: [f64; 2],
    pub(crate) down: [f64; 2],
    /// CSP's laid-out span and thickness, kept until the first edit so that
    /// imported text matches CSP.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) csp: Option<[f64; 2]>,
}

impl TextLayer {
    pub(crate) fn validate(&self) -> Result<(), String> {
        let inside = |v: f64, range: std::ops::RangeInclusive<f64>| range.contains(&v);
        let position = |v: [f64; 2]| v.iter().all(|v| inside(*v, -MAX_POSITION..=MAX_POSITION));
        let unit = |v: [f64; 2]| ((v[0] * v[0] + v[1] * v[1]).sqrt() - 1.0).abs() < 1e-6;
        let square = (self.along[0] * self.down[0] + self.along[1] * self.down[1]).abs() < 1e-6;
        if self.string.len() > MAX_TEXT
            || self.font.is_empty()
            || self.font.len() > MAX_FONT_NAME
            || !inside(self.size, 1.0..=MAX_SIZE)
            || self.wrap.is_some_and(|w| !inside(w, 1.0..=MAX_POSITION))
            || self
                .box_height
                .is_some_and(|h| !inside(h, 1.0..=MAX_POSITION))
            || !inside(self.line_spacing, 0.5..=5.0)
            || !position(self.start)
            || !unit(self.along)
            || !unit(self.down)
            || !square
            || self
                .csp
                .is_some_and(|csp| !csp.iter().all(|v| inside(*v, 0.0..=MAX_POSITION)))
        {
            return Err("Invalid text layer".into());
        }
        Ok(())
    }

    fn font(&self) -> Result<std::sync::Arc<ab_glyph::FontVec>, String> {
        crate::text_render::find_font(&self.font)
            .ok_or_else(|| format!("The font {} is not installed", self.font))
    }

    fn block(&self) -> Block<'_> {
        let measure = match self.csp {
            Some([span, thickness]) => Measure::Given { span, thickness },
            None => Measure::Natural {
                line_spacing: self.line_spacing,
            },
        };
        Block {
            string: &self.string,
            em: self.size as f32,
            color: self.color,
            vertical: self.vertical,
            align: self.align,
            start: self.start,
            along: self.along,
            down: self.down,
            measure,
            wrap: self.wrap.unwrap_or(f64::INFINITY),
            kerning: self.csp.is_none(),
        }
    }

    /// The text in its color over the whole canvas.
    pub(crate) fn render(
        &self,
        canvas: [usize; 2],
        cancelled: &impl Fn() -> bool,
    ) -> Result<Pixels, String> {
        crate::text_render::render(&self.block(), &*self.font()?, canvas, cancelled)
    }

    /// This text in Dudel's own layout, for editing: imported text leaves CSP's
    /// measurements behind without its lines moving.
    pub(crate) fn released(&self) -> Self {
        let Some([span, thickness]) = self.csp else {
            return self.clone();
        };
        let mut text = Self {
            csp: None,
            ..self.clone()
        };
        let mut drop = 0.0;
        if let Ok((lines, pitch, height)) = self
            .font()
            .and_then(|font| crate::text_render::line_metrics(&self.block(), &font))
        {
            let spacing = (thickness / lines as f64 / pitch).clamp(0.5, 5.0);
            text.line_spacing = spacing;
            // CSP hangs a line from its bottom and Dudel from its top.
            if !self.vertical {
                drop = pitch * spacing - height;
            }
        }
        // CSP aligns within the laid-out length, and Dudel within the box.
        let room = self.wrap.unwrap_or(span) - span;
        let back = match self.align {
            Align::Left => 0.0,
            Align::Center => room / 2.0,
            Align::Right => room,
        };
        for axis in 0..2 {
            text.start[axis] += self.down[axis] * drop - self.along[axis] * back;
        }
        text
    }

    /// This text after a transform, or None when the record can't say the
    /// same thing as the transformed pixels. Moves, turns, flips and uniform
    /// scales keep lines straight and evenly sized; anything else doesn't.
    pub(crate) fn mapped(&self, matrix: Matrix) -> Option<Self> {
        let [a, b, c, d, _, _] = matrix.0;
        let (across, scale) = (c.hypot(d), a.hypot(b));
        let uniform = (scale - across).abs() <= 1e-6 * scale.max(across);
        if !uniform || (a * c + b * d).abs() > 1e-6 * scale * across {
            return None;
        }
        let turn = |[x, y]: [f64; 2]| [(a * x + c * y) / scale, (b * x + d * y) / scale];
        let text = Self {
            size: self.size * scale,
            wrap: self.wrap.map(|wrap| wrap * scale),
            box_height: self.box_height.map(|height| height * scale),
            start: matrix.point(self.start[0], self.start[1]),
            along: turn(self.along),
            down: turn(self.down),
            csp: self.csp.map(|csp| csp.map(|v| v * scale)),
            ..self.clone()
        };
        text.validate().is_ok().then_some(text)
    }

    /// How long and how tall the text is, in canvas pixels.
    pub(crate) fn extent(&self) -> Result<[f64; 2], String> {
        let [length, height] = crate::text_render::extent(&self.block(), &*self.font()?)?;
        Ok([length, height.max(self.box_height.unwrap_or(0.0))])
    }

    /// Where the caret stands before each character, and at the end.
    pub(crate) fn carets(&self) -> Result<Vec<crate::text_render::Caret>, String> {
        crate::text_render::carets(&self.block(), &*self.font()?)
    }

    /// A point in text space, in canvas pixels.
    pub(crate) fn canvas_point(&self, along: f64, down: f64) -> [f64; 2] {
        [
            self.start[0] + self.along[0] * along + self.down[0] * down,
            self.start[1] + self.along[1] * along + self.down[1] * down,
        ]
    }

    /// A canvas point in text space: along the lines and down across them.
    pub(crate) fn text_point(&self, at: [f64; 2]) -> [f64; 2] {
        let offset = [at[0] - self.start[0], at[1] - self.start[1]];
        [
            offset[0] * self.along[0] + offset[1] * self.along[1],
            offset[0] * self.down[0] + offset[1] * self.down[1],
        ]
    }

    /// The characters this text's font can't draw.
    pub(crate) fn missing_characters(&self) -> Vec<char> {
        self.font().map_or_else(
            |_| Vec::new(),
            |font| crate::text_render::missing_characters(&self.string, &font),
        )
    }
}

/// Whether a transform lands every pixel exactly on another pixel, so that
/// moved pixels are as good as a redraw.
pub(crate) fn whole_pixel(matrix: Matrix) -> bool {
    let [a, b, c, d, x, y] = matrix.0;
    [a, b, c, d].iter().all(|v| [0.0, 1.0, -1.0].contains(v))
        && x.fract() == 0.0
        && y.fract() == 0.0
}

/// Text layers move and transform whole. `blocker` is why the target can't
/// take pixels: for a text layer that holds only with a selection, or for
/// `paired` transforms, whose reflected copy a record can't describe.
pub(crate) fn whole_layer_blocker(
    blocker: Option<&'static str>,
    doc: &Document,
    paired: bool,
) -> Option<&'static str> {
    match blocker {
        Some(reason) if reason == BLOCKER => {
            if paired {
                Some(BLOCKER)
            } else if doc.selection.is_some() || doc.selection_mask.is_some() {
                Some(DESELECT)
            } else {
                None
            }
        }
        other => other,
    }
}

/// Bring the active text layer along after its pixels were transformed from
/// `before` into `doc`: redrawn crisply from its new record when possible, and
/// otherwise left as the pixels the transform made. None means a transform no
/// record can describe.
pub(crate) fn carry(
    before: &Document,
    doc: &mut Document,
    matrix: Option<Matrix>,
    cancel: Option<&std::sync::atomic::AtomicBool>,
) -> Result<(), String> {
    let index = before.active;
    let Some(text) = &before.layers[index].text else {
        return Ok(());
    };
    let Some((matrix, text)) = matrix.and_then(|m| Some((m, text.mapped(m)?))) else {
        doc.layers[index].text = None;
        return Ok(());
    };
    let stopped = || crate::fill::check_cancel(cancel).is_err();
    match text.render([doc.width, doc.height], &stopped) {
        Ok(pixels) => {
            doc.layers[index].pixels = pixels;
            doc.layers[index].text = Some(text);
        }
        Err(_) => {
            crate::fill::check_cancel(cancel)?;
            // Without its font, only exact pixels still match the record.
            doc.layers[index].text = whole_pixel(matrix).then_some(text);
        }
    }
    Ok(())
}

/// Carry every text layer along when the whole canvas moves or turns. The
/// layers' pixels have already moved, so nothing is redrawn: the record only
/// has to say the same thing as the pixels.
pub(crate) fn follow_canvas(
    doc: &mut Document,
    place: &dyn Fn([f64; 2]) -> [f64; 2],
    turn: &dyn Fn([f64; 2]) -> [f64; 2],
) {
    for text in doc.layers.iter_mut().filter_map(|l| l.text.as_mut()) {
        text.start = place(text.start);
        text.along = turn(text.along);
        text.down = turn(text.down);
    }
}

/// Give the layer at `index` this text and draw it. False when nothing changed.
pub(crate) fn set(
    doc: &mut Document,
    index: usize,
    text: TextLayer,
    cancelled: &impl Fn() -> bool,
) -> Result<bool, String> {
    if doc.layers[index].text.as_ref() == Some(&text) {
        return Ok(false);
    }
    let pixels = text.render([doc.width, doc.height], cancelled)?;
    let layer = &mut doc.layers[index];
    layer.pixels = pixels;
    layer.text = Some(text);
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{document::LayerId, document_edits::Edit, text_render::TEST_FONT};

    fn text(string: &str) -> TextLayer {
        TextLayer {
            string: string.into(),
            font: TEST_FONT.into(),
            size: 40.0,
            color: [200, 10, 30],
            align: Align::Left,
            vertical: false,
            wrap: None,
            box_height: None,
            line_spacing: 1.0,
            start: [20.0, 20.0],
            along: [1.0, 0.0],
            down: [0.0, 1.0],
            csp: None,
        }
    }

    /// A document whose active layer is text saying "Hemlo", above a raster.
    fn document() -> Document {
        let doc = Document::new(300, 200);
        let (doc, outcome) = Edit::NewText {
            id: LayerId::new(),
            text: Box::new(text("Hemlo")),
        }
        .prepare(&doc)
        .unwrap();
        assert!(outcome.changed && outcome.undoable);
        doc
    }

    fn ink(doc: &Document, index: usize) -> usize {
        let pixels = &doc.layers[index].pixels;
        (0..doc.width * doc.height)
            .filter(|p| pixels.rgba(*p)[3] != 0)
            .count()
    }

    #[test]
    fn damaged_records_are_refused() {
        assert!(text("Hemlo").validate().is_ok());
        let cases: [fn(&mut TextLayer); 13] = [
            |t| t.string = "x".repeat(MAX_TEXT + 1),
            |t| t.font.clear(),
            |t| t.font = "x".repeat(MAX_FONT_NAME + 1),
            |t| t.size = 0.0,
            |t| t.size = f64::NAN,
            |t| t.size = MAX_SIZE + 1.0,
            |t| t.wrap = Some(f64::INFINITY),
            |t| t.line_spacing = f64::NAN,
            |t| t.start = [f64::NAN, 0.0],
            |t| t.along = [2.0, 0.0],
            |t| t.down = [1.0, 0.0],
            |t| t.csp = Some([f64::NAN, 1.0]),
            |t| t.csp = Some([-1.0, 1.0]),
        ];
        for (index, damage) in cases.iter().enumerate() {
            let mut damaged = text("Hemlo");
            damage(&mut damaged);
            assert!(damaged.validate().is_err(), "case {index}");
        }
    }

    #[test]
    fn a_damaged_record_in_a_project_is_an_error_not_a_panic() {
        let mut doc = document();
        let active = doc.active;
        doc.layers[active].text.as_mut().unwrap().size = -1.0;
        let mut bytes = Vec::new();
        assert!(crate::tile_project::encode(&doc, &mut bytes).is_err());
    }

    #[test]
    fn only_plain_raster_layers_can_hold_text() {
        let mut doc = document();
        let active = doc.active;
        doc.layers[active].vector = Some(Default::default());
        assert!(doc.validate().is_err());
        doc.layers[active].vector = None;
        doc.layers[active].folder = true;
        assert!(doc.validate().is_err());
    }

    #[test]
    fn text_needs_the_newest_header_and_other_projects_do_not() {
        let doc = document();
        assert_eq!(
            crate::tile_project::magic(&doc),
            crate::tile_project::TEXT_MAGIC
        );
        // Text outranks every feature that picks an older header.
        let mut vectors = doc.clone();
        let other = vectors.active - 1;
        vectors.layers[other].vector = Some(Default::default());
        assert_eq!(
            crate::tile_project::magic(&vectors),
            crate::tile_project::TEXT_MAGIC
        );
        let (plain, _) = Edit::Rasterize(doc.layers[doc.active].id)
            .prepare(&doc)
            .unwrap();
        assert_ne!(
            crate::tile_project::magic(&plain),
            crate::tile_project::TEXT_MAGIC
        );
    }

    #[test]
    fn older_builds_cannot_open_a_text_project() {
        let doc = document();
        let mut bytes = Vec::new();
        crate::tile_project::encode(&doc, &mut bytes).unwrap();
        // Every header an earlier build accepts.
        for older in [
            crate::binary_project::MAGIC,
            crate::tile_project::MAGIC,
            crate::tile_project::MASK_MAGIC,
            crate::tile_project::STYLE_MAGIC,
            crate::tile_project::VECTOR_MAGIC,
        ] {
            assert!(!bytes.starts_with(older));
        }
        assert!(crate::binary_project::encode(&doc, &mut Vec::new()).is_err());
    }

    #[test]
    fn projects_and_recovery_keep_the_record_and_the_pixels() {
        let doc = document();
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("text.dudel");
        crate::storage::save_project(&path, &doc).unwrap();
        let back = crate::storage::load_project(&path).unwrap();
        assert_eq!(back.layers[back.active].text, doc.layers[doc.active].text);
        assert_eq!(ink(&back, back.active), ink(&doc, doc.active));

        let recovery = directory.path().join("text.recovery");
        crate::storage::save_recovery(
            &recovery,
            &crate::storage::Recovery {
                document: doc.clone(),
                original_path: Some(path),
            },
        )
        .unwrap();
        let back = crate::storage::read_recovery(&recovery).unwrap().document;
        assert_eq!(back.layers[back.active].text, doc.layers[doc.active].text);
    }

    #[test]
    fn opening_never_needs_the_font() {
        let mut doc = document();
        let active = doc.active;
        doc.layers[active].text.as_mut().unwrap().font = "No Such Font".into();
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("text.dudel");
        crate::storage::save_project(&path, &doc).unwrap();
        let back = crate::storage::load_project(&path).unwrap();
        assert!(ink(&back, back.active) > 0);
        // Changing the words does, and says which font is missing.
        let target = back.layers[back.active].id;
        let mut changed = back.layers[back.active].text.clone().unwrap();
        changed.string = "Other".into();
        let error = Edit::SetText {
            target,
            text: Box::new(changed),
        }
        .prepare(&back)
        .err()
        .unwrap();
        assert!(error.contains("No Such Font"));
    }

    #[test]
    fn editing_redraws_and_only_a_real_change_counts() {
        let doc = document();
        let target = doc.layers[doc.active].id;
        let before = ink(&doc, doc.active);
        let same = doc.layers[doc.active].text.clone().unwrap();
        let (_, outcome) = Edit::SetText {
            target,
            text: Box::new(same),
        }
        .prepare(&doc)
        .unwrap();
        assert!(!outcome.changed);
        let (longer, outcome) = Edit::SetText {
            target,
            text: Box::new(text("Hemlo Hemlo")),
        }
        .prepare(&doc)
        .unwrap();
        assert!(outcome.changed);
        assert!(ink(&longer, longer.active) > before);
        // The source document is untouched, which is what undo restores.
        assert_eq!(ink(&doc, doc.active), before);
    }

    #[test]
    fn locked_and_non_text_layers_refuse_new_words() {
        let mut doc = document();
        let active = doc.active;
        let target = doc.layers[active].id;
        doc.layers[active].locked = true;
        let edit = Edit::SetText {
            target,
            text: Box::new(text("Other")),
        };
        assert!(edit.prepare(&doc).is_err());
        doc.layers[active].locked = false;
        let raster = doc.layers[active - 1].id;
        let edit = Edit::SetText {
            target: raster,
            text: Box::new(text("Other")),
        };
        assert!(edit.prepare(&doc).is_err());
    }

    #[test]
    fn invalid_text_never_reaches_the_document() {
        let doc = Document::new(100, 100);
        let mut bad = text("Hemlo");
        bad.size = 0.0;
        let edit = Edit::NewText {
            id: LayerId::new(),
            text: Box::new(bad),
        };
        assert!(edit.prepare(&doc).is_err());
    }

    #[test]
    fn pixel_edits_are_refused_so_words_and_pixels_stay_together() {
        let doc = document();
        let target = doc.layers[doc.active].id;
        assert_eq!(doc.paint_blocker(), Some(BLOCKER));
        assert_eq!(
            crate::paint_target::Target::Raster.blocker(&doc, doc.active, false),
            Some(BLOCKER)
        );
        let refused = [
            Edit::Clear { whole_layer: true },
            Edit::ClearTarget {
                target,
                whole_layer: true,
            },
        ];
        for edit in refused {
            assert_eq!(edit.prepare(&doc).err().as_deref(), Some(BLOCKER));
        }
    }

    /// The layer's pixels must match what its record would draw now.
    fn assert_record_matches_pixels(doc: &Document) {
        let layer = &doc.layers[doc.active];
        let drawn = layer
            .text
            .as_ref()
            .unwrap()
            .render([doc.width, doc.height], &|| false)
            .unwrap();
        let differing = (0..doc.width * doc.height)
            .filter(|p| drawn.rgba(*p) != layer.pixels.rgba(*p))
            .count();
        assert_eq!(differing, 0);
    }

    #[test]
    fn canvas_turns_carry_text_along() {
        for turns in 1..4 {
            let (doc, outcome) = Edit::Rotate(turns).prepare(&document()).unwrap();
            assert!(outcome.changed);
            assert_record_matches_pixels(&doc);
            let (doc, _) = Edit::Rotate(4 - turns).prepare(&doc).unwrap();
            let text = doc.layers[doc.active].text.as_ref().unwrap();
            assert_eq!(text.start, [20.0, 20.0]);
            assert_eq!(text.along, [1.0, 0.0]);
            assert_eq!(text.down, [0.0, 1.0]);
            doc.validate().unwrap();
        }
    }

    #[test]
    fn canvas_size_and_crop_carry_text_along() {
        let source = document();
        let settings = crate::canvas_size::Settings {
            width: 400,
            height: 300,
            anchor: [1, 1],
            dpi: None,
        };
        let (grown, _) = Edit::CanvasSize(settings).prepare(&source).unwrap();
        let offset = settings.offset(source.width, source.height);
        let text = grown.layers[grown.active].text.as_ref().unwrap();
        assert_eq!(
            text.start,
            [20.0 + offset[0] as f64, 20.0 + offset[1] as f64]
        );
        assert_record_matches_pixels(&grown);

        let mut selected = source.clone();
        selected.selection = Some(crate::selection::Selection {
            left: 10,
            top: 5,
            right: 290,
            bottom: 190,
        });
        let (cropped, outcome) = Edit::CropCanvas.prepare(&selected).unwrap();
        assert!(outcome.changed);
        let text = cropped.layers[cropped.active].text.as_ref().unwrap();
        assert_eq!(text.start, [10.0, 15.0]);
        assert_record_matches_pixels(&cropped);
    }

    fn moved(doc: &Document, offset: [i64; 2]) -> Document {
        let target = doc.layers[doc.active].id;
        Edit::MoveTarget { target, offset }.prepare(doc).unwrap().0
    }

    fn transformed(doc: &Document, settings: crate::affine::Settings) -> Document {
        let target = doc.layers[doc.active].id;
        let settings = Box::new(settings);
        Edit::AffineSettingsTarget { target, settings }
            .prepare(doc)
            .unwrap()
            .0
    }

    fn record(doc: &Document) -> &TextLayer {
        doc.layers[doc.active].text.as_ref().unwrap()
    }

    #[test]
    fn moves_keep_text_editable_and_bring_back_what_left_the_canvas() {
        let doc = moved(&document(), [30, 15]);
        assert_eq!(record(&doc).start, [50.0, 35.0]);
        assert_record_matches_pixels(&doc);
        let before = ink(&document(), document().active);
        let gone = moved(&doc, [-120, 0]);
        assert!(ink(&gone, gone.active) < before);
        let back = moved(&gone, [90, -15]);
        assert_eq!(record(&back).start, [20.0, 20.0]);
        assert_eq!(ink(&back, back.active), before, "redrawn from the record");
    }

    #[test]
    fn flips_and_half_turns_keep_text_editable() {
        use crate::transform::Transform;
        for (transform, start, along, down) in [
            (
                Transform::FlipHorizontal,
                [280.0, 20.0],
                [-1.0, 0.0],
                [0.0, 1.0],
            ),
            (
                Transform::FlipVertical,
                [20.0, 180.0],
                [1.0, 0.0],
                [0.0, -1.0],
            ),
            (
                Transform::RotateHalfTurn,
                [280.0, 180.0],
                [-1.0, 0.0],
                [0.0, -1.0],
            ),
        ] {
            let source = document();
            let flipped = Edit::Transform(transform).prepare(&source).unwrap().0;
            let text = record(&flipped);
            assert_eq!((text.start, text.along, text.down), (start, along, down));
            assert_record_matches_pixels(&flipped);
            flipped.validate().unwrap();
        }
    }

    #[test]
    fn uniform_scale_and_rotation_redraw_crisply() {
        let mut settings = crate::affine::Settings {
            scale_percent: [200.0, 200.0],
            ..Default::default()
        };
        let doc = transformed(&document(), settings);
        assert_eq!(record(&doc).size, 80.0);
        assert_record_matches_pixels(&doc);
        settings.scale_percent = [100.0, 100.0];
        settings.angle_degrees = 30.0;
        let doc = transformed(&doc, settings);
        assert_eq!(record(&doc).size, 80.0);
        assert!((record(&doc).along[1] - 0.5).abs() < 1e-9);
        assert_record_matches_pixels(&doc);
    }

    #[test]
    fn boxes_scale_with_their_text() {
        let mut source = document();
        let active = source.active;
        source.layers[active].text.as_mut().unwrap().wrap = Some(100.0);
        let doc = transformed(
            &source,
            crate::affine::Settings {
                scale_percent: [50.0, 50.0],
                ..Default::default()
            },
        );
        assert_eq!(record(&doc).wrap, Some(50.0));
    }

    #[test]
    fn transforms_a_record_cannot_describe_leave_plain_pixels() {
        let stretched = crate::affine::Settings {
            scale_percent: [200.0, 100.0],
            ..Default::default()
        };
        let warped = crate::affine::Settings {
            warp: Some(crate::affine::Warp::Corners([
                [0.0, 0.0],
                [300.0, 20.0],
                [280.0, 200.0],
                [10.0, 180.0],
            ])),
            ..Default::default()
        };
        for settings in [stretched, warped] {
            let doc = transformed(&document(), settings);
            assert!(doc.layers[doc.active].text.is_none());
            assert!(ink(&doc, doc.active) > 0);
        }
    }

    #[test]
    fn a_size_past_the_limit_cannot_stay_text() {
        let mut source = text("Hemlo");
        source.size = 8000.0;
        let double = Matrix([2.0, 0.0, 0.0, 2.0, 0.0, 0.0]);
        assert!(source.mapped(double).is_none());
    }

    #[test]
    fn without_its_font_only_whole_pixel_moves_stay_text() {
        let mut source = document();
        let active = source.active;
        source.layers[active].text.as_mut().unwrap().font = "No Such Font".into();
        let shifted = moved(&source, [10, 0]);
        assert_eq!(record(&shifted).start, [30.0, 20.0]);
        assert!(ink(&shifted, shifted.active) > 0);
        let turned = transformed(
            &source,
            crate::affine::Settings {
                angle_degrees: 30.0,
                ..Default::default()
            },
        );
        assert!(turned.layers[turned.active].text.is_none());
    }

    #[test]
    fn text_refuses_partial_moves_and_mirror_pairs() {
        let mut doc = document();
        let target = doc.layers[doc.active].id;
        let settings = crate::affine::Settings {
            mirror: Some(crate::affine::Mirror {
                center: [150.0, 100.0],
                angle_degrees: 0.0,
            }),
            ..Default::default()
        };
        let edit = Edit::AffineSettingsTarget {
            target,
            settings: Box::new(settings),
        };
        assert_eq!(edit.prepare(&doc).err().as_deref(), Some(BLOCKER));
        doc.selection = Some(crate::selection::Selection {
            left: 0,
            top: 0,
            right: 50,
            bottom: 50,
        });
        for edit in [
            Edit::MoveTarget {
                target,
                offset: [1, 0],
            },
            Edit::Transform(crate::transform::Transform::FlipHorizontal),
        ] {
            assert_eq!(edit.prepare(&doc).err().as_deref(), Some(DESELECT));
        }
    }

    /// The rectangle `doc`-sized pixels have ink in.
    fn ink_box(text: &TextLayer, canvas: [usize; 2]) -> [usize; 4] {
        let pixels = text.render(canvas, &|| false).unwrap();
        let mut found = [usize::MAX, usize::MAX, 0, 0];
        for y in 0..canvas[1] {
            for x in 0..canvas[0] {
                if pixels.rgba(y * canvas[0] + x)[3] > 64 {
                    found = [
                        found[0].min(x),
                        found[1].min(y),
                        found[2].max(x),
                        found[3].max(y),
                    ];
                }
            }
        }
        found
    }

    #[test]
    fn imported_layout_is_kept_until_editing_and_then_stays_put() {
        for (align, lines) in [
            (Align::Left, "Hemlo\nHemlo"),
            (Align::Center, "Hemlo\nHe"),
            (Align::Right, "Hemlo\nHe"),
        ] {
            let mut imported = text(lines);
            imported.align = align;
            imported.start = [40.0, 30.0];
            imported.wrap = Some(220.0);
            // Lines a little apart, as CSP lays them out.
            imported.csp = Some([110.0, 120.0]);
            let released = imported.released();
            assert!(released.csp.is_none());
            released.validate().unwrap();
            let [before, after] = [&imported, &released].map(|t| ink_box(t, [320, 240]));
            for side in 0..4 {
                assert!(
                    before[side].abs_diff(after[side]) <= 4,
                    "{align:?} {before:?} {after:?}"
                );
            }
        }
    }

    #[test]
    fn a_box_keeps_its_size_and_grows_with_its_text() {
        let mut boxed = text("Hemlo");
        boxed.wrap = Some(200.0);
        boxed.box_height = Some(150.0);
        assert_eq!(boxed.extent().unwrap(), [200.0, 150.0]);
        boxed.string = "Hemlo\n".repeat(10);
        assert!(boxed.extent().unwrap()[1] > 150.0);
        let double = Matrix([2.0, 0.0, 0.0, 2.0, 0.0, 0.0]);
        assert_eq!(boxed.mapped(double).unwrap().box_height, Some(300.0));
    }

    #[test]
    fn every_character_has_a_caret_on_its_line() {
        let mut wrapped = text("one two\nthree");
        wrapped.wrap = Some(90.0);
        let carets = wrapped.carets().unwrap();
        assert_eq!(carets.len(), "one two\nthree".chars().count() + 1);
        assert_eq!(carets[0].along, 0.0);
        assert!(carets[1].along > 0.0);
        let tops: Vec<f64> = carets.iter().map(|c| c.top).collect();
        assert!(tops.windows(2).all(|t| t[0] <= t[1]), "lines run downward");
        assert!(carets.last().unwrap().top > carets[0].top);
    }

    #[test]
    fn text_of_our_own_is_already_released() {
        assert_eq!(text("Hemlo").released(), text("Hemlo"));
    }

    #[test]
    fn its_mask_and_other_layers_stay_editable() {
        let mut doc = document();
        let active = doc.active;
        let mask = LayerId::new();
        doc.version = 2;
        doc.layers[active].mask = Some(crate::layer_mask::LayerMask {
            id: mask,
            enabled: true,
            linked: true,
            coverage: crate::layer_mask::Mask::new(doc.width, doc.height, 255).unwrap(),
        });
        assert!(
            crate::paint_target::Target::Mask(mask)
                .blocker(&doc, active, false)
                .is_none()
        );
        let (doc, _) = Edit::SelectLayer(active - 1).prepare(&doc).unwrap();
        assert!(doc.paint_blocker().is_none());
    }

    #[test]
    fn rasterizing_keeps_the_pixels_and_drops_the_words() {
        let doc = document();
        let target = doc.layers[doc.active].id;
        let (flat, outcome) = Edit::Rasterize(target).prepare(&doc).unwrap();
        assert!(outcome.changed);
        assert!(flat.layers[flat.active].text.is_none());
        assert_eq!(ink(&flat, flat.active), ink(&doc, doc.active));
        assert!(flat.paint_blocker().is_none());
    }

    #[test]
    fn merging_leaves_plain_pixels() {
        let doc = document();
        let (merged, _) = Edit::Merge.prepare(&doc).unwrap();
        assert!(merged.layers[merged.active].text.is_none());
        assert!(ink(&merged, merged.active) > 0);
    }

    #[test]
    fn the_queue_draws_text_and_undo_steps_back_through_it() {
        let mut app = crate::Dudel {
            doc: Document::new(300, 200),
            ..Default::default()
        };
        let layers = app.doc.layers.len();
        let id = LayerId::new();
        let new = Edit::NewText {
            id,
            text: Box::new(text("Hemlo")),
        };
        app.dispatch_document_edit(new, "Added", "Not added");
        app.flush_render_queue();
        assert_eq!(app.doc.layers.len(), layers + 1);
        let short = ink(&app.doc, app.doc.active);
        assert!(short > 0);
        // A brush on the layer stops before any stroke begins.
        assert_eq!(app.vector_stroke_blocker(), Some(BLOCKER));

        let set = Edit::SetText {
            target: id,
            text: Box::new(text("Hemlo Hemlo")),
        };
        app.dispatch_document_edit(set, "Changed", "Not changed");
        app.flush_render_queue();
        assert!(ink(&app.doc, app.doc.active) > short);

        app.undo();
        app.flush_render_queue();
        let layer = &app.doc.layers[app.doc.active];
        assert_eq!(layer.text.as_ref().unwrap().string, "Hemlo");
        assert_eq!(ink(&app.doc, app.doc.active), short);

        app.undo();
        app.flush_render_queue();
        assert_eq!(app.doc.layers.len(), layers);
        assert!(app.doc.layers.iter().all(|l| l.text.is_none()));
    }

    #[test]
    fn a_cancelled_drawing_changes_nothing() {
        let mut doc = document();
        let active = doc.active;
        let before = doc.layers[active].text.clone();
        let result = set(&mut doc, active, text("Other"), &|| true);
        assert!(result.is_err());
        assert_eq!(doc.layers[active].text, before);
    }

    #[test]
    fn projected_layers_know_they_hold_text() {
        let doc = document();
        let context = crate::pending_layers::PendingLayers::from_document(&doc);
        assert_eq!(
            context.paint_blocker(doc.layers[doc.active].id),
            Some(BLOCKER)
        );
    }
}
