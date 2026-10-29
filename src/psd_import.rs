//! Bounded PSD v1 reader. Layout follows Adobe's Photoshop File Formats
//! Specification (November 2019). Unsupported layer features use the saved
//! composite with explicit conversion notes; no guessed layer reconstruction.
use crate::{
    document::{Document, Layer, MAX_ENTRIES, MAX_RASTER_LAYERS},
    import_report::Imported,
};
use std::{io::Read, path::Path};

/// Largest single decoded channel image.
const LIMIT: usize = 512 * 1024 * 1024;
/// PSD v1 files end at 2 GB; larger documents use PSB.
const MAX_FILE: usize = 2 << 30;
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
        b"diss" => Dissolve,
        b"pass" => PassThrough,
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
/// Which way a file lists its layer records.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Order {
    /// Photoshop's order: bottom layer first, a group's end divider below
    /// its contents.
    BottomFirst,
    /// Some other writers list the top layer first.
    TopFirst,
    /// No groups: either order nests, so the saved image has to decide.
    Undetermined,
}
/// Depth of each record read bottom first, or None if the group markers do
/// not nest that way. Dividers (type 3) get no entry of their own.
fn nesting(groups: impl Iterator<Item = u32>) -> Option<Vec<Option<u8>>> {
    let mut depth = 0u8;
    let mut depths = Vec::new();
    for group in groups {
        match group {
            3 => {
                depth += 1;
                if depth > crate::document::MAX_DEPTH {
                    return None;
                }
                depths.push(None);
                continue;
            }
            1 | 2 => depth = depth.checked_sub(1)?,
            _ => {}
        }
        depths.push(Some(depth));
    }
    (depth == 0).then_some(depths)
}
fn layers(
    data: &[u8],
    width: usize,
    height: usize,
    cancel: &impl Fn() -> bool,
) -> Result<(Document, Order, Vec<String>), String> {
    // After the layer records come the global layer mask (how masks are shown
    // in the editor, not pixels) and document-wide blocks such as patterns,
    // text engine data and linked files. Those only matter to layers that use
    // them, and such layers carry their own markers, checked per layer.
    let mut info = Reader(data).section()?;
    let count = (info.u16()? as i16).unsigned_abs() as usize;
    if count == 0 || count > MAX_ENTRIES * 2 {
        return Err("PSD layer count exceeds limits or is empty".into());
    }
    let mut records = Vec::new();
    let mut notes = Vec::new();
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
        let mut shapes = true;
        let mut fill = 255u8;
        let mut rendered = None;
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
                b"tsly" if value.0.first().is_some_and(|v| *v <= 1) => {
                    shapes = value.u8()? == 1;
                }
                b"clbl" if value.u8()? == 1 => {}
                // Photoshop stores what text and smart objects look like in the
                // layer's own channels, so they import as pixels.
                b"TySh" | b"tySh" => rendered = Some("text"),
                b"PlLd" | b"SoLd" | b"SoLE" | b"plLd" => rendered = Some("smart object"),
                b"iOpa" => fill = value.u8()?,
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
        // Bit 4 (with bit 3 saying it is meaningful) marks pixel data that does
        // not affect the document: normal for group records, unknown elsewhere.
        if flags & 24 == 24 && group == 0 {
            return Err("PSD layer pixels are marked irrelevant to its appearance".into());
        }
        if fill != 255 {
            use crate::blend::BlendMode::*;
            // These modes treat fill opacity differently from opacity.
            if matches!(
                layer.blend,
                ColorBurn
                    | LinearBurn
                    | ColorDodge
                    | LinearDodge
                    | VividLight
                    | LinearLight
                    | HardMix
                    | Difference
            ) {
                return Err("PSD fill opacity on this blend mode needs the saved composite".into());
            }
            layer.opacity *= f32::from(fill) / 255.0;
        }
        if let Some(kind) = rendered {
            notes.push(format!(
                "“{}” is a {kind} layer; Dudel imported its pixels. Its {} cannot be edited here.",
                layer.name,
                if kind == "text" { "text" } else { "source" }
            ));
        }
        // With Transparency Shapes off, Photoshop's dodge modes blend the
        // alpha-weighted color at full strength: CSP's glow modes.
        if !shapes {
            use crate::blend::BlendMode::*;
            layer.blend = match layer.blend {
                LinearDodge => AddGlow,
                ColorDodge => GlowDodge,
                other => other,
            };
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
                .filter(|b| *b <= crate::document::MAX_PAYLOAD)
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
    // Dudel's stack runs bottom to top with each folder above its contents.
    // Photoshop writes records that way; other writers list them top first.
    // Group markers only nest correctly in the file's own direction.
    let order = if records.iter().all(|r| r.group == 0) {
        Order::Undetermined
    } else if nesting(records.iter().map(|r| r.group)).is_some() {
        Order::BottomFirst
    } else if nesting(records.iter().rev().map(|r| r.group)).is_some() {
        Order::TopFirst
    } else {
        return Err("PSD groups do not nest in either layer order".into());
    };
    if order == Order::TopFirst {
        records.reverse();
    }
    let depths =
        nesting(records.iter().map(|r| r.group)).ok_or("PSD group nesting exceeds limits")?;
    let mut result = Vec::new();
    for (mut record, depth) in records.into_iter().zip(depths) {
        if let Some(depth) = depth {
            record.layer.depth = depth;
            result.push(record.layer);
        }
    }
    if result.len() > MAX_ENTRIES || result.iter().filter(|l| !l.folder).count() > MAX_RASTER_LAYERS
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
    Ok((doc, order, notes))
}
/// Whether reversing a flat stack matches the saved image better, judged on
/// a grid of sample pixels.
fn reversed_matches_better(doc: &Document, saved: &[u8]) -> bool {
    let step = (doc.width * doc.height / 16384).max(1);
    let mut reversed = doc.clone();
    reversed.layers.reverse();
    let error = |doc: &Document| -> u64 {
        (0..doc.width * doc.height)
            .step_by(step)
            .map(|i| {
                let rendered = doc.composite_display_pixel(i, true);
                (0..4)
                    .map(|c| u64::from(rendered[c].abs_diff(saved[i * 4 + c])))
                    .sum::<u64>()
            })
            .sum()
    };
    error(&reversed) < error(doc)
}
/// The flattened image a PSD stores after its layers, as straight RGBA.
#[allow(clippy::too_many_arguments)]
fn saved_image(
    data: &[u8],
    width: usize,
    height: usize,
    channels: usize,
    depth: u16,
    has_alpha: bool,
    cancel: &impl Fn() -> bool,
) -> Result<Vec<u8>, String> {
    let raw = decode(data, width, height, channels, depth, cancel)?;
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
    remove_white_matte(&mut pixels);
    Ok(pixels)
}
/// Photoshop stores a transparent document's saved image blended against
/// white: a pixel of color C and alpha a is stored as C·a + 255·(1 − a).
/// Such an image never has a semi-transparent pixel darker than its white
/// share, so that decides it; other writers store plain colors. Returns
/// whether the matte was removed.
fn remove_white_matte(pixels: &mut [u8]) -> bool {
    let partial = || pixels.chunks_exact(4).filter(|p| p[3] > 0 && p[3] < 255);
    let total = partial().count();
    let darker = partial()
        .filter(|p| {
            p[..3]
                .iter()
                .any(|c| u16::from(*c) + 1 < 255 - u16::from(p[3]))
        })
        .count();
    if total == 0 || darker * 100 > total {
        return false;
    }
    for pixel in pixels.chunks_exact_mut(4) {
        let a = f32::from(pixel[3]);
        for c in &mut pixel[..3] {
            *c = if a == 0.0 {
                0
            } else {
                ((f32::from(*c) - (255.0 - a)) * 255.0 / a)
                    .round()
                    .clamp(0.0, 255.0) as u8
            };
        }
    }
    true
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
    if file.metadata().map_err(|e| e.to_string())?.len() > MAX_FILE as u64 {
        return Err("PSD exceeds 2 GB PSD limit".into());
    }
    let mut bytes = Vec::new();
    file.take(MAX_FILE as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > MAX_FILE {
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
    // A negative layer count marks the first extra channel as the merged
    // image's transparency.
    let has_alpha = if layer_data.0.len() >= 6 {
        let mut r = Reader(layer_data.0);
        let info = r.u32()?;
        info >= 2 && (r.u16()? as i16) < 0
    } else {
        false
    };
    let mut notes = Vec::new();
    if profile {
        notes.push(
            "The embedded color profile was not converted; Dudel uses the stored RGB values."
                .into(),
        );
    }
    // An empty layer list means the document is just its background image.
    let background_only = Reader(layer_data.0).u32().is_ok_and(|length| length == 0);
    if background_only {
        notes.push("This file holds only a background image; it was imported as one layer.".into());
    } else if layered && depth == 8 && !layer_data.0.is_empty() {
        match layers(layer_data.0, width, height, cancel) {
            Ok((mut document, order, layer_notes)) => {
                notes.push("Imported editable raster layers, supported groups, masks and blend modes. Rendering uses Dudel's compositor.".into());
                notes.extend(layer_notes);
                let flipped = match order {
                    Order::BottomFirst => false,
                    Order::TopFirst => true,
                    Order::Undetermined => {
                        has_composite
                            && document.layers.len() > 1
                            && saved_image(
                                reader.0, width, height, channels, depth, has_alpha, cancel,
                            )
                            .is_ok_and(|saved| reversed_matches_better(&document, &saved))
                    }
                };
                if order == Order::Undetermined && flipped {
                    document.layers.reverse();
                    document.active = document.layers.len() - 1;
                }
                if flipped {
                    notes.push("This file lists its layers top first, unlike Photoshop. Dudel read them in that order to match the file's saved image.".into());
                }
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
        let reason = notes
            .iter()
            .find_map(|n| n.split_once("unavailable: ").map(|(_, why)| why.to_owned()))
            .unwrap_or_default();
        return Err(format!(
            "The layers could not be imported ({reason}) and the file has no saved flattened image to fall back to. Save it with Maximize Compatibility enabled."
        ));
    }
    let pixels = saved_image(reader.0, width, height, channels, depth, has_alpha, cancel)?;
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
        blend: [u8; 4],
        /// Transparency Shapes Layer flag, when written.
        shapes: Option<u8>,
        flags: u8,
        /// Extra additional-info blocks (key, value).
        keys: &'static [(&'static [u8; 4], &'static [u8])],
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
            records.extend(b"8BIM");
            records.extend(spec.blend);
            records.extend([255, 0, spec.flags, 0]);
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
                let mut value = kind.to_be_bytes().to_vec();
                if kind != 3 {
                    value.extend(b"8BIM");
                    value.extend(spec.blend);
                }
                extra.extend(b"8BIMlsct");
                extra.extend(section(&value));
            }
            if let Some(flag) = spec.shapes {
                extra.extend(b"8BIMtsly");
                extra.extend(section(&[flag, 0, 0, 0]));
            }
            for (key, value) in spec.keys {
                extra.extend(b"8BIM");
                extra.extend(*key);
                extra.extend(section(value));
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
            blend: *b"norm",
            shapes: None,
            flags: 0,
            keys: &[],
        }
    }
    fn group(name: &'static str, kind: u32) -> Spec {
        Spec {
            name,
            left: 0,
            color: [0; 4],
            mask: false,
            section: Some(kind),
            blend: *b"norm",
            shapes: None,
            flags: 0,
            keys: &[],
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
    fn pass_through_dissolve_and_glow_dodges_import_as_editable_modes() {
        use crate::blend::BlendMode;
        let file = layer_file(&[
            raster("Paper", 0, [250, 250, 250, 255], false),
            group("</Layer set>", 3),
            Spec {
                blend: *b"lddg",
                shapes: Some(0),
                ..raster("Glow", 0, [200, 100, 0, 128], false)
            },
            Spec {
                blend: *b"div ",
                shapes: Some(1),
                ..raster("Dodge", 0, [9, 9, 9, 255], false)
            },
            Spec {
                blend: *b"diss",
                ..raster("Grain", 0, [1, 2, 3, 255], false)
            },
            Spec {
                blend: *b"pass",
                ..group("Lights", 1)
            },
        ]);
        let imported = parse(&file, &|| false).unwrap();
        assert!(imported.notes.iter().any(|n| n.contains("editable")));
        let blends: Vec<_> = imported.document.layers.iter().map(|l| l.blend).collect();
        assert_eq!(
            blends,
            [
                BlendMode::Normal,
                BlendMode::AddGlow,
                BlendMode::ColorDodge,
                BlendMode::Dissolve,
                BlendMode::PassThrough,
            ]
        );
    }
    #[test]
    fn top_first_files_are_read_in_their_own_order() {
        // Grouped: the folder record before its contents marks top-first.
        let file = layer_file(&[
            group("Folder", 1),
            raster("Ink", 0, [0, 0, 0, 255], false),
            group("</Layer set>", 3),
            raster("Paper", 0, [250, 250, 250, 255], false),
        ]);
        let imported = parse(&file, &|| false).unwrap();
        let stack: Vec<_> = imported
            .document
            .layers
            .iter()
            .map(|l| (l.name.as_str(), l.depth))
            .collect();
        assert_eq!(stack, [("Paper", 0), ("Ink", 1), ("Folder", 0)]);
        assert!(imported.notes.iter().any(|n| n.contains("top first")));
        // Flat: the saved image decides. Records list "Top" first, and
        // only a blue saved image means Top really is on top.
        for (saved, top_first) in [([0u8, 0, 255], true), ([255, 0, 0], false)] {
            let mut file = layer_file(&[
                raster("Top", 0, [0, 0, 255, 255], false),
                raster("Base", 0, [255, 0, 0, 255], false),
            ]);
            // Raw composite, 2 x 1 pixels, planar R, G, B.
            file.extend([0, 0]);
            for value in saved {
                file.extend([value, 0]);
            }
            let imported = parse(&file, &|| false).unwrap();
            let names: Vec<_> = imported
                .document
                .layers
                .iter()
                .map(|l| l.name.as_str())
                .collect();
            let expected = if top_first {
                ["Base", "Top"]
            } else {
                ["Top", "Base"]
            };
            assert_eq!(names, expected);
            assert_eq!(
                imported.notes.iter().any(|n| n.contains("top first")),
                top_first
            );
            assert_eq!(imported.document.active, 1);
        }
    }
    #[test]
    fn photoshop_records_text_fill_opacity_and_irrelevant_pixels() {
        use crate::blend::BlendMode;
        // Photoshop marks group records' pixels as irrelevant (flags 8 | 16),
        // stores text pixels in the layer and may set fill opacity.
        let file = layer_file(&[
            group("</Layer set>", 3),
            Spec {
                keys: &[(b"TySh", &[0, 0, 0, 1]), (b"iOpa", &[128, 0, 0, 0])],
                ..raster("Title", 0, [9, 9, 9, 255], false)
            },
            Spec {
                flags: 24,
                ..group("Group", 1)
            },
        ]);
        let imported = parse(&file, &|| false).unwrap();
        let text = &imported.document.layers[0];
        assert_eq!(text.pixels.rgba(0), [9, 9, 9, 255]);
        assert!((text.opacity - 128.0 / 255.0).abs() < 1e-6);
        assert!(
            imported
                .notes
                .iter()
                .any(|n| n.contains("“Title” is a text layer"))
        );
        // On an ordinary raster layer the same flag means its pixels do not
        // show what Photoshop draws, so they are not imported as art.
        let irrelevant = layer_file(&[Spec {
            flags: 24,
            ..raster("Shape", 0, [1, 1, 1, 255], false)
        }]);
        assert!(parse(&irrelevant, &|| false).is_err());
        // Fill opacity differs from opacity for the dodge/burn family.
        let dodge = layer_file(&[Spec {
            blend: *b"div ",
            keys: &[(b"iOpa", &[128, 0, 0, 0])],
            ..raster("Dodge", 0, [1, 1, 1, 255], false)
        }]);
        assert!(parse(&dodge, &|| false).is_err());
        assert_eq!(BlendMode::Normal, imported.document.layers[0].blend);
    }
    #[test]
    fn white_matte_is_removed_only_from_matted_images() {
        // Photoshop: 50% red stored as red blended with white.
        let mut matted = vec![255, 128, 128, 128, 0, 0, 0, 255, 255, 255, 255, 0];
        assert!(remove_white_matte(&mut matted));
        assert_eq!(matted, [255, 2, 2, 128, 0, 0, 0, 255, 0, 0, 0, 0]);
        // A straight dark semi-transparent pixel cannot be matted.
        let mut straight = vec![20, 20, 20, 128, 200, 10, 10, 64];
        assert!(!remove_white_matte(&mut straight));
        assert_eq!(straight, [20, 20, 20, 128, 200, 10, 10, 64]);
    }
    #[test]
    fn background_only_files_open_as_one_layer_with_a_plain_note() {
        let mut file = fixture(&[]);
        file.truncate(34);
        // Layer and mask section holding an empty layer list.
        file.extend(section(&[0, 0, 0, 0]));
        file.extend([0, 0, 10, 20, 30, 40, 50, 60]);
        let imported = parse(&file, &|| false).unwrap();
        assert_eq!(imported.document.layers.len(), 1);
        assert!(
            imported
                .notes
                .iter()
                .any(|n| n.contains("only a background image"))
        );
        assert!(!imported.notes.iter().any(|n| n.contains("unavailable")));
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
