//! CSP text layers. CSP saves a text layer's words and layout but not its
//! pixels, so Dudel draws the text itself with the same system font, and the
//! layer stays editable. Its record keeps CSP's layout until the first edit.
//!
//! `TextLayerAttributes` holds a few sections of character runs and then a
//! run of little-endian (tag, byte length, data) records: 0x1F the font's
//! full name, 0x20 its size in 1/100 pt, 0x21 the direction (16 vertical),
//! 0x22 the color (three 32-bit channels), 0x23 the alignment (0 left, 1
//! right, 2 centered), 0x2A the text box (left, top, right, bottom), 0x3A
//! the rotation (1800 upright, 0 turned half a turn) and 0x40 the laid-out
//! text as four corners in 1/100 px from the box corner the text starts at:
//! top-left for horizontal text, top-right for vertical (the corner
//! opposite when turned). Lines wrap at spaces to the box and share the
//! laid-out thickness (one font line each across horizontal text, an em or
//! so across vertical). Horizontal baselines sit one descent above each
//! line's bottom; vertical ones sit 0.88 em from the line's top, where
//! vertical layout puts the baseline of turned Latin text. Lines
//! are aligned within the laid-out width. Vertical Latin text reads
//! downward, turned a quarter clockwise. CSP does not kern. Checked against
//! CSP's saved previews at 600 dpi: horizontal, vertical, turned, wrapped,
//! left-, right- and center-aligned Tahoma lines.
use crate::{text_layer::TextLayer, text_render::Align};

/// A text layer as CSP laid it out.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Text {
    pub(crate) string: String,
    /// The font's full name, such as "Tahoma Regular".
    pub(crate) font: String,
    /// Points.
    pub(crate) size: f64,
    pub(crate) color: [u8; 3],
    pub(crate) vertical: bool,
    /// 0 left, 1 right, 2 centered.
    pub(crate) align: u32,
    /// Canvas pixels: where the first line's start and top meet, the unit
    /// vectors along the lines and down across them, the laid-out length
    /// of the longest line, the thickness of all lines together and the
    /// room lines wrap to.
    pub(crate) start: [f64; 2],
    pub(crate) along: [f64; 2],
    pub(crate) down: [f64; 2],
    pub(crate) span: f64,
    pub(crate) thickness: f64,
    pub(crate) wrap: f64,
}

/// Records that must hold the values of the checked samples (one style
/// with default spacing and anti-aliasing); other values keep the layer
/// skipped until a sample shows what they do.
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

/// The text a layer shows, or why Dudel cannot draw it yet.
pub(crate) fn parse(string: &[u8], attributes: &[u8]) -> Result<Text, String> {
    let string = std::str::from_utf8(string)
        .map_err(|_| "damaged text")?
        .replace("\r\n", "\n")
        .replace('\r', "\n");
    if string.trim().is_empty() {
        return Err("empty text".into());
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
    let align = u32::try_from(int(0x23, 0)?)
        .ok()
        .filter(|a| *a <= 2)
        .ok_or("text alignment Dudel does not draw yet")?;
    let vertical = match int(0x21, 0)? {
        0 => false,
        16 => true,
        _ => return Err("text direction Dudel does not draw yet".into()),
    };
    let turned = match int(0x3A, 0)? {
        1800 => false,
        0 => true,
        _ => return Err("rotated text Dudel does not draw yet".into()),
    };
    let [left, top, right, bottom] = [0, 1, 2, 3].map(|i| int(0x2A, i).map(f64::from));
    let (left, top, right, bottom) = (left?, top?, right?, bottom?);
    // Upright, horizontal lines run right and stack down; vertical ones run
    // down and stack leftward. Turned text runs the other way from the
    // opposite corner.
    let (mut along, mut down, mut origin) = if vertical {
        ([0.0, 1.0], [-1.0, 0.0], [right, top])
    } else {
        ([1.0, 0.0], [0.0, 1.0], [left, top])
    };
    if turned {
        along = along.map(|v: f64| -v);
        down = down.map(|v: f64| -v);
        origin = if vertical {
            [left, bottom]
        } else {
            [right, bottom]
        };
    }
    let dot = |a: [f64; 2], b: [f64; 2]| a[0] * b[0] + a[1] * b[1];
    let mut extent = [[f64::INFINITY, f64::NEG_INFINITY]; 2];
    for corner in 0..4 {
        let point = [
            f64::from(int(0x40, corner * 2)?) / 100.0,
            f64::from(int(0x40, corner * 2 + 1)?) / 100.0,
        ];
        for (range, axis) in extent.iter_mut().zip([along, down]) {
            let v = dot(point, axis);
            *range = [range[0].min(v), range[1].max(v)];
        }
    }
    let [[a0, a1], [d0, d1]] = extent;
    let start = [
        origin[0] + along[0] * a0 + down[0] * d0,
        origin[1] + along[1] * a0 + down[1] * d0,
    ];
    let wrap = if vertical { bottom - top } else { right - left };
    if !(size > 0.0 && size < 10_000.0 && a1 > a0 && d1 > d0 && start.iter().all(|v| v.is_finite()))
    {
        return Err("damaged text attributes".into());
    }
    Ok(Text {
        string,
        font,
        size,
        color,
        vertical,
        align,
        start,
        along,
        down,
        span: a1 - a0,
        thickness: d1 - d0,
        wrap: wrap.max(a1 - a0),
    })
}

impl Text {
    /// The layer's text at the canvas `dpi`. CSP sizes the em square, and its
    /// layout stays until the first edit, so the text draws as CSP laid it out.
    pub(crate) fn record(&self, dpi: f64) -> TextLayer {
        TextLayer {
            string: self.string.clone(),
            font: self.font.clone(),
            size: self.size / 72.0 * dpi,
            color: self.color,
            align: match self.align {
                1 => Align::Right,
                2 => Align::Center,
                _ => Align::Left,
            },
            vertical: self.vertical,
            rightward: false,
            letters: Default::default(),
            format: Default::default(),
            wrap: Some(self.wrap),
            box_height: None,
            line_spacing: 1.0,
            start: self.start,
            along: self.along,
            down: self.down,
            csp: Some([self.span, self.thickness]),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Text records after the font name: direction, alignment, rotation,
    /// box and laid-out corners.
    fn attributes(direction: u32, align: u32, rotation: u32, corners: [i32; 8]) -> Vec<u8> {
        let mut out = vec![0; 8];
        let mut record = |tag: u32, data: &[u8]| {
            out.extend(tag.to_le_bytes());
            out.extend((data.len() as u32).to_le_bytes());
            out.extend(data);
        };
        record(0x1F, b"Tahoma Regular");
        record(0x20, &1500u32.to_le_bytes());
        record(0x21, &direction.to_le_bytes());
        record(
            0x22,
            &[0x357B_357Bu32, u32::MAX, 0].map(u32::to_le_bytes).concat(),
        );
        record(0x23, &align.to_le_bytes());
        for (tag, value) in PLAIN {
            record(tag, value);
        }
        record(
            0x2A,
            &[797i32, 552, 956, 971].map(i32::to_le_bytes).concat(),
        );
        record(0x3A, &rotation.to_le_bytes());
        record(0x40, &corners.map(i32::to_le_bytes).concat());
        out
    }

    #[test]
    fn layout_follows_direction_rotation_and_alignment() {
        // The owner's vertical sample: down from the box's top-right.
        let vertical = [-12500, 0, 0, 0, 0, 40800, -12500, 40800];
        let text = parse(b"Vertical", &attributes(16, 0, 1800, vertical)).unwrap();
        assert_eq!(text.color, [53, 255, 0]);
        assert_eq!(
            (text.start, text.along, text.down),
            ([956.0, 552.0], [0.0, 1.0], [-1.0, 0.0])
        );
        assert_eq!((text.span, text.wrap), (408.0, 419.0));
        // Turned half a turn: up from the bottom-left.
        let turned = [12500, 0, 0, 0, 0, -40800, 12500, -40800];
        let text = parse(b"Up", &attributes(16, 2, 0, turned)).unwrap();
        assert_eq!(
            (text.start, text.along, text.down),
            ([797.0, 971.0], [0.0, -1.0], [1.0, 0.0])
        );
        let flat = [1800, 0, 70800, 0, 70800, 15100, 1800, 15100];
        let text = parse(b"one\r\ntwo", &attributes(0, 1, 1800, flat)).unwrap();
        assert_eq!(
            (text.string.as_str(), text.start, text.span),
            ("one\ntwo", [815.0, 552.0], 690.0)
        );
        assert!(parse(b"x", &attributes(0, 3, 1800, flat)).is_err());
        assert!(parse(b"x", &attributes(0, 0, 900, flat)).is_err());
        assert!(parse(b"x", &attributes(32, 0, 1800, flat)).is_err());
    }
}
