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
    /// A length that is 8 bytes in PSB (large document) files.
    fn length(&mut self, big: bool) -> Result<usize, String> {
        if big {
            let n = u64::from_be_bytes(self.take(8)?.try_into().unwrap());
            usize::try_from(n).map_err(|_| "PSB length exceeds memory".to_string())
        } else {
            Ok(self.u32()? as usize)
        }
    }
    fn section_wide(&mut self, big: bool) -> Result<Reader<'a>, String> {
        let n = self.length(big)?;
        Ok(Reader(self.take(n)?))
    }
    fn rect(&mut self) -> Result<[i32; 4], String> {
        Ok([self.i32()?, self.i32()?, self.i32()?, self.i32()?])
    }
}
/// Tagged blocks whose length is 8 bytes in PSB files, as listed by the
/// independent psd-tools reader (MIT).
const PSB_WIDE_KEYS: [&[u8; 4]; 21] = [
    b"LMsk", b"Lr16", b"Lr32", b"Layr", b"Mt16", b"Mt32", b"Mtrn", b"Alph", b"FMsk", b"lnk2",
    b"lnk3", b"lnkE", b"FXid", b"FEid", b"FELS", b"PxSD", b"pths", b"extd", b"extn", b"cinf",
    b"artd",
];
fn wide_key(big: bool, key: &[u8]) -> bool {
    big && PSB_WIDE_KEYS.iter().any(|k| &k[..] == key)
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
    let side = 0..=crate::document::MAX_SIDE as i64;
    if !side.contains(&w) || !side.contains(&h) || w * h > crate::document::MAX_PIXELS as i64 {
        return Err("PSD layer dimensions exceed import limits".into());
    }
    Ok([w as usize, h as usize])
}
/// Channel samples, raw, RLE (row counts are 4 bytes in PSB) or ZIP.
fn decode(
    data: &[u8],
    width: usize,
    height: usize,
    channels: usize,
    depth: u16,
    big: bool,
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
                .map(|_| {
                    if big {
                        reader.u32().map(|n| n as usize)
                    } else {
                        reader.u16().map(usize::from)
                    }
                })
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
        for row in out.chunks_mut(stride.max(1)) {
            check(cancel)?;
            if depth == 16 {
                // Prediction is on big-endian samples, not individual bytes.
                let mut previous = 0u16;
                for sample in row.chunks_exact_mut(2) {
                    previous = previous.wrapping_add(u16::from_be_bytes([sample[0], sample[1]]));
                    sample.copy_from_slice(&previous.to_be_bytes());
                }
            } else {
                for x in 1..row.len() {
                    row[x] = row[x].wrapping_add(row[x - 1]);
                }
            }
        }
    }
    Ok(out)
}

/// One 8-bit channel of `width` x `height`: a compression code, then raw,
/// RLE or ZIP rows (as in layer channels and pattern blocks).
pub(crate) fn decode_plane(data: &[u8], width: usize, height: usize) -> Result<Vec<u8>, String> {
    decode(data, width, height, 1, 8, false, &|| false)
}

#[derive(Clone, Copy)]
enum ColorMode {
    Gray,
    Rgb,
    /// Cyan, magenta, yellow, black, stored inverted (255 is no ink).
    Cmyk,
}
impl ColorMode {
    fn channels(self) -> usize {
        match self {
            Self::Gray => 1,
            Self::Rgb => 3,
            Self::Cmyk => 4,
        }
    }
}

/// Turns a document's channel samples into RGB: gray and RGB as stored,
/// CMYK through the file's color profile when it has one (as Photoshop
/// displays it), otherwise by the plain ink formula.
struct Colors {
    mode: ColorMode,
    cmyk: Option<std::sync::Arc<moxcms::Transform8BitExecutor>>,
}
impl Colors {
    fn new(mode: ColorMode, icc: Option<&[u8]>) -> Self {
        let cmyk = icc
            .filter(|_| matches!(mode, ColorMode::Cmyk))
            .and_then(|icc| moxcms::ColorProfile::new_from_slice(icc).ok())
            .filter(|profile| profile.color_space == moxcms::DataColorSpace::Cmyk)
            .and_then(|profile| {
                profile
                    .create_transform_8bit(
                        moxcms::Layout::Rgba,
                        &moxcms::ColorProfile::new_srgb(),
                        moxcms::Layout::Rgba,
                        moxcms::TransformOptions::default(),
                    )
                    .ok()
            });
        Self { mode, cmyk }
    }
    /// RGB of `count` pixels whose channel `c` holds `sample(c, i)`.
    fn rgb(
        &self,
        count: usize,
        sample: impl Fn(usize, usize) -> u8,
        cancel: &impl Fn() -> bool,
    ) -> Result<Vec<[u8; 3]>, String> {
        let mut out = Vec::with_capacity(count);
        match self.mode {
            ColorMode::Gray => out.extend((0..count).map(|i| [sample(0, i); 3])),
            ColorMode::Rgb => out.extend((0..count).map(|i| [0, 1, 2].map(|c| sample(c, i)))),
            ColorMode::Cmyk => {
                const CHUNK: usize = 1 << 16;
                let mut ink = vec![0u8; CHUNK * 4];
                let mut converted = vec![0u8; CHUNK * 4];
                for start in (0..count).step_by(CHUNK) {
                    check(cancel)?;
                    let n = CHUNK.min(count - start);
                    for (i, pixel) in ink[..n * 4].chunks_exact_mut(4).enumerate() {
                        for (c, value) in pixel.iter_mut().enumerate() {
                            *value = 255 - sample(c, start + i);
                        }
                    }
                    match &self.cmyk {
                        Some(transform) => {
                            transform
                                .transform(&ink[..n * 4], &mut converted[..n * 4])
                                .map_err(|e| format!("CMYK color conversion: {e}"))?;
                            out.extend(
                                converted[..n * 4]
                                    .chunks_exact(4)
                                    .map(|p| [p[0], p[1], p[2]]),
                            );
                        }
                        None => out.extend(ink[..n * 4].chunks_exact(4).map(|p| {
                            let paper = |v: u8| u16::from(255 - v);
                            let k = paper(p[3]);
                            [0, 1, 2].map(|c| ((paper(p[c]) * k + 127) / 255) as u8)
                        })),
                    }
                }
            }
        }
        Ok(out)
    }
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
/// Import notes (something left out or changed) and details (background).
type Messages = (Vec<String>, Vec<String>);
/// Notes of one kind collected per layer, then written as one line each.
#[derive(Default)]
struct Grouped(Vec<(String, Vec<String>)>);
impl Grouped {
    fn add(&mut self, kind: String, name: &str) {
        match self.0.iter_mut().find(|(k, _)| *k == kind) {
            Some((_, names)) => names.push(name.to_owned()),
            None => self.0.push((kind, vec![name.to_owned()])),
        }
    }
    fn lines(self) -> impl Iterator<Item = String> {
        self.0
            .into_iter()
            .map(|(kind, names)| format!("{kind}: {}.", crate::clip_metadata::name_list(&names)))
    }
}
/// A layer's mask record: the user mask, its density and feather, the vector
/// mask's density and feather, and the "real" user mask Photoshop stores
/// separately (channel -3) when the layer also has a vector mask.
#[derive(Clone, Copy)]
struct MaskInfo {
    rect: [i32; 4],
    default: u8,
    flags: u8,
    density: Option<u8>,
    feather: f64,
    vector_density: Option<u8>,
    vector_feather: f64,
    real: Option<([i32; 4], u8, u8)>,
}
/// What a fill or shape layer paints.
#[derive(Clone)]
enum Fill {
    Solid([u8; 3]),
    Gradient(crate::psd_vector::Gradient),
}
struct Record {
    layer: Layer,
    rect: [i32; 4],
    channels: Vec<(i16, usize)>,
    group: u32,
    mask: Option<MaskInfo>,
    solid: Option<Fill>,
    /// Vector mask path; for a shape layer it outlines the fill.
    vector: Option<crate::psd_vector::VectorMask>,
    stroke: Option<crate::psd_vector::Stroke>,
    /// Whether a shape layer's fill is drawn (strokes may stand alone).
    filled: bool,
    effects: crate::layer_effects::Effects,
    /// Layer opacity without fill opacity, which effects ignore.
    opacity: f32,
    fill: u8,
    /// For shape and fill layers, the coverage their effects follow: the
    /// path's fill, without the shape's stroke or the fill's transparency.
    source: Option<Vec<f32>>,
    /// Effect layers: under the layer, clipped to it, and over its clip group.
    below: Vec<Layer>,
    inside: Vec<Layer>,
    above: Vec<Layer>,
    /// Left out of the import (named in the notes).
    dropped: bool,
}

/// Whether Photoshop's fill opacity acts differently from opacity in this
/// blend mode (Dudel draws fill as opacity).
pub(crate) fn fill_differs(blend: crate::blend::BlendMode) -> bool {
    use crate::blend::BlendMode::*;
    matches!(
        blend,
        ColorBurn
            | LinearBurn
            | ColorDodge
            | LinearDodge
            | VividLight
            | LinearLight
            | HardMix
            | Difference
    )
}

/// Draw a layer's effects as layers (Photoshop's Create Layers), from its
/// final pixels and mask.
fn effect_layers(
    record: &mut Record,
    width: usize,
    height: usize,
    budget: &mut usize,
    cancel: &impl Fn() -> bool,
) -> Result<(), String> {
    let effects = std::mem::take(&mut record.effects.list);
    let source = record.source.take();
    // Effects that follow the layer's own pixels stay editable on it, with
    // its fill; shape outlines have no native form, so those bake.
    let native: Vec<_> = effects
        .iter()
        .cloned()
        .map(crate::layer_effects::LayerEffect::new)
        .collect();
    if source.is_none()
        && native.len() <= crate::document::MAX_EFFECTS
        && native.iter().all(|e| e.validate().is_ok())
    {
        record.layer.effects = native;
        return Ok(());
    }
    [record.below, record.inside, record.above] = crate::layer_effects::draw(
        &effects,
        &record.layer,
        record.opacity,
        record.fill,
        None,
        source.as_deref(),
        [width, height],
        budget,
        cancel,
    )?;
    Ok(())
}
/// Photoshop's mask density (how much of what the mask hides stays hidden)
/// and feather (a Gaussian blur of `feather` pixels).
fn soften_mask(
    mask: &mut crate::layer_mask::Mask,
    density: Option<u8>,
    feather: f64,
    width: usize,
    height: usize,
    cancel: &impl Fn() -> bool,
) -> Result<(), String> {
    if feather > 0.0 {
        let mut values: Vec<f32> = (0..width * height)
            .map(|i| f32::from(mask.at(i % width, i / width)))
            .collect();
        crate::layer_effects::gaussian(&mut values, width, height, feather, cancel)?;
        let mut writer = mask.writer();
        for (i, v) in values.iter().enumerate() {
            writer.set(i % width, i / width, v.round().clamp(0.0, 255.0) as u8);
        }
    }
    if let Some(density) = density.filter(|d| *d < 255) {
        let mut writer = mask.writer();
        for y in 0..height {
            check(cancel)?;
            for x in 0..width {
                let v = u16::from(writer.at(x, y));
                writer.set(x, y, (255 - (255 - v) * u16::from(density) / 255) as u8);
            }
        }
    }
    Ok(())
}
/// A shape layer: its fill clipped by the path, with any stroke on top.
/// Returns the fill's coverage, which is what the layer's effects follow.
#[allow(clippy::too_many_arguments)]
fn draw_shape(
    layer: &mut Layer,
    paint: &dyn Fn(usize, usize) -> [u8; 4],
    vector: Option<&crate::psd_vector::VectorMask>,
    stroke: Option<crate::psd_vector::Stroke>,
    filled: bool,
    // The vector mask's density and feather.
    (density, feather): (Option<u8>, f64),
    width: usize,
    height: usize,
    cancel: &impl Fn() -> bool,
) -> Result<Vec<f32>, String> {
    // A fill layer without a path covers the whole canvas.
    let mut fill = match vector {
        Some(vector) => {
            let mut fill = vector.coverage(width, height, cancel)?;
            if vector.inverted {
                fill.iter_mut().for_each(|c| *c = 1.0 - *c);
            }
            fill
        }
        None => vec![1.0; width * height],
    };
    let outline = match (stroke, vector) {
        (Some(stroke), Some(vector)) => Some((
            vector.stroke_coverage(&stroke, &fill, width, height, cancel)?,
            stroke,
        )),
        _ => None,
    };
    // The stroke follows the sharp outline; the fill takes the mask's
    // feather and density.
    if vector.is_some() && feather > 0.0 {
        crate::layer_effects::gaussian(&mut fill, width, height, feather, cancel)?;
    }
    if let Some(density) = density.filter(|d| *d < 255) {
        let density = f32::from(density) / 255.0;
        fill.iter_mut()
            .for_each(|c| *c = 1.0 - (1.0 - *c) * density);
    }
    let mut writer = layer.pixels.writer();
    for y in 0..height {
        check(cancel)?;
        for x in 0..width {
            let i = y * width + x;
            let (s, stroke_color) = outline.as_ref().map_or((0.0, [0; 3]), |(plane, stroke)| {
                (plane[i] * stroke.opacity, stroke.color)
            });
            let covered = if filled { fill[i].clamp(0.0, 1.0) } else { 0.0 };
            if s <= 0.0 && covered <= 0.0 {
                continue;
            }
            let color = paint(x, y);
            let f = covered * f32::from(color[3]) / 255.0;
            let alpha = s + f * (1.0 - s);
            let a = (alpha * 255.0).round() as u8;
            if a == 0 {
                continue;
            }
            let rgb: [u8; 3] = std::array::from_fn(|c| {
                ((f32::from(stroke_color[c]) * s + f32::from(color[c]) * f * (1.0 - s)) / alpha)
                    .round() as u8
            });
            writer
                .rgba_mut(i)
                .copy_from_slice(&[rgb[0], rgb[1], rgb[2], a]);
        }
    }
    Ok(fill)
}
/// A vector mask on an ordinary layer or folder becomes its layer mask,
/// multiplied into any raster mask it already has.
#[allow(clippy::too_many_arguments)]
fn apply_vector_mask(
    layer: &mut Layer,
    vector: &crate::psd_vector::VectorMask,
    density: Option<u8>,
    feather: f64,
    width: usize,
    height: usize,
    cancel: &impl Fn() -> bool,
) -> Result<(), String> {
    let mut coverage = vector.coverage(width, height, cancel)?;
    for c in &mut coverage {
        let v = c.clamp(0.0, 1.0);
        *c = if vector.inverted { 1.0 - v } else { v };
    }
    if feather > 0.0 {
        crate::layer_effects::gaussian(&mut coverage, width, height, feather, cancel)?;
    }
    let density = f32::from(density.unwrap_or(255)) / 255.0;
    let value = |i: usize| 1.0 - (1.0 - coverage[i]) * density;
    if layer.mask.is_none() {
        layer.mask = Some(crate::layer_mask::LayerMask {
            id: crate::document::LayerId::new(),
            enabled: true,
            linked: true,
            coverage: crate::layer_mask::Mask::new(width, height, 255)?,
        });
    }
    let mask = &mut layer.mask.as_mut().unwrap().coverage;
    for y in 0..height {
        check(cancel)?;
        for x in 0..width {
            let existing = f32::from(mask.at(x, y));
            let next = (existing * value(y * width + x)).round() as u8;
            if next != mask.at(x, y) {
                mask.writer().set(x, y, next);
            }
        }
    }
    Ok(())
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
#[allow(clippy::too_many_arguments)]
fn layers(
    data: &[u8],
    width: usize,
    height: usize,
    depth: u16,
    colors: &Colors,
    global_angle: f64,
    patterns: &[crate::pattern::Pattern],
    big: bool,
    cancel: &impl Fn() -> bool,
) -> Result<(Document, Order, Messages), String> {
    let mode = colors.mode;
    // After the layer records come the global layer mask (how masks are shown
    // in the editor, not pixels) and document-wide blocks such as patterns,
    // text engine data and linked files. Those only matter to layers that use
    // them, and such layers carry their own markers, checked per layer.
    let mut info = Reader(data).section_wide(big)?;
    let count = (info.u16()? as i16).unsigned_abs() as usize;
    if count == 0 || count > MAX_ENTRIES * 2 {
        return Err("PSD layer count exceeds limits or is empty".into());
    }
    let mut records = Vec::new();
    let (mut notes, mut details) = (Vec::new(), Vec::new());
    let gray = matches!(mode, ColorMode::Gray);
    // Per-layer notes of one kind become one line naming the layers.
    let mut grouped = Grouped::default();
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
            let len = info.length(big)?;
            if !(-3..mode.channels() as i16).contains(&id)
                || channels.iter().any(|(other, _)| *other == id)
            {
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
            let rect = mask_data.rect()?;
            geometry(rect)?;
            let default = mask_data.u8()?;
            let flags = mask_data.u8()?;
            // 1 position relative to the layer, 2 disabled, 4 inverted,
            // 8 rendered from other data (a vector mask), 16 parameters follow.
            if flags & !31 != 0 {
                return Err("Unsupported PSD mask flags".into());
            }
            let f64 = |r: &mut Reader| -> Result<f64, String> {
                let v = f64::from_be_bytes(r.take(8)?.try_into().unwrap());
                if v.is_finite() && (0.0..=1000.0).contains(&v) {
                    Ok(v)
                } else {
                    Err("Invalid PSD mask feather".into())
                }
            };
            let mut info = MaskInfo {
                rect,
                default,
                flags,
                density: None,
                feather: 0.0,
                vector_density: None,
                vector_feather: 0.0,
                real: None,
            };
            if flags & 16 != 0 {
                let parameters = mask_data.u8()?;
                if parameters & 1 != 0 {
                    info.density = Some(mask_data.u8()?);
                }
                if parameters & 2 != 0 {
                    info.feather = f64(&mut mask_data)?;
                }
                if parameters & 4 != 0 {
                    info.vector_density = Some(mask_data.u8()?);
                }
                if parameters & 8 != 0 {
                    info.vector_feather = f64(&mut mask_data)?;
                }
            }
            if mask_data.0.len() >= 18 {
                let flags = mask_data.u8()?;
                let default = mask_data.u8()?;
                let rect = mask_data.rect()?;
                geometry(rect)?;
                if flags & !15 != 0 {
                    return Err("Unsupported PSD mask flags".into());
                }
                info.real = Some((rect, default, flags));
            }
            Some(info)
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
        let mut adjustment = None;
        let mut legacy_brightness = None;
        let mut unsupported = None;
        let mut vector = None;
        let mut stroke = None;
        let mut filled = true;
        // Effects in the newest form present: lmfx, then lfx2, then lfxs.
        let mut effects: [Option<crate::layer_effects::Effects>; 3] = Default::default();
        let mut legacy_effects = false;
        while extra.0.len() >= 12 {
            if extra.take(4)? != b"8BIM" {
                return Err("Unsupported PSD additional-info signature".into());
            }
            let key = extra.take(4)?;
            let len = extra.length(wide_key(big, key))?;
            let mut value = Reader(extra.take(len)?);
            if !len.is_multiple_of(2) {
                extra.take(1)?;
            }
            match key {
                b"SoCo" => {
                    solid = Some(Fill::Solid(crate::psd_vector::solid_fill(
                        &crate::psd_vector::versioned(value.0)?,
                    )?));
                }
                b"GdFl" => {
                    solid = Some(Fill::Gradient(crate::psd_vector::gradient(
                        &crate::psd_vector::versioned(value.0)?,
                    )?));
                }
                b"vscg" => {
                    let (fill, settings) = crate::psd_vector::keyed(value.0)?;
                    solid = Some(match fill.as_str() {
                        "SoCo" => Fill::Solid(crate::psd_vector::solid_fill(&settings)?),
                        "GdFl" => Fill::Gradient(crate::psd_vector::gradient(&settings)?),
                        _ => {
                            return Err(format!("PSD shape fill {fill} needs the saved composite"));
                        }
                    });
                }
                b"vmsk" | b"vsms" => {
                    vector = Some(crate::psd_vector::vector_mask(value.0, width, height)?);
                }
                b"vstk" => {
                    (filled, stroke) =
                        crate::psd_vector::stroke(&crate::psd_vector::versioned(value.0)?)?;
                }
                // Live-shape parameters, pixel-aligned rendering and a vector
                // version word; the path itself is in vmsk/vsms.
                b"vogk" | b"sn2P" | b"vowv" => {}
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
                b"lmfx" | b"lfx2" | b"lfxs" => {
                    let slot = [&b"lmfx"[..], b"lfx2", b"lfxs"]
                        .iter()
                        .position(|k| *k == key)
                        .unwrap();
                    effects[slot] = Some(crate::layer_effects::parse(
                        value.0,
                        global_angle,
                        patterns,
                    )?);
                }
                // The pre-CS effects block, which Photoshop still writes next
                // to the descriptor form.
                b"lrFX" => legacy_effects = true,
                b"infx" | b"knko" if value.u8()? == 0 => {}
                b"lyid" | b"lclr" | b"lnsr" | b"shmd" | b"fxrp" | b"lyvr" => {}
                // Brightness/contrast from CS3 on; `brit` holds legacy values.
                b"CgEd" => {
                    adjustment = Some(crate::adjustment::brightness_contrast_descriptor(
                        &crate::psd_vector::versioned(value.0)?,
                    ));
                }
                _ => match crate::adjustment::from_psd(key, value.0, gray) {
                    Ok(Some(settings)) if key == b"brit" => legacy_brightness = Some(settings),
                    Ok(Some(settings)) => adjustment = Some(settings),
                    Err(error) if error.starts_with("Damaged") => return Err(error),
                    Err(kind) => unsupported = Some(kind),
                    Ok(None) => {
                        return Err(format!(
                            "PSD layer feature {} needs the saved composite",
                            String::from_utf8_lossy(key)
                        ));
                    }
                },
            }
        }
        if extra.0.iter().any(|b| *b != 0) {
            return Err("Truncated PSD additional info".into());
        }
        let effects = effects.into_iter().flatten().next();
        if legacy_effects && effects.is_none() {
            return Err("Legacy PSD layer effects need the saved composite".into());
        }
        let mut effects = effects.unwrap_or_default();
        // Bit 4 (with bit 3 saying it is meaningful) marks pixel data that does
        // not affect the document: normal for group records, unknown elsewhere.
        // A disabled vector mask does not apply.
        if vector.as_ref().is_some_and(|v| v.disabled) {
            vector = None;
        }
        if stroke.is_some() && (solid.is_none() || vector.is_none()) {
            return Err("PSD shape stroke without a shape needs the saved composite".into());
        }
        // Shape layers are drawn from their path and fill, not their pixels.
        let adjustment = adjustment.or(legacy_brightness);
        // CMYK adjustments name CMYK channels (curves channel 1 is cyan) and
        // usually define the picture; Dudel's adjustments act on RGB.
        if matches!(colors.mode, ColorMode::Cmyk) && (adjustment.is_some() || unsupported.is_some())
        {
            return Err("Adjustment layers in CMYK files need the saved composite".into());
        }
        let dropped = unsupported.is_some() || (adjustment.is_some() && group != 0);
        if let Some(kind) = &unsupported {
            grouped.add(
                format!("Adjustment layers left out ({kind} cannot be applied yet)"),
                &layer.name,
            );
        }
        if let Some(settings) = adjustment {
            layer.adjustment = Some(crate::adjustment::Adjustment::new(settings));
        }
        if flags & 24 == 24
            && group == 0
            && solid.is_none()
            && layer.adjustment.is_none()
            && !dropped
        {
            return Err("PSD layer pixels are marked irrelevant to its appearance".into());
        }
        if fill != 255 {
            if fill_differs(layer.blend) {
                return Err("PSD fill opacity on this blend mode needs the saved composite".into());
            }
            if group == 0 {
                layer.fill = f32::from(fill) / 255.0;
            } else {
                // Folders have no fill of their own; it fades the whole group.
                layer.opacity *= f32::from(fill) / 255.0;
            }
        }
        if group == 3 {
            effects.list.clear();
        }
        if !effects.skipped.is_empty() {
            grouped.add(
                format!("Layer effects left out ({})", effects.skipped.join(", ")),
                &layer.name,
            );
        }
        if !effects.approximated.is_empty() {
            grouped.add(
                format!(
                    "Layer effects drawn without some settings ({})",
                    effects.approximated.join(", ")
                ),
                &layer.name,
            );
        }
        if let Some(kind) = rendered {
            grouped.add(
                if kind == "text" {
                    "Text layers imported as pixels (their text cannot be edited here)".into()
                } else {
                    "Smart objects imported as pixels (their source cannot be edited here)".into()
                },
                &layer.name,
            );
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
            vector,
            stroke,
            filled,
            effects,
            opacity: f32::from(opacity) / 255.0,
            fill,
            source: None,
            below: Vec::new(),
            inside: Vec::new(),
            above: Vec::new(),
            dropped,
        });
    }
    notes.extend(grouped.lines());
    let mut decoded_budget = 0usize;
    for record in &mut records {
        check(cancel)?;
        if record.dropped {
            // Keep the stream aligned, but never decode a skipped layer's pixels.
            for &(_, bytes) in &record.channels {
                info.take(bytes)?;
            }
            continue;
        }
        let [w, h] = geometry(record.rect)?;
        let mut planes = std::collections::BTreeMap::new();
        for &(id, bytes) in &record.channels {
            let data = info.take(bytes)?;
            if record.group != 0 && id > -2 {
                continue;
            }
            let [cw, ch] = match id {
                -2 => geometry(record.mask.ok_or("PSD mask channel has no metadata")?.rect)?,
                -3 => geometry(
                    record
                        .mask
                        .and_then(|m| m.real)
                        .ok_or("PSD real mask channel has no metadata")?
                        .0,
                )?,
                _ => [w, h],
            };
            decoded_budget = decoded_budget
                .checked_add(cw * ch)
                .filter(|b| *b <= crate::document::MAX_PAYLOAD)
                .ok_or("PSD layer payload exceeds memory budget")?;
            let plane = decode(data, cw, ch, 1, depth, big, cancel)?;
            planes.insert(id, if depth == 16 { to_8_bit(&plane) } else { plane });
        }
        if let Some(fill) = &record.solid {
            // A gradient spans the shape's path, else the layer's own bounds.
            let rect = record.rect.map(f64::from);
            let layer_box = record.vector.as_ref().and_then(|v| v.bounds()).unwrap_or(
                if rect[2] > rect[0] && rect[3] > rect[1] {
                    [rect[1], rect[0], rect[3], rect[2]]
                } else {
                    [0.0, 0.0, width as f64, height as f64]
                },
            );
            let paint: Box<dyn Fn(usize, usize) -> [u8; 4] + '_> = match fill {
                Fill::Solid(c) => {
                    let c = *c;
                    Box::new(move |_, _| [c[0], c[1], c[2], 255])
                }
                Fill::Gradient(g) => Box::new(g.painter(layer_box, [width, height])),
            };
            let fill = draw_shape(
                &mut record.layer,
                &paint,
                record.vector.as_ref(),
                record.stroke,
                record.filled,
                record
                    .mask
                    .map_or((None, 0.0), |m| (m.vector_density, m.vector_feather)),
                width,
                height,
                cancel,
            )?;
            if !record.effects.list.is_empty() {
                record.source = Some(fill);
            }
        } else if record.group == 0 && w * h != 0 {
            if !(0..mode.channels() as i16).all(|id| planes.contains_key(&id)) {
                return Err("PSD layer is missing color pixels".into());
            }
            let rgb = colors.rgb(w * h, |c, i| planes[&(c as i16)][i], cancel)?;
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
                    let [red, green, blue] = rgb[i];
                    let rgba = [red, green, blue, planes.get(&-1).map_or(255, |a| a[i])];
                    if rgba != [0; 4] {
                        writer
                            .rgba_mut(dy as usize * width + dx as usize)
                            .copy_from_slice(&rgba);
                    }
                }
            }
        }
        // With a vector mask, the -2 channel holds the mask Photoshop rendered
        // from it (drawn here from the path instead); the user's own mask is
        // then the "real" one in channel -3.
        let user_mask = record
            .mask
            .and_then(|info| match (info.real, planes.get(&-3)) {
                (Some((rect, default, flags)), Some(plane)) => Some((rect, default, flags, plane)),
                _ if info.flags & 8 != 0 && record.vector.is_some() => None,
                _ => planes
                    .get(&-2)
                    .map(|plane| (info.rect, info.default, info.flags, plane)),
            });
        if record.mask.is_some() && user_mask.is_none() && !planes.contains_key(&-2) {
            return Err("PSD mask metadata has no pixels".into());
        }
        if let Some((rect, default, flags, plane)) = user_mask {
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
            let info = record.mask.unwrap();
            soften_mask(
                &mut coverage,
                info.density,
                info.feather,
                width,
                height,
                cancel,
            )?;
            record.layer.mask = Some(crate::layer_mask::LayerMask {
                id: crate::document::LayerId::new(),
                enabled: flags & 2 == 0,
                linked: true,
                coverage,
            });
        }
        if let (None, Some(vector)) = (&record.solid, &record.vector) {
            let (density, feather) = record
                .mask
                .map_or((None, 0.0), |m| (m.vector_density, m.vector_feather));
            apply_vector_mask(
                &mut record.layer,
                vector,
                density,
                feather,
                width,
                height,
                cancel,
            )?;
        }
        if record.group == 0 && !record.effects.list.is_empty() {
            effect_layers(record, width, height, &mut decoded_budget, cancel)?;
        }
    }
    if records.iter().any(|r| !r.layer.effects.is_empty()) {
        details.push(
            "Layer effects stay editable: select a layer and open fx in the layer properties."
                .into(),
        );
    }
    if records.iter().any(|r| {
        !r.below.is_empty()
            || !r.inside.is_empty()
            || !r.above.is_empty()
            || (r.group != 0 && !r.effects.list.is_empty())
    }) {
        notes.push("Layer effects became separate layers named after their layer, like Photoshop's Create Layers; they no longer follow edits to that layer.".into());
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
    // Effects drawn over a layer wait until the layers clipped to it end.
    let mut pending: Vec<Layer> = Vec::new();
    let mut pending_depth = 0;
    // Where each open folder's contents start in `result`.
    let mut starts = Vec::new();
    for (mut record, depth) in records.into_iter().zip(depths) {
        let Some(depth) = depth else {
            // A folder starting ends the clip group below it.
            result.append(&mut pending);
            starts.push(result.len());
            continue;
        };
        if record.dropped {
            continue;
        }
        if depth < pending_depth || (depth == pending_depth && !record.layer.clipped) {
            result.append(&mut pending);
        }
        if record.group != 0 {
            let start = starts.pop().ok_or("PSD group nesting exceeds limits")?;
            let native: Vec<_> = record
                .effects
                .list
                .iter()
                .cloned()
                .map(crate::layer_effects::LayerEffect::new)
                .collect();
            if !native.is_empty()
                && record.fill == 255
                && native.len() <= crate::document::MAX_EFFECTS
                && native.iter().all(|e| e.validate().is_ok())
            {
                // Native folder effects follow its contents as Photoshop's do.
                record.layer.effects = native;
            } else if !record.effects.list.is_empty() {
                // A folder's effects follow its composited contents.
                let mut contents = Document::new(width, height);
                contents.layers = result[start..].to_vec();
                for layer in &mut contents.layers {
                    layer.depth -= depth + 1;
                }
                let alpha: Vec<f32> = contents
                    .composite_transparent()
                    .chunks_exact(4)
                    .map(|p| f32::from(p[3]) / 255.0)
                    .collect();
                let [mut below, inside, above] = crate::layer_effects::draw(
                    &record.effects.list,
                    &record.layer,
                    record.opacity,
                    record.fill,
                    Some(&alpha),
                    None,
                    [width, height],
                    &mut decoded_budget,
                    cancel,
                )?;
                below.iter_mut().for_each(|l| l.depth = depth);
                result.splice(start..start, below);
                (record.inside, record.above) = (inside, above);
            }
        }
        let mut layers = record.below;
        layers.push(record.layer);
        layers.extend(record.inside);
        for layer in &mut layers {
            layer.depth = depth;
        }
        result.extend(layers);
        if !record.above.is_empty() {
            pending.extend(record.above.into_iter().map(|mut layer| {
                layer.depth = depth;
                layer
            }));
            pending_depth = depth;
        }
    }
    result.append(&mut pending);
    if result.is_empty() {
        return Err("No layer Dudel can import remains".into());
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
    Ok((doc, order, (notes, details)))
}
/// Whether reversing a flat stack matches the saved image better, judged on
/// a grid of sample pixels.
fn reversed_matches_better(doc: &Document, saved: &[u8]) -> bool {
    let step = (doc.width * doc.height / 16384).max(1);
    let mut reversed = doc.clone();
    reversed.layers.reverse();
    let error = |doc: &Document| -> u64 {
        // Compare the same appearance the imported document will render.
        // Expand once per candidate rather than once per sampled pixel.
        let expanded = if doc.needs_expansion() {
            crate::layer_effects::expand(doc, false, &|| false).ok()
        } else {
            None
        };
        let doc = expanded.as_ref().unwrap_or(doc);
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
    colors: &Colors,
    has_alpha: bool,
    big: bool,
    cancel: &impl Fn() -> bool,
) -> Result<Vec<u8>, String> {
    let mode = colors.mode;
    let raw = decode(data, width, height, channels, depth, big, cancel)?;
    let samples = width * height;
    let value = |c: usize, i: usize| {
        let at = (c * samples + i) * usize::from(depth / 8);
        if depth == 8 {
            raw[at]
        } else {
            sample_8_bit([raw[at], raw[at + 1]])
        }
    };
    let rgb = colors.rgb(samples, value, cancel)?;
    let mut pixels = vec![0; samples * 4];
    for (i, pixel) in pixels.chunks_exact_mut(4).enumerate() {
        if i % width == 0 {
            check(cancel)?;
        }
        let [red, green, blue] = rgb[i];
        pixel.copy_from_slice(&[
            red,
            green,
            blue,
            if has_alpha && channels > mode.channels() {
                value(mode.channels(), i)
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
/// A big-endian 16-bit sample rounded to 8 bits.
fn sample_8_bit(sample: [u8; 2]) -> u8 {
    ((u32::from(u16::from_be_bytes(sample)) + 128) / 257) as u8
}
fn to_8_bit(samples: &[u8]) -> Vec<u8> {
    samples
        .chunks_exact(2)
        .map(|sample| sample_8_bit([sample[0], sample[1]]))
        .collect()
}
fn parse(data: &[u8], cancel: &impl Fn() -> bool) -> Result<Imported, String> {
    parse_mode(data, cancel, true)
}
/// High-depth Photoshop files put their layer list in Lr16, leaving the
/// ordinary list empty. Its signed count still controls merged transparency.
fn layer_count(data: &[u8], depth: u16, big: bool) -> Result<i16, String> {
    if data.is_empty() {
        return Ok(0);
    }
    if let Some(mut list) = high_depth_layers(data, depth, big)? {
        return list.u16().map(|v| v as i16);
    }
    let mut primary = Reader(data).section_wide(big)?;
    Ok(if primary.0.is_empty() {
        0
    } else {
        primary.u16()? as i16
    })
}
/// The Lr16 layer list of a 16-bit file: the same layout as the ordinary
/// layer list, with 16-bit channel samples.
fn high_depth_layers(data: &[u8], depth: u16, big: bool) -> Result<Option<Reader<'_>>, String> {
    let mut reader = Reader(data);
    reader.section_wide(big)?;
    if depth != 16 || reader.0.is_empty() {
        return Ok(None);
    }
    // Some writers omit the global mask field entirely.
    if !reader.0.starts_with(b"8BIM") && !reader.0.starts_with(b"8B64") {
        reader.section()?; // Global layer mask (display metadata).
    }
    while !reader.0.is_empty() {
        if reader.0.len() < 12 && reader.0.iter().all(|b| *b == 0) {
            break;
        }
        let signature = reader.take(4)?;
        if signature != b"8BIM" && signature != b"8B64" {
            return Err("Invalid PSD global additional-info signature".into());
        }
        let key = reader.take(4)?;
        let value = reader.section_wide(wide_key(big, key))?;
        let padding = (4 - value.0.len() % 4) % 4;
        reader.take(padding)?;
        if key == b"Lr16" {
            return Ok(Some(value));
        }
    }
    Ok(None)
}
/// The patterns in the document-wide blocks after the layer records, which
/// pattern effects name by id. Blocks that cannot be read give none.
fn global_patterns(data: &[u8], big: bool) -> Vec<crate::pattern::Pattern> {
    let mut reader = Reader(data);
    if reader.section_wide(big).is_err() {
        return Vec::new();
    }
    if !reader.0.starts_with(b"8BIM") && !reader.0.starts_with(b"8B64") && reader.section().is_err()
    {
        return Vec::new();
    }
    let mut patterns = Vec::new();
    while reader.0.len() >= 12 {
        let (Ok(signature), Ok(key)) = (reader.take(4), reader.take(4)) else {
            break;
        };
        if signature != b"8BIM" && signature != b"8B64" {
            break;
        }
        let Ok(value) = reader.section_wide(wide_key(big, key)) else {
            break;
        };
        if matches!(key, b"Patt" | b"Pat2" | b"Pat3") {
            patterns.extend(crate::pattern::read_psd(value.0));
        }
        if reader.take((4 - value.0.len() % 4) % 4).is_err() {
            break;
        }
    }
    patterns
}
fn parse_mode(data: &[u8], cancel: &impl Fn() -> bool, layered: bool) -> Result<Imported, String> {
    check(cancel)?;
    let mut reader = Reader(data);
    if reader.take(4)? != b"8BPS" {
        return Err("Expected a Photoshop PSD or PSB file".into());
    }
    // Version 2 is PSB, the large document format: the same layout with some
    // lengths widened to 8 bytes.
    let big = match reader.u16()? {
        1 => false,
        2 => true,
        _ => return Err("Unsupported Photoshop file version".into()),
    };
    if reader.take(6)? != [0; 6] {
        return Err("Invalid Photoshop file header".into());
    }
    let channels = reader.u16()? as usize;
    let height = reader.u32()? as usize;
    let width = reader.u32()? as usize;
    let depth = reader.u16()?;
    let mode = match reader.u16()? {
        1 => ColorMode::Gray,
        3 => ColorMode::Rgb,
        4 => ColorMode::Cmyk,
        other => {
            return Err(format!(
                "Unsupported PSD color mode {other}; RGB and grayscale are supported"
            ));
        }
    };
    if width == 0
        || height == 0
        || !(mode.channels()..=56).contains(&channels)
        || ![8, 16].contains(&depth)
    {
        return Err(
            "PSD import supports RGB/grayscale 8/16-bit canvases with valid channel counts".into(),
        );
    }
    if width > crate::document::MAX_SIDE
        || height > crate::document::MAX_SIDE
        || width * height > crate::document::MAX_PIXELS
    {
        return Err(format!(
            "This canvas is {width} × {height} px; Dudel canvases go {}",
            crate::document::SIZE_LIMIT
        ));
    }
    reader.section()?;
    let mut resources = reader.section()?;
    let mut has_composite = true;
    let mut resolution = None;
    let mut profile: Option<&[u8]> = None;
    // Photoshop's default light angle, for files without one.
    let mut global_angle = 120.0;
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
        if id == 1005 && data.0.len() == 16 {
            // Adobe ResolutionInfo: unsigned 16.16 pixels/inch; unit words
            // select UI display units and do not change these values.
            let x = u32::from_be_bytes(data.0[0..4].try_into().unwrap()) as f64 / 65536.0;
            let y = u32::from_be_bytes(data.0[8..12].try_into().unwrap()) as f64 / 65536.0;
            resolution = Some((x, y));
        } else if id == 1057 {
            let mut version = data;
            version.u32()?;
            has_composite = version.u8()? != 0;
        } else if id == 1039 {
            profile = Some(data.0).filter(|icc| !icc.is_empty());
        } else if id == 1037 && data.0.len() >= 4 {
            let mut angle = data;
            global_angle = f64::from(angle.u32()? as i32);
        }
    }
    let layer_data = reader.section_wide(big)?;
    // A negative layer count marks the first extra channel as the merged
    // image's transparency.
    let count = layer_count(layer_data.0, depth, big)?;
    let has_alpha = count < 0;
    let (mut notes, mut details) = (Vec::new(), Vec::new());
    let dpi = match resolution {
        Some((x, y))
            if crate::document::validate_dpi(x).is_ok()
                && crate::document::validate_dpi(y).is_ok() =>
        {
            if (x - y).abs() > 0.001 {
                notes.push(format!(
                    "Source resolution is {x} \u{00d7} {y} DPI. Dudel uses {x} DPI for both axes."
                ));
            }
            x
        }
        Some(_) => {
            notes.push("Invalid source resolution; using 72 DPI.".into());
            crate::document::default_dpi()
        }
        None => crate::document::default_dpi(),
    };
    if matches!(mode, ColorMode::Gray) {
        details
            .push("Expanded grayscale samples to equal RGB channels for editing in Dudel.".into());
    }
    let colors = Colors::new(mode, profile);
    if matches!(mode, ColorMode::Cmyk) {
        if colors.cmyk.is_some() {
            details.push(
                "Converted CMYK to RGB through the file's color profile, as Photoshop displays it."
                    .into(),
            );
        } else {
            notes.push(
                "CMYK converted to RGB without a usable color profile, so colors are approximate."
                    .into(),
            );
        }
        notes.push("This CMYK file is edited in RGB: blend modes and adjustments now act on RGB colors, so overlapping layers can differ slightly from Photoshop.".into());
    } else if profile.is_some() {
        notes.push(
            "The embedded color profile was not converted; Dudel uses the stored channel values."
                .into(),
        );
    }
    // An empty layer list means the document is just its background image.
    let background_only = count == 0;
    if background_only {
        details
            .push("This file holds only a background image; it was imported as one layer.".into());
    } else if layered && !layer_data.0.is_empty() {
        // A 16-bit list is handed over in the ordinary list's framing.
        let list = match high_depth_layers(layer_data.0, depth, big)? {
            Some(list) => {
                let mut framed = if big {
                    (list.0.len() as u64).to_be_bytes().to_vec()
                } else {
                    u32::try_from(list.0.len())
                        .map_err(|_| "PSD 16-bit layer list is too large")?
                        .to_be_bytes()
                        .to_vec()
                };
                framed.extend_from_slice(list.0);
                std::borrow::Cow::Owned(framed)
            }
            None => std::borrow::Cow::Borrowed(layer_data.0),
        };
        match layers(
            &list,
            width,
            height,
            depth,
            &colors,
            global_angle,
            &global_patterns(layer_data.0, big),
            big,
            cancel,
        ) {
            Ok((mut document, order, (layer_notes, layer_details))) => {
                document.dpi = dpi;
                details.push("Imported editable raster layers, supported groups, masks and blend modes. Rendering uses Dudel's compositor.".into());
                if depth == 16 {
                    details.push("Converted 16-bit channels to Dudel's 8-bit channels.".into());
                }
                notes.extend(layer_notes);
                details.extend(layer_details);
                let flipped = match order {
                    Order::BottomFirst => false,
                    Order::TopFirst => true,
                    Order::Undetermined => {
                        has_composite
                            && document.layers.len() > 1
                            && saved_image(
                                reader.0, width, height, channels, depth, &colors, has_alpha, big,
                                cancel,
                            )
                            .is_ok_and(|saved| reversed_matches_better(&document, &saved))
                    }
                };
                if order == Order::Undetermined && flipped {
                    document.layers.reverse();
                    document.active = document.layers.len() - 1;
                }
                if flipped {
                    details.push("This file lists its layers top first, unlike Photoshop. Dudel read them in that order to match the file's saved image.".into());
                }
                return Ok(Imported {
                    document,
                    notes,
                    details,
                });
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
            .unwrap_or_else(|| {
                if depth == 16 {
                    "16-bit layers are not imported yet".into()
                } else {
                    "no editable layers were read".into()
                }
            });
        return Err(format!(
            "The layers could not be imported ({reason}) and the file has no saved flattened image to fall back to. Save it with Maximize Compatibility enabled."
        ));
    }
    let pixels = saved_image(
        reader.0, width, height, channels, depth, &colors, has_alpha, big, cancel,
    )?;
    if depth == 16 {
        details.push("Converted 16-bit channels to Dudel's 8-bit channels.".into());
    }
    let mut document = Document::new(width, height);
    document.dpi = dpi;
    document.layers[0].name = "PSD composite (flattened)".into();
    document.layers[0].pixels = pixels.into();
    document.validate()?;
    Ok(Imported {
        document,
        notes,
        details,
    })
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
        clipping: u8,
        /// Extra additional-info blocks (key, value).
        keys: &'static [(&'static [u8; 4], &'static [u8])],
    }
    fn layer_file(specs: &[Spec]) -> Vec<u8> {
        layer_file_mode(specs, ColorMode::Rgb)
    }
    fn layer_file_mode(specs: &[Spec], mode: ColorMode) -> Vec<u8> {
        let mut records = Vec::new();
        let mut planes = Vec::new();
        for spec in specs {
            let size = i32::from(spec.section.is_none());
            for value in [0, spec.left, size, spec.left + size] {
                records.extend(value.to_be_bytes());
            }
            let channels: &[(i16, u8)] = match (spec.section, spec.mask, mode) {
                (Some(_), false, _) => &[],
                (Some(_), true, _) => &[(-2, 4)],
                (None, false, ColorMode::Rgb) => &[(0, 0), (1, 1), (2, 2), (-1, 3)],
                (None, true, ColorMode::Rgb) => &[(0, 0), (1, 1), (2, 2), (-1, 3), (-2, 4)],
                (None, false, ColorMode::Gray) => &[(0, 0), (-1, 3)],
                (None, true, ColorMode::Gray) => &[(0, 0), (-1, 3), (-2, 4)],
                // K (index 5) is stored as 255: no black ink.
                (None, false, ColorMode::Cmyk) => &[(0, 0), (1, 1), (2, 2), (3, 5), (-1, 3)],
                (None, true, ColorMode::Cmyk) => {
                    &[(0, 0), (1, 1), (2, 2), (3, 5), (-1, 3), (-2, 4)]
                }
            };
            records.extend((channels.len() as u16).to_be_bytes());
            for &(id, index) in channels {
                records.extend(id.to_be_bytes());
                records.extend(3u32.to_be_bytes());
                planes.extend([
                    0,
                    0,
                    match index {
                        4 => 75,
                        5 => 255,
                        _ => spec.color[index as usize],
                    },
                ]);
            }
            records.extend(b"8BIM");
            records.extend(spec.blend);
            records.extend([255, spec.clipping, spec.flags, 0]);
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
        if matches!(mode, ColorMode::Gray) {
            file[12..14].copy_from_slice(&2u16.to_be_bytes());
            file[24..26].copy_from_slice(&1u16.to_be_bytes());
        }
        if matches!(mode, ColorMode::Cmyk) {
            file[12..14].copy_from_slice(&5u16.to_be_bytes());
            file[24..26].copy_from_slice(&4u16.to_be_bytes());
        }
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
            clipping: 0,
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
            clipping: 0,
            keys: &[],
        }
    }
    #[test]
    fn dpi_resources_survive_layered_and_flattened_psd_and_psb_import() {
        let layered = layer_file(&[raster("Ink", 0, [10, 20, 30, 255], false)]);
        let flat = fixture(&[0, 0, 10, 20, 30, 40, 50, 60]);
        let mut big = flat[..34].to_vec();
        big[4..6].copy_from_slice(&2u16.to_be_bytes());
        big.extend(0u64.to_be_bytes());
        big.extend_from_slice(&flat[38..]);
        for base in [layered, flat, big] {
            assert_eq!(parse(&base, &|| false).unwrap().document.dpi, 72.0);
            for (x, y, expected) in [
                (600.5, 600.5, 600.5),
                (300.0, 600.0, 300.0),
                (0.0, 0.0, 72.0),
            ] {
                let mut info = Vec::new();
                info.extend(((x * 65536.0) as u32).to_be_bytes());
                info.extend([0, 2, 0, 2]); // display in pixels/cm; values remain pixels/inch
                info.extend(((y * 65536.0) as u32).to_be_bytes());
                info.extend([0, 2, 0, 2]);
                let mut resource = b"8BIM".to_vec();
                resource.extend(1005u16.to_be_bytes());
                resource.extend([0, 0]);
                resource.extend(section(&info));
                let mut file = base.clone();
                file.splice(30..34, section(&resource));
                let imported = parse(&file, &|| false).unwrap();
                assert_eq!(imported.document.dpi, expected);
                if x != y {
                    assert!(imported.notes.iter().any(|n| n.contains("both axes")));
                }
                if x == 0.0 {
                    assert!(
                        imported
                            .notes
                            .iter()
                            .any(|n| n.contains("Invalid source resolution"))
                    );
                }
            }
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
        assert!(imported.details.iter().any(|n| n.contains("editable")));
    }
    #[test]
    fn mask_density_reveals_and_feather_softens() {
        // Left half hidden, right half shown.
        let mut mask = crate::layer_mask::Mask::new(8, 1, 255).unwrap();
        for x in 0..4 {
            mask.writer().set(x, 0, 0);
        }
        let mut dense = mask.clone();
        soften_mask(&mut dense, Some(64), 0.0, 8, 1, &|| false).unwrap();
        // A quarter density hides only a quarter of what the mask hides.
        assert_eq!((dense.at(0, 0), dense.at(7, 0)), (191, 255));
        let mut feathered = mask;
        soften_mask(&mut feathered, None, 2.0, 8, 1, &|| false).unwrap();
        assert!(feathered.at(3, 0) > 30 && feathered.at(4, 0) < 225);
        assert!(feathered.at(3, 0) < feathered.at(4, 0));
    }
    #[test]
    fn dropped_adjustments_do_not_decode_irrelevant_channel_payloads() {
        let mut file = layer_file(&[
            Spec {
                keys: &[(b"clrL", &[])],
                ..raster("Unsupported adjustment", 0, [91, 92, 93, 255], false)
            },
            raster("Keep", 1, [10, 20, 30, 255], false),
        ]);
        let channel = file
            .windows(9)
            .position(|w| w == [0, 0, 91, 0, 0, 92, 0, 0, 93])
            .unwrap();
        // These bytes belong to a record already reported as left out.
        file[channel..channel + 2].copy_from_slice(&99u16.to_be_bytes());
        let imported = parse(&file, &|| false).unwrap();
        assert_eq!(imported.document.layers.len(), 1);
        assert_eq!(imported.document.layers[0].name, "Keep");
        assert_eq!(
            imported.document.layers[0].pixels.rgba(1),
            [10, 20, 30, 255]
        );
        assert!(imported.notes.iter().any(|n| n.contains("left out")));
    }
    #[test]
    fn flat_layer_order_detection_includes_native_effects() {
        use crate::layer_effects::{Effect, LayerEffect, Paint};
        // Top-first: red painted green above green painted blue.
        // Photoshop's saved result is green, so the stack must be reversed.
        let mut doc = Document::new(1, 1);
        doc.add_layer();
        for (layer, (pixel, color)) in doc.layers.iter_mut().zip([
            ([255, 0, 0, 255], [0, 255, 0]),
            ([0, 255, 0, 255], [0, 0, 255]),
        ]) {
            layer.pixels = pixel.to_vec().into();
            let mut overlay = Effect::preset("Color Overlay").unwrap();
            overlay.paint = Paint::Color(color);
            layer.effects.push(LayerEffect::new(overlay));
        }
        assert!(reversed_matches_better(&doc, &[0, 255, 0, 255]));
        doc.layers.reverse();
        assert!(!reversed_matches_better(&doc, &[0, 255, 0, 255]));
    }
    #[test]
    fn grayscale_layers_keep_alpha_offsets_masks_and_groups() {
        let file = layer_file_mode(
            &[
                group("End", 3),
                raster("Depth", 1, [43, 0, 0, 128], true),
                group("Folder", 1),
            ],
            ColorMode::Gray,
        );
        let imported = parse(&file, &|| false).unwrap();
        let layer = &imported.document.layers[0];
        assert_eq!(layer.pixels.rgba(0), [0; 4]);
        assert_eq!(layer.pixels.rgba(1), [43, 43, 43, 128]);
        assert_eq!(layer.mask.as_ref().unwrap().coverage.at(1, 0), 75);
        assert_eq!(layer.depth, 1);
        assert!(imported.document.layers[1].folder);
        assert!(imported.details.iter().any(|n| n.contains("grayscale")));
        // A grayscale file must not accept RGB layer channel IDs as gray pixels.
        let mut invalid = layer_file(&[raster("Invalid", 0, [9; 4], false)]);
        invalid[24..26].copy_from_slice(&1u16.to_be_bytes());
        assert!(parse(&invalid, &|| false).is_err());
    }
    #[test]
    fn folder_masks_preserve_children_and_mask_coverage() {
        for blend in [*b"norm", *b"pass"] {
            let file = layer_file(&[
                group("End", 3),
                raster("Ink", 0, [20, 30, 40, 255], false),
                Spec {
                    mask: true,
                    blend,
                    ..group("Masked folder", 1)
                },
            ]);
            let imported = parse(&file, &|| false).unwrap();
            assert_eq!(imported.document.layers.len(), 2);
            let folder = &imported.document.layers[1];
            assert!(folder.folder);
            let mask = folder.mask.as_ref().unwrap();
            assert!(mask.enabled);
            assert_eq!(mask.coverage.at(0, 0), 75);
            assert_eq!(mask.coverage.at(1, 0), 255);
            assert_eq!(imported.document.composite_display_pixel(0, true)[3], 75);
            assert_eq!(
                imported.document.layers[0].pixels.rgba(0),
                [20, 30, 40, 255]
            );
            assert!(!imported.notes.iter().any(|n| n.contains("flattened")));
        }
    }
    #[test]
    fn grayscale_composite_distinguishes_transparency_from_extra_channels() {
        let mut file = fixture(&[0, 0, 17, 64, 128, 255]);
        file[12..14].copy_from_slice(&2u16.to_be_bytes());
        file[24..26].copy_from_slice(&1u16.to_be_bytes());
        let imported = parse(&file, &|| false).unwrap();
        assert_eq!(
            imported.document.layers[0].pixels.rgba(0),
            [17, 17, 17, 255]
        );
        // Negative layer count marks the extra channel as merged transparency.
        let mut layered = layer_file_mode(
            &[raster("Gray", 0, [17, 0, 0, 128], false)],
            ColorMode::Gray,
        );
        layered.extend([0, 0, 17, 64, 128, 255]);
        let composite = parse_mode(&layered, &|| false, false).unwrap();
        assert_eq!(
            composite.document.layers[0].pixels.rgba(0),
            [17, 17, 17, 128]
        );
        assert_eq!(
            composite.document.layers[0].pixels.rgba(1),
            [64, 64, 64, 255]
        );
        for channels in [0u16, 57] {
            file[12..14].copy_from_slice(&channels.to_be_bytes());
            assert!(parse(&file, &|| false).is_err());
        }
    }
    #[test]
    fn sixteen_bit_prediction_resets_each_row_and_channel() {
        use std::io::Write;
        // Independent deltas include carry, wraparound and different row starts.
        let deltas = [0x12ffu16, 0x0102, 0xfffe, 3, 0x8000, 0x8001, 0x0100, 0xff00];
        let expected = [0x12ffu16, 0x1401, 0xfffe, 1, 0x8000, 1, 0x0100, 0];
        let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
        encoder
            .write_all(
                &deltas
                    .into_iter()
                    .flat_map(u16::to_be_bytes)
                    .collect::<Vec<_>>(),
            )
            .unwrap();
        let mut data = vec![0, 3];
        data.extend(encoder.finish().unwrap());
        assert_eq!(
            decode(&data, 2, 2, 2, 16, false, &|| false).unwrap(),
            expected
                .into_iter()
                .flat_map(u16::to_be_bytes)
                .collect::<Vec<_>>()
        );
        assert!(decode(&data[..data.len() - 2], 2, 2, 2, 16, false, &|| false).is_err());
        assert!(decode(&data, 2, 2, 2, 16, false, &|| true).is_err());
        // Decode a one-channel document through the real import path as well.
        let mut file = fixture(&data);
        file[12..14].copy_from_slice(&1u16.to_be_bytes());
        file[14..18].copy_from_slice(&4u32.to_be_bytes());
        file[22..24].copy_from_slice(&16u16.to_be_bytes());
        file[24..26].copy_from_slice(&1u16.to_be_bytes());
        let imported = parse(&file, &|| false).unwrap();
        for (i, sample) in expected.into_iter().enumerate() {
            let gray = ((u32::from(sample) + 128) / 257) as u8;
            assert_eq!(
                imported.document.layers[0].pixels.rgba(i),
                [gray, gray, gray, 255]
            );
        }
    }
    #[test]
    fn sixteen_bit_files_import_their_lr16_layers() {
        // Photoshop leaves the ordinary list empty and writes Lr16: the same
        // records, with 16-bit samples. Two 1 x 1 layers, left and right.
        let layer = |left: i32, color: [u16; 4], name: &str| {
            let mut record = Vec::new();
            for value in [0, left, 1, left + 1] {
                record.extend(value.to_be_bytes());
            }
            record.extend(4u16.to_be_bytes());
            for id in [0i16, 1, 2, -1] {
                record.extend(id.to_be_bytes());
                record.extend(4u32.to_be_bytes());
            }
            record.extend(b"8BIMnorm");
            record.extend([255, 0, 0, 0]);
            let mut extra = 0u32.to_be_bytes().to_vec();
            extra.extend(0u32.to_be_bytes());
            extra.push(name.len() as u8);
            extra.extend(name.as_bytes());
            extra.extend(vec![0; (4 - (name.len() + 1) % 4) % 4]);
            record.extend(section(&extra));
            let mut planes = Vec::new();
            for sample in [color[0], color[1], color[2], color[3]] {
                planes.extend([0, 0]);
                planes.extend(sample.to_be_bytes());
            }
            (record, planes)
        };
        let (first, first_planes) = layer(0, [0xffff, 0x8000, 0x0101, 0xffff], "Left");
        let (second, second_planes) = layer(1, [0x0000, 0x4040, 0xfefe, 0x8080], "Right");
        let mut list = 2u16.to_be_bytes().to_vec();
        list.extend([first, second].concat());
        list.extend([first_planes, second_planes].concat());
        let mut info = section(&[]);
        info.extend(section(&[]));
        info.extend(b"8BIMLr16");
        info.extend(section(&list));
        info.extend(vec![0; (4 - list.len() % 4) % 4]);
        let mut file = fixture(&[]);
        file[22..24].copy_from_slice(&16u16.to_be_bytes());
        file.truncate(34);
        file.extend(section(&info));
        // A saved image, which the layers are preferred over.
        file.extend([0, 0]);
        file.extend([0u8; 2 * 2 * 3]);
        let imported = parse(&file, &|| false).unwrap();
        let document = &imported.document;
        let names: Vec<_> = document.layers.iter().map(|l| l.name.as_str()).collect();
        assert_eq!(names, ["Left", "Right"]);
        assert_eq!(document.layers[0].pixels.rgba(0), [255, 128, 1, 255]);
        assert_eq!(document.layers[1].pixels.rgba(1), [0, 64, 254, 128]);
        assert!(imported.details.iter().any(|d| d.contains("16-bit")));
        assert!(!imported.notes.iter().any(|n| n.contains("flattened")));
    }
    #[test]
    fn sixteen_bit_alternate_layer_count_controls_composite_alpha() {
        for mask_field in [true, false] {
            let mut info = section(&[]);
            if mask_field {
                info.extend(section(&[]));
            }
            // An odd-sized unrelated block tests document-level four-byte alignment.
            info.extend(b"8BIMtest");
            info.extend(section(&[1]));
            info.extend([0; 3]);
            info.extend(b"8BIMLr16");
            info.extend(section(&(-1i16).to_be_bytes()));
            info.extend([0; 2]);
            let mut file = fixture(&[]);
            file[12..14].copy_from_slice(&2u16.to_be_bytes());
            file[22..24].copy_from_slice(&16u16.to_be_bytes());
            file[24..26].copy_from_slice(&1u16.to_be_bytes());
            file.truncate(34);
            file.extend(section(&info));
            file.extend([0, 0]);
            for sample in [0x1111u16, 0x4040, 0x8080, 0xffff] {
                file.extend(sample.to_be_bytes());
            }
            let imported = parse(&file, &|| false).unwrap();
            assert_eq!(
                imported.document.layers[0].pixels.rgba(0),
                [17, 17, 17, 128]
            );
            assert_eq!(
                imported.document.layers[0].pixels.rgba(1),
                [64, 64, 64, 255]
            );
            assert!(
                !imported
                    .notes
                    .iter()
                    .any(|n| n.contains("only a background"))
            );
        }
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
        assert!(imported.details.iter().any(|n| n.contains("editable")));
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
        assert!(imported.details.iter().any(|n| n.contains("top first")));
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
                imported.details.iter().any(|n| n.contains("top first")),
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
        assert_eq!(text.opacity, 1.0);
        assert!((text.fill - 128.0 / 255.0).abs() < 1e-6);
        assert!(
            imported.notes.iter().any(|n| n
                == "Text layers imported as pixels (their text cannot be edited here): “Title”.")
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
    fn layer_effects_become_layers_around_their_layer() {
        use crate::psd_vector::{Descriptor, Value};
        fn object(class: &str, items: Vec<(&str, Value)>) -> Value {
            Value::Object(Descriptor {
                class: class.into(),
                items: items.into_iter().map(|(k, v)| (k.into(), v)).collect(),
            })
        }
        let unit = |unit: &str, v: f64| Value::Unit(unit.into(), v);
        let enumerated = |v: &str| Value::Enum("x".into(), v.into());
        let green = || {
            object(
                "RGBC",
                vec![
                    ("Rd  ", Value::Number(0.0)),
                    ("Grn ", Value::Number(255.0)),
                    ("Bl  ", Value::Number(0.0)),
                ],
            )
        };
        let stroke = |style: &str| {
            object(
                "FrFX",
                vec![
                    ("enab", Value::Bool(true)),
                    ("Styl", enumerated(style)),
                    ("PntT", enumerated("SClr")),
                    ("Md  ", enumerated("Nrml")),
                    ("Opct", unit("#Prc", 100.0)),
                    ("Sz  ", unit("#Pxl", 1.0)),
                    ("Clr ", green()),
                ],
            )
        };
        let effects = |master: bool| {
            let settings = Descriptor {
                class: "null".into(),
                items: vec![
                    ("masterFXSwitch".into(), Value::Bool(master)),
                    (
                        "DrSh".into(),
                        object(
                            "DrSh",
                            vec![
                                ("enab", Value::Bool(true)),
                                ("Md  ", enumerated("Mltp")),
                                ("Clr ", green()),
                                ("Opct", unit("#Prc", 50.0)),
                                ("Dstn", unit("#Pxl", 0.0)),
                                ("blur", unit("#Pxl", 0.0)),
                            ],
                        ),
                    ),
                    (
                        "SoFi".into(),
                        object(
                            "SoFi",
                            vec![
                                ("enab", Value::Bool(true)),
                                ("Md  ", enumerated("Nrml")),
                                ("Clr ", green()),
                            ],
                        ),
                    ),
                    (
                        "frameFXMulti".into(),
                        Value::List(vec![stroke("CtrF"), stroke("InsF")]),
                    ),
                    (
                        "ChFX".into(),
                        object("ChFX", vec![("enab", Value::Bool(true))]),
                    ),
                ],
            };
            let mut data = vec![0, 0, 0, 0, 0, 0, 0, 16];
            data.extend(crate::psd_vector::encode(&settings));
            &*Box::leak(data.into_boxed_slice())
        };
        let file = |master: bool| {
            let keys: &'static [(&'static [u8; 4], &'static [u8])] = Box::leak(
                vec![(b"lfx2", effects(master)), (b"lrFX", &[0u8, 0][..])].into_boxed_slice(),
            );
            layer_file(&[
                Spec {
                    keys,
                    ..raster("Base", 0, [255, 0, 0, 255], false)
                },
                Spec {
                    clipping: 1,
                    ..raster("Clipped", 0, [0, 0, 255, 255], false)
                },
            ])
        };
        let imported = parse(&file(true), &|| false).unwrap();
        // The effects stay editable on the layer; compositing draws them as
        // layers around it.
        assert_eq!(imported.document.layers.len(), 2);
        assert_eq!(imported.document.layers[0].effects.len(), 4);
        let expanded = crate::layer_effects::expand(&imported.document, false, &|| false).unwrap();
        let stack: Vec<_> = expanded
            .layers
            .iter()
            .map(|l| (l.name.as_str(), l.clipped, l.opacity))
            .collect();
        // Shadow under the layer, overlay and inside stroke clipped to it,
        // the centered stroke over the layers clipped to it.
        assert_eq!(
            stack,
            [
                ("Base's Drop Shadow", false, 0.5),
                ("Base", false, 1.0),
                ("Base's Color Overlay", true, 1.0),
                ("Base's Stroke", true, 1.0),
                ("Clipped", true, 1.0),
                ("Base's Stroke", false, 1.0),
            ]
        );
        let overlay = &expanded.layers[2];
        assert_eq!(overlay.pixels.rgba(0), [0, 255, 0, 255]);
        assert_eq!(overlay.pixels.rgba(1), [0, 0, 0, 0]);
        assert_eq!(expanded.layers[0].blend, crate::blend::BlendMode::Multiply);
        assert!(imported.notes.iter().any(|n| n.contains("satin")));
        assert!(imported.details.iter().any(|n| n.contains("stay editable")));
        // With the effects switched off the layer imports alone.
        let off = parse(&file(false), &|| false).unwrap();
        assert_eq!(off.document.layers.len(), 2);
    }
    #[test]
    fn folder_effects_follow_the_folder_contents() {
        use crate::psd_vector::{Descriptor, Value};
        let effect = |class: &str, mode: &str| {
            let green = Descriptor {
                class: "RGBC".into(),
                items: vec![
                    ("Rd  ".into(), Value::Number(0.0)),
                    ("Grn ".into(), Value::Number(255.0)),
                    ("Bl  ".into(), Value::Number(0.0)),
                ],
            };
            (
                class.to_string(),
                Value::Object(Descriptor {
                    class: class.into(),
                    items: vec![
                        ("enab".into(), Value::Bool(true)),
                        ("Md  ".into(), Value::Enum("BlnM".into(), mode.into())),
                        ("Clr ".into(), Value::Object(green)),
                    ],
                }),
            )
        };
        let settings = Descriptor {
            class: "null".into(),
            items: vec![effect("DrSh", "Nrml"), effect("SoFi", "Nrml")],
        };
        let mut data = vec![0, 0, 0, 0, 0, 0, 0, 16];
        data.extend(crate::psd_vector::encode(&settings));
        let data: &'static [u8] = Box::leak(data.into_boxed_slice());
        let keys: &'static [(&'static [u8; 4], &'static [u8])] =
            Box::leak(vec![(b"lfx2", data)].into_boxed_slice());
        let file = layer_file(&[
            group("</Layer set>", 3),
            raster("Inside", 1, [255, 0, 0, 255], false),
            Spec {
                keys,
                ..group("Group", 1)
            },
        ]);
        let imported = parse(&file, &|| false).unwrap();
        // The folder keeps its effects, drawn as Photoshop's Create Layers
        // would draw them.
        let folder = imported.document.layers.last().unwrap();
        assert!(folder.folder && folder.effects.len() == 2);
        let doc = &crate::layer_effects::expand(&imported.document, false, &|| false).unwrap();
        let stack: Vec<_> = doc
            .layers
            .iter()
            .map(|l| (l.name.as_str(), l.depth, l.clipped))
            .collect();
        // The shadow goes under the folder's contents, the overlay clips to
        // the folder.
        assert_eq!(
            stack,
            [
                ("Group's Drop Shadow", 0, false),
                ("Inside", 1, false),
                ("Group", 0, false),
                ("Group's Color Overlay", 0, true),
            ]
        );
        // The folder's contents cover pixel 1 only; the shadow (no offset or
        // blur) adds nothing there, and the overlay paints only it.
        assert_eq!(doc.layers[0].pixels.rgba(0), [0, 0, 0, 0]);
        let rendered = doc.composite_transparent();
        assert_eq!(&rendered[..8], &[0, 0, 0, 0, 0, 255, 0, 255]);
        // Effects drawn over a layer stay out of a folder that starts above
        // it: the folder ends the layer's clip group.
        let Value::Object(shadow) = &settings.items[0].1 else {
            unreachable!()
        };
        let stroke = Descriptor {
            class: "null".into(),
            items: vec![(
                "FrFX".into(),
                Value::Object(Descriptor {
                    class: "FrFX".into(),
                    items: vec![
                        ("enab".into(), Value::Bool(true)),
                        ("Styl".into(), Value::Enum("FStl".into(), "CtrF".into())),
                        ("Sz  ".into(), Value::Unit("#Pxl".into(), 1.0)),
                        ("Clr ".into(), shadow.get("Clr ").unwrap().clone()),
                    ],
                }),
            )],
        };
        let mut data = vec![0, 0, 0, 0, 0, 0, 0, 16];
        data.extend(crate::psd_vector::encode(&stroke));
        // Blocks are padded to even lengths.
        data.resize(data.len().next_multiple_of(2), 0);
        let data: &'static [u8] = Box::leak(data.into_boxed_slice());
        let keys: &'static [(&'static [u8; 4], &'static [u8])] =
            Box::leak(vec![(b"lfx2", data)].into_boxed_slice());
        let file = layer_file(&[
            Spec {
                keys,
                ..raster("Below", 0, [255, 0, 0, 255], false)
            },
            group("</Layer set>", 3),
            raster("Inside", 1, [255, 0, 0, 255], false),
            group("Group", 1),
        ]);
        let imported = parse(&file, &|| false).unwrap();
        let expanded = crate::layer_effects::expand(&imported.document, false, &|| false).unwrap();
        let stack: Vec<_> = expanded
            .layers
            .iter()
            .map(|l| (l.name.as_str(), l.depth))
            .collect();
        assert_eq!(
            stack,
            [
                ("Below", 0),
                ("Below's Stroke", 0),
                ("Inside", 1),
                ("Group", 0)
            ]
        );
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
                .details
                .iter()
                .any(|n| n.contains("only a background image"))
        );
        assert!(!imported.notes.iter().any(|n| n.contains("unavailable")));
    }
    #[test]
    fn cmyk_layers_convert_to_rgb_with_a_note() {
        // Stored CMYK is inverted: 0 is full ink. Full cyan, no other ink.
        let file = layer_file_mode(
            &[raster("Cyan", 0, [0, 255, 255, 200], false)],
            ColorMode::Cmyk,
        );
        let imported = parse(&file, &|| false).unwrap();
        assert_eq!(
            imported.document.layers[0].pixels.rgba(0),
            [0, 255, 255, 200]
        );
        assert!(
            imported
                .notes
                .iter()
                .any(|n| n.contains("without a usable color profile"))
        );
        assert!(imported.notes.iter().any(|n| n.contains("edited in RGB")));
        // A gray K darkens every channel.
        let colors = Colors::new(ColorMode::Cmyk, None);
        let rgb = colors
            .rgb(1, |c, _| [255, 255, 255, 128][c], &|| false)
            .unwrap();
        assert_eq!(rgb, vec![[128; 3]]);
        // A profile that is not CMYK is not used for CMYK samples.
        assert!(
            Colors::new(ColorMode::Cmyk, Some(b"not a profile"))
                .cmyk
                .is_none()
        );
    }
    #[test]
    fn psb_files_read_wide_lengths_and_rle_row_counts() {
        let mut file = fixture(&[]);
        file.truncate(34);
        file[4..6].copy_from_slice(&2u16.to_be_bytes());
        // Layer and mask section (8-byte length) holding an empty layer list
        // (8-byte length).
        file.extend(8u64.to_be_bytes());
        file.extend(0u64.to_be_bytes());
        // RLE composite: three channel rows of two literal samples, each row
        // counted in 4 bytes.
        file.extend(1u16.to_be_bytes());
        for _ in 0..3 {
            file.extend(3u32.to_be_bytes());
        }
        for row in [[10, 20], [30, 40], [50, 60]] {
            file.extend([1, row[0], row[1]]);
        }
        let imported = parse(&file, &|| false).unwrap();
        assert_eq!(
            imported.document.layers[0].pixels.rgba(0),
            [10, 30, 50, 255]
        );
        assert_eq!(
            imported.document.layers[0].pixels.rgba(1),
            [20, 40, 60, 255]
        );
        // Read as a PSD, the same bytes do not line up.
        file[4..6].copy_from_slice(&1u16.to_be_bytes());
        assert!(parse(&file, &|| false).is_err());
        file[4..6].copy_from_slice(&3u16.to_be_bytes());
        assert!(parse(&file, &|| false).is_err());
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
