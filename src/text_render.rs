//! Drawing text with installed fonts. Text is laid out in its own space (along
//! the lines and down across them), then placed on the canvas: at right angles
//! each pixel of the upright glyph lands on one canvas pixel, and at any other
//! angle the outlines are turned first so that no pixel is missed.
use crate::pixels::Pixels;
use ab_glyph::{
    Font, FontVec, GlyphId, OutlineCurve, Point, PxScale, PxScaleFactor, PxScaleFont, ScaleFont,
    point,
};
use ab_glyph_rasterizer::{Rasterizer, point as raster_point};
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex, Once, OnceLock},
};

/// Loaded fonts kept for reuse, in file bytes; the newest always stays.
const CACHE_BYTES: usize = 64 << 20;

/// One face of an installed font file.
struct Face {
    path: PathBuf,
    index: u32,
    /// Full names, and family and style joined, in every language.
    names: Vec<String>,
}

struct Loaded {
    path: PathBuf,
    index: u32,
    bytes: usize,
    font: Arc<FontVec>,
}

static FACES: OnceLock<Vec<Face>> = OnceLock::new();
static LOADED: Mutex<Vec<Loaded>> = Mutex::new(Vec::new());

/// Font files installed on this system.
fn font_files() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(windows) = std::env::var_os("WINDIR") {
        dirs.push(PathBuf::from(windows).join("Fonts"));
    }
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        dirs.push(PathBuf::from(local).join("Microsoft/Windows/Fonts"));
    }
    if let Some(home) = std::env::var_os("HOME") {
        let home = PathBuf::from(home);
        dirs.extend([
            home.join("Library/Fonts"),
            home.join(".local/share/fonts"),
            home.join(".fonts"),
        ]);
    }
    dirs.extend(
        [
            "/Library/Fonts",
            "/System/Library/Fonts",
            "/usr/share/fonts",
            "/usr/local/share/fonts",
        ]
        .map(PathBuf::from),
    );
    let mut files = Vec::new();
    let mut pending = dirs;
    while let Some(dir) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                pending.push(path);
            } else if path.extension().and_then(|e| e.to_str()).is_some_and(|e| {
                ["ttf", "otf", "ttc", "otc"].contains(&e.to_ascii_lowercase().as_str())
            }) {
                files.push(path);
            }
        }
    }
    files
}

/// The faces in a font file; unreadable files and faces have none.
fn faces_in(path: &Path) -> Vec<Face> {
    let Ok(data) = std::fs::read(path) else {
        return Vec::new();
    };
    let count = ttf_parser::fonts_in_collection(&data).unwrap_or(1);
    let mut faces = Vec::new();
    for index in 0..count {
        let Ok(face) = ttf_parser::Face::parse(&data, index) else {
            continue;
        };
        // Names come in several languages; any of them may be the one a file
        // saved.
        let named = |id: u16| -> Vec<String> {
            face.names()
                .into_iter()
                .filter(|n| n.name_id == id)
                .filter_map(|n| n.to_string())
                .collect()
        };
        let styles = named(ttf_parser::name_id::SUBFAMILY);
        let mut names = named(ttf_parser::name_id::FULL_NAME);
        for family in named(ttf_parser::name_id::FAMILY) {
            names.extend(styles.iter().map(|style| format!("{family} {style}")));
        }
        faces.push(Face {
            path: path.to_owned(),
            index,
            names,
        });
    }
    faces
}

/// Every installed face. The first call reads each font file once, so it
/// belongs off the UI thread; fonts installed afterwards wait for a restart.
fn faces() -> &'static [Face] {
    FACES.get_or_init(|| {
        let files = font_files();
        crate::parallel::run(files.len(), |index| faces_in(&files[index]))
            .into_iter()
            .flatten()
            .flatten()
            .collect()
    })
}

fn load(face: &Face) -> Option<Arc<FontVec>> {
    let cache = || LOADED.lock().unwrap_or_else(|e| e.into_inner());
    {
        let mut loaded = cache();
        let found = loaded
            .iter()
            .position(|l| l.index == face.index && l.path == face.path);
        if let Some(found) = found {
            let entry = loaded.remove(found);
            let font = entry.font.clone();
            loaded.push(entry);
            return Some(font);
        }
    }
    let data = std::fs::read(&face.path).ok()?;
    let bytes = data.len();
    let font = Arc::new(FontVec::try_from_vec_and_index(data, face.index).ok()?);
    let mut loaded = cache();
    loaded.push(Loaded {
        path: face.path.clone(),
        index: face.index,
        bytes,
        font: font.clone(),
    });
    while loaded.len() > 1 && loaded.iter().map(|l| l.bytes).sum::<usize>() > CACHE_BYTES {
        loaded.remove(0);
    }
    Some(font)
}

/// Start reading the font index in the background, once per run. One thread
/// for the life of the app, so it doesn't add up the way short-lived ones do.
pub(crate) fn warm_up() {
    static STARTED: Once = Once::new();
    STARTED.call_once(|| {
        let _ = std::thread::Builder::new()
            .name("font index".into())
            .spawn(|| {
                faces();
            });
    });
}

/// Whether fonts can be looked up without waiting for the index.
pub(crate) fn ready() -> bool {
    cfg!(test) || FACES.get().is_some()
}

/// One name for each installed face, sorted. Empty until the index is ready.
pub(crate) fn font_names() -> &'static [String] {
    static NAMES: OnceLock<Vec<String>> = OnceLock::new();
    let Some(faces) = FACES.get() else {
        return &[];
    };
    NAMES.get_or_init(|| {
        let mut names: Vec<String> = faces
            .iter()
            .filter_map(|face| {
                let ascii = face.names.iter().find(|n| n.is_ascii());
                ascii.or(face.names.first()).cloned()
            })
            .collect();
        names.sort_by_cached_key(|name| name.to_lowercase());
        names.dedup();
        names
    })
}

/// A sensible font for new text.
pub(crate) fn default_font() -> Option<String> {
    let names = font_names();
    [
        "Segoe UI",
        "Arial",
        "Noto Sans",
        "DejaVu Sans",
        "Liberation Sans",
    ]
    .iter()
    .find_map(|wanted| names.iter().find(|name| name == wanted))
    .or(names.first())
    .cloned()
}

/// An installed font's file and its face index in that file.
pub(crate) fn font_file(name: &str) -> Option<(Vec<u8>, u32)> {
    let face = FACES
        .get()?
        .iter()
        .find(|f| f.names.iter().any(|n| n == name))?;
    Some((std::fs::read(&face.path).ok()?, face.index))
}

/// Tests name this font to get `test_font`, so that they don't depend on what
/// the machine has installed.
#[cfg(test)]
pub(crate) const TEST_FONT: &str = "Dudel Test Font";

/// The font built into egui, which every platform has.
#[cfg(test)]
pub(crate) fn test_font() -> FontVec {
    let fonts = eframe::egui::FontDefinitions::default();
    let data = fonts.font_data["Ubuntu-Light"].font.to_vec();
    FontVec::try_from_vec(data).unwrap()
}

/// The installed font whose full name (or family and style) is `name`.
pub(crate) fn find_font(name: &str) -> Option<Arc<FontVec>> {
    #[cfg(test)]
    if name == TEST_FONT {
        return Some(Arc::new(test_font()));
    }
    load(faces().iter().find(|f| f.names.iter().any(|n| n == name))?)
}

/// The characters of `string` that `font` cannot draw, each named once.
pub(crate) fn missing_characters(string: &str, font: &FontVec) -> Vec<char> {
    let mut missing = Vec::new();
    for c in string.chars() {
        if font.glyph_id(c).0 == 0 && !c.is_control() && !c.is_whitespace() && !missing.contains(&c)
        {
            missing.push(c);
        }
    }
    missing
}

#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Align {
    Left,
    Right,
    Center,
}

/// How long and thick a block's lines are.
#[derive(Clone, Copy)]
pub(crate) enum Measure {
    /// Set by whoever laid the text out: the length lines align within and
    /// the thickness of all lines together.
    Given { span: f64, thickness: f64 },
    /// From the font: lines this multiple of its line height apart, aligned
    /// within the wrap width, or the longest line when nothing wraps.
    Natural { line_spacing: f64 },
}

/// Text laid out as a block on the canvas.
pub(crate) struct Block<'a> {
    pub(crate) string: &'a str,
    /// The em square in canvas pixels.
    pub(crate) em: f32,
    pub(crate) color: [u8; 3],
    pub(crate) vertical: bool,
    pub(crate) align: Align,
    /// Canvas pixels: where the first line's start and top meet, the unit
    /// vectors along the lines and down across them, and the room lines wrap
    /// to.
    pub(crate) start: [f64; 2],
    pub(crate) along: [f64; 2],
    pub(crate) down: [f64; 2],
    pub(crate) measure: Measure,
    pub(crate) wrap: f64,
    pub(crate) kerning: bool,
}

impl Block<'_> {
    fn canvas(&self, along: f64, down: f64) -> [f64; 2] {
        [
            self.start[0] + self.along[0] * along + self.down[0] * down,
            self.start[1] + self.along[1] * along + self.down[1] * down,
        ]
    }

    /// True when text-space pixels fall exactly on canvas pixels.
    fn right_angled(&self) -> bool {
        let axis =
            |v: [f64; 2]| (v[0].abs() == 1.0 && v[1] == 0.0) || (v[0] == 0.0 && v[1].abs() == 1.0);
        axis(self.along) && axis(self.down) && self.along[0] * self.down[0] == 0.0
    }
}

/// One glyph and where it sits in text space.
struct Placed {
    id: GlyphId,
    pen: f32,
    baseline: f32,
}

/// The size `font` is drawn at for an em square of `em` pixels: `em` sizes the
/// em square, and ab_glyph scales ascent to descent.
fn font_scale(font: &FontVec, em: f32) -> Result<PxScale, String> {
    let units = font.units_per_em().ok_or("Unusable font")?;
    Ok(PxScale::from(
        em * (font.ascent_unscaled() - font.descent_unscaled()) / units,
    ))
}

/// Length of a line, or of its glyphs so far.
fn advance(font: &PxScaleFont<&FontVec>, line: &str, kerning: bool) -> f32 {
    let mut last = None;
    line.chars()
        .map(|c| {
            let id = font.glyph_id(c);
            let kern = last
                .filter(|_| kerning)
                .map_or(0.0, |last| font.kern(last, id));
            last = Some(id);
            font.h_advance(id) + kern
        })
        .sum()
}

/// A block's lines, the length they align within and how far apart they sit.
fn arrange(block: &Block, scaled: &PxScaleFont<&FontVec>) -> (Vec<String>, f64, f32) {
    // Lines wrap at spaces to the box; trailing spaces never wrap.
    let mut lines = Vec::new();
    for paragraph in block.string.split('\n') {
        let mut line = String::new();
        for word in paragraph.split_inclusive(' ') {
            let candidate = format!("{line}{word}");
            let length = advance(scaled, candidate.trim_end(), block.kerning);
            if !line.is_empty() && f64::from(length) > block.wrap {
                lines.push(std::mem::take(&mut line));
                line = word.to_owned();
            } else {
                line = candidate;
            }
        }
        lines.push(line);
    }
    let length = |line: &String| f64::from(advance(scaled, line.trim_end(), block.kerning));
    let (span, line_height) = match block.measure {
        Measure::Given { span, thickness } => (span, (thickness / lines.len() as f64) as f32),
        Measure::Natural { line_spacing } => {
            let span = if block.wrap.is_finite() {
                block.wrap
            } else {
                lines.iter().map(length).fold(0.0, f64::max)
            };
            let natural = scaled.height() + scaled.line_gap();
            (span, natural * line_spacing as f32)
        }
    };
    (lines, span, line_height)
}

/// How long and how tall a block is, in canvas pixels.
pub(crate) fn extent(block: &Block, font: &FontVec) -> Result<[f64; 2], String> {
    let scaled = font.as_scaled(font_scale(font, block.em)?);
    let (lines, span, line_height) = arrange(block, &scaled);
    let gaps = (lines.len() - 1) as f64;
    Ok([
        span,
        f64::from(line_height) * gaps + f64::from(scaled.height()),
    ])
}

/// Where every glyph goes in text space. Shaping belongs here: nothing after
/// this knows how glyphs were chosen or spaced.
fn layout(block: &Block, font: &FontVec, scale: PxScale) -> Vec<Placed> {
    let scaled = font.as_scaled(scale);
    let (lines, span, line_height) = arrange(block, &scaled);
    let length = |line: &String| f64::from(advance(&scaled, line.trim_end(), block.kerning));
    let mut placed = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        let used = length(line);
        let offset = match block.align {
            Align::Right => span - used,
            Align::Center => (span - used) / 2.0,
            Align::Left => 0.0,
        } as f32;
        let top = line_height * index as f32;
        let baseline = match block.measure {
            _ if block.vertical => top + 0.88 * block.em,
            Measure::Given { .. } => top + line_height + scaled.descent(),
            // Extra spacing goes below a line, so the first stays at the top.
            Measure::Natural { .. } => top + scaled.ascent(),
        };
        let mut pen = offset;
        let mut last = None;
        for c in line.chars() {
            let id = scaled.glyph_id(c);
            if let Some(last) = last.filter(|_| block.kerning) {
                pen += scaled.kern(last, id);
            }
            last = Some(id);
            placed.push(Placed { id, pen, baseline });
            pen += scaled.h_advance(id);
        }
    }
    placed
}

/// How much of each pixel the text covers, over the part of the canvas it
/// reaches.
struct Coverage {
    rect: [usize; 4],
    cells: Vec<f32>,
}

impl Coverage {
    fn add(&mut self, x: usize, y: usize, value: f32) {
        let [left, top, right, bottom] = self.rect;
        if (left..right).contains(&x) && (top..bottom).contains(&y) {
            let cell = &mut self.cells[(y - top) * (right - left) + x - left];
            *cell = (*cell + value).min(1.0);
        }
    }
}

/// The canvas pixels that a text-space rectangle can reach.
fn reach(block: &Block, bounds: ab_glyph::Rect) -> [f64; 4] {
    let [min, max] = [bounds.min, bounds.max].map(|p| [f64::from(p.x), f64::from(p.y)]);
    let corners = [
        block.canvas(min[0], min[1]),
        block.canvas(max[0], min[1]),
        block.canvas(min[0], max[1]),
        block.canvas(max[0], max[1]),
    ];
    let side = |axis: usize, pick: fn(f64, f64) -> f64| {
        corners.iter().map(|c| c[axis]).reduce(pick).unwrap()
    };
    [
        side(0, f64::min).floor() - 1.0,
        side(1, f64::min).floor() - 1.0,
        side(0, f64::max).ceil() + 1.0,
        side(1, f64::max).ceil() + 1.0,
    ]
}

/// Draw a glyph at an angle: its outline is turned, then rasterized.
fn draw_turned(
    block: &Block,
    placed: &Placed,
    outline: &ab_glyph::Outline,
    factor: PxScaleFactor,
    edges: [f64; 4],
    coverage: &mut Coverage,
) {
    let [left, top, right, bottom] = edges;
    let canvas_rows = coverage.rect[1] as f64..coverage.rect[3] as f64;
    // Rows outside the canvas are skipped; columns can't be, because the
    // rasterizer's rows run on into each other.
    let first = top.max(canvas_rows.start);
    let last = bottom.min(canvas_rows.end);
    if first >= last || left >= right {
        return;
    }
    let map = |p: Point| {
        let [x, y] = block.canvas(
            f64::from(placed.pen + p.x * factor.horizontal),
            f64::from(placed.baseline - p.y * factor.vertical),
        );
        raster_point((x - left) as f32, (y - first) as f32)
    };
    let mut raster = Rasterizer::new((right - left) as usize, (last - first) as usize);
    for curve in &outline.curves {
        match *curve {
            OutlineCurve::Line(a, b) => raster.draw_line(map(a), map(b)),
            OutlineCurve::Quad(a, b, c) => raster.draw_quad(map(a), map(b), map(c)),
            OutlineCurve::Cubic(a, b, c, d) => raster.draw_cubic(map(a), map(b), map(c), map(d)),
        }
    }
    raster.for_each_pixel_2d(|x, y, value| {
        let x = left + f64::from(x);
        if x >= 0.0 {
            coverage.add(x as usize, (first + f64::from(y)) as usize, value);
        }
    });
}

/// The text in its color over the whole canvas.
pub(crate) fn render(
    block: &Block,
    font: &FontVec,
    canvas: [usize; 2],
    cancelled: &impl Fn() -> bool,
) -> Result<Pixels, String> {
    let [width, height] = canvas;
    let scale = font_scale(font, block.em)?;
    let scaled = font.as_scaled(scale);
    let factor = scaled.scale_factor();
    let mut drawn = Vec::new();
    let mut rect = [f64::MAX, f64::MAX, f64::MIN, f64::MIN];
    for placed in layout(block, font, scale) {
        let glyph = placed
            .id
            .with_scale_and_position(scale, point(placed.pen, placed.baseline));
        let Some(outlined) = font.outline_glyph(glyph) else {
            continue;
        };
        let edges = reach(block, outlined.px_bounds());
        for side in 0..2 {
            rect[side] = rect[side].min(edges[side]);
            rect[side + 2] = rect[side + 2].max(edges[side + 2]);
        }
        drawn.push((placed, outlined, edges));
    }
    let mut pixels = Pixels::sparse(width, height);
    let clamp = |value: f64, size: usize| value.clamp(0.0, size as f64) as usize;
    let rect = [
        clamp(rect[0], width),
        clamp(rect[1], height),
        clamp(rect[2], width),
        clamp(rect[3], height),
    ];
    let [left, top, right, bottom] = rect;
    if left >= right || top >= bottom {
        return Ok(pixels);
    }
    let mut coverage = Coverage {
        rect,
        cells: vec![0.0; (right - left) * (bottom - top)],
    };
    let straight = block.right_angled();
    for (placed, outlined, edges) in &drawn {
        if cancelled() {
            return Err("Text drawing canceled".into());
        }
        if !straight {
            if let Some(outline) = font.outline(placed.id) {
                draw_turned(block, placed, &outline, factor, *edges, &mut coverage);
            }
            continue;
        }
        let bounds = outlined.px_bounds();
        outlined.draw(|gx, gy, value| {
            // The middle of this pixel along and across the text.
            let a = f64::from(bounds.min.x + gx as f32) + 0.5;
            let d = f64::from(bounds.min.y + gy as f32) + 0.5;
            let [x, y] = block.canvas(a, d);
            if x >= 0.0 && y >= 0.0 {
                coverage.add(x.floor() as usize, y.floor() as usize, value);
            }
        });
    }
    let mut writer = pixels.writer();
    let mut row = vec![0u8; (right - left) * 4];
    for (band, line) in coverage.cells.chunks_exact(right - left).enumerate() {
        if line.iter().all(|v| *v == 0.0) {
            continue;
        }
        row.fill(0);
        for (pixel, value) in row.chunks_exact_mut(4).zip(line) {
            let alpha = (value * 255.0).round() as u8;
            if alpha != 0 {
                let [r, g, b] = block.color;
                pixel.copy_from_slice(&[r, g, b, alpha]);
            }
        }
        let y = top + band;
        writer.write_region(width, [left, y, right, y + 1], &row);
    }
    Ok(pixels)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block<'a>(string: &'a str, angle: f64) -> Block<'a> {
        let (sin, cos) = angle.to_radians().sin_cos();
        Block {
            string,
            em: 60.0,
            color: [10, 20, 30],
            vertical: false,
            align: Align::Left,
            start: [150.0, 150.0],
            along: [cos, sin],
            down: [-sin, cos],
            measure: Measure::Given {
                span: 200.0,
                thickness: 80.0,
            },
            wrap: f64::INFINITY,
            kerning: false,
        }
    }

    fn draw(block: &Block) -> Vec<u8> {
        let pixels = render(block, &test_font(), [400, 400], &|| false).unwrap();
        (0..400 * 400).map(|p| pixels.rgba(p)[3]).collect()
    }

    fn ink(alpha: &[u8]) -> u64 {
        alpha.iter().map(|a| u64::from(*a)).sum()
    }

    #[test]
    fn any_angle_keeps_the_same_ink() {
        let upright = ink(&draw(&block("Hemlo", 0.0)));
        assert!(upright > 100_000);
        for angle in [7.0, 30.0, 45.0, 123.0, 200.0, 331.0] {
            let turned = ink(&draw(&block("Hemlo", angle)));
            let change = turned.abs_diff(upright) as f64 / upright as f64;
            assert!(change < 0.02, "{angle} degrees changed ink by {change}");
        }
    }

    #[test]
    fn turned_text_has_no_holes() {
        // Inside a thick stem every pixel is solid, so a pixel with no ink
        // between two solid ones on a row is a hole.
        let mut thick = block("I", 30.0);
        thick.em = 300.0;
        let alpha = draw(&thick);
        for row in alpha.chunks_exact(400) {
            for x in 1..399 {
                assert!(!(row[x] == 0 && row[x - 1] == 255 && row[x + 1] == 255));
            }
        }
    }

    #[test]
    fn right_angles_are_exact_turns() {
        let upright = draw(&block("Hemlo", 0.0));
        let mut turned = block("Hemlo", 0.0);
        (turned.along, turned.down) = ([0.0, 1.0], [-1.0, 0.0]);
        let turned = draw(&turned);
        // A quarter turn about the start: (150 + a, 150 + d) goes to
        // (150 - d, 150 + a), and pixel columns count from their left edge.
        let mut compared = 0;
        for y in 0..400usize {
            for x in 0..400usize {
                let (a, d) = (x as i64 - 150, y as i64 - 150);
                let (tx, ty) = (149 - d, 150 + a);
                if (0..400).contains(&tx) && (0..400).contains(&ty) {
                    assert_eq!(
                        upright[y * 400 + x],
                        turned[ty as usize * 400 + tx as usize]
                    );
                    compared += usize::from(upright[y * 400 + x] != 0);
                }
            }
        }
        assert!(compared > 1000);
    }

    #[test]
    fn clipped_text_matches_the_same_text_on_a_larger_canvas() {
        const MARGIN: usize = 300;
        const LARGE: usize = 400 + 2 * MARGIN;
        for angle in [0.0, 30.0, 200.0] {
            for start in [
                [-60.0, 150.0],
                [330.0, 150.0],
                [150.0, -60.0],
                [150.0, 340.0],
            ] {
                let mut edge = block("Hemlo", angle);
                edge.start = start;
                let clipped = draw(&edge);
                edge.start = start.map(|v| v + MARGIN as f64);
                let whole = render(&edge, &test_font(), [LARGE, LARGE], &|| false).unwrap();
                for y in 0..400 {
                    for x in 0..400 {
                        let expected = whole.rgba((y + MARGIN) * LARGE + x + MARGIN)[3];
                        let drawn = clipped[y * 400 + x];
                        assert!(
                            drawn.abs_diff(expected) <= 1,
                            "{angle} degrees from {start:?} at {x}, {y}: {drawn}, not {expected}"
                        );
                    }
                }
            }
            let mut away = block("Hemlo", angle);
            away.start = [1000.0, 1000.0];
            assert_eq!(ink(&draw(&away)), 0);
        }
    }

    #[test]
    fn kerning_only_moves_glyphs_when_asked() {
        let font = test_font();
        let scale = font_scale(&font, 60.0).unwrap();
        let pens = |kerning| {
            let mut text = block("AVAVAV", 0.0);
            text.kerning = kerning;
            let placed = layout(&text, &font, scale);
            placed.last().unwrap().pen
        };
        assert!(pens(true) <= pens(false));
    }

    #[test]
    fn natural_lines_follow_the_font_and_alignment() {
        let font = test_font();
        let scale = font_scale(&font, 60.0).unwrap();
        let scaled = font.as_scaled(scale);
        let natural = scaled.height() + scaled.line_gap();
        let lines = |align, line_spacing: f64| {
            let mut text = block("Wide line\nhi", 0.0);
            text.align = align;
            text.measure = Measure::Natural { line_spacing };
            let placed = layout(&text, &font, scale);
            let (first, last) = (placed.first().unwrap(), placed.last().unwrap());
            // The first glyph of the second line.
            let second = &placed[placed.len() - 2];
            (first.baseline, last.baseline, second.pen)
        };
        let (first, second, _) = lines(Align::Left, 1.0);
        assert_eq!(first, scaled.ascent());
        assert!((second - first - natural).abs() < 0.01);
        let (spaced_first, spaced_second, _) = lines(Align::Left, 2.0);
        assert_eq!(spaced_first, first);
        assert!((spaced_second - spaced_first - 2.0 * natural).abs() < 0.01);
        let (_, _, left) = lines(Align::Left, 1.0);
        let (_, _, center) = lines(Align::Center, 1.0);
        let (_, _, right) = lines(Align::Right, 1.0);
        assert_eq!(left, 0.0);
        assert!(center > 0.0);
        assert!((right - 2.0 * center).abs() < 0.01);
    }

    #[test]
    fn natural_lines_wrap_and_align_within_the_box() {
        let font = test_font();
        let scale = font_scale(&font, 60.0).unwrap();
        let mut text = block("one two three", 0.0);
        text.measure = Measure::Natural { line_spacing: 1.0 };
        text.wrap = 150.0;
        text.align = Align::Right;
        let placed = layout(&text, &font, scale);
        let rows: std::collections::BTreeSet<_> =
            placed.iter().map(|p| p.baseline.to_bits()).collect();
        assert!(rows.len() >= 2);
        let scaled = font.as_scaled(scale);
        let end = placed.last().unwrap();
        assert!((end.pen + scaled.h_advance(end.id) - 150.0).abs() < 0.5);
    }

    #[test]
    fn missing_characters_are_named_once() {
        let font = test_font();
        assert_eq!(missing_characters("a b\nc", &font), Vec::<char>::new());
        assert_eq!(
            missing_characters("a\u{1F7D8}b\u{1F7D8}", &font),
            ['\u{1F7D8}']
        );
    }

    #[test]
    fn cancelling_stops_the_drawing() {
        let result = render(&block("Hemlo", 0.0), &test_font(), [400, 400], &|| true);
        assert_eq!(result.err().as_deref(), Some("Text drawing canceled"));
    }
}
