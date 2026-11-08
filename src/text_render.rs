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
    /// The name it is listed and saved by, and its family and style.
    name: String,
    family: String,
    style: String,
    weight: u16,
    italic: bool,
}

/// One face of a family, as the style menu lists it.
pub(crate) struct Styled {
    /// The full name, which text records keep.
    pub(crate) name: String,
    pub(crate) style: String,
    pub(crate) weight: u16,
    pub(crate) italic: bool,
}

/// A font family and its faces, regular first.
pub(crate) struct Family {
    pub(crate) name: String,
    pub(crate) faces: Vec<Styled>,
}

impl Family {
    /// The face closest to a weight and slant.
    pub(crate) fn closest(&self, weight: u16, italic: bool) -> &Styled {
        self.faces
            .iter()
            .min_by_key(|face| (face.italic != italic, face.weight.abs_diff(weight)))
            .expect("a family has a face")
    }
}

/// Installed fonts by family, sorted. Empty until the index is ready.
pub(crate) fn families() -> &'static [Family] {
    static FAMILIES: OnceLock<Vec<Family>> = OnceLock::new();
    let Some(faces) = FACES.get() else {
        return &[];
    };
    FAMILIES.get_or_init(|| {
        let mut families: std::collections::BTreeMap<String, Family> = Default::default();
        for face in faces {
            let family = families
                .entry(face.family.to_lowercase())
                .or_insert_with(|| Family {
                    name: face.family.clone(),
                    faces: Vec::new(),
                });
            if !family.faces.iter().any(|f| f.name == face.name) {
                family.faces.push(Styled {
                    name: face.name.clone(),
                    style: face.style.clone(),
                    weight: face.weight,
                    italic: face.italic,
                });
            }
        }
        let mut families: Vec<Family> = families.into_values().collect();
        for family in &mut families {
            family
                .faces
                .sort_by_key(|f| (f.italic, f.weight.abs_diff(400), f.weight));
        }
        families
    })
}

/// The family a font belongs to, and which of its faces it is.
pub(crate) fn family_of(font: &str) -> Option<(&'static Family, usize)> {
    let face = FACES
        .get()?
        .iter()
        .find(|face| face.names.iter().any(|n| n == font))?;
    let family = families().iter().find(|f| f.name == face.family)?;
    let index = family.faces.iter().position(|f| f.name == face.name)?;
    Some((family, index))
}

/// A loaded font and its face within the file, which shaping needs.
#[derive(Debug)]
pub(crate) struct TextFont {
    font: FontVec,
    index: u32,
}

impl std::ops::Deref for TextFont {
    type Target = FontVec;
    fn deref(&self) -> &FontVec {
        &self.font
    }
}

struct Loaded {
    path: PathBuf,
    index: u32,
    bytes: usize,
    font: Arc<TextFont>,
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
        let families = named(ttf_parser::name_id::FAMILY);
        for family in &families {
            names.extend(styles.iter().map(|style| format!("{family} {style}")));
        }
        // Menus show English names where there are some: fonts carry names in
        // several languages, and "Negreta" is no use to most artists. The
        // typographic family groups faces like "Arial Black" with Arial.
        let english_named = |id: u16| -> Vec<String> {
            face.names()
                .into_iter()
                .filter(|n| n.name_id == id && n.language().primary_language() == "English")
                .filter_map(|n| n.to_string())
                .collect()
        };
        let english = |id: u16, found: Vec<String>| {
            let ascii = found.iter().find(|n| n.is_ascii()).cloned();
            english_named(id)
                .into_iter()
                .next()
                .or(ascii)
                .or_else(|| found.into_iter().next())
        };
        let Some(name) = english(ttf_parser::name_id::FULL_NAME, names.clone()) else {
            continue;
        };
        let typographic = ttf_parser::name_id::TYPOGRAPHIC_FAMILY;
        let family = english(typographic, named(typographic))
            .or_else(|| english(ttf_parser::name_id::FAMILY, families))
            .unwrap_or_else(|| name.clone());
        let typographic = ttf_parser::name_id::TYPOGRAPHIC_SUBFAMILY;
        let style = english(typographic, named(typographic))
            .or_else(|| english(ttf_parser::name_id::SUBFAMILY, styles))
            .unwrap_or_else(|| "Regular".into());
        faces.push(Face {
            path: path.to_owned(),
            index,
            names,
            name,
            family,
            style,
            weight: face.weight().to_number(),
            italic: face.is_italic(),
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

fn load(face: &Face) -> Option<Arc<TextFont>> {
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
    let font = Arc::new(TextFont {
        font: FontVec::try_from_vec_and_index(data, face.index).ok()?,
        index: face.index,
    });
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

/// Installed fonts that fill in characters a text's own font lacks, in the order
/// they are tried. Japanese comes first: most Latin fonts have none of it.
const FALLBACKS: [&str; 10] = [
    "Yu Gothic Medium",
    "Yu Gothic",
    "Meiryo",
    "Noto Sans JP",
    "Noto Sans CJK JP",
    "Microsoft YaHei",
    "Malgun Gothic",
    "Noto Sans CJK SC",
    "Segoe UI Symbol",
    "DejaVu Sans",
];

/// The regular face of a font family, or any face of it.
fn family(name: &str) -> Option<Arc<TextFont>> {
    let faces = faces();
    let regular = format!("{name} Regular");
    let prefix = format!("{name} ");
    let named = |test: &dyn Fn(&str) -> bool| {
        faces
            .iter()
            .find(|face| face.names.iter().any(|n| test(n.as_str())))
    };
    load(named(&|n| n == name || n == regular).or_else(|| named(&|n| n.starts_with(&prefix)))?)
}

/// The installed fonts that can draw what `font` can't in `string`.
pub(crate) fn fallbacks(string: &str, font: &FontVec) -> Vec<Arc<TextFont>> {
    let mut found: Vec<Arc<TextFont>> = Vec::new();
    for c in missing_characters(string, font) {
        if found.iter().any(|font| font.glyph_id(c).0 != 0) {
            continue;
        }
        let covering = FALLBACKS
            .iter()
            .filter_map(|name| family(name))
            .find(|font| font.glyph_id(c).0 != 0);
        found.extend(covering);
    }
    found
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

/// Tests name this font to get `test_font`, so that they don't depend on what
/// the machine has installed.
#[cfg(test)]
pub(crate) const TEST_FONT: &str = "Dudel Test Font";

/// The font built into egui, which every platform has.
#[cfg(test)]
pub(crate) fn test_font() -> TextFont {
    let fonts = eframe::egui::FontDefinitions::default();
    let data = fonts.font_data["Ubuntu-Light"].font.to_vec();
    TextFont {
        font: FontVec::try_from_vec(data).unwrap(),
        index: 0,
    }
}

/// The installed font whose full name (or family and style) is `name`.
pub(crate) fn find_font(name: &str) -> Option<Arc<TextFont>> {
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
    /// Lines of a box stretch to fill it, all but each paragraph's last.
    Justify,
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
    /// Glyphs stand facing along `down` rather than against it, so that a
    /// frame whose lines stack the other way doesn't mirror them.
    pub(crate) rightward: bool,
    /// Every character stands upright down the line, Latin included.
    pub(crate) upright_letters: bool,
    /// Extra space after each character, in canvas pixels.
    pub(crate) tracking: f32,
    /// Thicken or slant glyphs, for fonts with no bold or italic face.
    pub(crate) faux_bold: bool,
    pub(crate) faux_italic: bool,
    pub(crate) underline: bool,
    pub(crate) strikethrough: bool,
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
    /// How each character is drawn, when some differ from the rest; empty
    /// when every character looks as the fields above say.
    pub(crate) looks: Vec<Look>,
    /// How many bold or italic faces follow the text's own font among its
    /// fonts, before those that fill in missing characters.
    pub(crate) variants: usize,
}

/// How one character of a block is drawn.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Look {
    /// Its font among the block's: 0 for the text's own, or a bold or italic
    /// face of the same family.
    pub(crate) face: usize,
    pub(crate) faux_bold: bool,
    pub(crate) faux_italic: bool,
    pub(crate) underline: bool,
    pub(crate) strikethrough: bool,
    pub(crate) color: [u8; 3],
}

impl Block<'_> {
    /// How character `index` is drawn.
    fn look(&self, index: usize) -> Look {
        self.looks.get(index).copied().unwrap_or(Look {
            face: 0,
            faux_bold: self.faux_bold,
            faux_italic: self.faux_italic,
            underline: self.underline,
            strikethrough: self.strikethrough,
            color: self.color,
        })
    }

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

/// Where the caret stands before a character, in text space.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Caret {
    pub(crate) along: f64,
    pub(crate) top: f64,
    pub(crate) height: f64,
}

/// One glyph, the font it comes from and where it sits in text space. An
/// upright glyph stands across the lines, as Japanese does in vertical text:
/// `pen` is then its origin along the line, and `baseline` across it.
struct Placed {
    face: usize,
    upright: bool,
    /// See `Block::rightward`.
    rightward: bool,
    /// How far a faux italic leans, and how much a faux bold widens strokes.
    slant: f32,
    bold: f32,
    color: [u8; 3],
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

/// Whether a character stands upright in vertical text.
fn upright(c: char) -> bool {
    matches!(c as u32,
        0x1100..=0x11FF       // Hangul jamo
        | 0x2E80..=0x303F     // CJK radicals, symbols and punctuation
        | 0x3040..=0x30FF     // kana
        | 0x3100..=0x318F     // bopomofo and compatibility jamo
        | 0x31A0..=0x4DBF     // kana extensions and CJK extension A
        | 0x4E00..=0x9FFF     // CJK ideographs
        | 0xA960..=0xA97F
        | 0xAC00..=0xD7FF     // Hangul syllables
        | 0xF900..=0xFAFF     // CJK compatibility ideographs
        | 0xFE30..=0xFE4F     // CJK compatibility forms
        | 0xFF00..=0xFFEF     // full and half width forms
        | 0x20000..=0x3FFFF) // CJK extensions
}

/// Whether a character of this block stands upright across its lines.
fn stands(block: &Block, c: char) -> bool {
    block.vertical && (block.upright_letters || upright(c))
}

/// Characters a line may not start with, by the usual Japanese rules.
const NO_LINE_START: &str =
    "、。，．・：；？！ー）」』】〕〉》ぁぃぅぇぉっゃゅょゎァィゥェォッャュョヮゝゞ々";
/// Characters a line may not end with.
const NO_LINE_END: &str = "（「『【〔〈《";

/// Whether a line can break between two characters. Latin text breaks at
/// spaces, and CJK text between characters.
fn may_break(before: char, after: char) -> bool {
    before == ' '
        || ((upright(before) || upright(after))
            && !NO_LINE_START.contains(after)
            && !NO_LINE_END.contains(before))
}

/// A text's font and those behind it, all at one size.
struct Scaled<'a> {
    fonts: Vec<PxScaleFont<&'a FontVec>>,
    /// Where an underline and a strikethrough sit above the baseline, and how
    /// thick they are, in canvas pixels.
    underline: [f32; 2],
    strikethrough: [f32; 2],
    /// One per font, or none when this text isn't shaped.
    shapers: Vec<rustybuzz::Face<'a>>,
    /// See `Block::variants`.
    variants: usize,
}

impl<'a> Scaled<'a> {
    fn new(fonts: &[&'a TextFont], block: &Block) -> Result<Self, String> {
        let scaled = fonts
            .iter()
            .map(|font| Ok(Font::as_scaled(&font.font, font_scale(font, block.em)?)))
            .collect::<Result<Vec<_>, String>>()?;
        if scaled.is_empty() {
            return Err("Unusable font".into());
        }
        let mut shapers: Vec<_> = fonts
            .iter()
            .filter(|_| block.kerning)
            .filter_map(|font| rustybuzz::Face::from_slice(font.as_slice(), font.index))
            .collect();
        if shapers.len() != scaled.len() {
            shapers.clear();
        }
        // The font's own lines, or the usual places when it has none.
        let face = ttf_parser::Face::parse(fonts[0].as_slice(), fonts[0].index).ok();
        let factor = scaled[0].scale_factor().vertical;
        let metrics = |found: Option<ttf_parser::LineMetrics>, position: f32| {
            found
                .filter(|m| m.thickness > 0)
                .map_or([position * block.em, 0.05 * block.em], |m| {
                    [
                        f32::from(m.position) * factor,
                        f32::from(m.thickness) * factor,
                    ]
                })
        };
        let underline = metrics(face.as_ref().and_then(|f| f.underline_metrics()), -0.1);
        let strikethrough = metrics(face.as_ref().and_then(|f| f.strikeout_metrics()), 0.25);
        Ok(Self {
            variants: block.variants.min(scaled.len() - 1),
            fonts: scaled,
            shapers,
            underline,
            strikethrough,
        })
    }
    /// The text's own font, which sets its line spacing.
    fn main(&self) -> &PxScaleFont<&'a FontVec> {
        &self.fonts[0]
    }
    /// The first font that has `c`, or the text's own to show it's missing.
    fn face(&self, c: char) -> usize {
        let has = |index: usize| self.fonts[index].glyph_id(c).0 != 0;
        if has(0) {
            return 0;
        }
        (1 + self.variants..self.fonts.len())
            .find(|&index| has(index))
            .unwrap_or(0)
    }
    /// The font for character `index` of a block: its bold or italic face
    /// when it has one with the character, else as `face` finds.
    fn face_at(&self, block: &Block, c: char, index: usize) -> usize {
        let wanted = block.look(index).face;
        if wanted != 0 && wanted < self.fonts.len() && self.fonts[wanted].glyph_id(c).0 != 0 {
            wanted
        } else {
            self.face(c)
        }
    }
}

/// One shaped glyph. `pen` runs along the line, and `across` is its offset from
/// the baseline, or for an upright glyph from the middle of the line. `char`
/// is the first character of the line it draws.
struct Item {
    face: usize,
    char: usize,
    upright: bool,
    id: GlyphId,
    pen: f32,
    across: f32,
    /// How far the glyph moves the pen, tracking aside.
    advance: f32,
}

/// A line's glyphs, and how far along it each character starts. `pens` has one
/// more entry than there are characters, for the end of the line.
struct Shaped {
    items: Vec<Item>,
    pens: Vec<f32>,
    /// A line laid out in both directions: how long it shows without its
    /// trailing spaces, since its pens no longer run in order.
    bidi_width: Option<f32>,
}

impl Shaped {
    /// The length of the line without its trailing spaces.
    fn used(&self, line: &str) -> f64 {
        match self.bidi_width {
            Some(width) => f64::from(width),
            None => f64::from(self.pens[line.trim_end().chars().count()]),
        }
    }
}

/// Lay a line out character by character from `start`. Imported text comes
/// this way, because CSP doesn't shape. The pen must add up from where the line
/// starts: adding the same widths in another order rounds differently, which
/// is enough to change imported pixels.
fn plain_line(fonts: &Scaled, line: &str, kerning: bool, start: f32, tracking: f32) -> Shaped {
    let mut items = Vec::new();
    let mut pens = Vec::new();
    let mut pen = start;
    let mut last = None;
    for (char, c) in line.chars().enumerate() {
        let face = fonts.face(c);
        let font = &fonts.fonts[face];
        let id = font.glyph_id(c);
        // Only glyphs of one font have kerning between them.
        if let Some((_, last)) = last.filter(|(previous, _)| kerning && *previous == face) {
            pen += font.kern(last, id);
        }
        last = Some((face, id));
        pens.push(pen);
        items.push(Item {
            face,
            char,
            upright: false,
            id,
            pen,
            across: 0.0,
            advance: font.h_advance(id),
        });
        pen += font.h_advance(id) + tracking;
    }
    pens.push(pen);
    Shaped {
        items,
        pens,
        bidi_width: None,
    }
}

/// Shape a line, one run of a single font and direction at a time. `first`
/// is the line's first character in the whole block.
fn shaped_line(fonts: &Scaled, line: &str, block: &Block, first: usize) -> Shaped {
    let chars: Vec<char> = line.chars().collect();
    let run_of = |i: usize| {
        (
            fonts.face_at(block, chars[i], first + i),
            stands(block, chars[i]),
        )
    };
    let mut starts = vec![None; chars.len()];
    let mut items = Vec::new();
    let mut pen = 0.0f32;
    let mut at = 0;
    while at < chars.len() {
        let (face, stands) = run_of(at);
        // Spaces join the run they follow.
        let end = (at + 1..chars.len())
            .find(|&i| !chars[i].is_whitespace() && run_of(i) != (face, stands))
            .unwrap_or(chars.len());
        let text: String = chars[at..end].iter().collect();
        let mut buffer = rustybuzz::UnicodeBuffer::new();
        buffer.push_str(&text);
        buffer.guess_segment_properties();
        if stands {
            // Vertical shaping also brings in the fonts' vertical forms.
            buffer.set_direction(rustybuzz::Direction::TopToBottom);
        }
        let output = rustybuzz::shape(&fonts.shapers[face], &[], buffer);
        let factor = fonts.fonts[face].scale_factor().horizontal;
        let bytes: Vec<usize> = text.char_indices().map(|(byte, _)| byte).collect();
        for (info, place) in output.glyph_infos().iter().zip(output.glyph_positions()) {
            let [x, y] = [place.x_offset, place.y_offset].map(|v| v as f32 * factor);
            let char_at = bytes.partition_point(|byte| *byte < info.cluster as usize);
            let char = (at + char_at).min(chars.len() - 1);
            starts[char].get_or_insert(pen);
            let id = GlyphId(info.glyph_id as u16);
            let (glyph_pen, across, advance) = if stands {
                (pen - y, -x, -place.y_advance as f32 * factor)
            } else {
                (pen + x, -y, place.x_advance as f32 * factor)
            };
            items.push(Item {
                face,
                char,
                upright: stands,
                id,
                pen: glyph_pen,
                across,
                advance,
            });
            pen += advance + tracking_of(block, block.look(first + char));
        }
        at = end;
    }
    // Characters inside a ligature have no glyph of their own.
    let mut pens = Vec::with_capacity(chars.len() + 1);
    let mut last = 0.0;
    for start in starts {
        last = start.unwrap_or(last);
        pens.push(last);
    }
    pens.push(pen);
    Shaped {
        items,
        pens,
        bidi_width: None,
    }
}

/// Whether a line has anything written right to left. Only such lines need
/// the bidirectional algorithm; every other line is laid out as before.
pub(crate) fn has_right_to_left(line: &str) -> bool {
    use unicode_bidi::BidiClass::{AL, R, RLE, RLI, RLO};
    line.chars()
        .any(|c| matches!(unicode_bidi::bidi_class(c), R | AL | RLE | RLO | RLI))
}

/// Shape a line that mixes directions. The Unicode bidirectional algorithm
/// puts its runs in the order they show; each run is shaped in its own
/// direction, so Hebrew or Arabic reads right to left while Latin words and
/// numbers among it still read left to right. The caret before a character
/// stands on the side reading starts from. Trailing spaces stay at the end.
fn bidi_line(fonts: &Scaled, line: &str, block: &Block, first: usize) -> Option<Shaped> {
    let chars: Vec<char> = line.chars().collect();
    let content = line.trim_end().chars().count();
    let text: String = chars[..content].iter().collect();
    let info = unicode_bidi::BidiInfo::new(&text, None);
    let [paragraph] = info.paragraphs.as_slice() else {
        return None;
    };
    let (levels, runs) = info.visual_runs(paragraph, paragraph.range.clone());
    let mut char_at = vec![content; text.len() + 1];
    for (index, (byte, _)) in text.char_indices().enumerate() {
        char_at[byte] = index;
    }
    let run_of = |i: usize| {
        (
            fonts.face_at(block, chars[i], first + i),
            stands(block, chars[i]),
        )
    };
    let mut items = Vec::new();
    let mut lead = vec![None; chars.len()];
    let mut pen = 0.0f32;
    let shaping = Shaping {
        fonts,
        block,
        chars: &chars,
        first,
    };
    for run in runs {
        let rtl = levels[run.start].is_rtl();
        let (from, to) = (char_at[run.start], char_at[run.end]);
        // Pieces of one font; a right-to-left run shows its last piece first.
        let mut pieces = Vec::new();
        let mut at = from;
        while at < to {
            let key = run_of(at);
            let end = (at + 1..to)
                .find(|&i| !chars[i].is_whitespace() && run_of(i) != key)
                .unwrap_or(to);
            pieces.push(at..end);
            at = end;
        }
        if rtl {
            pieces.reverse();
        }
        for range in pieces {
            pen = shape_piece(&shaping, range, rtl, pen, &mut items, &mut lead);
        }
    }
    let width = pen;
    if content < chars.len() {
        let trailing = content..chars.len();
        pen = shape_piece(&shaping, trailing, false, pen, &mut items, &mut lead);
    }
    // A right-to-left line begins at its right and ends at its left.
    let rtl_line = paragraph.level.is_rtl();
    let mut pens = Vec::with_capacity(chars.len() + 1);
    let mut last = if rtl_line { width } else { 0.0 };
    for start in lead {
        last = start.unwrap_or(last);
        pens.push(last);
    }
    pens.push(if rtl_line && content == chars.len() {
        0.0
    } else {
        pen
    });
    Some(Shaped {
        items,
        pens,
        bidi_width: Some(width),
    })
}

/// A line being shaped a piece at a time.
struct Shaping<'s, 'a> {
    fonts: &'s Scaled<'a>,
    block: &'s Block<'s>,
    chars: &'s [char],
    /// The line's first character in the whole block.
    first: usize,
}

/// Shape `range` of a line in one direction from `pen`, noting where the
/// caret before each of its characters stands. Returns where the pen ends.
fn shape_piece(
    shaping: &Shaping,
    range: std::ops::Range<usize>,
    rtl: bool,
    mut pen: f32,
    items: &mut Vec<Item>,
    lead: &mut [Option<f32>],
) -> f32 {
    let Shaping {
        fonts,
        block,
        chars,
        first: line_first,
    } = *shaping;
    let first = chars[range.start];
    let (face, stands) = (
        fonts.face_at(block, first, line_first + range.start),
        stands(block, first),
    );
    let text: String = chars[range.clone()].iter().collect();
    let mut buffer = rustybuzz::UnicodeBuffer::new();
    buffer.push_str(&text);
    buffer.guess_segment_properties();
    buffer.set_direction(if stands {
        rustybuzz::Direction::TopToBottom
    } else if rtl {
        rustybuzz::Direction::RightToLeft
    } else {
        rustybuzz::Direction::LeftToRight
    });
    let output = rustybuzz::shape(&fonts.shapers[face], &[], buffer);
    let factor = fonts.fonts[face].scale_factor().horizontal;
    let bytes: Vec<usize> = text.char_indices().map(|(byte, _)| byte).collect();
    for (info, place) in output.glyph_infos().iter().zip(output.glyph_positions()) {
        let [x, y] = [place.x_offset, place.y_offset].map(|v| v as f32 * factor);
        let char_at = bytes.partition_point(|byte| *byte < info.cluster as usize);
        let char = (range.start + char_at).min(range.end - 1);
        let (glyph_pen, across, advance) = if stands {
            (pen - y, -x, -place.y_advance as f32 * factor)
        } else {
            (pen + x, -y, place.x_advance as f32 * factor)
        };
        let caret = &mut lead[char];
        *caret = Some(if rtl {
            caret.map_or(pen + advance, |c: f32| c.max(pen + advance))
        } else {
            caret.unwrap_or(pen)
        });
        items.push(Item {
            face,
            char,
            upright: stands,
            id: GlyphId(info.glyph_id as u16),
            pen: glyph_pen,
            across,
            advance,
        });
        pen += advance + tracking_of(block, block.look(line_first + char));
    }
    pen
}

/// The space after each character, which a faux bold widens.
fn tracking(block: &Block) -> f32 {
    block.tracking
        + if block.faux_bold {
            bold_width(block)
        } else {
            0.0
        }
}

/// The space after a character drawn with `look`.
fn tracking_of(block: &Block, look: Look) -> f32 {
    block.tracking
        + if look.faux_bold {
            bold_width(block)
        } else {
            0.0
        }
}

/// How much a faux bold thickens each stroke.
fn bold_width(block: &Block) -> f32 {
    block.em * 0.03
}

/// Shape a line whose first character is `first` in the whole block.
fn shape(block: &Block, fonts: &Scaled, line: &str, first: usize) -> Shaped {
    if fonts.shapers.is_empty() {
        plain_line(fonts, line, block.kerning, 0.0, tracking(block))
    } else {
        has_right_to_left(line)
            .then(|| bidi_line(fonts, line, block, first))
            .flatten()
            .unwrap_or_else(|| shaped_line(fonts, line, block, first))
    }
}

/// Add a paragraph's lines, wrapped at spaces as CSP does.
fn wrap_at_spaces(block: &Block, fonts: &Scaled, paragraph: &str, lines: &mut Vec<String>) {
    let mut line = String::new();
    for word in paragraph.split_inclusive(' ') {
        let candidate = format!("{line}{word}");
        let length = plain_line(fonts, candidate.trim_end(), block.kerning, 0.0, 0.0).pens;
        if !line.is_empty() && f64::from(*length.last().unwrap()) > block.wrap {
            lines.push(std::mem::take(&mut line));
            line = word.to_owned();
        } else {
            line = candidate;
        }
    }
    lines.push(line);
}

/// Add a paragraph's lines, wrapped wherever a line may break. Each piece is
/// measured on its own, which is close enough to choose a break and doesn't
/// shape the line over again at every one.
/// `first` is the paragraph's first character in the whole block.
fn wrap_anywhere(
    block: &Block,
    fonts: &Scaled,
    paragraph: &str,
    first: usize,
    lines: &mut Vec<String>,
) {
    let chars: Vec<char> = paragraph.chars().collect();
    let widths: Vec<f64> = chars
        .iter()
        .enumerate()
        .map(|(i, &c)| {
            let advance = if stands(block, c) {
                f64::from(block.em)
            } else {
                let font = &fonts.fonts[fonts.face_at(block, c, first + i)];
                f64::from(font.h_advance(font.glyph_id(c)))
            };
            advance + f64::from(tracking_of(block, block.look(first + i)))
        })
        .collect();
    // A piece's length, and without its trailing spaces.
    let measure = |range: std::ops::Range<usize>, trimmed: bool| -> f64 {
        let mut end = range.end;
        while trimmed && end > range.start && chars[end - 1].is_whitespace() {
            end -= 1;
        }
        widths[range.start..end].iter().sum()
    };
    let mut pieces = Vec::new();
    let mut start = 0;
    for i in 0..chars.len() {
        if chars
            .get(i + 1)
            .is_some_and(|next| may_break(chars[i], *next))
        {
            pieces.push(start..i + 1);
            start = i + 1;
        }
    }
    pieces.push(start..chars.len());
    // A word too long for a line by itself breaks wherever it reaches the edge.
    let pieces = pieces.into_iter().flat_map(|piece| {
        if measure(piece.clone(), true) > block.wrap {
            piece.map(|i| i..i + 1).collect()
        } else {
            vec![piece]
        }
    });
    let mut line = String::new();
    let mut length = 0.0;
    for piece in pieces {
        if !line.is_empty() && length + measure(piece.clone(), true) > block.wrap {
            lines.push(std::mem::take(&mut line));
            length = 0.0;
        }
        length += measure(piece.clone(), false);
        line.extend(&chars[piece]);
    }
    lines.push(line);
}

/// A block's lines, shaped, the length they align within and how far apart
/// they sit.
fn arrange(block: &Block, fonts: &Scaled) -> (Vec<String>, Vec<Shaped>, f64, f32) {
    let (lines, shaped, span, line_height, ..) = arrange_paragraphs(block, fonts);
    (lines, shaped, span, line_height)
}

/// `arrange`, with whether each line ends its paragraph and where it starts in the block.
#[allow(clippy::type_complexity)]
fn arrange_paragraphs(
    block: &Block,
    fonts: &Scaled,
) -> (Vec<String>, Vec<Shaped>, f64, f32, Vec<bool>, Vec<usize>) {
    // Lines wrap to the box; trailing spaces never wrap.
    let mut lines = Vec::new();
    let mut ends = Vec::new();
    // Each line's first character in the whole block: wrapping loses none,
    // and a line break is a character of its own.
    let mut firsts = Vec::new();
    let mut at = 0;
    for paragraph in block.string.split('\n') {
        let first = lines.len();
        if !block.wrap.is_finite() {
            lines.push(paragraph.to_owned());
        } else if fonts.shapers.is_empty() {
            wrap_at_spaces(block, fonts, paragraph, &mut lines);
        } else {
            wrap_anywhere(block, fonts, paragraph, at, &mut lines);
        }
        ends.extend((first..lines.len()).map(|line| line + 1 == lines.len()));
        for line in &lines[first..] {
            firsts.push(at);
            at += line.chars().count();
        }
        at += 1;
    }
    let shaped: Vec<Shaped> = lines
        .iter()
        .zip(&firsts)
        .map(|(line, first)| shape(block, fonts, line, *first))
        .collect();
    let (span, line_height) = match block.measure {
        Measure::Given { span, thickness } => (span, (thickness / lines.len() as f64) as f32),
        Measure::Natural { line_spacing } => {
            let span = if block.wrap.is_finite() {
                block.wrap
            } else {
                lines
                    .iter()
                    .zip(&shaped)
                    .map(|(line, shaped)| shaped.used(line))
                    .fold(0.0, f64::max)
            };
            let natural = fonts.main().height() + fonts.main().line_gap();
            (span, natural * line_spacing as f32)
        }
    };
    (lines, shaped, span, line_height, ends, firsts)
}

/// How long and how tall a block is, in canvas pixels.
pub(crate) fn extent(block: &Block, fonts: &[&TextFont]) -> Result<[f64; 2], String> {
    let scaled = Scaled::new(fonts, block)?;
    let (lines, _, span, line_height) = arrange(block, &scaled);
    let gaps = (lines.len() - 1) as f64;
    Ok([
        span,
        f64::from(line_height) * gaps + f64::from(scaled.main().height()),
    ])
}

/// A block's line count, with its font's natural line pitch and glyph height.
pub(crate) fn line_metrics(
    block: &Block,
    fonts: &[&TextFont],
) -> Result<(usize, f64, f64), String> {
    let scaled = Scaled::new(fonts, block)?;
    let (lines, ..) = arrange(block, &scaled);
    let height = f64::from(scaled.main().height());
    let gap = f64::from(scaled.main().line_gap());
    Ok((lines.len(), height + gap, height))
}

/// A line drawn under or through the text, as text-space corners.
type Rule = [[f32; 2]; 4];

/// A rule and its color.
type ColoredRule = (Rule, [u8; 3]);

/// Where every glyph goes in text space, where the caret can stand, and the
/// underlines and strikethroughs.
fn layout(block: &Block, fonts: &Scaled) -> (Vec<Placed>, Vec<Caret>, Vec<ColoredRule>) {
    let scaled = fonts.main();
    let (lines, shaped, span, line_height, ends, firsts) = arrange_paragraphs(block, fonts);
    let mut placed = Vec::new();
    let mut rules = Vec::new();
    // One caret per character and one at the end. Wrapped lines lose no
    // characters, and a line break is a character of its own.
    let mut carets = Vec::new();
    let mut source = block.string.chars().peekable();
    let height = f64::from(scaled.height());
    let side = if block.rightward { 1.0 } else { -1.0 };
    for (index, (line, glyphs)) in lines.iter().zip(shaped).enumerate() {
        let used = glyphs.used(line);
        let offset = match block.align {
            Align::Right => span - used,
            Align::Center => (span - used) / 2.0,
            Align::Left | Align::Justify => 0.0,
        } as f32;
        let top = line_height * index as f32;
        let baseline = match block.measure {
            // The same places in the line, seen from its other side.
            _ if block.vertical && block.rightward => top + 0.12 * block.em,
            _ if block.vertical => top + 0.88 * block.em,
            _ if block.rightward => top - scaled.descent(),
            Measure::Given { .. } => top + line_height + scaled.descent(),
            // Extra spacing goes below a line, so the first stays at the top.
            Measure::Natural { .. } => top + scaled.ascent(),
        };
        let middle = top + 0.5 * block.em;
        // Plain lines start at their offset; shaped ones are moved there.
        let (glyphs, offset) = if fonts.shapers.is_empty() {
            let tracking = tracking(block);
            (
                plain_line(fonts, line, block.kerning, offset, tracking),
                0.0,
            )
        } else {
            (glyphs, offset)
        };
        // A justified line shares out its spare room between words, or
        // between characters where there are no spaces.
        let trimmed: Vec<char> = line.trim_end().chars().collect();
        let spaces = trimmed.iter().filter(|c| **c == ' ').count();
        // Spare room is shared out in reading order, which a line in both
        // directions doesn't show in: it stays as laid out.
        let stretches = block.align == Align::Justify
            && glyphs.bidi_width.is_none()
            && block.wrap.is_finite()
            && !ends[index]
            && trimmed.len() > 1;
        let gaps = if spaces > 0 {
            spaces
        } else {
            trimmed.len().saturating_sub(1)
        };
        let extra = if stretches {
            ((span - used) / gaps as f64).max(0.0) as f32
        } else {
            0.0
        };
        let line_chars: Vec<char> = line.chars().collect();
        let shift = |char: usize| {
            let before = if spaces > 0 {
                line_chars[..char.min(trimmed.len())]
                    .iter()
                    .filter(|c| **c == ' ')
                    .count()
            } else {
                char.min(trimmed.len() - usize::from(!trimmed.is_empty()))
            };
            extra * before as f32
        };
        // Where each character shows along the line, for styled rules.
        let mut reach = vec![None::<[f32; 2]>; line_chars.len()];
        for item in glyphs.items {
            let pen = offset + item.pen + shift(item.char);
            let look = block.look(firsts[index] + item.char);
            if !block.looks.is_empty() {
                let [from, to] = [pen, pen + item.advance];
                let seen = &mut reach[item.char];
                *seen = Some(seen.map_or([from, to], |[a, b]| [a.min(from), b.max(to)]));
            }
            placed.push(Placed {
                face: item.face,
                upright: item.upright,
                rightward: block.rightward,
                slant: if look.faux_italic { 0.21 } else { 0.0 },
                bold: if look.faux_bold {
                    bold_width(block)
                } else {
                    0.0
                },
                color: look.color,
                id: item.id,
                pen,
                baseline: if item.upright { middle } else { baseline }
                    + if block.rightward {
                        -item.across
                    } else {
                        item.across
                    },
            });
        }
        let caret = |pen: f32| Caret {
            along: f64::from(offset + pen),
            top: f64::from(top),
            height,
        };
        let count = glyphs.pens.len() - 1;
        for (char, pen) in glyphs.pens[..count].iter().enumerate() {
            carets.push(caret(*pen + shift(char)));
            source.next();
        }
        let end = glyphs.pens[count] + shift(count);
        // Underlines and strikethroughs run the length of what the line shows.
        let length = if stretches { span as f32 } else { used as f32 };
        let rule = |[from, to]: [f32; 2], [position, thickness]: [f32; 2]| {
            let [near, far] = [position - thickness / 2.0, position + thickness / 2.0]
                .map(|y| baseline + side * y);
            [[from, near], [to, near], [to, far], [from, far]]
        };
        if !trimmed.is_empty() && block.looks.is_empty() {
            let from = offset + glyphs.bidi_width.map_or(glyphs.pens[0], |_| 0.0);
            for (wanted, metrics) in [
                (block.underline, fonts.underline),
                (block.strikethrough, fonts.strikethrough),
            ] {
                if wanted {
                    rules.push((rule([from, from + length], metrics), block.color));
                }
            }
        } else if !trimmed.is_empty() {
            // Each run of characters marked alike in reading order gets one
            // rule in its color, across the room between them too.
            for (wanted, metrics) in [
                ((|l: Look| l.underline) as fn(Look) -> bool, fonts.underline),
                (|l: Look| l.strikethrough, fonts.strikethrough),
            ] {
                let mut run: Option<([f32; 2], [u8; 3])> = None;
                for (char, seen) in reach.iter().enumerate().take(trimmed.len()) {
                    let look = block.look(firsts[index] + char);
                    let marked = wanted(look).then_some(look.color);
                    match (&mut run, marked, seen) {
                        (Some((extent, color)), Some(now), Some([a, b])) if *color == now => {
                            *extent = [extent[0].min(*a), extent[1].max(*b)];
                        }
                        // A character with no glyph of its own joins its run.
                        (Some((_, color)), Some(now), None) if *color == now => {}
                        _ => {
                            if let Some((extent, color)) = run.take() {
                                rules.push((rule(extent, metrics), color));
                            }
                            run = marked.zip(*seen).map(|(color, extent)| (extent, color));
                        }
                    }
                }
                if let Some((extent, color)) = run {
                    rules.push((rule(extent, metrics), color));
                }
            }
        }
        match source.peek() {
            Some('\n') => {
                carets.push(caret(end));
                source.next();
            }
            None => carets.push(caret(end)),
            Some(_) => {}
        }
    }
    (placed, carets, rules)
}

/// Where the caret stands before each character of a block, and at its end.
pub(crate) fn carets(block: &Block, fonts: &[&TextFont]) -> Result<Vec<Caret>, String> {
    Ok(layout(block, &Scaled::new(fonts, block)?).1)
}

/// How much of each pixel the text covers, over the part of the canvas it
/// reaches.
struct Coverage {
    rect: [usize; 4],
    cells: Vec<f32>,
    /// For text in more than one color: each pixel's colors weighted by how
    /// much each covers it, and the total weight.
    paint: Option<Vec<[f32; 4]>>,
}

impl Coverage {
    fn add(&mut self, x: usize, y: usize, value: f32, color: [u8; 3]) {
        let [left, top, right, bottom] = self.rect;
        if (left..right).contains(&x) && (top..bottom).contains(&y) {
            let index = (y - top) * (right - left) + x - left;
            let cell = &mut self.cells[index];
            *cell = (*cell + value).min(1.0);
            if let Some(paint) = &mut self.paint {
                let [r, g, b] = color.map(f32::from);
                let mix = &mut paint[index];
                *mix = [
                    mix[0] + r * value,
                    mix[1] + g * value,
                    mix[2] + b * value,
                    mix[3] + value,
                ];
            }
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

impl Placed {
    /// Where a point of the glyph's outline falls in text space, with the
    /// glyph moved `nudge` along its own width: a faux bold draws it more than
    /// once.
    fn text_point_moved(&self, p: Point, factor: PxScaleFactor, nudge: f32) -> (f32, f32) {
        let y = p.y * factor.vertical;
        let x = p.x * factor.horizontal + y * self.slant + nudge;
        let side = if self.rightward { 1.0 } else { -1.0 };
        if self.upright {
            (self.pen - y, self.baseline + side * x)
        } else {
            (self.pen + x, self.baseline + side * y)
        }
    }
}

impl Placed {
    /// Whether the glyph must be drawn from its outline rather than from its
    /// upright pixels.
    fn needs_outline(&self) -> bool {
        self.upright || self.rightward || self.slant != 0.0 || self.bold != 0.0
    }
}

/// Fill an underline or strikethrough.
fn draw_rule(block: &Block, rule: &Rule, color: [u8; 3], edges: [f64; 4], coverage: &mut Coverage) {
    let [left, top, right, bottom] = edges;
    let first = top.max(coverage.rect[1] as f64);
    let last = bottom.min(coverage.rect[3] as f64);
    if first >= last || left >= right {
        return;
    }
    let map = |[along, across]: [f32; 2]| {
        let [x, y] = block.canvas(f64::from(along), f64::from(across));
        raster_point((x - left) as f32, (y - first) as f32)
    };
    let mut raster = Rasterizer::new((right - left) as usize, (last - first) as usize);
    for side in 0..4 {
        raster.draw_line(map(rule[side]), map(rule[(side + 1) % 4]));
    }
    raster.for_each_pixel_2d(|x, y, value| {
        let x = left + f64::from(x);
        if x >= 0.0 {
            coverage.add(x as usize, (first + f64::from(y)) as usize, value, color);
        }
    });
}

/// Draw a glyph from its outline, at any angle.
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
    // A faux bold draws the glyph again, moved a little, so strokes thicken.
    let nudges: &[f32] = if placed.bold > 0.0 {
        &[0.0, 0.5, 1.0]
    } else {
        &[0.0]
    };
    let mut raster = Rasterizer::new((right - left) as usize, (last - first) as usize);
    for nudge in nudges {
        let map = |p: Point| {
            let (along, across) = placed.text_point_moved(p, factor, nudge * placed.bold);
            let [x, y] = block.canvas(f64::from(along), f64::from(across));
            raster_point((x - left) as f32, (y - first) as f32)
        };
        let mut pass = Rasterizer::new((right - left) as usize, (last - first) as usize);
        for curve in &outline.curves {
            match *curve {
                OutlineCurve::Line(a, b) => pass.draw_line(map(a), map(b)),
                OutlineCurve::Quad(a, b, c) => pass.draw_quad(map(a), map(b), map(c)),
                OutlineCurve::Cubic(a, b, c, d) => pass.draw_cubic(map(a), map(b), map(c), map(d)),
            }
        }
        if nudges.len() == 1 {
            raster = pass;
            break;
        }
        pass.for_each_pixel_2d(|x, y, value| {
            let x = left + f64::from(x);
            if x >= 0.0 {
                coverage.add(
                    x as usize,
                    (first + f64::from(y)) as usize,
                    value,
                    placed.color,
                );
            }
        });
    }
    if nudges.len() > 1 {
        return;
    }
    raster.for_each_pixel_2d(|x, y, value| {
        let x = left + f64::from(x);
        if x >= 0.0 {
            coverage.add(
                x as usize,
                (first + f64::from(y)) as usize,
                value,
                placed.color,
            );
        }
    });
}

/// The text in its color over the whole canvas.
pub(crate) fn render(
    block: &Block,
    fonts: &[&TextFont],
    canvas: [usize; 2],
    cancelled: &impl Fn() -> bool,
) -> Result<Pixels, String> {
    let [width, height] = canvas;
    let scaled = Scaled::new(fonts, block)?;
    let mut drawn = Vec::new();
    let mut rect = [f64::MAX, f64::MAX, f64::MIN, f64::MIN];
    let (placed_glyphs, _, rules) = layout(block, &scaled);
    let rule_edges: Vec<[f64; 4]> = rules
        .iter()
        .map(|(rule, _)| {
            let [a0, a1] = [rule[0][0], rule[1][0]];
            let [d0, d1] = [rule[0][1].min(rule[2][1]), rule[0][1].max(rule[2][1])];
            reach(
                block,
                ab_glyph::Rect {
                    min: point(a0, d0),
                    max: point(a1, d1),
                },
            )
        })
        .collect();
    for edges in &rule_edges {
        for side in 0..2 {
            rect[side] = rect[side].min(edges[side]);
            rect[side + 2] = rect[side + 2].max(edges[side + 2]);
        }
    }
    for placed in placed_glyphs {
        let font = &scaled.fonts[placed.face];
        let glyph = placed
            .id
            .with_scale_and_position(font.scale, point(placed.pen, placed.baseline));
        let outlined = fonts[placed.face].outline_glyph(glyph);
        let bounds = if placed.needs_outline() {
            // The outline is turned, slanted or thickened, so its box is
            // found from the corners it maps to.
            let Some(outline) = fonts[placed.face].outline(placed.id) else {
                continue;
            };
            let factor = font.scale_factor();
            let [min, max] = [outline.bounds.min, outline.bounds.max];
            let corners = [
                point(min.x, min.y),
                point(max.x, min.y),
                point(min.x, max.y),
                point(max.x, max.y),
            ];
            let mut found = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
            for corner in corners {
                for nudge in [0.0, placed.bold] {
                    let (a, d) = placed.text_point_moved(corner, factor, nudge);
                    found = [
                        found[0].min(a),
                        found[1].min(d),
                        found[2].max(a),
                        found[3].max(d),
                    ];
                }
            }
            ab_glyph::Rect {
                min: point(found[0], found[1]),
                max: point(found[2], found[3]),
            }
        } else {
            let Some(outlined) = &outlined else {
                continue;
            };
            outlined.px_bounds()
        };
        let edges = reach(block, bounds);
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
    // Text all in one color is drawn in it; otherwise each pixel mixes the
    // colors that cover it.
    let mut colors: Vec<[u8; 3]> = drawn.iter().map(|(placed, ..)| placed.color).collect();
    colors.extend(rules.iter().map(|(_, color)| *color));
    colors.sort_unstable();
    colors.dedup();
    let cells = (right - left) * (bottom - top);
    let mut coverage = Coverage {
        rect,
        cells: vec![0.0; cells],
        paint: (colors.len() > 1).then(|| vec![[0.0; 4]; cells]),
    };
    let color = colors.first().copied().unwrap_or(block.color);
    let straight = block.right_angled();
    for (placed, outlined, edges) in &drawn {
        if cancelled() {
            return Err("Text drawing canceled".into());
        }
        // A glyph wholly off the canvas adds nothing, and drawing it anyway
        // was most of the time for large text running past the canvas.
        let [x0, y0, x1, y1] = *edges;
        if x1 <= left as f64 || y1 <= top as f64 || x0 >= right as f64 || y0 >= bottom as f64 {
            continue;
        }
        if !straight || placed.needs_outline() {
            if let Some(outline) = fonts[placed.face].outline(placed.id) {
                let factor = scaled.fonts[placed.face].scale_factor();
                draw_turned(block, placed, &outline, factor, *edges, &mut coverage);
            }
            continue;
        }
        let Some(outlined) = outlined else {
            continue;
        };
        let bounds = outlined.px_bounds();
        outlined.draw(|gx, gy, value| {
            // The middle of this pixel along and across the text.
            let a = f64::from(bounds.min.x + gx as f32) + 0.5;
            let d = f64::from(bounds.min.y + gy as f32) + 0.5;
            let [x, y] = block.canvas(a, d);
            if x >= 0.0 && y >= 0.0 {
                coverage.add(x.floor() as usize, y.floor() as usize, value, placed.color);
            }
        });
    }
    for ((rule, color), edges) in rules.iter().zip(&rule_edges) {
        draw_rule(block, rule, *color, *edges, &mut coverage);
    }
    let mut writer = pixels.writer();
    let mut row = vec![0u8; (right - left) * 4];
    let span = right - left;
    for (band, line) in coverage.cells.chunks_exact(span).enumerate() {
        if line.iter().all(|v| *v == 0.0) {
            continue;
        }
        row.fill(0);
        for (column, (pixel, value)) in row.chunks_exact_mut(4).zip(line).enumerate() {
            let alpha = (value * 255.0).round() as u8;
            if alpha != 0 {
                let [r, g, b] = match &coverage.paint {
                    Some(paint) => {
                        let [r, g, b, weight] = paint[band * span + column];
                        [r, g, b].map(|v| (v / weight).round().clamp(0.0, 255.0) as u8)
                    }
                    None => color,
                };
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
            rightward: false,
            upright_letters: false,
            tracking: 0.0,
            faux_bold: false,
            faux_italic: false,
            underline: false,
            strikethrough: false,
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
            looks: Vec::new(),
            variants: 0,
        }
    }

    fn draw(block: &Block) -> Vec<u8> {
        let pixels = render(block, &[&test_font()], [400, 400], &|| false).unwrap();
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
                let whole = render(&edge, &[&test_font()], [LARGE, LARGE], &|| false).unwrap();
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
        let pens = |kerning| {
            let mut text = block("AVAVAV", 0.0);
            text.kerning = kerning;
            let placed = layout(&text, &Scaled::new(&[&font], &text).unwrap()).0;
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
            let placed = layout(&text, &Scaled::new(&[&font], &text).unwrap()).0;
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
        let placed = layout(&text, &Scaled::new(&[&font], &text).unwrap()).0;
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
        let result = render(&block("Hemlo", 0.0), &[&test_font()], [400, 400], &|| true);
        assert_eq!(result.err().as_deref(), Some("Text drawing canceled"));
    }
}
