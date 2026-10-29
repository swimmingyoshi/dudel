//! Bounded PSD v1 reader. Layout follows Adobe's Photoshop File Formats
//! Specification (November 2019). Unsupported layer features use the saved
//! composite with explicit conversion notes; no guessed layer reconstruction.
use crate::{
    document::{Document, Layer, MAX_ENTRIES, MAX_RASTER_LAYERS},
    import_report::Imported,
};
use std::{io::Read, path::Path};

const LIMIT: usize = 512 * 1024 * 1024;
struct Reader<'a>(&'a [u8]);
impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        if n > self.0.len() {
            return Err("Truncated PSD section".into());
        }
        let (data, rest) = self.0.split_at(n);
        self.0 = rest;
        Ok(data)
    }
    fn u8(&mut self) -> Result<u8, String> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16, String> {
        Ok(u16::from_be_bytes(self.take(2)?.try_into().unwrap()))
    }
    fn u32(&mut self) -> Result<u32, String> {
        Ok(u32::from_be_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn i32(&mut self) -> Result<i32, String> {
        Ok(self.u32()? as i32)
    }
    fn section(&mut self) -> Result<Reader<'a>, String> {
        let n = self.u32()? as usize;
        Ok(Reader(self.take(n)?))
    }
    fn rect(&mut self) -> Result<[i32; 4], String> {
        Ok([self.i32()?, self.i32()?, self.i32()?, self.i32()?])
    }
}
fn check(cancel: &impl Fn() -> bool) -> Result<(), String> {
    if cancel() {
        Err("PSD import canceled".into())
    } else {
        Ok(())
    }
}
fn geometry([top, left, bottom, right]: [i32; 4]) -> Result<[usize; 2], String> {
    let w = i64::from(right) - i64::from(left);
    let h = i64::from(bottom) - i64::from(top);
    if !(0..=16384).contains(&w) || !(0..=16384).contains(&h) || w * h > 67_108_864 {
        return Err("PSD layer dimensions exceed import limits".into());
    }
    Ok([w as usize, h as usize])
}
fn decode(
    data: &[u8],
    width: usize,
    height: usize,
    channels: usize,
    depth: u16,
    cancel: &impl Fn() -> bool,
) -> Result<Vec<u8>, String> {
    check(cancel)?;
    let stride = width
        .checked_mul(usize::from(depth / 8))
        .ok_or("PSD row overflow")?;
    let rows = height
        .checked_mul(channels)
        .ok_or("PSD row count overflow")?;
    let size = stride
        .checked_mul(rows)
        .filter(|n| *n <= LIMIT)
        .ok_or("PSD decode exceeds memory limit")?;
    let mut reader = Reader(data);
    let compression = reader.u16()?;
    let mut out = match compression {
        0 => reader.take(size)?.to_vec(),
        1 => {
            let lengths: Vec<usize> = (0..rows)
                .map(|_| reader.u16().map(usize::from))
                .collect::<Result<_, _>>()?;
            let mut output = Vec::with_capacity(size);
            for length in lengths {
                check(cancel)?;
                let end = output.len() + stride;
                let mut row = Reader(reader.take(length)?);
                while !row.0.is_empty() {
                    let code = row.u8()? as i8;
                    match code {
                        0..=127 => {
                            let n = code as usize + 1;
                            if output.len() + n > end {
                                return Err("PSD RLE row overflow".into());
                            }
                            output.extend_from_slice(row.take(n)?);
                        }
                        -127..=-1 => {
                            let n = (1 - i16::from(code)) as usize;
                            if output.len() + n > end {
                                return Err("PSD RLE row overflow".into());
                            }
                            let value = row.u8()?;
                            output.resize(output.len() + n, value);
                        }
                        -128 => {}
                    }
                }
                if output.len() != end {
                    return Err("PSD RLE row is incomplete".into());
                }
            }
            output
        }
        2 | 3 => {
            let mut decoder = flate2::read::ZlibDecoder::new(reader.0);
            let mut output = vec![0; size];
            for row in output.chunks_mut(stride.max(1)) {
                check(cancel)?;
                decoder
                    .read_exact(row)
                    .map_err(|e| format!("PSD ZIP: {e}"))?;
            }
            let mut extra = [0];
            if decoder.read(&mut extra).map_err(|e| e.to_string())? != 0
                || decoder.total_in() as usize != reader.0.len()
            {
                return Err("PSD ZIP stream length mismatch".into());
            }
            reader.0 = &[];
            output
        }
        _ => return Err("Unsupported PSD compression".into()),
    };
    if !reader.0.is_empty() {
        return Err("Trailing PSD channel data".into());
    }
    if compression == 3 {
        if depth != 8 {
            return Err("16-bit PSD ZIP prediction is not supported".into());
        }
        for row in out.chunks_mut(stride.max(1)) {
            check(cancel)?;
            for x in 1..row.len() {
                row[x] = row[x].wrapping_add(row[x - 1]);
            }
        }
    }
    Ok(out)
}

fn blend(key: &[u8]) -> Result<crate::blend::BlendMode, String> {
    use crate::blend::BlendMode::*;
    Ok(match key {
        b"norm" => Normal,
        b"mul " => Multiply,
        b"scrn" => Screen,
        b"over" => Overlay,
        b"dark" => Darken,
        b"lite" => Lighten,
        b"div " => ColorDodge,
        b"idiv" => ColorBurn,
        b"hLit" => HardLight,
        b"sLit" => SoftLight,
        b"diff" => Difference,
        b"smud" => Exclusion,
        b"lbrn" => LinearBurn,
        b"fsub" => Subtract,
        b"dkCl" => DarkerColor,
        b"lddg" => LinearDodge,
        b"lgCl" => LighterColor,
        b"vLit" => VividLight,
        b"lLit" => LinearLight,
        b"pLit" => PinLight,
        b"hMix" => HardMix,
        b"fdiv" => Divide,
        b"hue " => Hue,
        b"sat " => Saturation,
        b"colr" => Color,
        b"lum " => Luminosity,
        _ => {
            return Err(format!(
                "Unsupported PSD blend mode {}",
                String::from_utf8_lossy(key)
            ));
        }
    })
}
struct Record {
    layer: Layer,
    rect: [i32; 4],
    channels: Vec<(i16, usize)>,
    group: u32,
    mask: Option<([i32; 4], u8, u8)>,
    solid: Option<[u8; 3]>,
}

// A plain RGB solid-color fill is editable as a rasterized paper layer. Other
// descriptors (gradients, pattern fills, vector masks) take the composite path.
fn solid_color(mut r: Reader<'_>) -> Result<[u8; 3], String> {
    fn key<'a>(r: &mut Reader<'a>) -> Result<&'a [u8], String> {
        let size = r.u32()? as usize;
        if size > 4096 {
            return Err("PSD descriptor key exceeds bounds".into());
        }
        r.take(if size == 0 { 4 } else { size })
    }
    fn descriptor(r: &mut Reader<'_>, class: &[u8], fields: u32) -> Result<(), String> {
        let n = r.u32()? as usize;
        if n > 2048 {
            return Err("PSD descriptor name exceeds bounds".into());
        }
        r.take(n * 2)?;
        if key(r)? != class || r.u32()? != fields {
            return Err("Unsupported PSD solid-fill descriptor".into());
        }
        Ok(())
    }
    if r.u32()? != 16 {
        return Err("Unsupported PSD descriptor version".into());
    }
    descriptor(&mut r, b"null", 1)?;
    if key(&mut r)? != b"Clr " || r.take(4)? != b"Objc" {
        return Err("Unsupported PSD solid color".into());
    }
    descriptor(&mut r, b"RGBC", 3)?;
    let mut color = [None; 3];
    for _ in 0..3 {
        let channel = match key(&mut r)? {
            b"Rd  " => 0,
            b"Grn " => 1,
            b"Bl  " => 2,
            _ => return Err("Unknown solid color channel".into()),
        };
        if color[channel].is_some() || r.take(4)? != b"doub" {
            return Err("Invalid PSD solid channel".into());
        }
        let value = f64::from_be_bytes(r.take(8)?.try_into().unwrap());
        if !value.is_finite() || !(0.0..=255.0).contains(&value) {
            return Err("PSD solid color outside RGB range".into());
        }
        color[channel] = Some(value.round() as u8);
    }
    if !r.0.is_empty() {
        return Err("Trailing PSD solid-fill data".into());
    }
    Ok(color.map(Option::unwrap))
}
fn layers(
    data: &[u8],
    width: usize,
    height: usize,
    cancel: &impl Fn() -> bool,
) -> Result<Document, String> {
    let mut outer = Reader(data);
    let mut info = outer.section()?;
    if !outer.0.is_empty() && (!outer.section()?.0.is_empty() || outer.0.iter().any(|b| *b != 0)) {
        return Err("PSD global masks or effects require the saved composite".into());
    }
    let count = (info.u16()? as i16).unsigned_abs() as usize;
    if count == 0 || count > MAX_ENTRIES * 2 {
        return Err("PSD layer count exceeds limits or is empty".into());
    }
    let mut records = Vec::new();
    for _ in 0..count {
        check(cancel)?;
        let rect = info.rect()?;
        geometry(rect)?;
        let count = info.u16()? as usize;
        if count > 56 {
            return Err("Too many PSD layer channels".into());
        }
        let mut channels = Vec::new();
        for _ in 0..count {
            let id = info.u16()? as i16;
            let len = info.u32()? as usize;
            if ![-2, -1, 0, 1, 2].contains(&id) || channels.iter().any(|(other, _)| *other == id) {
                return Err("Unsupported or duplicate PSD layer channel".into());
            }
            channels.push((id, len));
        }
        if info.take(4)? != b"8BIM" {
            return Err("Invalid PSD layer signature".into());
        }
        let mode = blend(info.take(4)?)?;
        let opacity = info.u8()?;
        let clipping = info.u8()?;
        if clipping > 1 {
            return Err("Invalid PSD clipping mode".into());
        }
        let flags = info.u8()?;
        info.u8()?;
        if flags & 24 == 24 {
            return Err("PSD layer appearance requires the saved composite".into());
        }
        let mut extra = info.section()?;
        let mut mask_data = extra.section()?;
        let mask = if mask_data.0.is_empty() {
            None
        } else {
            if mask_data.0.len() != 20 {
                return Err("Extended PSD masks need the saved composite".into());
            }
            let r = mask_data.rect()?;
            geometry(r)?;
            let default = mask_data.u8()?;
            let flags = mask_data.u8()?;
            if flags & !7 != 0 {
                return Err("Unsupported PSD mask flags".into());
            }
            Some((r, default, flags))
        };
        let ranges = extra.section()?;
        if !ranges
            .0
            .chunks_exact(8)
            .all(|r| r == [0, 0, 255, 255, 0, 0, 255, 255])
            || ranges.0.len() % 8 != 0
        {
            return Err("PSD Blend If requires the saved composite".into());
        }
        let len = extra.u8()? as usize;
        let name = String::from_utf8_lossy(extra.take(len)?).into_owned();
        extra.take((4 - (len + 1) % 4) % 4)?;
        let mut layer = Layer::raster(name, width, height);
        layer.opacity = opacity as f32 / 255.;
        layer.visible = flags & 2 == 0;
        layer.alpha_locked = flags & 1 != 0;
        layer.clipped = clipping == 1;
        layer.blend = mode;
        let mut group = 0;
        let mut solid = None;
        while extra.0.len() >= 12 {
            if extra.take(4)? != b"8BIM" {
                return Err("Unsupported PSD additional-info signature".into());
            }
            let key = extra.take(4)?;
            let len = extra.u32()? as usize;
            let mut value = Reader(extra.take(len)?);
            if !len.is_multiple_of(2) {
                extra.take(1)?;
            }
            match key {
                b"SoCo" => {
                    solid = Some(solid_color(value)?);
                }
                b"luni" => {
                    let n = value.u32()? as usize;
                    if n > 2048 {
                        return Err("PSD layer name too long".into());
                    }
                    let units = (0..n).map(|_| value.u16()).collect::<Result<Vec<_>, _>>()?;
                    layer.name = String::from_utf16_lossy(&units);
                }
                b"lsct" | b"lsdk" => {
                    group = value.u32()?;
                    if group > 3 {
                        return Err("Unknown PSD group type".into());
                    }
                    if value.0.len() >= 8 {
                        if value.take(4)? != b"8BIM" {
                            return Err("Invalid PSD group signature".into());
                        }
                        layer.blend = blend(value.take(4)?)?;
                    }
                }
                b"lspf" => {
                    let locks = value.u32()?;
                    layer.locked = locks & (0x80000000 | 2 | 4) != 0;
                    layer.alpha_locked |= locks & 1 != 0;
                }
                // Transparency shapes only change layer effects, fill opacity
                // and Blend If, which all take the composite path.
                b"tsly" if value.u8()? <= 1 => {}
                b"clbl" if value.u8()? == 1 => {}
                b"infx" | b"knko" if value.u8()? == 0 => {}
                b"lyid" | b"lclr" | b"lnsr" | b"shmd" | b"fxrp" | b"lyvr" => {}
                _ => {
                    return Err(format!(
                        "PSD layer feature {} needs the saved composite",
                        String::from_utf8_lossy(key)
                    ));
                }
            }
        }
        if extra.0.iter().any(|b| *b != 0) {
            return Err("Truncated PSD additional info".into());
        }
        if group == 1 || group == 2 {
            layer.folder = true;
            layer.collapsed = group == 2;
            layer.alpha_locked = false;
            layer.pixels = crate::pixels::Pixels::default();
        }
        records.push(Record {
            layer,
            rect,
            channels,
            group,
            mask,
            solid,
        });
    }
    let mut decoded_budget = 0usize;
    for record in &mut records {
        check(cancel)?;
        let [w, h] = geometry(record.rect)?;
        let mut planes = std::collections::BTreeMap::new();
        for &(id, bytes) in &record.channels {
            let data = info.take(bytes)?;
            if record.group != 0 {
                continue;
            }
            let [cw, ch] = if id == -2 {
                geometry(record.mask.ok_or("PSD mask channel has no metadata")?.0)?
            } else {
                [w, h]
            };
            decoded_budget = decoded_budget
                .checked_add(cw * ch)
                .filter(|b| *b <= LIMIT)
                .ok_or("PSD layer payload exceeds memory budget")?;
            planes.insert(id, decode(data, cw, ch, 1, 8, cancel)?);
        }
        if let Some(color) = record.solid {
            let row = [color[0], color[1], color[2], 255].repeat(width);
            let mut writer = record.layer.pixels.writer();
            for y in 0..height {
                check(cancel)?;
                writer.write_region(width, [0, y, width, y + 1], &row);
            }
        } else if record.group == 0 && w * h != 0 {
            if !(0..=2).all(|id| planes.contains_key(&id)) {
                return Err("PSD layer is missing RGB pixels".into());
            }
            let mut writer = record.layer.pixels.writer();
            for y in 0..h {
                check(cancel)?;
                let dy = i64::from(record.rect[0]) + y as i64;
                if !(0..height as i64).contains(&dy) {
                    continue;
                }
                for x in 0..w {
                    let dx = i64::from(record.rect[1]) + x as i64;
                    if !(0..width as i64).contains(&dx) {
                        continue;
                    }
                    let i = y * w + x;
                    let rgba = [
                        planes[&0][i],
                        planes[&1][i],
                        planes[&2][i],
                        planes.get(&-1).map_or(255, |a| a[i]),
                    ];
                    if rgba != [0; 4] {
                        writer
                            .rgba_mut(dy as usize * width + dx as usize)
                            .copy_from_slice(&rgba);
                    }
                }
            }
        }
        if let Some((rect, default, flags)) = record.mask {
            if record.group != 0 {
                return Err("PSD group masks need the saved composite".into());
            }
            let plane = planes.get(&-2).ok_or("PSD mask metadata has no pixels")?;
            let [mw, mh] = geometry(rect)?;
            let invert = |v: u8| if flags & 4 != 0 { 255 - v } else { v };
            let mut coverage = crate::layer_mask::Mask::new(width, height, invert(default))?;
            let offset = if flags & 1 != 0 {
                [record.rect[0], record.rect[1]]
            } else {
                [0, 0]
            };
            for y in 0..mh {
                check(cancel)?;
                let dy = i64::from(rect[0]) + i64::from(offset[0]) + y as i64;
                if !(0..height as i64).contains(&dy) {
                    continue;
                }
                for x in 0..mw {
                    let dx = i64::from(rect[1]) + i64::from(offset[1]) + x as i64;
                    if (0..width as i64).contains(&dx) {
                        coverage
                            .writer()
                            .set(dx as usize, dy as usize, invert(plane[y * mw + x]));
                    }
                }
            }
            record.layer.mask = Some(crate::layer_mask::LayerMask {
                id: crate::document::LayerId::new(),
                enabled: flags & 2 == 0,
                linked: true,
                coverage,
            });
        }
    }
    // Records run bottom to top, like Dudel's stack: a group's end divider
    // comes first, then its contents, then the folder record above them.
    let mut depth = 0u8;
    let mut result = Vec::new();
    for mut record in records {
        match record.group {
            3 => {
                depth += 1;
                if depth > crate::document::MAX_DEPTH {
                    return Err("PSD group nesting exceeds limits".into());
                }
                continue;
            }
            1 | 2 => {
                depth = depth
                    .checked_sub(1)
                    .ok_or("PSD group without an end divider")?
            }
            _ => {}
        }
        record.layer.depth = depth;
        result.push(record.layer);
    }
    if depth != 0
        || result.len() > MAX_ENTRIES
        || result.iter().filter(|l| !l.folder).count() > MAX_RASTER_LAYERS
    {
        return Err("PSD layer hierarchy exceeds limits or is incomplete".into());
    }
    let mut doc = Document::new(width, height);
    doc.layers = result;
    doc.active = doc.layers.len().saturating_sub(1);
    doc.version = if doc.layers.iter().any(|l| l.mask.is_some()) {
        2
    } else {
        1
    };
    doc.validate()?;
    Ok(doc)
}

pub(crate) fn import(path: &Path, cancel: &impl Fn() -> bool) -> Result<Imported, String> {
    parse(&read(path)?, cancel)
}
/// The flattened image the source editor saved, for checking a layered import.
pub(crate) fn saved_composite(path: &Path) -> Result<Imported, String> {
    parse_mode(&read(path)?, &|| false, false)
}
fn read(path: &Path) -> Result<Vec<u8>, String> {
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    if file.metadata().map_err(|e| e.to_string())?.len() > LIMIT as u64 {
        return Err("PSD exceeds 512 MiB input limit".into());
    }
    let mut bytes = Vec::new();
    file.take(LIMIT as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > LIMIT {
        return Err("PSD exceeds input limit".into());
    }
    Ok(bytes)
}
fn parse(data: &[u8], cancel: &impl Fn() -> bool) -> Result<Imported, String> {
    parse_mode(data, cancel, true)
}
fn parse_mode(data: &[u8], cancel: &impl Fn() -> bool, layered: bool) -> Result<Imported, String> {
    check(cancel)?;
    let mut reader = Reader(data);
    if reader.take(4)? != b"8BPS" || reader.u16()? != 1 || reader.take(6)? != [0; 6] {
        return Err("Expected a Photoshop PSD v1 file (PSB is not supported)".into());
    }
    let channels = reader.u16()? as usize;
    let height = reader.u32()? as usize;
    let width = reader.u32()? as usize;
    let depth = reader.u16()?;
    let mode = reader.u16()?;
    if width == 0
        || height == 0
        || width > 4096
        || height > 4096
        || !(3..=56).contains(&channels)
        || ![8, 16].contains(&depth)
        || mode != 3
    {
        return Err("PSD import supports RGB 8/16-bit canvases up to 4096 × 4096".into());
    }
    reader.section()?;
    let mut resources = reader.section()?;
    let mut has_composite = true;
    let mut profile = false;
    while !resources.0.is_empty() {
        if resources.take(4)? != b"8BIM" {
            return Err("Invalid PSD resource signature".into());
        }
        let id = resources.u16()?;
        let len = resources.u8()? as usize;
        resources.take(len)?;
        if !(len + 1).is_multiple_of(2) {
            resources.take(1)?;
        }
        let data = resources.section()?;
        if data.0.len() % 2 != 0 {
            resources.take(1)?;
        }
        if id == 1057 {
            let mut version = data;
            version.u32()?;
            has_composite = version.u8()? != 0;
        } else if id == 1039 {
            profile = !data.0.is_empty();
        }
    }
    let layer_data = reader.section()?;
    let mut notes = Vec::new();
    if profile {
        notes.push(
            "The embedded color profile was not converted; Dudel uses the stored RGB values."
                .into(),
        );
    }
    if layered && depth == 8 && !layer_data.0.is_empty() {
        match layers(layer_data.0, width, height, cancel) {
            Ok(document) => {
                notes.push("Imported editable raster layers, supported groups, masks and blend modes. Rendering uses Dudel's compositor.".into());
                return Ok(Imported { document, notes });
            }
            Err(error) => {
                check(cancel)?;
                notes.push(format!("Imported the saved flattened composite because editable conversion was unavailable: {error}"));
            }
        }
    } else {
        notes.push("Imported the saved flattened composite as one layer.".into());
    }
    if !has_composite {
        return Err("PSD needs a saved compatibility composite for unsupported layer features. Save it with Maximize Compatibility enabled.".into());
    }
    let has_alpha = if layer_data.0.len() >= 6 {
        let mut r = Reader(layer_data.0);
        let info = r.u32()?;
        info >= 2 && (r.u16()? as i16) < 0
    } else {
        false
    };
    let raw = decode(reader.0, width, height, channels, depth, cancel)?;
    let samples = width * height;
    let value = |c: usize, i: usize| {
        let at = (c * samples + i) * usize::from(depth / 8);
        if depth == 8 {
            raw[at]
        } else {
            ((u16::from_be_bytes([raw[at], raw[at + 1]]) as u32 + 128) / 257) as u8
        }
    };
    let mut pixels = vec![0; samples * 4];
    for (i, pixel) in pixels.chunks_exact_mut(4).enumerate() {
        if i % width == 0 {
            check(cancel)?;
        }
        pixel.copy_from_slice(&[
            value(0, i),
            value(1, i),
            value(2, i),
            if has_alpha && channels >= 4 {
                value(3, i)
            } else {
                255
            },
        ]);
    }
    if depth == 16 {
        notes.push("Converted 16-bit channels to Dudel's 8-bit channels.".into());
    }
    let mut document = Document::new(width, height);
    document.layers[0].name = "PSD composite (flattened)".into();
    document.layers[0].pixels = pixels.into();
    document.validate()?;
    Ok(Imported { document, notes })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(compressed: &[u8]) -> Vec<u8> {
        let mut b = b"8BPS\0\x01\0\0\0\0\0\0\0\x03\0\0\0\x01\0\0\0\x02\0\x08\0\x03".to_vec();
        b.extend([0; 12]);
        b.extend(compressed);
        b
    }
    fn section(bytes: &[u8]) -> Vec<u8> {
        let mut out = (bytes.len() as u32).to_be_bytes().to_vec();
        out.extend(bytes);
        out
    }
    /// One 1 × 1 layer record at `left`, or a group record (`section` 1 = open
    /// folder, 3 = end divider) without pixels.
    struct Spec {
        name: &'static str,
        left: i32,
        color: [u8; 4],
        mask: bool,
        section: Option<u32>,
    }
    fn layer_file(specs: &[Spec]) -> Vec<u8> {
        let mut records = Vec::new();
        let mut planes = Vec::new();
        for spec in specs {
            let size = i32::from(spec.section.is_none());
            for value in [0, spec.left, size, spec.left + size] {
                records.extend(value.to_be_bytes());
            }
            let channels: &[(i16, u8)] = match (spec.section, spec.mask) {
                (Some(_), _) => &[],
                (None, false) => &[(0, 0), (1, 1), (2, 2), (-1, 3)],
                (None, true) => &[(0, 0), (1, 1), (2, 2), (-1, 3), (-2, 4)],
            };
            records.extend((channels.len() as u16).to_be_bytes());
            for &(id, index) in channels {
                records.extend(id.to_be_bytes());
                records.extend(3u32.to_be_bytes());
                planes.extend([
                    0,
                    0,
                    if index == 4 {
                        75
                    } else {
                        spec.color[index as usize]
                    },
                ]);
            }
            records.extend(b"8BIMnorm");
            records.extend([255, 0, 0, 0]);
            let mut extra = Vec::new();
            if spec.mask {
                let mut m = Vec::new();
                for value in [0, spec.left, 1, spec.left + 1] {
                    m.extend(value.to_be_bytes());
                }
                m.extend([255, 0, 0, 0]);
                extra.extend(section(&m));
            } else {
                extra.extend(0u32.to_be_bytes());
            }
            extra.extend(0u32.to_be_bytes());
            extra.push(spec.name.len() as u8);
            extra.extend(spec.name.as_bytes());
            extra.extend(vec![0; (4 - (spec.name.len() + 1) % 4) % 4]);
            if let Some(kind) = spec.section {
                extra.extend(b"8BIMlsct");
                extra.extend(section(&kind.to_be_bytes()));
            }
            records.extend(section(&extra));
        }
        let mut info = (-(specs.len() as i16)).to_be_bytes().to_vec();
        info.extend(records);
        info.extend(planes);
        if !info.len().is_multiple_of(2) {
            info.push(0);
        }
        let mut layers = section(&info);
        layers.extend(0u32.to_be_bytes());
        let mut file = fixture(&[]);
        file.truncate(34);
        file.extend(section(&layers));
        file
    }
    fn raster(name: &'static str, left: i32, color: [u8; 4], mask: bool) -> Spec {
        Spec {
            name,
            left,
            color,
            mask,
            section: None,
        }
    }
    fn group(name: &'static str, kind: u32) -> Spec {
        Spec {
            name,
            left: 0,
            color: [0; 4],
            mask: false,
            section: Some(kind),
        }
    }
    #[test]
    fn editable_layers_preserve_order_offsets_alpha_and_masks() {
        // Records are stored bottom first.
        let file = layer_file(&[
            raster("Base", 0, [40, 50, 60, 255], false),
            raster("Top", 1, [10, 20, 30, 128], true),
        ]);
        let imported = parse(&file, &|| false).unwrap();
        assert_eq!(imported.document.layers.len(), 2);
        assert_eq!(imported.document.layers[0].name, "Base");
        assert_eq!(imported.document.layers[1].name, "Top");
        assert_eq!(imported.document.layers[1].pixels.rgba(0), [0; 4]);
        assert_eq!(
            imported.document.layers[1].pixels.rgba(1),
            [10, 20, 30, 128]
        );
        let mask = &imported.document.layers[1].mask.as_ref().unwrap().coverage;
        assert_eq!(mask.at(0, 0), 255);
        assert_eq!(mask.at(1, 0), 75);
        assert!(imported.notes.iter().any(|n| n.contains("editable")));
    }
    #[test]
    fn groups_nest_with_the_divider_below_their_contents() {
        // Paper, then Outer { Inner { Ink }, Shade }, as Photoshop writes it.
        let file = layer_file(&[
            raster("Paper", 0, [250, 250, 250, 255], false),
            group("</Layer set>", 3),
            group("</Layer set>", 3),
            raster("Ink", 0, [0, 0, 0, 255], false),
            group("Inner", 1),
            raster("Shade", 1, [90, 90, 120, 200], false),
            group("Outer", 2),
        ]);
        let doc = parse(&file, &|| false).unwrap().document;
        let stack: Vec<_> = doc
            .layers
            .iter()
            .map(|l| (l.name.as_str(), l.folder, l.depth))
            .collect();
        assert_eq!(
            stack,
            [
                ("Paper", false, 0),
                ("Ink", false, 2),
                ("Inner", true, 1),
                ("Shade", false, 1),
                ("Outer", true, 0),
            ]
        );
        assert!(doc.layers[4].collapsed && !doc.layers[2].collapsed);
        assert_eq!(doc.parent(1), Some(2));
        assert_eq!(doc.parent(3), Some(4));
        // A folder record without its divider is not a valid hierarchy.
        let broken = layer_file(&[raster("Ink", 0, [0, 0, 0, 255], false), group("Lone", 1)]);
        match parse(&broken, &|| false) {
            Ok(imported) => assert!(imported.notes.iter().any(|n| n.contains("divider"))),
            Err(error) => assert!(error.contains("PSD"), "{error}"),
        }
    }
    #[test]
    fn composite_raw_rle_zip_and_prediction_decode_exactly() {
        use std::io::Write;
        let expected = vec![10, 30, 50, 255, 20, 40, 60, 255];
        let cases = [
            vec![0, 0, 10, 20, 30, 40, 50, 60],
            vec![0, 1, 0, 3, 0, 3, 0, 3, 1, 10, 20, 1, 30, 40, 1, 50, 60],
        ];
        for data in cases {
            assert_eq!(
                parse(&fixture(&data), &|| false).unwrap().document.layers[0].pixels,
                expected
            );
        }
        for prediction in [false, true] {
            let mut zip = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
            zip.write_all(if prediction {
                &[10, 10, 30, 10, 50, 10]
            } else {
                &[10, 20, 30, 40, 50, 60]
            })
            .unwrap();
            let mut data = vec![0, if prediction { 3 } else { 2 }];
            data.extend(zip.finish().unwrap());
            assert_eq!(
                parse(&fixture(&data), &|| false).unwrap().document.layers[0].pixels,
                expected
            );
        }
    }
    #[test]
    fn truncated_and_overflowing_files_fail_without_panicking() {
        let data = fixture(&[0, 0, 10, 20, 30, 40, 50, 60]);
        for end in 0..data.len() {
            assert!(parse(&data[..end], &|| false).is_err(), "{end}");
        }
        assert!(parse(&data, &|| true).is_err());
        assert!(
            parse(
                &fixture(&[0, 1, 0, 2, 0, 2, 0, 2, 254, 10, 254, 20, 254, 30]),
                &|| false
            )
            .is_err()
        );
    }
}
