//! Patterns for pattern overlays and strokes: an RGBA tile repeated across
//! the canvas. Read from and written to Photoshop's pattern blocks (`Patt`),
//! which effects refer to by id; saved in projects as PNG.
use std::sync::Arc;

/// Largest pattern side kept: Photoshop's presets are far smaller, and a
/// project stores each pattern with every effect that uses it.
pub(crate) const MAX_SIDE: usize = 1024;

#[derive(Clone, Debug)]
pub(crate) struct Pattern {
    pub name: String,
    /// Photoshop's identifier, which its effects refer to.
    pub id: String,
    pub width: usize,
    pub height: usize,
    /// Straight RGBA, row by row.
    pub rgba: Arc<[u8]>,
}
impl PartialEq for Pattern {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name
            && self.id == other.id
            && self.width == other.width
            && self.height == other.height
            && (Arc::ptr_eq(&self.rgba, &other.rgba) || self.rgba == other.rgba)
    }
}

impl Pattern {
    pub(crate) fn new(
        name: String,
        id: String,
        width: usize,
        height: usize,
        rgba: Vec<u8>,
    ) -> Result<Self, String> {
        let pattern = Self {
            name,
            id,
            width,
            height,
            rgba: rgba.into(),
        };
        pattern.validate()?;
        Ok(pattern)
    }
    pub(crate) fn validate(&self) -> Result<(), String> {
        if self.width == 0
            || self.height == 0
            || self.width > MAX_SIDE
            || self.height > MAX_SIDE
            || self.rgba.len() != self.width * self.height * 4
            || self.name.len() > 4096
            || self.id.len() > 255
        {
            return Err("Invalid pattern".into());
        }
        Ok(())
    }
    /// The pixel at (`x`, `y`), repeating in both directions.
    pub(crate) fn at(&self, x: i64, y: i64) -> [u8; 4] {
        let x = x.rem_euclid(self.width as i64) as usize;
        let y = y.rem_euclid(self.height as i64) as usize;
        let i = (y * self.width + x) * 4;
        self.rgba[i..i + 4].try_into().unwrap()
    }
    /// The pattern at a point in its own pixels, bilinearly blended
    /// (premultiplied, so transparent pixels add no color).
    pub(crate) fn sample(&self, x: f64, y: f64) -> [u8; 4] {
        let (x, y) = (x - 0.5, y - 0.5);
        let (x0, y0) = (x.floor(), y.floor());
        let (fx, fy) = ((x - x0) as f32, (y - y0) as f32);
        let (x0, y0) = (x0 as i64, y0 as i64);
        let mut sum = [0.0f32; 4];
        for (dx, dy, weight) in [
            (0, 0, (1.0 - fx) * (1.0 - fy)),
            (1, 0, fx * (1.0 - fy)),
            (0, 1, (1.0 - fx) * fy),
            (1, 1, fx * fy),
        ] {
            let p = self.at(x0 + dx, y0 + dy);
            let a = f32::from(p[3]) / 255.0 * weight;
            for c in 0..3 {
                sum[c] += f32::from(p[c]) * a;
            }
            sum[3] += a;
        }
        if sum[3] <= 0.0 {
            return [0; 4];
        }
        [
            (sum[0] / sum[3]).round() as u8,
            (sum[1] / sum[3]).round() as u8,
            (sum[2] / sum[3]).round() as u8,
            (sum[3] * 255.0).round() as u8,
        ]
    }
    /// The pattern a new overlay starts with: soft gray dots.
    pub(crate) fn default_dots() -> Self {
        let side = 16;
        let mut rgba = Vec::with_capacity(side * side * 4);
        for y in 0..side {
            for x in 0..side {
                let (dx, dy) = (x as f32 - 7.5, y as f32 - 7.5);
                let dot = (4.5 - (dx * dx + dy * dy).sqrt()).clamp(0.0, 1.0);
                let value = (230.0 - 150.0 * dot).round() as u8;
                rgba.extend([value, value, value, 255]);
            }
        }
        Self::new("Dots".into(), "dudel-dots-16".into(), side, side, rgba)
            .expect("the default pattern is valid")
    }
    /// The pattern as PNG bytes.
    fn png(&self) -> Result<Vec<u8>, String> {
        use image::ImageEncoder;
        let mut png = Vec::new();
        image::codecs::png::PngEncoder::new(&mut png)
            .write_image(
                &self.rgba,
                self.width as u32,
                self.height as u32,
                image::ExtendedColorType::Rgba8,
            )
            .map_err(|e| e.to_string())?;
        Ok(png)
    }
}

/// Saved form: the pixels as base64 PNG, compact in the project's JSON.
#[derive(serde::Serialize, serde::Deserialize)]
struct Saved {
    name: String,
    id: String,
    png: String,
}
impl serde::Serialize for Pattern {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let png = self.png().map_err(serde::ser::Error::custom)?;
        Saved {
            name: self.name.clone(),
            id: self.id.clone(),
            png: base64(&png),
        }
        .serialize(serializer)
    }
}
impl<'de> serde::Deserialize<'de> for Pattern {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        use serde::de::Error;
        let saved = Saved::deserialize(deserializer)?;
        let png = unbase64(&saved.png).ok_or_else(|| D::Error::custom("Invalid pattern data"))?;
        let decoder = image::codecs::png::PngDecoder::new(std::io::Cursor::new(png))
            .map_err(D::Error::custom)?;
        let (w, h) = image::ImageDecoder::dimensions(&decoder);
        if w as usize > MAX_SIDE || h as usize > MAX_SIDE {
            return Err(D::Error::custom("Invalid pattern"));
        }
        let rgba = image::DynamicImage::from_decoder(decoder)
            .map_err(D::Error::custom)?
            .into_rgba8()
            .into_raw();
        Self::new(saved.name, saved.id, w as usize, h as usize, rgba).map_err(D::Error::custom)
    }
}

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
pub(crate) fn base64(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |n, (i, b)| n | u32::from(*b) << (16 - 8 * i));
        for i in 0..4 {
            out.push(if i <= chunk.len() {
                ALPHABET[(n >> (18 - 6 * i) & 63) as usize] as char
            } else {
                '='
            });
        }
    }
    out
}
pub(crate) fn unbase64(text: &str) -> Option<Vec<u8>> {
    let text = text.trim_end_matches('=').as_bytes();
    let mut out = Vec::with_capacity(text.len() * 3 / 4);
    let (mut bits, mut count) = (0u32, 0);
    for c in text {
        bits = bits << 6 | ALPHABET.iter().position(|a| a == c)? as u32;
        count += 6;
        if count >= 8 {
            count -= 8;
            out.push((bits >> count) as u8);
        }
    }
    Some(out)
}

/// The patterns in a Photoshop pattern block (`Patt`, `Pat2` or `Pat3`).
/// Patterns Dudel cannot read (other color modes or depths) are left out.
pub(crate) fn read_psd(data: &[u8]) -> Vec<Pattern> {
    let mut patterns = Vec::new();
    let mut rest = data;
    while rest.len() >= 4 {
        let length = u32::from_be_bytes(rest[..4].try_into().unwrap()) as usize;
        let Some(record) = rest.get(4..4 + length) else {
            break;
        };
        if let Ok(pattern) = read_one(record) {
            patterns.push(pattern);
        }
        rest = &rest[(4 + length).next_multiple_of(4).min(rest.len())..];
    }
    patterns
}
fn read_one(data: &[u8]) -> Result<Pattern, String> {
    let mut at = 0;
    let mut take = |n: usize| -> Result<&[u8], String> {
        let bytes = data.get(at..at + n).ok_or("Truncated pattern")?;
        at += n;
        Ok(bytes)
    };
    let u32 = |b: &[u8]| u32::from_be_bytes(b.try_into().unwrap()) as usize;
    let u16 = |b: &[u8]| u16::from_be_bytes(b.try_into().unwrap()) as usize;
    if u32(take(4)?) != 1 {
        return Err("Unsupported pattern version".into());
    }
    let mode = u32(take(4)?);
    let colors = match mode {
        1 => 1, // grayscale
        3 => 3, // RGB
        _ => return Err("Unsupported pattern color mode".into()),
    };
    let (height, width) = (u16(take(2)?), u16(take(2)?));
    let units = u32(take(4)?);
    let name: Vec<u16> = take(units * 2)?
        .chunks_exact(2)
        .map(|c| u16::from_be_bytes([c[0], c[1]]))
        .take_while(|c| *c != 0)
        .collect();
    let name = String::from_utf16_lossy(&name);
    let id_length = usize::from(take(1)?[0]);
    let id = String::from_utf8_lossy(take(id_length)?).into_owned();
    if u32(take(4)?) != 3 {
        return Err("Unsupported pattern data version".into());
    }
    take(4)?; // length
    let [top, left, bottom, right] = [0; 4].map(|_| take(4).map(u32));
    let (top, left, bottom, right) = (top?, left?, bottom?, right?);
    if bottom <= top || right <= left || bottom - top != height || right - left != width {
        return Err("Invalid pattern bounds".into());
    }
    if width > MAX_SIDE || height > MAX_SIDE {
        return Err("Pattern too large".into());
    }
    let slots = u32(take(4)?);
    let mut rgba = vec![255; width * height * 4];
    // Color channels, then a user mask and a sheet mask; the first written
    // mask is the pattern's transparency.
    let mut alpha_written = false;
    for slot in 0..slots + 2 {
        if u32(take(4)?) == 0 {
            continue;
        }
        let length = u32(take(4)?);
        if length == 0 {
            continue;
        }
        let body = take(length)?;
        let target = if slot < colors {
            Some(slot)
        } else if slot >= slots && !alpha_written {
            alpha_written = true;
            Some(3)
        } else {
            None
        };
        let Some(target) = target else {
            continue;
        };
        let [depth, c_top, c_left, c_bottom, c_right] =
            [0, 4, 8, 12, 16].map(|o| body.get(o..o + 4).map(u32));
        let (Some(8), Some(c_top), Some(c_left), Some(c_bottom), Some(c_right)) =
            (depth, c_top, c_left, c_bottom, c_right)
        else {
            return Err("Unsupported pattern channel".into());
        };
        if c_top < top || c_left < left || c_bottom > bottom || c_right > right {
            return Err("Invalid pattern channel bounds".into());
        }
        let (w, h) = (
            c_right.saturating_sub(c_left),
            c_bottom.saturating_sub(c_top),
        );
        let compression = *body.get(22).ok_or("Truncated pattern channel")?;
        let mut framed = vec![0, compression];
        framed.extend_from_slice(&body[23..]);
        let plane = crate::psd_import::decode_plane(&framed, w, h)?;
        for y in 0..h {
            for x in 0..w {
                let (px, py) = (c_left - left + x, c_top - top + y);
                let value = plane[y * w + x];
                let pixel = &mut rgba[(py * width + px) * 4..][..4];
                if colors == 1 && target == 0 {
                    pixel[..3].fill(value);
                } else {
                    pixel[target] = value;
                }
            }
        }
    }
    Pattern::new(name, id, width, height, rgba)
}

/// A Photoshop pattern block (`Patt`) holding `patterns`, raw RGB channels
/// with the transparency as the user mask.
pub(crate) fn write_psd(patterns: &[&Pattern]) -> Vec<u8> {
    let mut out = Vec::new();
    for pattern in patterns {
        let (w, h) = (pattern.width, pattern.height);
        let mut record = Vec::new();
        record.extend(1u32.to_be_bytes());
        record.extend(3u32.to_be_bytes()); // RGB
        record.extend((h as u16).to_be_bytes());
        record.extend((w as u16).to_be_bytes());
        let mut name: Vec<u16> = pattern.name.encode_utf16().collect();
        name.push(0);
        record.extend((name.len() as u32).to_be_bytes());
        name.iter().for_each(|u| record.extend(u.to_be_bytes()));
        let id = &pattern.id.as_bytes()[..pattern.id.len().min(255)];
        record.push(id.len() as u8);
        record.extend(id);
        let opaque = pattern.rgba.chunks_exact(4).all(|p| p[3] == 255);
        let mut list = Vec::new();
        for bound in [0, 0, h, w] {
            list.extend((bound as u32).to_be_bytes());
        }
        const SLOTS: usize = 24;
        list.extend((SLOTS as u32).to_be_bytes());
        for slot in 0..SLOTS + 2 {
            let channel = match slot {
                0..=2 => Some(slot),
                SLOTS if !opaque => Some(3),
                _ => None,
            };
            let Some(channel) = channel else {
                list.extend(0u32.to_be_bytes());
                continue;
            };
            list.extend(1u32.to_be_bytes());
            let mut body = Vec::new();
            body.extend(8u32.to_be_bytes());
            for bound in [0, 0, h, w] {
                body.extend((bound as u32).to_be_bytes());
            }
            body.extend(8u16.to_be_bytes());
            body.push(0); // raw
            body.extend(pattern.rgba.chunks_exact(4).map(|p| p[channel]));
            list.extend((body.len() as u32).to_be_bytes());
            list.extend(body);
        }
        record.extend(3u32.to_be_bytes());
        record.extend((list.len() as u32).to_be_bytes());
        record.extend(list);
        out.extend((record.len() as u32).to_be_bytes());
        out.extend(&record);
        out.resize(out.len().next_multiple_of(4), 0);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_round_trips() {
        for n in 0..10 {
            let bytes: Vec<u8> = (0..n).map(|i| (i * 37 + 5) as u8).collect();
            assert_eq!(unbase64(&base64(&bytes)).unwrap(), bytes);
        }
        assert_eq!(base64(b"Man"), "TWFu");
        assert_eq!(base64(b"Ma"), "TWE=");
    }

    #[test]
    fn patterns_survive_projects_and_photoshop_blocks() {
        let mut rgba = Vec::new();
        for i in 0..6 * 4 {
            rgba.extend([
                i as u8 * 10,
                200 - i as u8,
                7,
                if i % 5 == 0 { 0 } else { 255 },
            ]);
        }
        let pattern = Pattern::new("Grid".into(), "abc-1".into(), 6, 4, rgba).unwrap();
        let json = serde_json::to_string(&pattern).unwrap();
        assert_eq!(serde_json::from_str::<Pattern>(&json).unwrap(), pattern);
        let block = write_psd(&[&pattern, &Pattern::default_dots()]);
        let back = read_psd(&block);
        assert_eq!(back, vec![pattern, Pattern::default_dots()]);
    }

    #[test]
    fn samples_repeat_and_blend_premultiplied() {
        let pattern = Pattern::new(
            "Two".into(),
            "two".into(),
            2,
            1,
            vec![255, 0, 0, 255, 0, 0, 255, 0],
        )
        .unwrap();
        assert_eq!(pattern.at(-1, 5), [0, 0, 255, 0]);
        assert_eq!(pattern.sample(0.5, 0.5), [255, 0, 0, 255]);
        // Halfway to a transparent pixel: half as opaque, still red.
        assert_eq!(pattern.sample(1.0, 0.5), [255, 0, 0, 128]);
    }
}
