//! Photoshop (.psd) export: every layer and folder with its name, blend mode,
//! opacity, clipping, visibility, locks and mask, plus the flattened image.
//! Adjustment layers are written as Photoshop's own; layer effects as
//! Photoshop layer styles or, by choice or where Photoshop cannot show them,
//! as the layers they draw (Photoshop's Create Layers).
use crate::{
    blend::BlendMode,
    document::{Document, Layer},
};
use std::sync::atomic::{AtomicBool, Ordering};

/// Photoshop's key for a blend mode. The glow modes are the dodge modes
/// with Transparency Shapes off (`tsly`).
fn blend_key(mode: BlendMode) -> &'static [u8; 4] {
    use BlendMode::*;
    match mode {
        Normal => b"norm",
        Dissolve => b"diss",
        PassThrough => b"pass",
        Multiply => b"mul ",
        Screen => b"scrn",
        Overlay => b"over",
        Darken => b"dark",
        Lighten => b"lite",
        ColorDodge | GlowDodge => b"div ",
        ColorBurn => b"idiv",
        HardLight => b"hLit",
        SoftLight => b"sLit",
        Difference => b"diff",
        Exclusion => b"smud",
        LinearBurn => b"lbrn",
        Subtract => b"fsub",
        DarkerColor => b"dkCl",
        LinearDodge | AddGlow => b"lddg",
        LighterColor => b"lgCl",
        VividLight => b"vLit",
        LinearLight => b"lLit",
        PinLight => b"pLit",
        HardMix => b"hMix",
        Divide => b"fdiv",
        Hue => b"hue ",
        Saturation => b"sat ",
        Color => b"colr",
        Luminosity => b"lum ",
    }
}

/// How layer effects are written.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Effects {
    /// As Photoshop layer styles, editable there; layers whose effects
    /// Photoshop cannot show get them drawn as layers instead.
    Editable,
    /// Drawn as layers, like Photoshop's Create Layers: the same look in
    /// apps that ignore layer styles, such as Live2D Cubism.
    AsLayers,
}

/// Layer styles (`lfx2` blocks) by the layer that carries them.
type Styles = Vec<(crate::document::LayerId, Vec<u8>)>;

/// The layers the file holds, bottom first, with the layer style (`lfx2`)
/// written on each layer that keeps its effects editable.
pub(crate) fn written(
    doc: &Document,
    effects: Effects,
    cancel: &dyn Fn() -> bool,
) -> Result<(Document, Styles), String> {
    let mut styles = Vec::new();
    let mut drawn = doc.clone();
    if effects == Effects::Editable {
        for layer in drawn.layers.iter_mut().filter(|l| !l.effects.is_empty()) {
            if let Some(style) = crate::layer_effects::to_psd(&layer.effects) {
                styles.push((layer.id, style));
                layer.effects.clear();
            }
        }
    }
    let mut expanded = crate::layer_effects::expand(&drawn, false, cancel)?;
    // Keep the effects on the layers that carry them as styles, so a reader
    // of this document sees what the file says. Fill opacity is written as
    // Photoshop's fill, except in modes where Photoshop draws fill
    // differently; there it stays folded into opacity.
    for layer in &mut expanded.layers {
        let Some(source) = doc.layers.iter().find(|l| l.id == layer.id) else {
            continue;
        };
        if styles.iter().any(|(id, _)| *id == layer.id) {
            layer.effects = source.effects.clone();
        }
        if source.fill != 1.0 && !crate::psd_import::fill_differs(source.blend) {
            layer.opacity = source.opacity;
            layer.fill = source.fill;
        }
    }
    Ok((expanded, styles))
}

/// What the export leaves out or changes, one line per kind.
pub(crate) fn notes(doc: &Document, effects: Effects) -> Vec<String> {
    let names = |keep: &dyn Fn(&Layer) -> bool| -> Vec<String> {
        doc.layers
            .iter()
            .filter(|l| keep(l))
            .map(|l| l.name.clone())
            .collect()
    };
    let list = crate::clip_metadata::name_list;
    let mut notes = Vec::new();
    let approximated = names(&|l| {
        l.adjustment
            .as_ref()
            .is_some_and(|a| crate::adjustment::psd_approximated(a.settings()))
    });
    if !approximated.is_empty() {
        notes.push(format!(
            "Clip Studio Paint's Hue/Saturation/Luminosity and tone curves have no exact Photoshop form; written as the closest Photoshop adjustment: {}.",
            list(&approximated)
        ));
    }
    let has_effects = |l: &Layer| l.effects.iter().any(|e| e.enabled);
    let drawn = match effects {
        Effects::AsLayers => names(&has_effects),
        Effects::Editable => {
            names(&|l| has_effects(l) && crate::layer_effects::to_psd(&l.effects).is_none())
        }
    };
    if !drawn.is_empty() {
        notes.push(format!(
            "{}: {}.",
            match effects {
                Effects::AsLayers =>
                    "Layer effects are written as separate layers, like Photoshop's Create Layers",
                Effects::Editable =>
                    "Photoshop cannot show these layers' effects (glow blend modes, gradient glows, or two glows or bevels of a kind), so they are written as separate layers",
            },
            list(&drawn)
        ));
    }
    notes
}

/// PackBits, as Photoshop's RLE rows use it.
fn packbits(row: &[u8], out: &mut Vec<u8>) {
    let mut i = 0;
    while i < row.len() {
        let mut run = 1;
        while i + run < row.len() && run < 128 && row[i + run] == row[i] {
            run += 1;
        }
        if run >= 2 {
            out.push((1 - run as i32) as u8);
            out.push(row[i]);
            i += run;
            continue;
        }
        // A literal stretch ends where a run of three starts.
        let start = i;
        while i < row.len() && i - start < 128 {
            if i + 2 < row.len() && row[i] == row[i + 1] && row[i] == row[i + 2] {
                break;
            }
            i += 1;
        }
        out.push((i - start - 1) as u8);
        out.extend_from_slice(&row[start..i]);
    }
}

/// Each channel of interleaved rows, compressed: its row byte counts and data.
type Plane = (Vec<u16>, Vec<u8>);
fn planes(
    channels: usize,
    rows: usize,
    mut row: impl FnMut(usize) -> Vec<u8>,
    cancel: &AtomicBool,
) -> Result<Vec<Plane>, String> {
    let mut out = vec![(Vec::with_capacity(rows), Vec::new()); channels];
    let mut plane = Vec::new();
    for y in 0..rows {
        if y % 64 == 0 {
            crate::fill::check_cancel(Some(cancel))?;
        }
        let bytes = row(y);
        for (c, (counts, data)) in out.iter_mut().enumerate() {
            plane.clear();
            plane.extend(bytes.iter().skip(c).step_by(channels));
            let start = data.len();
            packbits(&plane, data);
            // A 30,000-pixel row packs to at most 30,235 bytes.
            counts.push((data.len() - start) as u16);
        }
    }
    Ok(out)
}
/// A layer channel: RLE compression, row byte counts, then the rows.
fn channel_data((counts, data): Plane) -> Vec<u8> {
    let mut out = Vec::with_capacity(2 + counts.len() * 2 + data.len());
    out.extend(1u16.to_be_bytes());
    counts.iter().for_each(|c| out.extend(c.to_be_bytes()));
    out.extend(data);
    out
}

/// Smallest rectangle holding every nonzero byte (hidden color included).
fn pixel_bounds(layer: &Layer, width: usize, height: usize) -> Option<[usize; 4]> {
    let mut bounds: Option<[usize; 4]> = None;
    layer
        .pixels
        .visit_nonzero(width, height, [0, 0, width, height], |x, y, _| {
            let b = bounds.get_or_insert([x, y, x + 1, y + 1]);
            *b = [b[0].min(x), b[1].min(y), b[2].max(x + 1), b[3].max(y + 1)];
        });
    bounds
}

fn rect([left, top, right, bottom]: [usize; 4]) -> impl Iterator<Item = u8> {
    [top, left, bottom, right]
        .into_iter()
        .flat_map(|v| (v as u32).to_be_bytes())
}

/// Tagged layer information; lengths here are always a multiple of four.
fn tagged(out: &mut Vec<u8>, key: &[u8; 4], data: &[u8]) {
    out.extend(b"8BIM");
    out.extend(key);
    out.extend((data.len() as u32).to_be_bytes());
    out.extend(data);
}

/// Folder record kinds (`lsct`): open, closed, and the end divider written
/// below a folder's contents.
const OPEN: u32 = 1;
const CLOSED: u32 = 2;
const DIVIDER: u32 = 3;

struct Writer<'a> {
    width: usize,
    height: usize,
    cancel: &'a AtomicBool,
    count: usize,
    records: Vec<u8>,
    channels: Vec<u8>,
    styles: Styles,
}
impl Writer<'_> {
    /// One layer record and its channel data. `section` marks folder records.
    fn record(&mut self, layer: &Layer, section: Option<u32>) -> Result<(), String> {
        let (w, h) = (self.width, self.height);
        // Folders and adjustment layers have no pixels of their own.
        let pixelless = section.is_some() || layer.adjustment.is_some();
        let bounds = (!pixelless)
            .then(|| pixel_bounds(layer, w, h))
            .flatten()
            .unwrap_or([0; 4]);
        let [left, top, right, bottom] = bounds;
        let mut channels: Vec<(i16, Vec<u8>)> = if pixelless {
            // Empty color channels, as Photoshop writes them.
            [-1, 0, 1, 2].map(|id| (id, vec![0, 0])).into()
        } else {
            let rgba = planes(
                4,
                bottom - top,
                |y| layer.pixels.region(w, [left, top + y, right, top + y + 1]),
                self.cancel,
            )?;
            let mut rgba = rgba.into_iter().map(channel_data);
            let [r, g, b, a] = std::array::from_fn(|_| rgba.next().unwrap());
            vec![(-1, a), (0, r), (1, g), (2, b)]
        };
        let mut mask_record = Vec::new();
        if let Some(mask) = &layer.mask {
            let coverage = &mask.coverage;
            if coverage.dimensions() != [w, h] {
                return Err("A layer mask does not match the canvas".into());
            }
            let area = coverage.painted_bounds().unwrap_or([0; 4]);
            let plane = planes(
                1,
                area[3] - area[1],
                |y| {
                    (area[0]..area[2])
                        .map(|x| coverage.at(x, area[1] + y))
                        .collect()
                },
                self.cancel,
            )?;
            channels.push((-2, channel_data(plane.into_iter().next().unwrap())));
            mask_record.extend(rect(area));
            mask_record.push(coverage.default_value());
            // Bit 1: disabled. Bit 0 would place the mask relative to the layer.
            mask_record.push(if mask.enabled { 0 } else { 2 });
            mask_record.extend([0, 0]);
        }
        self.records.extend(rect(bounds));
        self.records.extend((channels.len() as u16).to_be_bytes());
        for (id, data) in &channels {
            self.records.extend(id.to_be_bytes());
            self.records.extend((data.len() as u32).to_be_bytes());
        }
        self.records.extend(b"8BIM");
        self.records.extend(blend_key(layer.blend));
        self.records
            .push((layer.opacity.clamp(0.0, 1.0) * 255.0).round() as u8);
        self.records.push(u8::from(layer.clipped));
        // Bit 0 protects transparency, bit 1 hides; bits 3 and 4 mark a
        // folder or adjustment record's pixels as irrelevant.
        self.records.push(
            u8::from(layer.alpha_locked)
                | if layer.visible { 0 } else { 2 }
                | if pixelless { 8 | 16 } else { 0 },
        );
        self.records.push(0);
        let mut extra = Vec::new();
        extra.extend((mask_record.len() as u32).to_be_bytes());
        extra.extend(mask_record);
        // Blend If left open: gray, then red, green and blue.
        extra.extend(32u32.to_be_bytes());
        for _ in 0..4 {
            extra.extend([0, 0, 255, 255, 0, 0, 255, 255]);
        }
        // Pascal name (ASCII), padded to four bytes; `luni` has the real one.
        let name: Vec<u8> = layer
            .name
            .chars()
            .take(255)
            .map(|c| {
                if c.is_ascii() && !c.is_ascii_control() {
                    c as u8
                } else {
                    b'_'
                }
            })
            .collect();
        extra.push(name.len() as u8);
        extra.extend(&name);
        extra.extend(std::iter::repeat_n(0, (4 - (name.len() + 1) % 4) % 4));
        let mut units: Vec<u16> = layer.name.encode_utf16().take(2048).collect();
        let mut luni = (units.len() as u32).to_be_bytes().to_vec();
        if units.len() % 2 == 1 {
            units.push(0);
        }
        units.iter().for_each(|u| luni.extend(u.to_be_bytes()));
        tagged(&mut extra, b"luni", &luni);
        if let Some((_, style)) = self.styles.iter().find(|(id, _)| *id == layer.id) {
            let mut style = style.clone();
            style.resize(style.len().next_multiple_of(4), 0);
            tagged(&mut extra, b"lfx2", &style);
        }
        if let Some(adjustment) = &layer.adjustment {
            for (key, data) in crate::adjustment::to_psd(adjustment.settings()) {
                tagged(&mut extra, &key, &data);
            }
        }
        if let Some(kind) = section {
            let mut lsct = kind.to_be_bytes().to_vec();
            lsct.extend(b"8BIM");
            lsct.extend(blend_key(layer.blend));
            tagged(&mut extra, b"lsct", &lsct);
        }
        // With Transparency Shapes off, Photoshop's dodge modes glow.
        if matches!(layer.blend, BlendMode::GlowDodge | BlendMode::AddGlow) {
            tagged(&mut extra, b"tsly", &[0; 4]);
        }
        // Image and position locks, and transparency.
        let locks = if layer.locked { 2 | 4 } else { 0 } | u32::from(layer.alpha_locked);
        tagged(&mut extra, b"lspf", &locks.to_be_bytes());
        if layer.fill != 1.0 {
            let fill = (layer.fill.clamp(0.0, 1.0) * 255.0).round() as u8;
            tagged(&mut extra, b"iOpa", &[fill, 0, 0, 0]);
        }
        self.records.extend((extra.len() as u32).to_be_bytes());
        self.records.extend(extra);
        for (_, data) in channels {
            self.channels.extend(data);
        }
        self.count += 1;
        Ok(())
    }
    fn divider(&mut self) -> Result<(), String> {
        let mut end = Layer::new_folder("</Layer group>".into());
        end.folder = false;
        self.record(&end, Some(DIVIDER))
    }
}

/// The complete file. Every layer is written bottom first, each folder's
/// end divider below its contents, as Photoshop does.
pub(crate) fn encode(
    doc: &Document,
    effects: Effects,
    cancel: &AtomicBool,
) -> Result<Vec<u8>, String> {
    doc.validate()?;
    let (width, height) = (doc.width, doc.height);
    let stopped = || cancel.load(Ordering::Relaxed);
    let check = || crate::fill::check_cancel(Some(cancel));
    let mut composite = doc.composite_bounds_checked([0, 0, width, height], true, check)?;
    // Photoshop stores the flattened image blended against white: color C
    // at alpha a as C·a + 255·(1 − a).
    for pixel in composite.chunks_exact_mut(4) {
        let a = u32::from(pixel[3]);
        for c in &mut pixel[..3] {
            *c = ((u32::from(*c) * a + 255 * (255 - a) + 127) / 255) as u8;
        }
    }
    let (expanded, styles) = written(doc, effects, &stopped)?;
    let mut writer = Writer {
        width,
        height,
        cancel,
        count: 0,
        records: Vec::new(),
        channels: Vec::new(),
        styles,
    };
    let mut open = 0;
    for layer in &expanded.layers {
        // Entering a folder writes its divider first.
        while open < layer.depth + u8::from(layer.folder) {
            writer.divider()?;
            open += 1;
        }
        let section = layer
            .folder
            .then_some(if layer.collapsed { CLOSED } else { OPEN });
        writer.record(layer, section)?;
        if layer.folder {
            open = layer.depth;
        }
    }
    if writer.count == 0 {
        // Photoshop needs a layer; an empty one keeps the file layered.
        writer.record(&Layer::raster("Layer 1".into(), width, height), None)?;
    }
    let count = i16::try_from(writer.count).map_err(|_| "Too many layers for a PSD")?;
    let mut info = Vec::new();
    // Negative: the flattened image's extra channel is its transparency.
    info.extend((-count).to_be_bytes());
    info.extend(writer.records);
    info.extend(writer.channels);
    info.extend(std::iter::repeat_n(0, (4 - info.len() % 4) % 4));
    let too_large = || "This document is too large for a PSD file".to_string();
    let info_len = u32::try_from(info.len()).map_err(|_| too_large())?;
    // The patterns layer styles paint with, which they name by id.
    let mut global = Vec::new();
    let patterns = crate::layer_effects::patterns_in(&doc.layers);
    if !patterns.is_empty() {
        let block = crate::pattern::write_psd(&patterns);
        global.extend(b"8BIMPatt");
        global.extend(
            u32::try_from(block.len())
                .map_err(|_| too_large())?
                .to_be_bytes(),
        );
        global.extend(block);
    }
    let layers_len = u32::try_from(global.len())
        .ok()
        .and_then(|g| info_len.checked_add(8)?.checked_add(g))
        .ok_or_else(too_large)?;

    let mut out = Vec::with_capacity(info.len() + composite.len() / 2);
    out.extend(b"8BPS");
    out.extend(1u16.to_be_bytes());
    out.extend([0; 6]);
    out.extend(4u16.to_be_bytes());
    out.extend((height as u32).to_be_bytes());
    out.extend((width as u32).to_be_bytes());
    out.extend(8u16.to_be_bytes());
    out.extend(3u16.to_be_bytes()); // RGB
    out.extend(0u32.to_be_bytes()); // no color mode data
    // ResolutionInfo (1005): 16.16 pixels per inch, shown in inches.
    let dpi = ((doc.dpi * 65536.0).round() as u32).to_be_bytes();
    let mut resolution = Vec::new();
    for _ in 0..2 {
        resolution.extend(dpi);
        resolution.extend([0, 1, 0, 1]);
    }
    out.extend((4 + 2 + 2 + 4 + resolution.len() as u32).to_be_bytes());
    out.extend(b"8BIM");
    out.extend(1005u16.to_be_bytes());
    out.extend([0, 0]); // empty name, padded
    out.extend((resolution.len() as u32).to_be_bytes());
    out.extend(resolution);
    out.extend(layers_len.to_be_bytes());
    out.extend(info_len.to_be_bytes());
    out.extend(info);
    out.extend(0u32.to_be_bytes()); // no global layer mask
    out.extend(global);
    check()?;
    let row = width * 4;
    let merged = planes(
        4,
        height,
        |y| composite[y * row..(y + 1) * row].to_vec(),
        cancel,
    )?;
    out.extend(1u16.to_be_bytes());
    for (counts, _) in &merged {
        counts.iter().for_each(|c| out.extend(c.to_be_bytes()));
    }
    for (_, data) in merged {
        out.extend(data);
    }
    Ok(out)
}

pub(crate) fn write(
    doc: &Document,
    effects: Effects,
    path: &std::path::Path,
    cancel: &AtomicBool,
) -> Result<(), String> {
    let bytes = encode(doc, effects, cancel)?;
    crate::storage::atomic_stream(path, |file| {
        crate::fill::check_cancel(Some(cancel))?;
        std::io::Write::write_all(file, &bytes).map_err(|e| e.to_string())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer_mask::{LayerMask, Mask};

    fn unpack(mut data: &[u8], len: usize) -> Vec<u8> {
        let mut out = Vec::new();
        while out.len() < len {
            let n = data[0] as i8;
            if n >= 0 {
                out.extend(&data[1..=n as usize + 1]);
                data = &data[n as usize + 2..];
            } else {
                out.extend(std::iter::repeat_n(data[1], (1 - n as i32) as usize));
                data = &data[2..];
            }
        }
        assert!(data.is_empty());
        out
    }

    #[test]
    fn packbits_round_trips_runs_literals_and_long_rows() {
        let mut long = vec![7u8; 300];
        long.extend((0..300).map(|i| (i * 37 % 251) as u8));
        long.extend([1, 1, 2, 2, 2, 3]);
        for row in [vec![], vec![5], vec![5, 5], vec![1, 2], long] {
            let mut packed = Vec::new();
            packbits(&row, &mut packed);
            assert_eq!(unpack(&packed, row.len()), row);
            assert!(packed.len() <= row.len() + row.len().div_ceil(128));
        }
    }

    fn paint(layer: &mut Layer, width: usize, rect: [usize; 4], rgba: [u8; 4]) {
        let mut writer = layer.pixels.writer();
        for y in rect[1]..rect[3] {
            for x in rect[0]..rect[2] {
                writer.rgba_mut(y * width + x).copy_from_slice(&rgba);
            }
        }
    }

    fn import(bytes: &[u8]) -> crate::import_report::Imported {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("out.psd");
        std::fs::write(&path, bytes).unwrap();
        crate::psd_import::import(&path, &|| false).unwrap()
    }

    #[test]
    fn layers_folders_masks_and_settings_survive_a_round_trip() {
        let (w, h) = (300, 270);
        let mut doc = Document::new(w, h);
        doc.dpi = 350.0;
        let base = &mut doc.layers[0];
        base.name = "Paper".into();
        paint(base, w, [0, 0, w, h], [250, 245, 230, 255]);
        let mut ink = Layer::raster("Ink ✒ 線画".into(), w, h);
        paint(&mut ink, w, [20, 30, 280, 260], [10, 20, 30, 200]);
        // Hidden color under full transparency survives too.
        paint(&mut ink, w, [5, 5, 6, 6], [90, 0, 0, 0]);
        ink.blend = BlendMode::Multiply;
        ink.opacity = 0.6;
        ink.alpha_locked = true;
        ink.depth = 1;
        let mut coverage = Mask::new(w, h, 255).unwrap();
        for x in 0..w {
            coverage.writer().set(x, 100, 40);
        }
        ink.mask = Some(LayerMask {
            id: crate::document::LayerId::new(),
            enabled: false,
            linked: true,
            coverage,
        });
        let mut shade = Layer::raster("Shade".into(), w, h);
        paint(&mut shade, w, [50, 50, 60, 70], [0, 0, 255, 128]);
        shade.clipped = true;
        shade.depth = 1;
        shade.visible = false;
        let mut inner = Layer::new_folder("Empty".into());
        inner.depth = 1;
        inner.collapsed = true;
        let mut folder = Layer::new_folder("Character".into());
        folder.blend = BlendMode::PassThrough;
        folder.locked = true;
        let mut top = Layer::raster("Glow".into(), w, h);
        paint(&mut top, w, [200, 0, 300, 10], [255, 200, 0, 255]);
        top.blend = BlendMode::Screen;
        doc.layers.extend([ink, shade, inner, folder, top]);
        doc.version = 2;
        doc.validate().unwrap();

        let bytes = encode(&doc, Effects::Editable, &AtomicBool::new(false)).unwrap();
        let imported = import(&bytes);
        let back = imported.document;
        assert_eq!((back.width, back.height, back.dpi), (w, h, 350.0));
        assert_eq!(back.layers.len(), doc.layers.len());
        for (a, b) in doc.layers.iter().zip(&back.layers) {
            assert_eq!(a.name, b.name);
            assert_eq!(
                (a.folder, a.depth, a.blend, a.clipped, a.visible),
                (b.folder, b.depth, b.blend, b.clipped, b.visible),
                "{}",
                a.name
            );
            assert_eq!((a.locked, a.alpha_locked), (b.locked, b.alpha_locked));
            assert!((a.opacity - b.opacity).abs() < 0.5 / 255.0, "{}", a.name);
            if a.folder {
                assert_eq!(a.collapsed, b.collapsed);
            } else {
                assert_eq!(a.pixels.as_slice(), b.pixels.as_slice(), "{}", a.name);
            }
            assert_eq!(a.mask.is_some(), b.mask.is_some());
            if let (Some(a), Some(b)) = (&a.mask, &b.mask) {
                assert_eq!(a.enabled, b.enabled);
                for y in 0..h {
                    for x in 0..w {
                        assert_eq!(a.coverage.at(x, y), b.coverage.at(x, y));
                    }
                }
            }
        }
        // The flattened image is Dudel's own rendering.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("out.psd");
        std::fs::write(&path, &bytes).unwrap();
        let saved = crate::psd_import::saved_composite(&path).unwrap().document;
        let saved: Vec<u8> = (0..w * h)
            .flat_map(|i| saved.layers[0].pixels.rgba(i))
            .collect();
        assert_eq!(saved, doc.composite_transparent());
    }

    #[test]
    fn effects_become_layers_and_adjustment_layers_are_written() {
        let (w, h) = (64, 64);
        let mut doc = Document::new(w, h);
        paint(&mut doc.layers[0], w, [16, 16, 48, 48], [200, 30, 30, 255]);
        doc.layers[0].name = "Dot".into();
        doc.layers[0].effects = vec![crate::layer_effects::LayerEffect::new(
            crate::layer_effects::Effect::preset("Stroke").unwrap(),
        )];
        doc.layers[0].blend = BlendMode::AddGlow;
        let mut invert = Layer::raster("Invert".into(), w, h);
        invert.adjustment = Some(crate::adjustment::Adjustment::new(
            crate::adjustment::Settings::Invert,
        ));
        doc.layers.push(invert);
        doc.active = 0;
        let notes = notes(&doc, Effects::AsLayers);
        assert_eq!(notes.len(), 1, "{notes:?}");
        let back =
            import(&encode(&doc, Effects::AsLayers, &AtomicBool::new(false)).unwrap()).document;
        let expanded = crate::layer_effects::expand(&doc, false, &|| false).unwrap();
        assert!(expanded.layers.len() > 2);
        assert_eq!(back.layers.len(), expanded.layers.len());
        assert_eq!(back.layers[expanded.active].blend, BlendMode::AddGlow);
        for (a, b) in expanded.layers.iter().zip(&back.layers) {
            assert_eq!(a.adjustment, b.adjustment, "{}", a.name);
            if a.adjustment.is_none() {
                assert_eq!(a.pixels.as_slice(), b.pixels.as_slice(), "{}", a.name);
            }
        }
        assert_eq!(back.composite(), doc.composite());
    }

    #[test]
    fn every_adjustment_kind_survives_a_round_trip() {
        use crate::adjustment::{LevelsChannel, Settings};
        let mut all: Vec<Settings> = crate::adjustment::presets().into();
        all.extend([
            Settings::Posterize { levels: 7 },
            Settings::Threshold { level: 99 },
            Settings::Levels {
                channels: vec![
                    Some(LevelsChannel {
                        input: [10.0, 240.0],
                        output: [5.0, 250.0],
                        gamma: 1.25,
                    }),
                    None,
                    Some(LevelsChannel {
                        input: [0.0, 200.0],
                        output: [0.0, 255.0],
                        gamma: 0.8,
                    }),
                    None,
                ],
            },
            Settings::Curves {
                channels: vec![
                    (0, vec![[0, 0], [100, 140], [255, 255]]),
                    (2, vec![[0, 20], [255, 230]]),
                ],
                bezier: false,
            },
            Settings::BrightnessContrast {
                brightness: 30.0,
                contrast: -20.0,
                legacy: false,
            },
            Settings::BrightnessContrast {
                brightness: -40.0,
                contrast: 15.0,
                legacy: true,
            },
            Settings::Exposure {
                exposure: 0.5,
                offset: -0.0625,
                gamma: 1.25,
                encoding: 2.2,
            },
            Settings::HueSaturation {
                colorize: None,
                master: [20.0, -30.0, 10.0],
                ranges: vec![([15.0, 45.0, 75.0, 105.0], [10.0, 20.0, -5.0])],
            },
            Settings::HueSaturation {
                colorize: Some([200.0, 40.0, -10.0]),
                master: [0.0; 3],
                ranges: vec![],
            },
            Settings::GradientMap {
                stops: vec![
                    (0.0, [10.0, 20.0, 30.0], 0.5),
                    (0.25, [100.0, 0.0, 200.0], 0.3),
                    (1.0, [250.0, 240.0, 230.0], 0.5),
                ],
                reverse: true,
            },
            Settings::ColorBalance {
                tones: [[10.0, -20.0, 0.0], [0.0, 5.0, 5.0], [-100.0, 0.0, 100.0]],
                preserve_luminosity: true,
            },
            Settings::Vibrance {
                vibrance: 25.0,
                saturation: -10.0,
            },
            Settings::BlackWhite {
                weights: [50.0, 60.0, -40.0, 160.0, 20.0, 80.0],
                tint: Some([225.0, 211.0, 179.0]),
            },
            Settings::PhotoFilter {
                color: [236.0, 138.0, 0.0],
                density: 25.0,
                preserve_luminosity: true,
            },
            Settings::ChannelMixer {
                rows: [
                    [100.0, 0.0, 0.0, 0.0],
                    [0.0, 80.0, 20.0, 0.0],
                    [0.0, 0.0, 100.0, 5.0],
                ],
                monochrome: false,
            },
            Settings::SelectiveColor {
                colors: std::array::from_fn(|i| [i as f32, -(i as f32), 10.0, 0.0]),
                absolute: true,
            },
        ]);
        // CSP's kinds are approximated; see the next test.
        all.retain(|s| !crate::adjustment::psd_approximated(s));
        let (w, h) = (4, 4);
        for settings in all {
            let mut doc = Document::new(w, h);
            paint(&mut doc.layers[0], w, [0, 0, w, h], [200, 100, 50, 255]);
            let mut layer = Layer::raster("Adjust".into(), w, h);
            layer.adjustment = Some(crate::adjustment::Adjustment::new(settings.clone()));
            layer.opacity = 0.6;
            doc.layers.push(layer);
            let back =
                import(&encode(&doc, Effects::Editable, &AtomicBool::new(false)).unwrap()).document;
            let read = back.layers[1].adjustment.as_ref().map(|a| a.settings());
            // Identity curves and levels read back without channels.
            let neutral = |s: &Settings| match s {
                Settings::Curves { channels, .. } => channels
                    .iter()
                    .all(|(_, points)| points == &[[0, 0], [255, 255]]),
                Settings::Levels { channels } => channels
                    .iter()
                    .flatten()
                    .all(|c| c.input == [0.0, 255.0] && c.output == [0.0, 255.0] && c.gamma == 1.0),
                _ => false,
            };
            match read {
                Some(read) if neutral(&settings) && neutral(read) => {}
                read => assert_eq!(read, Some(&settings)),
            }
            assert_eq!(back.composite(), doc.composite(), "{}", settings.name());
        }
    }

    #[test]
    fn every_effect_stays_editable_through_a_round_trip() {
        use crate::layer_effects::{Effect, LayerEffect, PRESETS};
        let (w, h) = (96, 96);
        let mut doc = Document::new(w, h);
        paint(&mut doc.layers[0], w, [0, 0, w, h], [240, 240, 240, 255]);
        let mut shape = Layer::raster("Shape".into(), w, h);
        paint(&mut shape, w, [24, 24, 72, 72], [40, 120, 200, 255]);
        let mut effects: Vec<Effect> = PRESETS.iter().filter_map(|p| Effect::preset(p)).collect();
        effects.extend(Effect::bevel_presets(1));
        effects.sort_by_key(Effect::rank);
        shape.effects = effects.into_iter().map(LayerEffect::new).collect();
        doc.layers.push(shape);
        assert!(notes(&doc, Effects::Editable).is_empty());
        let back =
            import(&encode(&doc, Effects::Editable, &AtomicBool::new(false)).unwrap()).document;
        assert_eq!(back.layers.len(), 2, "effects stay on their layer");
        let (a, b) = (&doc.layers[1].effects, &back.layers[1].effects);
        assert_eq!(a.len(), b.len());
        for (a, b) in a.iter().zip(b) {
            assert_eq!(a.effect.name(), b.effect.name());
            assert_eq!(
                (a.effect.blend, a.effect.paint.clone()),
                (b.effect.blend, b.effect.paint.clone())
            );
            assert!((a.effect.opacity - b.effect.opacity).abs() < 1e-4);
        }
        let (x, y) = (doc.composite(), back.composite());
        let worst = x.iter().zip(&y).map(|(p, q)| p.abs_diff(*q)).max().unwrap();
        assert!(worst <= 1, "renders differ by {worst}");
    }

    #[test]
    fn folder_effects_stay_editable_through_a_round_trip() {
        use crate::layer_effects::{Effect, LayerEffect};
        let (w, h) = (64, 64);
        let mut doc = Document::new(w, h);
        doc.layers[0].depth = 1;
        paint(&mut doc.layers[0], w, [16, 16, 40, 40], [200, 30, 30, 255]);
        let mut folder = Layer::new_folder("Group".into());
        folder.effects = ["Drop Shadow", "Stroke"]
            .iter()
            .map(|p| LayerEffect::new(Effect::preset(p).unwrap()))
            .collect();
        doc.layers.push(folder);
        doc.validate().unwrap();
        assert!(notes(&doc, Effects::Editable).is_empty());
        let back =
            import(&encode(&doc, Effects::Editable, &AtomicBool::new(false)).unwrap()).document;
        assert_eq!(back.layers.len(), 2, "effects stay on the folder");
        assert!(back.layers[1].folder);
        assert_eq!(back.layers[1].effects.len(), 2);
        let (x, y) = (doc.composite(), back.composite());
        let worst = x.iter().zip(&y).map(|(p, q)| p.abs_diff(*q)).max().unwrap();
        assert!(worst <= 1, "renders differ by {worst}");
    }

    #[test]
    fn switched_off_effects_survive_a_round_trip() {
        use crate::layer_effects::{Effect, LayerEffect};
        let (w, h) = (32, 32);
        let mut doc = Document::new(w, h);
        paint(&mut doc.layers[0], w, [8, 8, 24, 24], [200, 30, 30, 255]);
        let mut shadow = LayerEffect::new(Effect::preset("Drop Shadow").unwrap());
        shadow.enabled = false;
        let stroke = LayerEffect::new(Effect::preset("Stroke").unwrap());
        doc.layers[0].effects = vec![shadow, stroke];
        let back =
            import(&encode(&doc, Effects::Editable, &AtomicBool::new(false)).unwrap()).document;
        assert_eq!(back.layers.len(), 1);
        let effects: Vec<_> = back.layers[0]
            .effects
            .iter()
            .map(|e| (e.effect.name(), e.enabled))
            .collect();
        assert_eq!(effects, [("Drop Shadow", false), ("Stroke", true)]);
        assert_eq!(back.composite(), doc.composite());
        // Only switched-off effects: still kept, nothing drawn.
        doc.layers[0].effects.truncate(1);
        let back =
            import(&encode(&doc, Effects::Editable, &AtomicBool::new(false)).unwrap()).document;
        assert_eq!(back.layers[0].effects.len(), 1);
        assert!(!back.layers[0].effects[0].enabled);
    }

    #[test]
    fn fill_opacity_survives_a_round_trip() {
        use crate::layer_effects::{Effect, LayerEffect};
        let (w, h) = (48, 48);
        let mut doc = Document::new(w, h);
        paint(&mut doc.layers[0], w, [0, 0, w, h], [240, 240, 240, 255]);
        let mut shape = Layer::raster("Shape".into(), w, h);
        paint(&mut shape, w, [12, 12, 36, 36], [40, 120, 200, 255]);
        shape.opacity = 0.8;
        shape.fill = 0.4;
        shape.effects = vec![LayerEffect::new(Effect::preset("Stroke").unwrap())];
        doc.layers.push(shape);
        let back =
            import(&encode(&doc, Effects::Editable, &AtomicBool::new(false)).unwrap()).document;
        assert_eq!(back.layers.len(), 2);
        let layer = &back.layers[1];
        assert!((layer.opacity - 0.8).abs() < 1.0 / 255.0);
        assert!((layer.fill - 0.4).abs() < 1.0 / 255.0);
        assert_eq!(layer.effects.len(), 1);
        let (x, y) = (doc.composite(), back.composite());
        let worst = x.iter().zip(&y).map(|(p, q)| p.abs_diff(*q)).max().unwrap();
        assert!(worst <= 2, "renders differ by {worst}");
    }

    #[test]
    fn effects_photoshop_cannot_show_are_drawn_as_layers() {
        use crate::layer_effects::{Effect, LayerEffect};
        let mut doc = Document::new(32, 32);
        paint(&mut doc.layers[0], 32, [8, 8, 24, 24], [0, 0, 0, 255]);
        let mut glow = Effect::preset("Outer Glow").unwrap();
        glow.blend = BlendMode::AddGlow;
        doc.layers[0].effects = vec![LayerEffect::new(glow)];
        assert_eq!(notes(&doc, Effects::Editable).len(), 1);
        let back =
            import(&encode(&doc, Effects::Editable, &AtomicBool::new(false)).unwrap()).document;
        assert!(back.layers.len() > 1);
        assert!(back.layers.iter().all(|l| l.effects.is_empty()));
    }

    #[test]
    fn csp_adjustments_are_written_as_close_photoshop_ones() {
        use crate::adjustment::{Adjustment, Settings};
        let bezier = Settings::Curves {
            channels: vec![(0, vec![[0, 0], [128, 200], [255, 255]])],
            bezier: true,
        };
        let hsl = Settings::HueSaturationLuminosity {
            hue: 30,
            saturation: -20,
            luminosity: 10,
        };
        let mut doc = Document::new(4, 4);
        for (i, settings) in [bezier.clone(), hsl].into_iter().enumerate() {
            let mut layer = Layer::raster(format!("CSP {i}"), 4, 4);
            layer.adjustment = Some(Adjustment::new(settings));
            doc.layers.push(layer);
        }
        assert_eq!(notes(&doc, Effects::Editable).len(), 1);
        let back =
            import(&encode(&doc, Effects::Editable, &AtomicBool::new(false)).unwrap()).document;
        let curve = back.layers[1].adjustment.clone().unwrap();
        assert!(matches!(
            curve.settings(),
            Settings::Curves { bezier: false, .. }
        ));
        let original = Adjustment::new(bezier);
        for v in 0..=255 {
            let v = v as f32;
            let (a, b) = (original.apply([v; 3])[0], curve.apply([v; 3])[0]);
            assert!((a - b).abs() <= 2.0, "{v}: {a} vs {b}");
        }
        assert!(matches!(
            back.layers[2].adjustment.as_ref().unwrap().settings(),
            Settings::HueSaturation { .. }
        ));
    }
}
