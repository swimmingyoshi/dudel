//! Strict parser for the observed offscreen attribute shapes: an alpha plane
//! with four color channels (RGB layers) or one (Gray layers).
//! Default-color words remain raw; paper and missing-reference semantics are separate.
use std::io::{Cursor, Read};

const PACKING: [u32; 16] = [
    33, 1, 4, 5, 65536, 4, 1024, 1, 256, 65536, 256, 256, 8, 8, 0, 0,
];
pub(crate) const GRAY_PACKING: [u32; 16] = [
    17, 1, 1, 2, 65536, 1, 256, 1, 256, 65536, 256, 256, 8, 8, 0, 0,
];
/// Layer masks: one coverage byte per pixel.
pub(crate) const MASK_PACKING: [u32; 16] = [1, 1, 0, 1, 0, 0, 0, 1, 256, 0, 256, 256, 8, 8, 0, 0];
/// Largest bitmap side. Layers may extend past the canvas (CSP keeps moved
/// pixels), so this exceeds Dudel's canvas side; decoding keeps the part on
/// the canvas.
pub(crate) const MAX_SIDE: u32 = 16384;
pub(crate) const MAX_BYTES: usize = 16 + 102 + 58 + 34 + 64 * 64 * 4;

#[derive(Debug, serde::Serialize)]
pub(crate) struct Attributes {
    pub(crate) width: u32,
    pub(crate) height: u32,
    columns: u32,
    rows: u32,
    packing: [u32; 16],
    init_color_words: Vec<u32>,
    pub(crate) block_sizes: Vec<u32>,
}
impl Attributes {
    pub(crate) fn paper_default(&self) -> bool {
        self.init_color_words == [20, 1, u32::MAX, 4, 4, 0, 0, 0, 0]
    }
    pub(crate) fn transparent_default(&self) -> bool {
        self.init_color_words == [20, 0, 0, 0, 4]
    }
    /// A mask whose unwritten blocks show the layer.
    pub(crate) fn opaque_default(&self) -> bool {
        self.init_color_words == [20, 1, u32::MAX, 0, 4]
    }
    /// Color bytes per pixel after the alpha plane: 4 (B, G, R, unused), 1
    /// (gray) or none (a mask's coverage).
    pub(crate) fn color_channels(&self) -> usize {
        if self.packing == GRAY_PACKING {
            1
        } else if self.packing == MASK_PACKING {
            0
        } else {
            4
        }
    }
    pub(crate) fn is_mask(&self) -> bool {
        self.packing == MASK_PACKING
    }
    /// Decoded bytes per 256 x 256 tile: the alpha plane plus the color plane.
    pub(crate) fn tile_bytes(&self) -> usize {
        256 * 256 * (1 + self.color_channels())
    }
    pub(crate) fn tile_origin(&self, index: u32) -> Result<[usize; 2], String> {
        if index >= self.columns * self.rows {
            return Err("Clip raster tile index exceeds attribute grid".into());
        }
        Ok([
            (index % self.columns) as usize * 256,
            (index / self.columns) as usize * 256,
        ])
    }
}
fn word(reader: &mut impl Read) -> Result<u32, String> {
    let mut bytes = [0; 4];
    reader
        .read_exact(&mut bytes)
        .map_err(|_| "Truncated clip offscreen attributes")?;
    Ok(u32::from_be_bytes(bytes))
}
fn tag(reader: &mut impl Read, expected: &str) -> Result<(), String> {
    if word(reader)? as usize != expected.len() {
        return Err("Invalid offscreen attribute tag length".into());
    }
    for character in expected.bytes() {
        let mut bytes = [0; 2];
        reader
            .read_exact(&mut bytes)
            .map_err(|_| "Truncated attribute tag")?;
        if bytes != [0, character] {
            return Err("Invalid offscreen attribute tag".into());
        }
    }
    Ok(())
}
pub(crate) fn parse(bytes: &[u8]) -> Result<Attributes, String> {
    if bytes.len() > MAX_BYTES {
        return Err("Offscreen attributes exceed observed layout bounds".into());
    }
    let mut reader = Cursor::new(bytes);
    let header = [
        word(&mut reader)?,
        word(&mut reader)?,
        word(&mut reader)?,
        word(&mut reader)?,
    ];
    if header[0] != 16
        || header[1] != 102
        || ![42, 58].contains(&header[2])
        || header.iter().map(|value| *value as u64).sum::<u64>() != bytes.len() as u64
    {
        return Err("Unsupported offscreen section lengths".into());
    }
    tag(&mut reader, "Parameter")?;
    let width = word(&mut reader)?;
    let height = word(&mut reader)?;
    let columns = word(&mut reader)?;
    let rows = word(&mut reader)?;
    if !(1..=MAX_SIDE).contains(&width)
        || !(1..=MAX_SIDE).contains(&height)
        || columns != width.div_ceil(256)
        || rows != height.div_ceil(256)
    {
        return Err("Unsupported offscreen bitmap/grid geometry".into());
    }
    let mut packing = [0; 16];
    for value in &mut packing {
        *value = word(&mut reader)?;
    }
    if packing != PACKING && packing != GRAY_PACKING && packing != MASK_PACKING {
        return Err(format!("Unsupported offscreen channel packing {packing:?}"));
    }
    tag(&mut reader, "InitColor")?;
    let mut init_color_words = Vec::new();
    for _ in 0..((header[2] - 22) / 4) {
        init_color_words.push(word(&mut reader)?);
    }
    if init_color_words[0] != 20 {
        return Err("Unsupported offscreen initial-color header".into());
    }
    // Preserve all remaining words without inventing a transparent/white mapping.
    tag(&mut reader, "BlockSize")?;
    let count = (columns * rows) as usize;
    if header[3] != 34 + count as u32 * 4
        || word(&mut reader)? != 12
        || word(&mut reader)? as usize != count
        || word(&mut reader)? != 4
    {
        return Err("Offscreen block-size table disagrees with grid".into());
    }
    let mut block_sizes = Vec::with_capacity(count);
    for _ in 0..count {
        let size = word(&mut reader)?;
        if !(104..=112 + 1024 * 1024).contains(&size) {
            return Err("Offscreen block size exceeds observed bounds".into());
        }
        block_sizes.push(size);
    }
    if reader.position() != bytes.len() as u64 {
        return Err("Trailing offscreen attribute bytes".into());
    }
    Ok(Attributes {
        width,
        height,
        columns,
        rows,
        packing,
        init_color_words,
        block_sizes,
    })
}

#[cfg(test)]
pub(crate) fn fixture(width: u32, height: u32, init: &[u32], sizes: &[u32]) -> Vec<u8> {
    fixture_packed(width, height, init, sizes, PACKING)
}
#[cfg(test)]
pub(crate) fn fixture_packed(
    width: u32,
    height: u32,
    init: &[u32],
    sizes: &[u32],
    packing: [u32; 16],
) -> Vec<u8> {
    fn label(out: &mut Vec<u8>, text: &str) {
        out.extend((text.len() as u32).to_be_bytes());
        for byte in text.bytes() {
            out.extend([0, byte]);
        }
    }
    let mut out = Vec::new();
    for word in [
        16,
        102,
        22 + init.len() as u32 * 4,
        34 + sizes.len() as u32 * 4,
    ] {
        out.extend(word.to_be_bytes());
    }
    label(&mut out, "Parameter");
    for word in [width, height, width.div_ceil(256), height.div_ceil(256)] {
        out.extend(word.to_be_bytes());
    }
    for word in packing {
        out.extend(word.to_be_bytes());
    }
    label(&mut out, "InitColor");
    for word in init {
        out.extend(word.to_be_bytes());
    }
    label(&mut out, "BlockSize");
    for word in [12, sizes.len() as u32, 4] {
        out.extend(word.to_be_bytes());
    }
    for word in sizes {
        out.extend(word.to_be_bytes());
    }
    out
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn edge_geometry_raw_defaults_and_size_table_are_preserved() {
        let bytes = fixture(
            257,
            824,
            &[20, 1, u32::MAX, 4, 4, 10, 20, 30, 40],
            &[104; 8],
        );
        let parsed = parse(&bytes).unwrap();
        assert_eq!(
            (parsed.width, parsed.height, parsed.columns, parsed.rows),
            (257, 824, 2, 4)
        );
        assert_eq!(
            parsed.init_color_words,
            [20, 1, u32::MAX, 4, 4, 10, 20, 30, 40]
        );
        assert_eq!(parsed.block_sizes, [104; 8]);
        let max = fixture(4096, 4096, &[20, 0, 0, 0, 4], &[104; 256]);
        assert_eq!(parse(&max).unwrap().block_sizes.len(), 256);
        // Layers moved partly off the canvas keep a bitmap larger than it.
        let past = fixture(5888, 6400, &[20, 0, 0, 0, 4], &[104; 23 * 25]);
        assert_eq!(parse(&past).unwrap().block_sizes.len(), 575);
        let huge = fixture(16385, 256, &[20, 0, 0, 0, 4], &[104; 65]);
        assert!(parse(&huge).is_err());
    }
    #[test]
    fn all_truncations_and_malformed_sections_geometry_packing_and_sizes_fail() {
        let bytes = fixture(16, 16, &[20, 0, 0, 0, 4], &[104]);
        for end in 0..bytes.len() {
            assert!(parse(&bytes[..end]).is_err());
        }
        for (offset, value) in [
            (0, 15),
            (4, 101),
            (8, 43),
            (12, 42),
            (16, 10),
            (38, 0),
            (42, 4097),
            (46, 2),
            (54, 32),
            (58, 2),
            (62, 3),
            (140, 19),
            (182, 11),
            (186, 2),
            (190, 8),
            (194, 103),
            (194, u32::MAX),
        ] {
            let mut changed = bytes.clone();
            changed[offset..offset + 4].copy_from_slice(&u32::to_be_bytes(value));
            assert!(parse(&changed).is_err(), "accepted change at {offset}");
        }
        let mut trailing = bytes.clone();
        trailing.push(0);
        assert!(parse(&trailing).is_err());
    }
}
