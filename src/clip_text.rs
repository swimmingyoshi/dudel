//! CSP text layers. CSP saves a text layer's words and layout but not its
//! pixels, so Dudel draws the line itself with the same system font.
//!
//! `TextLayerAttributes` holds a few sections of character runs and then a
//! run of little-endian (tag, byte length, data) records: 0x1F the font's
//! full name, 0x20 its size in 1/100 pt, 0x21 the direction (16 vertical),
//! 0x22 the color (three 32-bit channels), 0x23 the alignment, 0x2A the
//! text box (left, top, right, bottom) and 0x40
//! the laid-out line as four corners in 1/100 px, from the box's top-left
//! (horizontal) or bottom-left (vertical). The baseline sits one descent in
//! from the line box's far side. Vertical Latin text is turned a quarter
//! counter-clockwise and reads upward. CSP does not kern. Checked against
//! CSP's saved previews at 600 dpi: the owner's centered black Tahoma 18.7
//! pt lines land within a pixel or two (about 1.4 and 2.2 levels off on
//! average); a right-aligned white Tahoma Bold 12 pt line, whose box is
//! shorter than the line, lands 3 px right of and 4 px below CSP's.
use crate::pixels::Pixels;
use ab_glyph::{Font, FontVec, PxScale, ScaleFont, point};

/// One line of text as CSP laid it out.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Text {
    pub(crate) string: String,
    /// The font's full name, such as "Tahoma Regular".
    pub(crate) font: String,
    /// Points.
    pub(crate) size: f64,
    pub(crate) vertical: bool,
    pub(crate) color: [u8; 3],
    /// Where the line starts: the line box's left and top edges for
    /// horizontal text, its left edge and the text's lower end for
    /// vertical text (canvas pixels).
    pub(crate) origin: [f64; 2],
    /// The line box's thickness across the text (pixels).
    pub(crate) thickness: f64,
}

/// Records that must hold the values of the checked samples (one style
/// with default spacing and anti-aliasing); other values keep the layer
/// skipped until a sample shows what they do. Alignment needs no check: the
/// laid-out line already sits where it puts it.
const PLAIN: [(u32, &[u8]); 17] = [
    (0x25, &[0; 4]),
    (0x26, &[2, 0, 0, 0]),
    (0x2B, &[50, 0, 0, 0]),
    (0x2C, &[0x8D, 0x20, 0, 0]),
    (0x2D, &[0; 4]),
    (0x2E, &[0; 4]),
    (0x30, &[1, 0, 0, 0]),
    (0x34, &[1, 0, 0, 0]),
    (0x35, &[1, 0, 0, 0]),
    (0x37, &[0; 4]),
    (0x3D, &[0; 4]),
    (0x3E, &[0; 4]),
    (0x42, &[0; 4]),
    (0x43, &[0xE8, 0x03, 0, 0]),
    (0x44, &[0; 4]),
    (0x45, &[0; 4]),
    (0x49, &[1, 0, 0, 0]),
];

/// The line a text layer shows, or why Dudel cannot draw it yet.
pub(crate) fn parse(string: &[u8], attributes: &[u8]) -> Result<Text, String> {
    let string = std::str::from_utf8(string)
        .map_err(|_| "damaged text")?
        .to_owned();
    if string.is_empty() || string.contains(['\r', '\n']) {
        return Err("text with several lines Dudel does not draw yet".into());
    }
    // The records start at the font name; one style writes each once.
    let start = attributes
        .windows(4)
        .position(|w| w == [0x1F, 0, 0, 0])
        .ok_or("damaged text attributes")?;
    let mut records = std::collections::BTreeMap::new();
    let mut at = start;
    while at + 8 <= attributes.len() {
        let word = |i: usize| u32::from_le_bytes(attributes[i..i + 4].try_into().unwrap());
        let (tag, size) = (word(at), word(at + 4) as usize);
        let data = attributes
            .get(at + 8..at + 8 + size)
            .ok_or("damaged text attributes")?;
        if records.insert(tag, data).is_some() {
            return Err("text with several styles Dudel does not draw yet".into());
        }
        at += 8 + size;
    }
    // 0x4A is missing from files of older CSP versions.
    if PLAIN
        .iter()
        .any(|(tag, value)| records.get(tag) != Some(value))
        || records.get(&0x4A).is_some_and(|value| *value != [0; 4])
    {
        return Err("text styling Dudel does not draw yet".into());
    }
    let int = |tag: u32, index: usize| -> Result<i32, String> {
        records
            .get(&tag)
            .and_then(|data| data.get(index * 4..index * 4 + 4))
            .map(|b| i32::from_le_bytes(b.try_into().unwrap()))
            .ok_or_else(|| "damaged text attributes".to_string())
    };
    let font = std::str::from_utf8(records.get(&0x1F).ok_or("damaged text attributes")?)
        .map_err(|_| "damaged text attributes")?
        .to_owned();
    let size = f64::from(int(0x20, 0)?) / 100.0;
    let color = [int(0x22, 0)?, int(0x22, 1)?, int(0x22, 2)?].map(|v| (v as u32 >> 24) as u8);
    let vertical = match int(0x21, 0)? {
        0 => false,
        16 => true,
        _ => return Err("text direction Dudel does not draw yet".into()),
    };
    let (left, top, bottom) = (int(0x2A, 0)?, int(0x2A, 1)?, int(0x2A, 3)?);
    let mut span = [[f64::INFINITY, f64::NEG_INFINITY]; 2];
    for corner in 0..4 {
        for (axis, range) in span.iter_mut().enumerate() {
            let v = f64::from(int(0x40, corner * 2 + axis)?) / 100.0;
            *range = [range[0].min(v), range[1].max(v)];
        }
    }
    let [[x0, x1], [y0, y1]] = span;
    let (origin, thickness) = if vertical {
        ([f64::from(left) + x0, f64::from(bottom) + y1], x1 - x0)
    } else {
        ([f64::from(left) + x0, f64::from(top) + y0], y1 - y0)
    };
    if !(size > 0.0 && size < 10_000.0 && thickness > 0.0 && origin.iter().all(|v| v.is_finite())) {
        return Err("damaged text attributes".into());
    }
    Ok(Text {
        string,
        font,
        size,
        vertical,
        color,
        origin,
        thickness,
    })
}

/// Font files installed on this system.
fn font_files() -> Vec<std::path::PathBuf> {
    let mut dirs = Vec::new();
    if let Some(windows) = std::env::var_os("WINDIR") {
        dirs.push(std::path::PathBuf::from(windows).join("Fonts"));
    }
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        dirs.push(std::path::PathBuf::from(local).join("Microsoft/Windows/Fonts"));
    }
    if let Some(home) = std::env::var_os("HOME") {
        let home = std::path::PathBuf::from(home);
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
        .map(std::path::PathBuf::from),
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

/// The installed font whose full name (or family and style) is `name`.
pub(crate) fn find_font(name: &str) -> Option<FontVec> {
    for path in font_files() {
        let Ok(data) = std::fs::read(&path) else {
            continue;
        };
        let faces = ttf_parser::fonts_in_collection(&data).unwrap_or(1);
        for index in 0..faces {
            let Ok(face) = ttf_parser::Face::parse(&data, index) else {
                continue;
            };
            // Names come in several languages; any of them may be the one
            // CSP saved.
            let named = |id: u16| -> Vec<String> {
                face.names()
                    .into_iter()
                    .filter(|n| n.name_id == id)
                    .filter_map(|n| n.to_string())
                    .collect()
            };
            let styles = named(ttf_parser::name_id::SUBFAMILY);
            let matches = named(ttf_parser::name_id::FULL_NAME)
                .iter()
                .any(|full| full == name)
                || named(ttf_parser::name_id::FAMILY).iter().any(|family| {
                    styles
                        .iter()
                        .any(|style| format!("{family} {style}") == name)
                });
            if matches {
                return FontVec::try_from_vec_and_index(data, index).ok();
            }
        }
    }
    None
}

/// The text in its color over the whole canvas, at the canvas `dpi`.
pub(crate) fn render(
    text: &Text,
    font: &FontVec,
    dpi: f64,
    canvas: [usize; 2],
    cancelled: &impl Fn() -> bool,
) -> Result<Pixels, String> {
    let [width, height] = canvas;
    // CSP sizes the em square; ab_glyph scales ascent to descent.
    let em = (text.size / 72.0 * dpi) as f32;
    let units = font.units_per_em().ok_or("Unusable font")?;
    let scale = PxScale::from(em * (font.ascent_unscaled() - font.descent_unscaled()) / units);
    let scaled = font.as_scaled(scale);
    // Along the line: pen position; across: distance below the line box's
    // near edge, with the baseline one descent in from its far side.
    let baseline = text.thickness as f32 + scaled.descent();
    let mut coverage = vec![0f32; width * height];
    // CSP's line widths match the plain advances: no kerning.
    let mut pen = 0.0f32;
    for c in text.string.chars() {
        if cancelled() {
            return Err("Text drawing canceled".into());
        }
        let id = scaled.glyph_id(c);
        let glyph = id.with_scale_and_position(scale, point(pen, baseline));
        pen += scaled.h_advance(id);
        let Some(outline) = font.outline_glyph(glyph) else {
            continue;
        };
        let bounds = outline.px_bounds();
        outline.draw(|gx, gy, value| {
            // Line coordinates of this pixel's corner.
            let (along, across) = (bounds.min.x + gx as f32, bounds.min.y + gy as f32);
            let [x, y] = if text.vertical {
                // A quarter turn counter-clockwise: the line runs upward and
                // its top faces left.
                [
                    text.origin[0] + f64::from(across),
                    text.origin[1] - f64::from(along) - 1.0,
                ]
            } else {
                [
                    text.origin[0] + f64::from(along),
                    text.origin[1] + f64::from(across),
                ]
            };
            let (x, y) = (x.round(), y.round());
            if x >= 0.0 && y >= 0.0 && (x as usize) < width && (y as usize) < height {
                let cell = &mut coverage[y as usize * width + x as usize];
                *cell = (*cell + value).min(1.0);
            }
        });
    }
    let mut pixels = Pixels::sparse(width, height);
    let mut writer = pixels.writer();
    let mut row = vec![0u8; width * 4];
    for (y, line) in coverage.chunks_exact(width).enumerate() {
        if line.iter().all(|v| *v == 0.0) {
            continue;
        }
        row.fill(0);
        for (pixel, value) in row.chunks_exact_mut(4).zip(line) {
            let alpha = (value * 255.0).round() as u8;
            if alpha != 0 {
                pixel.copy_from_slice(&[text.color[0], text.color[1], text.color[2], alpha]);
            }
        }
        writer.write_region(width, [0, y, width, y + 1], &row);
    }
    Ok(pixels)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The owner's horizontal sample's records after the font name.
    fn attributes(direction: u32, color: u32) -> Vec<u8> {
        let mut out = vec![0; 8];
        let mut record = |tag: u32, data: &[u8]| {
            out.extend(tag.to_le_bytes());
            out.extend((data.len() as u32).to_le_bytes());
            out.extend(data);
        };
        record(0x1F, b"Tahoma Regular");
        record(0x20, &1870u32.to_le_bytes());
        record(0x21, &direction.to_le_bytes());
        record(0x22, &[color.to_le_bytes(), [0; 4], [0; 4]].concat());
        record(0x23, &2u32.to_le_bytes());
        for (tag, value) in PLAIN {
            record(tag, value);
        }
        record(
            0x2A,
            &[178i32, 36, 1866, 246].map(i32::to_le_bytes).concat(),
        );
        let corners = [11000i32, 0, 157800, 0, 157800, 18800, 11000, 18800];
        record(0x40, &corners.map(i32::to_le_bytes).concat());
        out
    }

    #[test]
    fn plain_single_lines_parse_and_others_wait_for_samples() {
        let text = parse(b"Text Layer Horizontal", &attributes(0, 0)).unwrap();
        assert_eq!(text.font, "Tahoma Regular");
        assert_eq!((text.size, text.vertical), (18.7, false));
        assert_eq!((text.origin, text.thickness), ([288.0, 36.0], 188.0));
        assert!(parse(b"Two\nlines", &attributes(0, 0)).is_err());
        // Colors are 32-bit channels; Dudel keeps their top byte.
        assert_eq!(
            parse(b"Red", &attributes(0, u32::MAX)).unwrap().color,
            [255, 0, 0]
        );
        assert!(parse(b"Sideways", &attributes(32, 0)).is_err());
        let vertical = parse(b"Up", &attributes(16, 0)).unwrap();
        // From the bottom of the box (246) plus the line's lower end (188).
        assert_eq!(
            (vertical.origin, vertical.thickness),
            ([288.0, 434.0], 1468.0)
        );
    }
}
