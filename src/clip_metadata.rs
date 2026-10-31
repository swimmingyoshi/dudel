//! Bounded read-only CSP metadata, strict raster import plans and rendering studies.
use crate::{
    clip_container,
    document::{MAX_DEPTH, MAX_ENTRIES, MAX_RASTER_LAYERS},
};
use rusqlite::{Connection, OpenFlags, Row, limits::Limit};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::Path,
    time::{Duration, Instant},
};

const MAX_SQL: usize = 32 * 1024 * 1024;
const MAX_MIPS: usize = (MAX_ENTRIES + 1) * 16;

#[derive(Debug, serde::Serialize)]
struct Canvas {
    id: i64,
    width: f64,
    height: f64,
    channel_bytes: i64,
    channel_order: i64,
    root: i64,
    current: i64,
    profile_bytes: i64,
    unit: i64,
    resolution: f64,
}
#[derive(Debug, serde::Serialize)]
struct Layer {
    id: i64,
    layer_type: i64,
    folder: i64,
    visibility: i64,
    opacity: i64,
    composite: i64,
    clip: i64,
    next: i64,
    child: i64,
    mipmap: i64,
    offset_x: i64,
    offset_y: i64,
    render_offscreen_offset_x: i64,
    render_offscreen_offset_y: i64,
    paper_draw_color: Option<PaperDrawColor>,
    /// An image material layer (a placed picture CSP can resize): it stores
    /// the resized result as an ordinary render bitmap. Import mode only.
    image: bool,
    /// A vector layer whose strokes Dudel draws. Import mode only.
    vector: bool,
    /// A screentone layer (a fill turned into dots by the tone effect).
    /// Import mode only.
    tone: bool,
    /// A correction layer Dudel imports as an adjustment layer. Import mode
    /// only.
    correction: bool,
}
impl Layer {
    // Observed ordinary folders may carry the closed UI bit (16).
    // Keep the raw value in reports; do not classify type-0 vector/text as folders.
    fn is_folder(&self) -> bool {
        matches!(self.folder, 1 | 17)
    }
    fn supported_kind(&self) -> bool {
        matches!(
            (self.layer_type, self.folder),
            (0, 1 | 17) | (256, 1) | (1 | 1584, 0)
        ) || self.is_raster()
            || self.is_image()
            || self.is_vector()
            || self.is_tone()
            || self.is_correction()
    }
    /// Correction layers are type 4096 with the layer mask bit (2).
    fn is_correction(&self) -> bool {
        self.correction && self.layer_type == 4098 && self.folder == 0
    }
    /// An ordinary raster layer; type 3 is one with a layer mask.
    fn is_raster(&self) -> bool {
        matches!(self.layer_type, 1 | 3) && self.folder == 0
    }

    fn is_image(&self) -> bool {
        self.image && self.layer_type == 0 && self.folder == 0
    }
    fn is_vector(&self) -> bool {
        self.vector && self.layer_type == 0 && self.folder == 0
    }
    fn is_tone(&self) -> bool {
        self.tone && self.layer_type == 2 && self.folder == 0
    }
}
#[derive(Debug, serde::Serialize)]
struct PaperDrawColor {
    enabled: i64,
    raw_rgb: [u32; 3],
    // Measured high-byte projection for the observed paper samples. This does
    // not establish profile conversion or general special-layer semantics.
    high_byte_rgb: [u8; 3],
}
#[derive(Debug, serde::Serialize)]
struct Entry {
    id: i64,
    depth: u8,
}
#[derive(Debug, serde::Serialize)]
struct FullScale {
    layer: i64,
    mipmap: i64,
    info: i64,
    offscreen: i64,
    attribute_bytes: usize,
    attributes: crate::clip_attributes::Attributes,
    block_layout_validated: bool,
    external_id: Option<String>,
    external_chunk_offset: Option<u64>,
}
#[derive(Debug, serde::Serialize)]
struct Report {
    scope: &'static str,
    canvas: Canvas,
    layers: Vec<Layer>,
    root_child_next_order: Vec<Entry>,
    full_scale: Vec<FullScale>,
    external_chunks: usize,
    diagnostics: Vec<String>,
    timeline_present: bool,
    unsupported_track_schema: bool,
    import_issues: Vec<String>,
    rendering_study_omissions: Vec<String>,
    /// For the import summary: editing-guide fields not imported, with the
    /// layers carrying them, and layers whose layer color was not applied.
    #[serde(skip)]
    guide_omissions: BTreeMap<&'static str, BTreeSet<i64>>,
    #[serde(skip)]
    tinted_layers: Vec<i64>,
    #[serde(skip)]
    document_properties: BTreeMap<i64, String>,
    #[serde(skip)]
    locks: BTreeMap<i64, i64>,
    /// Import mode: problems confined to one layer. That layer (and a
    /// folder's contents) is skipped with a note instead of failing the file.
    #[serde(skip)]
    layer_issues: BTreeMap<i64, Vec<String>>,
    /// Import mode: the cel each animation folder shows on the saved frame.
    #[serde(skip)]
    animation: Animation,
    /// Import mode: each image material layer's original picture and where
    /// it lands on the canvas.
    #[serde(skip)]
    images: BTreeMap<i64, ImageSource>,
    /// Import mode: each vector layer's stroke data.
    #[serde(skip)]
    vectors: BTreeMap<i64, ExternalData>,
    /// Import mode: each tone layer's effect and mask bitmap.
    #[serde(skip)]
    tones: BTreeMap<i64, (ToneEffect, MaskSource)>,
    /// Import mode: the layer mask of each masked raster or correction
    /// layer.
    #[serde(skip)]
    raster_masks: BTreeMap<i64, MaskSource>,
    /// Import mode: each correction layer's adjustment.
    #[serde(skip)]
    corrections: BTreeMap<i64, crate::adjustment::Settings>,
    /// Import mode: brush styles drawn as plain round brushes (no texture,
    /// spray, watercolor or blending), which vector strokes may use, with
    /// their anti-aliasing level.
    #[serde(skip)]
    plain_brush_styles: BTreeMap<u32, u8>,
}
/// An external chunk holding one layer's data.
#[derive(Debug)]
struct ExternalData {
    id: String,
    offset: u64,
}
/// A layer mask's full-size bitmap.
#[derive(Debug)]
struct MaskSource {
    attributes: crate::clip_attributes::Attributes,
    data: ExternalData,
}
/// An image material layer's original picture. CSP saves the original and
/// redraws the resized result when it opens the file.
#[derive(Debug)]
struct ImageSource {
    attributes: crate::clip_attributes::Attributes,
    external_id: String,
    external_chunk_offset: u64,
    /// Where the picture's top-left, top-right and bottom-left corners land.
    corners: [[f64; 2]; 3],
}
#[derive(Debug, Default)]
struct Animation {
    current_frame: Option<f64>,
    frame_rate: Option<f64>,
    /// Animation folder id to the cel name it shows, or None for no cel.
    folders: BTreeMap<i64, Option<String>>,
    /// Folder id to the external chunk holding its track's keyframes.
    mixers: BTreeMap<i64, String>,
    /// Folders whose keyframes could not be read; they show the cel name the
    /// track stores, which can lag behind the saved frame.
    uncertain: BTreeSet<i64>,
}

fn db_error(error: rusqlite::Error) -> String {
    format!("Clip metadata: {error}")
}

fn table_columns(db: &Connection, table: &str) -> Result<BTreeSet<String>, String> {
    let mut statement = db
        .prepare("SELECT name FROM pragma_table_info(?1) LIMIT 129")
        .map_err(db_error)?;
    let names = statement
        .query_map([table], |row| {
            let name = row.get_ref(0)?.as_str()?;
            if name.len() > 128 {
                return Err(rusqlite::Error::InvalidQuery);
            }
            Ok(name.to_owned())
        })
        .map_err(db_error)?
        .collect::<rusqlite::Result<BTreeSet<_>>>()
        .map_err(db_error)?;
    if names.len() > 128 {
        return Err("Clip schema column bound".into());
    }
    Ok(names)
}

fn document_features(db: &Connection, report: &mut Report) -> Result<(), String> {
    document_features_mode(db, report, false)
}
/// Strict inspection rejects the file; import mode skips just that layer.
fn layer_issue(report: &mut Report, study: bool, id: i64, message: String) {
    if study {
        report.layer_issues.entry(id).or_default().push(message);
    } else {
        report.import_issues.push(format!("Layer {id}: {message}"));
    }
}
/// An image material layer's original picture and placement. The layout of
/// `ResizableImageInfo` was read from every observed image layer: a header
/// whose first word is its own length (120), the original's size in words 8
/// and 9, and from byte 120 the four placed corners as (x, y) doubles in the
/// order top left, top right, bottom left, bottom right.
fn image_source(db: &Connection, id: i64) -> Result<ImageSource, String> {
    let (info, attribute, block): (Vec<u8>, Vec<u8>, Vec<u8>) = db
        .query_row(
            "SELECT l.ResizableImageInfo,o.Attribute,o.BlockData FROM Layer l \
             JOIN Mipmap m ON m.MainId=l.ResizableOriginalMipmap AND m.LayerId=l.MainId \
             JOIN MipmapInfo i ON i.MainId=m.BaseMipmapInfo AND i.LayerId=l.MainId AND i.ThisScale=100 \
             JOIN Offscreen o ON o.MainId=i.Offscreen AND o.LayerId=l.MainId \
             WHERE l.MainId=?1 AND length(l.ResizableImageInfo)=184 \
             AND length(o.Attribute)<=?2 AND length(o.BlockData)<=64",
            rusqlite::params![id, crate::clip_attributes::MAX_BYTES as i64],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .map_err(|_| "original picture not found")?;
    let word = |at: usize| u32::from_be_bytes(info[at..at + 4].try_into().unwrap());
    let double = |at: usize| f64::from_be_bytes(info[at..at + 8].try_into().unwrap());
    let attributes = crate::clip_attributes::parse(&attribute)?;
    if word(0) != 120 || [word(32), word(36)] != [attributes.width, attributes.height] {
        return Err("unrecognized placement record".into());
    }
    if !attributes.transparent_default()
        || attributes.width as usize > crate::document::MAX_SIDE
        || attributes.height as usize > crate::document::MAX_SIDE
    {
        return Err("original picture too large or not transparent".into());
    }
    let corner = |i: usize| [double(120 + i * 16), double(128 + i * 16)];
    let [tl, tr, bl, br] = [corner(0), corner(1), corner(2), corner(3)];
    let skew = (tr[0] + bl[0] - tl[0] - br[0]).abs() + (tr[1] + bl[1] - tl[1] - br[1]).abs();
    if [tl, tr, bl, br]
        .iter()
        .flatten()
        .any(|v| !v.is_finite() || v.abs() > 1e6)
        || skew > 0.5
    {
        return Err("distorted placement".into());
    }
    let external_id = std::str::from_utf8(&block)
        .ok()
        .filter(|id| id.len() == 40 && id.starts_with("extrnlid"))
        .ok_or("original picture has no data reference")?
        .to_owned();
    let offset: i64 = db
        .query_row(
            "SELECT Offset FROM ExternalChunk WHERE ExternalID=?1",
            [&external_id],
            |row| row.get(0),
        )
        .map_err(|_| "original picture data is missing")?;
    Ok(ImageSource {
        attributes,
        external_id,
        external_chunk_offset: u64::try_from(offset).map_err(|_| "invalid data offset")?,
        corners: [tl, tr, bl],
    })
}
/// Vector layers with exactly one stroke-data row, and the brush styles they
/// may use.
fn vector_layers(db: &Connection, report: &mut Report) -> Result<(), String> {
    let tables: BTreeSet<String> = rows(
        db,
        "SELECT name FROM sqlite_schema WHERE type='table'",
        4096,
        |row| row.get::<_, String>(0),
    )?
    .into_iter()
    .collect();
    if !tables.contains("VectorObjectList") || !tables.contains("BrushStyle") {
        return Ok(());
    }
    let style_columns = table_columns(db, "BrushStyle")?;
    let antialias = if style_columns.contains("AntiAlias") {
        "coalesce(AntiAlias,0)"
    } else {
        "0"
    };
    let mut plain = format!("SELECT MainId,{antialias} FROM BrushStyle WHERE 1");
    for column in [
        "PatternStyle",
        "CompositeMode",
        "TexturePattern",
        "UseWaterColor",
        "SprayFlag",
    ] {
        if style_columns.contains(column) {
            plain.push_str(&format!(" AND coalesce(\"{column}\",0)=0"));
        }
    }
    report.plain_brush_styles = rows(db, &plain, MAX_ENTRIES, |row| {
        Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?))
    })?
    .into_iter()
    .filter_map(|(id, level)| Some((u32::try_from(id).ok()?, level.clamp(0, 3) as u8)))
    .collect();
    let layers = rows(
        db,
        "SELECT l.MainId,v.VectorData FROM Layer l JOIN VectorObjectList v ON v.LayerId=l.MainId \
         WHERE l.LayerType=0 AND l.LayerFolder=0 AND coalesce(l.VectorNormalStrokeIndex,0)!=0 \
         AND (SELECT count(*) FROM VectorObjectList w WHERE w.LayerId=l.MainId)=1 \
         AND length(v.VectorData)=40",
        MAX_ENTRIES + 1,
        |row| Ok((row.get::<_, i64>(0)?, row.get::<_, Vec<u8>>(1)?)),
    )?;
    for (id, data) in layers {
        let Some(external) = std::str::from_utf8(&data)
            .ok()
            .filter(|id| id.starts_with("extrnlid"))
        else {
            continue;
        };
        let Ok(offset) = db.query_row(
            "SELECT Offset FROM ExternalChunk WHERE ExternalID=?1",
            [external],
            |row| row.get::<_, i64>(0),
        ) else {
            continue;
        };
        let Ok(offset) = u64::try_from(offset) else {
            continue;
        };
        if let Some(layer) = report.layers.iter_mut().find(|layer| layer.id == id) {
            layer.vector = true;
            report.vectors.insert(
                id,
                ExternalData {
                    id: external.to_owned(),
                    offset,
                },
            );
        }
    }
    Ok(())
}
/// A correction layer's settings as a Dudel adjustment (`FilterLayerInfo`:
/// kind, byte length, then the values). Only what CSP was checked against
/// on the owner's samples converts; anything else keeps the layer skipped.
fn correction(info: &[u8]) -> Result<crate::adjustment::Settings, String> {
    use crate::adjustment::Settings;
    let word = |at: usize| -> Option<i32> {
        Some(i32::from_be_bytes(info.get(at..at + 4)?.try_into().ok()?))
    };
    let kind = word(0).ok_or("damaged correction layer")?;
    if word(4).and_then(|n| usize::try_from(n).ok()) != Some(info.len().saturating_sub(8)) {
        return Err("damaged correction layer".into());
    }
    match kind {
        // out = (in + brightness - 128) / (1 - contrast) + 128 in 0..=255,
        // fitted on the owner's +30 / +40 sample within a quarter level.
        // Photoshop's legacy formula is the same line around 127.5, so the
        // values convert exactly. Negative contrast is not yet checked.
        1 if info.len() == 16 => {
            let (brightness, contrast) = (word(8).unwrap(), word(12).unwrap());
            if !(-150..=150).contains(&brightness) || !(0..=99).contains(&contrast) {
                return Err("brightness/contrast settings Dudel does not draw yet".into());
            }
            let c = contrast as f32 / 100.0;
            Ok(Settings::BrightnessContrast {
                brightness: brightness as f32 - 0.5 * c,
                contrast: contrast as f32 / 0.99,
                legacy: true,
            })
        }
        // Hue (degrees), saturation and luminosity (percent), drawn with
        // CSP's own whole-number HSV.
        4 if info.len() == 20 => {
            let (hue, saturation, luminosity) =
                (word(8).unwrap(), word(12).unwrap(), word(16).unwrap());
            let settings = Settings::HueSaturationLuminosity {
                hue,
                saturation,
                luminosity,
            };
            crate::adjustment::Adjustment::try_from(settings.clone())
                .map_err(|_| "hue/saturation/luminosity settings out of range".to_string())?;
            Ok(settings)
        }
        // 32 curves of a count and up to 32 (x, y) pairs (16-bit), the
        // first for all channels. Only that one may bend, with at most one
        // inner point: CSP's curve for those matched a quadratic Bezier.
        3 if info.len() == 8 + 32 * 130 => {
            let curve = |index: usize| -> Vec<[u16; 2]> {
                let at = 8 + index * 130;
                let half = |i: usize| u16::from_be_bytes([info[at + i * 2], info[at + i * 2 + 1]]);
                (0..usize::from(half(0)).min(32))
                    .map(|p| [half(1 + p * 2), half(2 + p * 2)])
                    .collect()
            };
            if (1..32).any(|index| curve(index) != [[0, 0], [u16::MAX, u16::MAX]]) {
                return Err("tone curves on single channels Dudel does not draw yet".into());
            }
            let points = curve(0);
            if !(2..=3).contains(&points.len()) {
                return Err(
                    "tone curves with more than one inner point Dudel does not draw yet".into(),
                );
            }
            let points: Vec<[u8; 2]> = points
                .iter()
                .map(|p| p.map(|v| ((u32::from(v) * 255 + 32767) / 65535) as u8))
                .collect();
            let settings = Settings::Curves {
                channels: vec![(0, points)],
                bezier: true,
            };
            crate::adjustment::Adjustment::try_from(settings.clone())
                .map_err(|_| "damaged tone curve".to_string())?;
            Ok(settings)
        }
        _ => Err("this correction layer kind is not supported yet".into()),
    }
}
/// A screentone effect: its dot density (percent) and, when Dudel can draw
/// its dots, their grid.
#[derive(Debug, Clone, Copy, PartialEq)]
struct ToneEffect {
    percent: u32,
    screen: Option<crate::screentone::Screen>,
}
/// The tone effect of a `LayerEffectInfo` whose only enabled effect is the
/// screentone. The layout (a length, version 2, then named effects) was read
/// from CSP's line-and-tone conversion output; each check below is a field
/// observed to be 0 for a disabled effect.
fn tone_effect(info: &[u8]) -> Option<ToneEffect> {
    let word = |at: usize| Some(u32::from_be_bytes(info.get(at..at + 4)?.try_into().ok()?));
    if word(0)? as usize != info.len() || word(4)? != 2 {
        return None;
    }
    let find = |name: &str| {
        let key: Vec<u8> = name.encode_utf16().flat_map(u16::to_be_bytes).collect();
        info.windows(key.len())
            .position(|w| w == key.as_slice())
            .map(|at| at + key.len())
    };
    // EffectEdge and EffectTone start with their on/off word; the later
    // effects start with a size word, then on/off.
    if word(find("EffectEdge")?)? != 0 {
        return None;
    }
    let tone = find("EffectTone")?;
    if word(tone)? != 1 {
        return None;
    }
    for name in [
        "EffectTextureMap",
        "EffectTonePosterize",
        "EffectWaterEdge",
        "EffectLine",
    ] {
        if let Some(at) = find(name)
            && word(at + 4)? != 0
        {
            return None;
        }
    }
    let density = word(tone + 20)?;
    if density > 100 {
        return None;
    }
    // After on/off: the resolution the tone was laid out for (f64), dot
    // shape, density source, density, lines (f64), a render flag, a zero,
    // angle, noise size, a further size, and the grid's offset as integers
    // and then as f64 pairs, in the order CSP saves them. Dudel draws round
    // dots (shape 1), squares (2) and lines (4) at the set density (source
    // 2) with the flag and offsets the observed tones have; other tones are
    // shaded evenly.
    let float = |at: usize| Some(f64::from_be_bytes(info.get(at..at + 8)?.try_into().ok()?));
    let screen = (|| {
        let shape = match word(tone + 12)? {
            1 => crate::screentone::Shape::Round,
            2 => crate::screentone::Shape::Square,
            4 => crate::screentone::Shape::Line,
            _ => return None,
        };
        let plain = word(tone + 16)? == 2
            && word(tone + 32)? == 1
            && word(tone + 52)? == 0
            && word(tone + 56)? == 0
            && float(tone + 60)? == 0.0
            && float(tone + 68)? == 0.0;
        plain.then_some(crate::screentone::Screen {
            lines: float(tone + 24)?,
            resolution: float(tone + 4)?,
            angle: word(tone + 40)?,
            shape,
        })
    })()
    .filter(|screen| screen.side().is_some());
    Some(ToneEffect {
        percent: density,
        screen,
    })
}
/// Screentone layers: their effect and layer mask.
fn tone_layers(db: &Connection, report: &mut Report) -> Result<(), String> {
    let candidates = rows(
        db,
        "SELECT MainId,LayerEffectInfo FROM Layer WHERE LayerType=2 AND LayerFolder=0 \
         AND coalesce(LayerLayerMaskMipmap,0)!=0 AND length(LayerEffectInfo)<=4096 \
         AND length(GradationFillInfo)>0",
        MAX_ENTRIES + 1,
        |row| Ok((row.get::<_, i64>(0)?, row.get::<_, Vec<u8>>(1)?)),
    )?;
    for (id, info) in candidates {
        let Some(effect) = tone_effect(&info) else {
            continue;
        };
        let Ok(mask) = mask_source(db, id) else {
            continue;
        };
        if let Some(layer) = report.layers.iter_mut().find(|layer| layer.id == id) {
            layer.tone = true;
            report.tones.insert(id, (effect, mask));
        }
    }
    Ok(())
}
/// A layer's full-size mask bitmap.
fn mask_source(db: &Connection, id: i64) -> Result<MaskSource, String> {
    let (attribute, block): (Vec<u8>, Vec<u8>) = db
        .query_row(
            "SELECT o.Attribute,o.BlockData FROM Layer l \
             JOIN Mipmap m ON m.MainId=l.LayerLayerMaskMipmap AND m.LayerId=l.MainId \
             JOIN MipmapInfo i ON i.MainId=m.BaseMipmapInfo AND i.LayerId=l.MainId AND i.ThisScale=100 \
             JOIN Offscreen o ON o.MainId=i.Offscreen AND o.LayerId=l.MainId \
             WHERE l.MainId=?1 AND length(o.Attribute)<=?2 AND length(o.BlockData)<=64",
            rusqlite::params![id, crate::clip_attributes::MAX_BYTES as i64],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(|_| "mask bitmap not found")?;
    let attributes = crate::clip_attributes::parse(&attribute)?;
    if !attributes.is_mask() {
        return Err("not a mask bitmap".into());
    }
    let external = std::str::from_utf8(&block)
        .ok()
        .filter(|id| id.starts_with("extrnlid"))
        .ok_or("mask has no data reference")?;
    let offset: i64 = db
        .query_row(
            "SELECT Offset FROM ExternalChunk WHERE ExternalID=?1",
            [external],
            |row| row.get(0),
        )
        .map_err(|_| "mask data is missing")?;
    Ok(MaskSource {
        attributes,
        data: ExternalData {
            id: external.to_owned(),
            offset: u64::try_from(offset).map_err(|_| "invalid mask offset")?,
        },
    })
}
fn document_features_mode(db: &Connection, report: &mut Report, study: bool) -> Result<(), String> {
    let columns = table_columns(db, "Layer")?;
    if study && columns.contains("VectorNormalStrokeIndex") {
        vector_layers(db, report)?;
    }
    if study && columns.contains("LayerEffectInfo") && columns.contains("GradationFillInfo") {
        tone_layers(db, report)?;
    }
    if study && columns.contains("FilterLayerInfo") && columns.contains("SpecialRenderType") {
        let candidates = rows(
            db,
            "SELECT MainId,FilterLayerInfo FROM Layer WHERE LayerType=4098 AND LayerFolder=0 \
             AND SpecialRenderType=13 AND length(FilterLayerInfo)<=8192",
            MAX_ENTRIES + 1,
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, Vec<u8>>(1)?)),
        )?;
        for (id, info) in candidates {
            match correction(&info) {
                Ok(settings) => {
                    report.corrections.insert(id, settings);
                    if let Some(layer) = report.layers.iter_mut().find(|layer| layer.id == id) {
                        layer.correction = true;
                    }
                }
                Err(error) => layer_issue(report, study, id, error),
            }
        }
    }
    if study && columns.contains("LayerLayerMaskMipmap") {
        let masked = rows(
            db,
            "SELECT MainId FROM Layer WHERE LayerType IN (1,3,4098) AND LayerFolder=0 AND coalesce(LayerLayerMaskMipmap,0)!=0",
            MAX_ENTRIES + 1,
            |row| row.get::<_, i64>(0),
        )?;
        for id in masked {
            match mask_source(db, id) {
                Ok(mask) => {
                    report.raster_masks.insert(id, mask);
                }
                Err(error) => layer_issue(
                    report,
                    study,
                    id,
                    format!("unsupported layer mask ({error})"),
                ),
            }
        }
    }
    if study && columns.contains("ResizableImageInfo") {
        let images = rows(
            db,
            "SELECT MainId FROM Layer WHERE LayerType=0 AND LayerFolder=0 AND length(ResizableImageInfo)>0",
            MAX_ENTRIES + 1,
            |row| row.get::<_, i64>(0),
        )?;
        for layer in &mut report.layers {
            layer.image = images.contains(&layer.id);
        }
        for id in images {
            match image_source(db, id) {
                Ok(source) => {
                    report.images.insert(id, source);
                }
                Err(error) => layer_issue(
                    report,
                    study,
                    id,
                    format!("unsupported image material layer ({error})"),
                ),
            }
        }
    }
    if [
        "LayerName",
        "LayerLock",
        "LayerMasking",
        "LayerLayerMaskMipmap",
        "LayerUsePaletteColor",
    ]
    .iter()
    .any(|name| !columns.contains(*name))
    {
        report
            .import_issues
            .push("Required name/lock/mask/palette metadata is unavailable".into());
        return Ok(());
    }
    let properties = rows(
        db,
        "SELECT MainId,LayerName,LayerLock,LayerMasking,LayerLayerMaskMipmap,LayerUsePaletteColor FROM Layer",
        MAX_ENTRIES + 1,
        |row| {
            let name = row.get_ref(1)?.as_str()?;
            if name.len() > 4096 {
                return Err(rusqlite::Error::InvalidQuery);
            }
            Ok((
                row.get::<_, i64>(0)?,
                name.to_owned(),
                row.get::<_, i64>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, i64>(4)?,
                row.get::<_, i64>(5)?,
            ))
        },
    )?;
    // The palette color (`LayerUsePaletteColor`) is the label shown in CSP's
    // Layer palette and does not change the picture; the layer color that
    // does is `DrawColor*`.
    for (id, name, lock, masking, mask, _palette) in properties {
        let study_folder_masking = study
            && id != report.canvas.root
            && masking == 32
            && report
                .layers
                .iter()
                .any(|layer| layer.id == id && layer.is_folder() && layer.supported_kind());
        if study_folder_masking {
            report.rendering_study_omissions.push(format!(
                "Layer {id}: folder LayerMasking=32 without a mask uses the native folder compositor"
            ));
        }
        report.locks.insert(id, lock);
        let tone = report.layers.iter().any(|l| l.id == id && l.is_tone());
        // Layers with a mask on them carry LayerMasking 0 or 1; CSP applied
        // the mask either way in the owner's files (the bit's meaning is
        // unknown, perhaps the mask's link to the layer).
        let correction = report.corrections.contains_key(&id);
        let masked = (tone || report.raster_masks.contains_key(&id))
            && (if correction { [32, 33] } else { [0, 1] }).contains(&masking);
        if (lock != 0 && (!study || lock & !17 != 0))
            || (mask != 0 && !masked)
            || (masking != if id == report.canvas.root { 32 } else { 0 }
                && !study_folder_masking
                && !masked)
        {
            layer_issue(report, study, id, "unsupported lock/mask metadata".into());
        }
        report.document_properties.insert(id, name);
    }
    for field in [
        "FilterLayerInfo",
        "LayerEffectInfo",
        "FilterLayerV132",
        "FilterLayerColorMixingInfo",
        "GradationFillInfo",
        "GradationFillColorMixingInfo",
        "TextLayerString",
        "TextLayerAttributes",
        "VectorNormalStrokeIndex",
        "VectorNormalFillIndex",
        "VectorNormalBalloonIndex",
        "MaterialContentType",
        "UsePreviewColorType",
        "UsePreviewMaskColorType",
        "EffectRangeType",
        "DraftLayer",
        "LayerColorTypeIndex",
        "LightTableInfo",
        "SpecialRulerManager",
        "RulerRange",
        "GuideMove",
        "AnimationFolder",
    ] {
        if !columns.contains(field) {
            continue;
        }
        let values = rows(
            db,
            &format!(
                "SELECT MainId,typeof(\"{field}\"),length(\"{field}\"),CASE WHEN typeof(\"{field}\")='integer' THEN \"{field}\" ELSE 0 END FROM Layer WHERE \"{field}\" IS NOT NULL"
            ),
            MAX_ENTRIES + 1,
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                ))
            },
        )?;
        for (id, kind, length, scalar) in values {
            if (kind == "integer" && scalar != 0) || (kind != "integer" && length != 0) {
                if study
                    && [
                        "LightTableInfo",
                        "SpecialRulerManager",
                        "RulerRange",
                        "GuideMove",
                    ]
                    .contains(&field)
                {
                    report.rendering_study_omissions.push(format!(
                        "Layer {id}: CSP editing guide {field} was not imported"
                    ));
                    report.guide_omissions.entry(field).or_default().insert(id);
                    continue;
                }
                if study && field == "AnimationFolder" {
                    continue;
                }
                if study
                    && field == "VectorNormalStrokeIndex"
                    && report.layers.iter().any(|l| l.id == id && l.is_vector())
                {
                    continue;
                }
                if study
                    && ["GradationFillInfo", "LayerEffectInfo"].contains(&field)
                    && report.layers.iter().any(|l| l.id == id && l.is_tone())
                {
                    continue;
                }
                if study && field == "FilterLayerInfo" && report.corrections.contains_key(&id) {
                    continue;
                }

                // Expression color Gray (1) is stored as gray tiles, which the
                // raster decoder expands; other expression colors stay skipped.
                if study
                    && field == "LayerColorTypeIndex"
                    && kind == "integer"
                    && scalar == 1
                    && report
                        .full_scale
                        .iter()
                        .any(|full| full.layer == id && full.attributes.color_channels() == 1)
                {
                    continue;
                }
                layer_issue(report, study, id, format!("unsupported {field}"));
            }
        }
    }
    // Paper, image material and vector layers carry fixed render-type values
    // (every observed layer of each kind has the same set); ordinary layers
    // none.
    for (
        field,
        paper_expected,
        image_expected,
        vector_expected,
        tone_expected,
        correction_expected,
    ) in [
        (
            "DrawToRenderOffscreenType",
            61,
            Some(20),
            Some(10),
            Some(60),
            None,
        ),
        ("SpecialRenderType", 20, None, None, None, Some(13)),
        ("DrawToRenderMipmapType", 31, Some(20), None, Some(32), None),
        (
            "MoveOffsetAndExpandType",
            31,
            Some(10),
            None,
            Some(30),
            None,
        ),
        ("FixOffsetAndExpandType", 31, Some(10), None, Some(30), None),
        (
            "RenderBoundForLayerMoveType",
            31,
            Some(10),
            None,
            Some(30),
            None,
        ),
        (
            "SetRenderThumbnailInfoType",
            31,
            Some(10),
            None,
            Some(30),
            Some(20),
        ),
        (
            "DrawRenderThumbnailType",
            31,
            None,
            None,
            Some(30),
            Some(20),
        ),
    ] {
        if !columns.contains(field) {
            if report.layers.iter().any(|layer| layer.layer_type == 1584) {
                report
                    .import_issues
                    .push(format!("Paper {field} metadata is unavailable"));
            }
            continue;
        }
        let values = rows(
            db,
            &format!("SELECT MainId,\"{field}\" FROM Layer"),
            MAX_ENTRIES + 1,
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, Option<i64>>(1)?)),
        )?;
        for (id, value) in values {
            let paper = report
                .layers
                .iter()
                .any(|layer| layer.id == id && layer.layer_type == 1584);
            let image = report
                .layers
                .iter()
                .any(|layer| layer.id == id && layer.is_image());
            let vector = report
                .layers
                .iter()
                .any(|layer| layer.id == id && layer.is_vector());
            let tone = report
                .layers
                .iter()
                .any(|layer| layer.id == id && layer.is_tone());
            let correction = report.corrections.contains_key(&id);
            if (paper && value != Some(paper_expected))
                || (image && value.filter(|v| *v != 0) != image_expected)
                || (vector && value.filter(|v| *v != 0) != vector_expected)
                || (tone && value.filter(|v| *v != 0) != tone_expected)
                || (correction && value.filter(|v| *v != 0) != correction_expected)
                || (!paper
                    && !image
                    && !vector
                    && !tone
                    && !correction
                    && value.is_some_and(|value| value != 0))
            {
                layer_issue(report, study, id, format!("unsupported {field}"));
            }
        }
    }
    if columns.contains("MonochromeFillInfo") {
        let mut canonical = Vec::new();
        for value in [62u32, 1, 17] {
            canonical.extend(value.to_be_bytes());
        }
        for byte in b"MonochromeSetting" {
            canonical.extend([0, *byte]);
        }
        for value in [0u32, 0, 0, 1] {
            canonical.extend(value.to_be_bytes());
        }
        let values = rows(
            db,
            "SELECT MainId,CASE WHEN length(MonochromeFillInfo)<=62 THEN MonochromeFillInfo ELSE NULL END,length(MonochromeFillInfo) FROM Layer WHERE MonochromeFillInfo IS NOT NULL",
            MAX_ENTRIES + 1,
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, Option<Vec<u8>>>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            },
        )?;
        let mut checked = BTreeSet::new();
        for (id, data, length) in values {
            let paper = report
                .layers
                .iter()
                .any(|layer| layer.id == id && layer.layer_type == 1584);
            if paper && data.as_deref() == Some(canonical.as_slice()) {
                checked.insert(id);
            } else if length != 0 {
                layer_issue(report, study, id, "unsupported monochrome fill".into());
            }
        }
        for layer in &report.layers {
            if layer.layer_type == 1584 && !checked.contains(&layer.id) {
                report.import_issues.push(format!(
                    "Layer {}: paper fill metadata is unavailable",
                    layer.id
                ));
            }
        }
    } else if report.layers.iter().any(|layer| layer.layer_type == 1584) {
        report
            .import_issues
            .push("Paper fill metadata is unavailable".into());
    }
    if study && columns.contains("DrawColorEnable") {
        let tinted = rows(
            db,
            "SELECT MainId FROM Layer WHERE LayerType!=1584 AND coalesce(DrawColorEnable,0)!=0",
            MAX_ENTRIES + 1,
            |row| row.get::<_, i64>(0),
        )?;
        for id in tinted {
            report.tinted_layers.push(id);
            report.rendering_study_omissions.push(format!(
                "Layer color on “{}” was not applied; its pixels keep their stored colors.",
                report
                    .document_properties
                    .get(&id)
                    .cloned()
                    .unwrap_or_default()
            ));
        }
    }
    let canvas_columns = table_columns(db, "Canvas")?;
    let switch = |field: &str| -> Result<Option<i64>, String> {
        if !canvas_columns.contains(field) {
            return Ok(None);
        }
        db.query_row(
            &format!("SELECT coalesce(\"{field}\",0) FROM Canvas"),
            [],
            |row| row.get(0),
        )
        .map(Some)
        .map_err(db_error)
    };
    // Color proofing and the canvas color adjustment keep their last settings
    // while switched off; only switched-on settings change what CSP shows.
    let simulating = switch("CanvasDoSimulateColor")? != Some(0);
    let adjusting = switch("CanvasUseColorAdjustment")? != Some(0);
    for (field, used) in [
        ("CanvasSrcProfileName", true),
        ("CanvasDstProfileName", true),
        ("CanvasDstProfile", true),
        ("CanvasSimulateSrcProfileName", simulating),
        ("CanvasSimulateDstProfileName", simulating),
        ("CanvasSimulateSrcProfile", simulating),
        ("CanvasSimulateDstProfile", simulating),
        ("CanvasColorAdjustmentToneCurve", adjusting),
        ("CanvasColorAdjustmentLevel", adjusting),
    ] {
        if !canvas_columns.contains(field) || !used {
            continue;
        }
        let length: i64 = db
            .query_row(
                &format!("SELECT coalesce(length(\"{field}\"),0) FROM Canvas"),
                [],
                |row| row.get(0),
            )
            .map_err(db_error)?;
        if length != 0 {
            report
                .import_issues
                .push(format!("Unsupported canvas {field}"));
        }
    }
    for field in ["CanvasDoSimulateColor", "CanvasUseColorAdjustment"] {
        if switch(field)?.is_some_and(|value| value != 0) {
            report
                .import_issues
                .push(format!("Unsupported canvas {field}"));
        }
    }
    if report.canvas.profile_bytes != 0 {
        report
            .import_issues
            .push("Canvas source color profile is unsupported".into());
    }
    if report.unsupported_track_schema || (report.timeline_present && !study) {
        report
            .import_issues
            .push("Animation/timeline content is unsupported by static raster import".into());
    } else if report.timeline_present {
        report.animation = saved_frame(db, &columns)?;
    }
    let flagged: Vec<i64> = report
        .layers
        .iter()
        .filter(|layer| {
            // Layers with a layer mask (tones, and rasters of type 3) also
            // set the 2 bit beside the visible bit.
            !layer.supported_kind()
                || !(if layer.is_tone() || matches!(layer.layer_type, 3 | 4098) {
                    &[2, 3][..]
                } else {
                    &[0, 1, 4, 5]
                })
                .contains(&layer.visibility)
                || (layer.composite != 0 && (!study || clip_blend(layer.composite).is_none()))
                || ![0, 1].contains(&layer.clip)
        })
        .map(|layer| layer.id)
        .collect();
    for id in flagged {
        layer_issue(
            report,
            study,
            id,
            "unsupported type/folder/visibility/blend/clipping metadata".into(),
        );
    }
    Ok(())
}

/// The frame a CSP file was saved on and the cel each animation folder shows
/// there. A folder's track stores that cel name under `ImageCelName`.
fn saved_frame(db: &Connection, layer_columns: &BTreeSet<String>) -> Result<Animation, String> {
    let mut animation = Animation::default();
    let timeline = table_columns(db, "TimeLine")?;
    if timeline.contains("CurrentFrame") {
        let rate = if timeline.contains("FrameRate") {
            "FrameRate"
        } else {
            "NULL"
        };
        if let Some((frame, rate)) = rows(
            db,
            &format!("SELECT CurrentFrame,{rate} FROM TimeLine"),
            1,
            |row| Ok((row.get::<_, f64>(0)?, row.get::<_, Option<f64>>(1)?)),
        )?
        .first()
        {
            animation.current_frame = Some(*frame).filter(|f| f.is_finite() && *f >= 0.0);
            animation.frame_rate = rate.filter(|r| r.is_finite() && *r > 0.0);
        }
    }
    if !layer_columns.contains("AnimationFolder") {
        return Ok(animation);
    }
    let folders = rows(
        db,
        "SELECT MainId FROM Layer WHERE coalesce(AnimationFolder,0)!=0",
        MAX_ENTRIES + 1,
        |row| row.get::<_, i64>(0),
    )?;
    for id in folders {
        animation.folders.insert(id, None);
    }
    let track_columns = table_columns(db, "Track")?;
    if animation.folders.is_empty()
        || !layer_columns.contains("LayerUuid")
        || !["TrackKind", "TrackValueMap", "LayerUuidWithTrack"]
            .iter()
            .all(|c| track_columns.contains(*c))
    {
        return Ok(animation);
    }
    let uuids: BTreeMap<String, i64> = rows(
        db,
        "SELECT MainId,LayerUuid FROM Layer WHERE typeof(LayerUuid)='text' AND length(LayerUuid)<=64",
        MAX_ENTRIES + 1,
        |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
    )?
    .into_iter()
    .map(|(id, uuid)| (uuid.replace('-', "").to_ascii_lowercase(), id))
    .collect();
    let mixer = if track_columns.contains("TrackActionMixer") {
        "CASE WHEN typeof(TrackActionMixer) IN ('text','blob') AND length(TrackActionMixer)<=64 THEN CAST(TrackActionMixer AS TEXT) END"
    } else {
        "NULL"
    };
    let tracks = rows(
        db,
        &format!(
            "SELECT TrackValueMap,LayerUuidWithTrack,{mixer} FROM Track WHERE TrackKind=2000 AND length(TrackValueMap)<=4096 AND length(LayerUuidWithTrack)=16"
        ),
        MAX_ENTRIES + 1,
        |row| {
            Ok((
                row.get::<_, Vec<u8>>(0)?,
                row.get::<_, Vec<u8>>(1)?,
                row.get::<_, Option<String>>(2)?,
            ))
        },
    )?;
    for (values, uuid, mixer) in tracks {
        let hex: String = uuid.iter().map(|b| format!("{b:02x}")).collect();
        if let Some(id) = uuids.get(&hex)
            && let Some(cel) = animation.folders.get_mut(id)
        {
            *cel = cel_name(&values);
            match mixer {
                Some(mixer) => {
                    animation.mixers.insert(*id, mixer);
                }
                None => {
                    animation.uncertain.insert(*id);
                }
            }
        }
    }
    Ok(animation)
}
/// Replace each animation folder's stored cel with the one its keyframes
/// show on the saved frame. Unreadable keyframes keep the stored cel.
fn resolve_keyframes(
    source: &mut File,
    inventory: &clip_container::Inventory,
    external: &[(String, u64)],
    animation: &mut Animation,
) {
    let (Some(frame), Some(frame_rate)) = (animation.current_frame, animation.frame_rate) else {
        animation.uncertain.extend(animation.mixers.keys());
        return;
    };
    for (folder, mixer) in &animation.mixers {
        let cel = (|| -> Result<Option<String>, String> {
            let (_, chunk) = external
                .iter()
                .find(|(id, _)| id == mixer)
                .ok_or("Missing track mixer chunk")?;
            let (offset, bytes) = inventory.external_body_bounds(*chunk, mixer.len())?;
            if bytes > 1024 * 1024 {
                return Err("Track mixer exceeds limits".into());
            }
            let mut body = vec![0; bytes as usize];
            source
                .seek(SeekFrom::Start(offset))
                .and_then(|_| source.read_exact(&mut body))
                .map_err(|e| e.to_string())?;
            let keys = crate::clip_timeline::keys(&body)?;
            Ok(keys.cel_at(frame, frame_rate).map(String::from))
        })();
        match cel {
            Ok(cel) => {
                animation.folders.insert(*folder, cel);
            }
            Err(_) => {
                animation.uncertain.insert(*folder);
            }
        }
    }
}
/// `ImageCelName` from a track value map: big-endian header size and entry
/// count, then entries of (byte length, UTF-16BE key, UTF-16BE value, ...).
fn cel_name(map: &[u8]) -> Option<String> {
    let u32_at = |at: usize| -> Option<usize> {
        Some(u32::from_be_bytes(map.get(at..at.checked_add(4)?)?.try_into().ok()?) as usize)
    };
    let text_at = |at: usize| -> Option<(String, usize)> {
        let start = at.checked_add(4)?;
        let end = start.checked_add(u32_at(at)?.checked_mul(2)?)?;
        let units: Vec<u16> = map
            .get(start..end)?
            .chunks_exact(2)
            .map(|c| u16::from_be_bytes([c[0], c[1]]))
            .collect();
        Some((String::from_utf16(&units).ok()?, end))
    };
    let mut at = u32_at(0)?;
    for _ in 0..u32_at(4)?.min(64) {
        let length = u32_at(at)?;
        if length < 8 || at.checked_add(length)? > map.len() {
            return None;
        }
        let (key, next) = text_at(at + 4)?;
        if key == "ImageCelName" {
            return text_at(next).map(|(value, _)| value);
        }
        at += length;
    }
    None
}

fn configure(db: &Connection) -> Result<(), String> {
    for (limit, value) in [
        (Limit::SQLITE_LIMIT_LENGTH, MAX_SQL as i32),
        (Limit::SQLITE_LIMIT_SQL_LENGTH, 16_384),
        // Real samples contain a 68-column Project table even though the fixed
        // queries never read it. This schema ceiling is separate from row caps.
        (Limit::SQLITE_LIMIT_COLUMN, 128),
        (Limit::SQLITE_LIMIT_EXPR_DEPTH, 32),
        (Limit::SQLITE_LIMIT_ATTACHED, 0),
        (Limit::SQLITE_LIMIT_WORKER_THREADS, 0),
    ] {
        db.set_limit(limit, value).map_err(db_error)?;
    }
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut calls = 0;
    db.progress_handler(
        1000,
        Some(move || {
            calls += 1;
            calls > 1000 || Instant::now() >= deadline
        }),
    )
    .map_err(db_error)?;
    db.execute_batch("PRAGMA query_only=ON; PRAGMA trusted_schema=OFF; PRAGMA cache_size=-2048; PRAGMA temp_store=MEMORY;").map_err(db_error)
}

struct Snapshot {
    // Drop the SQLite connection before removing its bounded temporary metadata file.
    db: Connection,
    _file: tempfile::NamedTempFile,
}
impl std::ops::Deref for Snapshot {
    type Target = Connection;
    fn deref(&self) -> &Connection {
        &self.db
    }
}

fn open_snapshot(read: impl Read, bytes: usize) -> Result<Snapshot, String> {
    if bytes == 0 || bytes > MAX_SQL {
        return Err("Clip SQLite snapshot exceeds inspection bounds".into());
    }
    // SQLite advises extra padding when deserializing untrusted bytes; the safe
    // stream API allocates exactly the input size. Use the ordinary file pager
    // on an isolated, bounded metadata copy instead. Never open the .clip as SQL.
    let mut file = tempfile::NamedTempFile::new().map_err(|error| error.to_string())?;
    let copied = std::io::copy(&mut read.take(bytes as u64), &mut file)
        .map_err(|error| error.to_string())?;
    if copied != bytes as u64 {
        return Err("Truncated clip SQLite snapshot".into());
    }
    let db = Connection::open_with_flags(
        file.path(),
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(db_error)?;
    configure(&db)?;
    Ok(Snapshot { db, _file: file })
}

fn rows<T>(
    db: &Connection,
    sql: &str,
    max: usize,
    map: impl FnMut(&Row<'_>) -> rusqlite::Result<T>,
) -> Result<Vec<T>, String> {
    // LIMIT also bounds stepping when a hostile table contains many rows.
    let mut statement = db
        .prepare(&format!("{sql} LIMIT {}", max + 1))
        .map_err(db_error)?;
    let data = statement
        .query_map([], map)
        .map_err(db_error)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(db_error)?;
    if data.len() > max {
        return Err("Clip metadata row count exceeds inspection bounds".into());
    }
    Ok(data)
}

fn ordinary_tables(db: &Connection) -> Result<(), String> {
    for name in [
        "Canvas",
        "Layer",
        "Mipmap",
        "MipmapInfo",
        "Offscreen",
        "ExternalChunk",
    ] {
        let (kind, prefix): (String, String) = db
            .query_row(
                "SELECT type, substr(sql,1,128) FROM sqlite_schema WHERE name=?1",
                [name],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .map_err(db_error)?;
        let prefix = prefix
            .split_whitespace()
            .take(2)
            .collect::<Vec<_>>()
            .join(" ")
            .to_ascii_uppercase();
        if kind != "table" || prefix != "CREATE TABLE" {
            return Err(format!("Clip metadata requires an ordinary {name} table"));
        }
    }
    Ok(())
}

fn layer_order(canvas: &Canvas, layers: &[Layer]) -> Result<Vec<Entry>, String> {
    let mut by_id = BTreeMap::new();
    for layer in layers {
        if layer.id <= 0 || by_id.insert(layer.id, layer).is_some() {
            return Err("Invalid or duplicate Layer.MainId".into());
        }
        if ![0, 1, 17].contains(&layer.folder)
            || (layer.folder == 17 && layer.layer_type != 0)
            || layer.next < 0
            || layer.child < 0
            || layer.mipmap < 0
            || (!layer.is_folder() && layer.mipmap == 0)
            || !(0..=256).contains(&layer.opacity)
            || (!layer.is_folder() && layer.child != 0)
        {
            return Err("Unsupported layer graph fields".into());
        }
    }
    let root = by_id.get(&canvas.root).ok_or("Missing root Layer.MainId")?;
    if root.folder != 1 || root.next != 0 || !by_id.contains_key(&canvas.current) {
        return Err("Invalid root/current layer reference".into());
    }
    let mut visited = BTreeSet::from([canvas.root]);
    let mut pending = vec![(root.child, 0u8)];
    let mut order = Vec::new();
    while let Some((id, depth)) = pending.pop() {
        if id == 0 {
            continue;
        }
        if depth > MAX_DEPTH {
            return Err("Clip folder graph exceeds Dudel depth limit".into());
        }
        if !visited.insert(id) {
            return Err("Cyclic or multiply referenced clip layer graph".into());
        }
        let layer = by_id.get(&id).ok_or("Dangling Layer.MainId reference")?;
        order.push(Entry { id, depth });
        pending.push((layer.next, depth));
        if layer.child != 0 {
            pending.push((layer.child, depth + 1));
        }
    }
    if visited.len() != layers.len() {
        return Err("Unreachable clip layer rows".into());
    }
    if layers
        .iter()
        .filter(|layer| layer.folder == 0 && matches!(layer.layer_type, 1 | 3))
        .count()
        > MAX_RASTER_LAYERS
    {
        return Err("Clip raster count exceeds Dudel layer limit".into());
    }
    Ok(order)
}

fn metadata(db: &Connection, actual_external: &[(String, u64)]) -> Result<Report, String> {
    ordinary_tables(db)?;
    let mut canvases = rows(
        db,
        "SELECT MainId,CanvasWidth,CanvasHeight,CanvasChannelBytes,CanvasDefaultChannelOrder,CanvasRootFolder,CanvasCurrentLayer,coalesce(length(CanvasSrcProfile),0),CanvasUnit,CanvasResolution FROM Canvas",
        1,
        |r| {
            Ok(Canvas {
                id: r.get(0)?,
                width: r.get(1)?,
                height: r.get(2)?,
                channel_bytes: r.get(3)?,
                channel_order: r.get(4)?,
                root: r.get(5)?,
                current: r.get(6)?,
                profile_bytes: r.get(7)?,
                unit: r.get(8)?,
                resolution: r.get(9)?,
            })
        },
    )?;
    let mut canvas = canvases.pop().ok_or("Missing clip canvas")?;
    if canvas.unit != 0 {
        // Unit 3 is inches: 2.5 x 3.5 at 1200 dpi is CSP's 3000 x 4200 px
        // canvas bitmap in every observed file. Unit 2 is millimetres,
        // rounded to the nearest pixel: 257 x 364 mm at 350 dpi is CSP's
        // 3541 x 5016 px bitmap (3541.3 x 5015.7). Other units are
        // unverified.
        if ![2, 3].contains(&canvas.unit)
            || !canvas.resolution.is_finite()
            || canvas.resolution <= 0.
        {
            return Err(format!(
                "Unsupported clip canvas unit {} (pixels, millimetres and inches are supported)",
                canvas.unit
            ));
        }
        let millimetres = canvas.unit == 2;
        let pixels = |size: f64| {
            let value = if millimetres {
                size / 25.4 * canvas.resolution
            } else {
                size * canvas.resolution
            };
            (millimetres || (value - value.round()).abs() < 0.01).then_some(value.round())
        };
        let (Some(width), Some(height)) = (pixels(canvas.width), pixels(canvas.height)) else {
            return Err("Clip canvas size in inches is not a whole number of pixels".into());
        };
        (canvas.width, canvas.height) = (width, height);
    }
    if canvas.width.is_finite()
        && canvas.height.is_finite()
        && (canvas.width > crate::document::MAX_SIDE as f64
            || canvas.height > crate::document::MAX_SIDE as f64
            || canvas.width * canvas.height > crate::document::MAX_PIXELS as f64)
    {
        return Err(format!(
            "This canvas is {} × {} px; Dudel canvases go {}",
            canvas.width,
            canvas.height,
            crate::document::SIZE_LIMIT
        ));
    }
    if canvas.id <= 0
        || !canvas.resolution.is_finite()
        || canvas.resolution <= 0.
        || !canvas.width.is_finite()
        || !canvas.height.is_finite()
        || canvas.width.fract() != 0.
        || canvas.height.fract() != 0.
        || !(1. ..=crate::document::MAX_SIDE as f64).contains(&canvas.width)
        || !(1. ..=crate::document::MAX_SIDE as f64).contains(&canvas.height)
        || canvas.width * canvas.height > crate::document::MAX_PIXELS as f64
    {
        return Err("Clip canvas exceeds Dudel dimensions or pixel limit".into());
    }
    let mut layers = rows(
        db,
        "SELECT MainId,LayerType,LayerFolder,LayerVisibility,LayerOpacity,LayerComposite,LayerClip,LayerNextIndex,LayerFirstChildIndex,LayerRenderMipmap,LayerOffsetX,LayerOffsetY,LayerRenderOffscrOffsetX,LayerRenderOffscrOffsetY FROM Layer",
        MAX_ENTRIES + 1,
        |r| {
            Ok(Layer {
                id: r.get(0)?,
                layer_type: r.get(1)?,
                folder: r.get(2)?,
                visibility: r.get(3)?,
                opacity: r.get(4)?,
                composite: r.get(5)?,
                clip: r.get(6)?,
                next: r.get(7)?,
                child: r.get(8)?,
                mipmap: r.get(9)?,
                offset_x: r.get(10)?,
                offset_y: r.get(11)?,
                render_offscreen_offset_x: r.get(12)?,
                render_offscreen_offset_y: r.get(13)?,
                paper_draw_color: None,
                image: false,
                vector: false,
                tone: false,
                correction: false,
            })
        },
    )?;
    let order = layer_order(&canvas, &layers)?;
    let mut externals = BTreeMap::new();
    let mut offsets = BTreeSet::new();
    for (id, offset) in actual_external {
        if externals.insert(id.clone(), *offset).is_some() || !offsets.insert(*offset) {
            return Err("Duplicate external chunk identity/offset".into());
        }
    }
    let external_rows = rows(
        db,
        "SELECT ExternalID,Offset FROM ExternalChunk",
        4096,
        |r| {
            // The observed BLOB-affinity column stores TEXT; accept exactly
            // TEXT/BLOB bytes, never numeric coercions or NULL.
            let bytes = r.get_ref(0)?.as_bytes()?;
            if bytes.len() > 1024 {
                return Err(rusqlite::Error::InvalidQuery);
            }
            Ok((
                String::from_utf8(bytes.to_vec()).map_err(|_| rusqlite::Error::InvalidQuery)?,
                r.get::<_, i64>(1)?,
            ))
        },
    )?;
    let mut indexed_ids = BTreeSet::new();
    for (id, offset) in external_rows {
        if offset < 0 || externals.get(&id) != Some(&(offset as u64)) || !indexed_ids.insert(id) {
            return Err(
                "ExternalChunk does not match a unique framed external identifier/header offset"
                    .into(),
            );
        }
    }
    // An Offscreen reference is resolved only through the verified SQL index.
    externals.retain(|id, _| indexed_ids.contains(id));
    let mips = rows(
        db,
        "SELECT MainId,CanvasId,LayerId,MipmapCount,BaseMipmapInfo FROM Mipmap",
        MAX_ENTRIES + 1,
        |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, i64>(3)?,
                r.get::<_, i64>(4)?,
            ))
        },
    )?;
    let infos = rows(
        db,
        "SELECT MainId,LayerId,ThisScale,Offscreen,NextIndex FROM MipmapInfo",
        MAX_MIPS,
        |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, f64>(2)?,
                r.get::<_, i64>(3)?,
                r.get::<_, i64>(4)?,
            ))
        },
    )?;
    let offscreens = rows(
        db,
        "SELECT MainId,CanvasId,LayerId,Attribute,BlockData FROM Offscreen",
        MAX_MIPS * 2,
        |r| {
            let attr = r.get_ref(3)?.as_blob()?;
            let block = r.get_ref(4)?.as_blob()?;
            if attr.len() > crate::clip_attributes::MAX_BYTES || block.len() > 1024 {
                return Err(rusqlite::Error::InvalidQuery);
            }
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, i64>(2)?,
                attr.to_vec(),
                block.to_vec(),
            ))
        },
    )?;
    let mut mip_by_id = BTreeMap::new();
    for mip in &mips {
        if mip.0 <= 0 || mip_by_id.insert(mip.0, mip).is_some() {
            return Err("Invalid/duplicate Mipmap.MainId".into());
        }
    }
    let mut info_by_id = BTreeMap::new();
    for info in &infos {
        if info.0 <= 0 || info_by_id.insert(info.0, info).is_some() {
            return Err("Invalid/duplicate MipmapInfo.MainId".into());
        }
    }
    let mut off_by_id = BTreeMap::new();
    for off in &offscreens {
        if off.0 <= 0 || off_by_id.insert(off.0, off).is_some() {
            return Err("Invalid/duplicate Offscreen.MainId".into());
        }
    }
    let mut full_scale = Vec::new();
    let mut diagnostics = vec!["Research inspection only: layer order direction, visibility bits, special layer types, profiles, masks/effects, block checksums and pixel/channel semantics are not validated. No import-ready status is implied.".into()];
    let mut timeline_present = false;
    let mut unsupported_track_schema = false;
    let tracks = rows(
        db,
        "SELECT type,substr(sql,1,128) FROM sqlite_schema WHERE name='Track'",
        1,
        |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
    )?;
    if let Some((kind, sql)) = tracks.first() {
        let prefix = sql
            .split_whitespace()
            .take(2)
            .collect::<Vec<_>>()
            .join(" ")
            .to_ascii_uppercase();
        if kind != "table" || prefix != "CREATE TABLE" {
            unsupported_track_schema = true;
            diagnostics.push(
                "Track metadata uses an unsupported schema; timeline support is unverified".into(),
            );
        } else if db
            .query_row("SELECT EXISTS(SELECT 1 FROM Track LIMIT 1)", [], |row| {
                row.get::<_, bool>(0)
            })
            .map_err(db_error)?
        {
            timeline_present = true;
            diagnostics.push("Timeline/track content is present; static raster import must not silently discard animation".into());
        }
    }
    let paper_columns: i64 = db.query_row("SELECT count(*) FROM pragma_table_info('Layer') WHERE name IN ('DrawColorEnable','DrawColorMainRed','DrawColorMainGreen','DrawColorMainBlue')",[],|row| row.get(0)).map_err(db_error)?;
    if paper_columns == 4 {
        let colors = rows(
            db,
            "SELECT MainId,DrawColorEnable,DrawColorMainRed,DrawColorMainGreen,DrawColorMainBlue FROM Layer WHERE LayerType=1584",
            MAX_ENTRIES,
            |row| {
                let raw_rgb = [
                    row.get::<_, u32>(2)?,
                    row.get::<_, u32>(3)?,
                    row.get::<_, u32>(4)?,
                ];
                Ok((
                    row.get::<_, i64>(0)?,
                    PaperDrawColor {
                        enabled: row.get(1)?,
                        raw_rgb,
                        high_byte_rgb: raw_rgb.map(|word| (word >> 24) as u8),
                    },
                ))
            },
        )?;
        for (id, color) in colors {
            let layer = layers
                .iter_mut()
                .find(|layer| layer.id == id)
                .ok_or("Unknown paper draw-color owner")?;
            layer.paper_draw_color = Some(color);
        }
    } else if layers.iter().any(|layer| layer.layer_type == 1584) {
        diagnostics.push(
            "Paper draw-color columns are absent/incomplete; paper rendering remains unsupported"
                .into(),
        );
    }
    if canvas.channel_bytes != 1 || canvas.channel_order != 33 || canvas.profile_bytes != 0 {
        diagnostics.push("Canvas channel/profile metadata differs from the observed sample".into());
    }
    let mut used_mips = BTreeSet::new();
    let mut used_infos = BTreeSet::new();
    for layer in &layers {
        if layer.render_offscreen_offset_x != 0 || layer.render_offscreen_offset_y != 0 {
            diagnostics.push(format!("Layer {} has separate render-offscreen offsets; placement requires both offset sources and controlled fixtures", layer.id));
        }
        if ![1, 256].contains(&layer.layer_type)
            || ![0, 1].contains(&layer.visibility)
            || layer.composite != 0
            || layer.clip != 0
        {
            diagnostics.push(format!(
                "Layer {} has uninterpreted type/visibility/blend/clipping fields",
                layer.id
            ));
        }
        if layer.is_folder() && layer.mipmap == 0 {
            continue;
        }
        let mip = mip_by_id
            .get(&layer.mipmap)
            .ok_or("Dangling LayerRenderMipmap MainId")?;
        if mip.1 != canvas.id
            || mip.2 != layer.id
            || !(1..=16).contains(&mip.3)
            || !used_mips.insert(mip.0)
        {
            return Err("Invalid mipmap owner/count/reference".into());
        }
        let mut id = mip.4;
        let mut previous_scale = f64::INFINITY;
        for index in 0..mip.3 {
            if !used_infos.insert(id) {
                return Err("Cyclic/shared mipmap chain".into());
            }
            let info = info_by_id.get(&id).ok_or("Dangling MipmapInfo.MainId")?;
            if info.1 != layer.id
                || !info.2.is_finite()
                || info.2 <= 0.
                || info.2 >= previous_scale
                || (index == 0 && info.2 != 100.)
            {
                return Err("Invalid mipmap owner/scale (expected 100 percent base)".into());
            }
            let off = off_by_id.get(&info.3).ok_or("Dangling Offscreen.MainId")?;
            if off.1 != canvas.id || off.2 != layer.id {
                return Err("Invalid offscreen owner".into());
            }
            if index == 0 {
                let attributes = crate::clip_attributes::parse(&off.3)
                    .map_err(|error| format!("Layer {}: {error}", layer.id))?;
                let external_id = std::str::from_utf8(&off.4)
                    .ok()
                    .filter(|id| {
                        id.len() == 40
                            && id.starts_with("extrnlid")
                            && id[8..].bytes().all(|byte| byte.is_ascii_hexdigit())
                    })
                    .map(str::to_owned);
                let offset = external_id
                    .as_ref()
                    .and_then(|id| externals.get(id))
                    .copied();
                if offset.is_none() {
                    diagnostics.push(format!("Layer {} full-scale block reference is unresolved; it must not be assumed blank", layer.id));
                }
                full_scale.push(FullScale {
                    layer: layer.id,
                    mipmap: mip.0,
                    info: info.0,
                    offscreen: off.0,
                    attribute_bytes: off.3.len(),
                    attributes,
                    block_layout_validated: false,
                    external_id,
                    external_chunk_offset: offset,
                });
            }
            previous_scale = info.2;
            id = info.4;
        }
        if id != 0 {
            return Err("Mipmap chain length disagrees with MipmapCount".into());
        }
    }
    if used_mips.len() != mips.len() || used_infos.len() != infos.len() {
        diagnostics.push("Unreferenced mipmap rows remain uninterpreted".into());
    }
    Ok(Report {
        scope: "Bounded read-only metadata, offscreen attributes and referenced block layout/checksums. No color conversion or artwork import.",
        canvas,
        layers,
        root_child_next_order: order,
        full_scale,
        external_chunks: actual_external.len(),
        diagnostics,
        timeline_present,
        unsupported_track_schema,
        import_issues: Vec::new(),
        rendering_study_omissions: Vec::new(),
        guide_omissions: BTreeMap::new(),
        tinted_layers: Vec::new(),
        document_properties: BTreeMap::new(),
        locks: BTreeMap::new(),
        layer_issues: BTreeMap::new(),
        images: BTreeMap::new(),
        vectors: BTreeMap::new(),
        tones: BTreeMap::new(),
        raster_masks: BTreeMap::new(),
        corrections: BTreeMap::new(),
        plain_brush_styles: BTreeMap::new(),
        animation: Animation::default(),
    })
}

fn read_source(input: &Path) -> Result<(File, clip_container::Inventory, Report), String> {
    read_source_mode(input, false)
}
fn read_source_mode(
    input: &Path,
    study: bool,
) -> Result<(File, clip_container::Inventory, Report), String> {
    let mut source = File::open(input).map_err(|error| error.to_string())?;
    let length = source.metadata().map_err(|error| error.to_string())?.len();
    let inventory = clip_container::scan(&mut source, length)?;
    let external = inventory.external_ids(&mut source)?;
    let (offset, bytes) = inventory.selected_sql_payload();
    source
        .seek(SeekFrom::Start(offset))
        .map_err(|error| error.to_string())?;
    let db = open_snapshot((&mut source).take(bytes), bytes as usize)?;
    let mut report = metadata(&db, &external)?;
    if study {
        document_features_mode(&db, &mut report, true)?;
        resolve_keyframes(&mut source, &inventory, &external, &mut report.animation);
    } else {
        document_features(&db, &mut report)?;
    }
    Ok((source, inventory, report))
}
pub(crate) fn inspect(input: &Path, output: Option<&Path>) -> Result<(), String> {
    let (mut source, inventory, mut report) = read_source(input)?;
    let mut budget = crate::clip_blocks::Budget::default();
    for full in &mut report.full_scale {
        if let (Some(id), Some(chunk)) = (&full.external_id, full.external_chunk_offset) {
            let (offset, size) = inventory.external_body_bounds(chunk, id.len())?;
            source
                .seek(SeekFrom::Start(offset))
                .map_err(|error| error.to_string())?;
            crate::clip_blocks::validate_layout(
                &mut source,
                size,
                &full.attributes.block_sizes,
                full.attributes.tile_bytes(),
                &mut budget,
            )
            .map_err(|error| format!("Layer {}: {error}", full.layer))?;
            full.block_layout_validated = true;
        }
    }
    clip_container::write_report(&report, output)
}

pub(crate) fn verify_rasters(input: &Path, output: Option<&Path>) -> Result<(), String> {
    #[derive(serde::Serialize)]
    struct Raster {
        layer: i64,
        statistics: crate::clip_raster::Statistics,
    }
    #[derive(serde::Serialize)]
    struct Verification {
        scope: &'static str,
        canvas: [usize; 2],
        rasters: Vec<Raster>,
        non_raster_layers: Vec<[i64; 2]>,
        diagnostics: Vec<String>,
    }
    let (mut source, inventory, report) = read_source(input)?;
    let canvas = [report.canvas.width as usize, report.canvas.height as usize];
    let mut budget = crate::clip_blocks::Budget::default();
    let mut rasters = Vec::new();
    let mut non_raster_layers = Vec::new();
    for layer in &report.layers {
        if layer.layer_type != 1 || layer.folder != 0 {
            non_raster_layers.push([layer.id, layer.layer_type]);
            continue;
        }
        let full = report
            .full_scale
            .iter()
            .find(|full| full.layer == layer.id)
            .ok_or("Missing full-scale raster descriptor")?;
        let (Some(id), Some(chunk)) = (&full.external_id, full.external_chunk_offset) else {
            return Err(format!(
                "Layer {}: unresolved raster data is not assumed blank",
                layer.id
            ));
        };
        let (offset, bytes) = inventory.external_body_bounds(chunk, id.len())?;
        source
            .seek(SeekFrom::Start(offset))
            .map_err(|error| error.to_string())?;
        let pixels = crate::clip_raster::decode(
            &mut source,
            bytes,
            &full.attributes,
            crate::clip_raster::Placement {
                canvas,
                layer_offset: [layer.offset_x, layer.offset_y],
                render_offset: [
                    layer.render_offscreen_offset_x,
                    layer.render_offscreen_offset_y,
                ],
            },
            &mut budget,
            &|| false,
        )
        .map_err(|error| format!("Layer {}: {error}", layer.id))?;
        rasters.push(Raster {
            layer: layer.id,
            statistics: crate::clip_raster::statistics(&pixels),
        });
    }
    clip_container::write_report(
        &Verification {
            scope: "Sparse RGBA conversion of resolved type-1 raster leaves only. No masks/effects, color profiles, composite, special layers, animation, document import or import-ready status.",
            canvas,
            rasters,
            non_raster_layers,
            diagnostics: report.diagnostics,
        },
        output,
    )
}

pub(crate) fn prepare_document(
    input: &Path,
    cancelled: &impl Fn() -> bool,
) -> Result<crate::document::Document, String> {
    prepare_document_mode(input, cancelled, true).map(|(doc, ..)| doc)
}

/// Native equivalents of the CSP blend identifiers, as listed by the
/// independent clip_to_psd reader (10 Glow dodge, 12 Add (glow), 30 Through).
fn clip_blend(value: i64) -> Option<crate::blend::BlendMode> {
    use crate::blend::BlendMode::*;
    Some(match value {
        0 => Normal,
        1 => Darken,
        2 => Multiply,
        3 => ColorBurn,
        4 => LinearBurn,
        5 => Subtract,
        6 => DarkerColor,
        7 => Lighten,
        8 => Screen,
        9 => ColorDodge,
        10 => GlowDodge,
        11 => LinearDodge,
        12 => AddGlow,
        13 => LighterColor,
        14 => Overlay,
        15 => SoftLight,
        16 => HardLight,
        17 => VividLight,
        18 => LinearLight,
        19 => PinLight,
        20 => HardMix,
        21 => Difference,
        22 => Exclusion,
        23 => Hue,
        24 => Saturation,
        25 => Color,
        26 => Luminosity,
        30 => PassThrough,
        36 => Divide,
        _ => return None,
    })
}

/// “A”, “B” and “C”, or the first few and how many more.
pub(crate) fn name_list(names: &[String]) -> String {
    const SHOWN: usize = 5;
    let quoted: Vec<String> = names.iter().take(SHOWN).map(|n| format!("“{n}”")).collect();
    match names.len() {
        0 => String::new(),
        1 => quoted[0].clone(),
        n if n <= SHOWN => format!("{} and {}", quoted[..n - 1].join(", "), quoted[n - 1]),
        n => format!("{} and {} more", quoted.join(", "), n - SHOWN),
    }
}

pub(crate) fn import_document(
    input: &Path,
    cancelled: &impl Fn() -> bool,
) -> Result<crate::import_report::Imported, String> {
    let (document, notes, mut details) = prepare_document_mode(input, cancelled, true)?;
    details.insert(0, "Dudel draws folders, clipping and blend modes itself, so small differences from Clip Studio Paint are possible.".into());
    Ok(crate::import_report::Imported {
        document,
        notes,
        details,
    })
}

pub(crate) fn import_preview(
    input: &Path,
    cancelled: &impl Fn() -> bool,
) -> Result<crate::import_report::Imported, String> {
    use image::ImageDecoder;
    if cancelled() {
        return Err("Import canceled".into());
    }
    let mut file = File::open(input).map_err(|e| e.to_string())?;
    let size = file.metadata().map_err(|e| e.to_string())?.len();
    let inventory = clip_container::scan(&mut file, size)?;
    let (offset, size) = inventory.selected_sql_payload();
    file.seek(SeekFrom::Start(offset))
        .map_err(|e| e.to_string())?;
    let db = open_snapshot(file.take(size), size as usize)?;
    ordinary_tables(&db)?;
    let previews = rows(
        &db,
        "SELECT ImageType,ImageWidth,ImageHeight,ImageData FROM CanvasPreview",
        1,
        |r| {
            let data = r.get_ref(3)?.as_blob()?;
            if data.len() > 64 * 1024 * 1024 {
                return Err(rusqlite::Error::InvalidQuery);
            }
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, u32>(1)?,
                r.get::<_, u32>(2)?,
                data.to_vec(),
            ))
        },
    )?;
    let (kind, width, height, data) = previews.into_iter().next().ok_or("No saved CSP preview")?;
    if kind != 1 || width == 0 || height == 0 || width > 4096 || height > 4096 {
        return Err("Unsupported saved CSP preview dimensions or encoding".into());
    }
    let mut reader =
        image::ImageReader::with_format(std::io::Cursor::new(data), image::ImageFormat::Png);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(4096);
    limits.max_image_height = Some(4096);
    limits.max_alloc = Some(128 * 1024 * 1024);
    reader.limits(limits);
    let decoder = reader.into_decoder().map_err(|e| e.to_string())?;
    if decoder.dimensions() != (width, height) {
        return Err("CSP preview header differs from metadata".into());
    }
    let pixels = image::DynamicImage::from_decoder(decoder)
        .map_err(|e| e.to_string())?
        .into_rgba8();
    if cancelled() {
        return Err("Import canceled".into());
    }
    let mut document = crate::document::Document::new(width as usize, height as usize);
    document.dpi = crate::document::default_dpi();
    // Preserve physical size when a preview is smaller than the canvas.
    // This fallback must still open files whose layer graph is unsupported.
    let geometry = db.query_row(
        "SELECT CanvasWidth,CanvasResolution,CanvasUnit FROM Canvas LIMIT 1",
        [],
        |r| {
            Ok((
                r.get::<_, f64>(0)?,
                r.get::<_, f64>(1)?,
                r.get::<_, i64>(2)?,
            ))
        },
    );
    if let Ok((source_width, resolution, unit)) = geometry {
        let pixels = match unit {
            0 => source_width,
            3 => source_width * resolution,
            _ => f64::NAN,
        };
        let dpi = resolution * f64::from(width) / pixels;
        if pixels.is_finite() && pixels > 0.0 && crate::document::validate_dpi(dpi).is_ok() {
            document.dpi = dpi;
        }
    }
    document.layers[0].name = "CSP saved preview (flattened)".into();
    document.layers[0].pixels = pixels.into_raw().into();
    document.validate()?;
    Ok(crate::import_report::Imported {
        document,
        notes: vec![format!(
            "Opened the saved preview at {width} × {height} px. This may be reduced resolution. Layers, masks and animation were flattened by CSP when it saved this preview; they are not editable here."
        )],
        details: Vec::new(),
    })
}
fn prepare_document_mode(
    input: &Path,
    cancelled: &impl Fn() -> bool,
    study: bool,
) -> Result<(crate::document::Document, Vec<String>, Vec<String>), String> {
    if cancelled() {
        return Err("Clip document assembly canceled".into());
    }
    let (mut source, inventory, mut report) = read_source_mode(input, study)?;
    // Strokes are read before planning, so a vector layer Dudel cannot draw
    // is skipped and named like any other unsupported layer.
    let mut vectors = BTreeMap::new();
    let mut failed = Vec::new();
    for (&id, data) in &report.vectors {
        match read_vector(&mut source, &inventory, data, &report.plain_brush_styles) {
            Ok(strokes) => {
                vectors.insert(id, strokes);
            }
            Err(error) => failed.push((id, error)),
        }
    }
    for (id, error) in failed {
        report
            .layer_issues
            .entry(id)
            .or_default()
            .push(format!("unsupported vector strokes ({error})"));
    }
    let canvas_size = [report.canvas.width as usize, report.canvas.height as usize];
    let mut masks = BTreeMap::new();
    let mut failed = Vec::new();
    let mut mask_budget = crate::clip_blocks::Budget::default();
    let sources = report
        .tones
        .iter()
        .map(|(id, (_, mask))| (*id, mask))
        .chain(report.raster_masks.iter().map(|(id, mask)| (*id, mask)));
    for (id, mask) in sources {
        let layer = report.layers.iter().find(|layer| layer.id == id).unwrap();
        let read = inventory
            .external_body_bounds(mask.data.offset, mask.data.id.len())
            .and_then(|(offset, bytes)| {
                source
                    .seek(SeekFrom::Start(offset))
                    .map_err(|error| error.to_string())?;
                crate::clip_raster::decode_mask(
                    &mut source,
                    bytes,
                    &mask.attributes,
                    crate::clip_raster::Placement {
                        canvas: canvas_size,
                        layer_offset: [layer.offset_x, layer.offset_y],
                        render_offset: [
                            layer.render_offscreen_offset_x,
                            layer.render_offscreen_offset_y,
                        ],
                    },
                    &mut mask_budget,
                    cancelled,
                )
            });
        match read {
            Ok(coverage) => {
                masks.insert(id, coverage);
            }
            Err(error) => failed.push((id, error)),
        }
    }
    for (id, error) in failed {
        report
            .layer_issues
            .entry(id)
            .or_default()
            .push(format!("unsupported layer mask ({error})"));
    }
    let root = report
        .layers
        .iter()
        .find(|layer| layer.id == report.canvas.root)
        .unwrap();
    let mut issues = report.import_issues;
    if !study
        && report
            .layers
            .iter()
            .any(|layer| (layer.id != report.canvas.root && layer.is_folder()) || layer.clip != 0)
    {
        issues.push("Folder/clipping import awaits controlled CSP rendering validation".into());
    }
    if root.opacity != 256
        || root.visibility != 1
        || root.composite != 0
        || root.clip != 0
        || [
            root.offset_x,
            root.offset_y,
            root.render_offscreen_offset_x,
            root.render_offscreen_offset_y,
        ] != [0; 4]
    {
        issues.push("Root container has unsupported rendering properties".into());
    }
    if let Some(reasons) = report.layer_issues.get(&report.canvas.root) {
        issues.push(format!("Root container: {}", reasons.join(", ")));
    }
    if !issues.is_empty() {
        return Err(format!("Unsupported clip content: {}", issues.join("; ")));
    }
    // The detailed omissions serve the study reports; the user gets a short
    // summary (the folder-masking detail is internal and left out).
    let (mut notes, mut details) = (Vec::new(), Vec::new());
    let layer_of = |id: i64| report.layers.iter().find(|layer| layer.id == id).unwrap();
    let name_of = |id: i64| {
        report
            .document_properties
            .get(&id)
            .cloned()
            .unwrap_or_default()
    };
    if !report.tinted_layers.is_empty() {
        let names: Vec<String> = report.tinted_layers.iter().map(|&id| name_of(id)).collect();
        notes.push(format!(
            "Layer color was not applied on {}; {} stored colors.",
            name_list(&names),
            if names.len() == 1 {
                "it keeps its"
            } else {
                "they keep their"
            }
        ));
    }
    if !report.guide_omissions.is_empty() {
        let kinds: Vec<&str> = report
            .guide_omissions
            .keys()
            .map(|field| match *field {
                "LightTableInfo" => "light table",
                "SpecialRulerManager" => "special rulers",
                "RulerRange" => "ruler ranges",
                "GuideMove" => "guides",
                other => other,
            })
            .collect();
        let layers: BTreeSet<i64> = report.guide_omissions.values().flatten().copied().collect();
        details.push(format!(
            "Drawing aids ({}) on {} were not imported; they do not change the picture.",
            kinds.join(", "),
            if layers.len() == 1 {
                "1 layer".to_string()
            } else {
                format!("{} layers", layers.len())
            }
        ));
    }
    // Skips grouped by reason, one note each.
    let mut skips: Vec<(String, Vec<String>)> = Vec::new();
    // Skip layers with unsupported content, and a skipped folder's contents,
    // so the rest of the file still opens. Each skip is listed for the user.
    let mut kept: Vec<(&Entry, Option<i64>)> = Vec::new();
    let mut ancestors: Vec<&Entry> = Vec::new();
    let mut skipped_folder: Option<u8> = None;
    // A clipped sibling depends on the nearest preceding unclipped sibling.
    // Removing that base must not reattach the clipping chain to an unrelated
    // surviving layer. Each folder has an independent clipping stack.
    let mut missing_clip_bases = Vec::new();
    for entry in &report.root_child_next_order {
        if skipped_folder.is_some_and(|depth| entry.depth > depth) {
            continue;
        }
        skipped_folder = None;
        while ancestors.last().is_some_and(|a| a.depth >= entry.depth) {
            ancestors.pop();
        }
        let layer = layer_of(entry.id);
        let known = layer.is_raster()
            || layer.is_correction()
            || layer.is_image()
            || layer.is_vector()
            || layer.is_tone()
            || layer.layer_type == 1584
            || (matches!(layer.layer_type, 0 | 256) && layer.is_folder());
        let depth = entry.depth as usize;
        missing_clip_bases.resize(depth + 1, false);
        if layer.clip != 1 {
            missing_clip_bases[depth] = !known || report.layer_issues.contains_key(&entry.id);
        }
        let reasons = match report.layer_issues.get(&entry.id) {
            Some(reasons) => skip_reason(reasons),
            None if !known => "unsupported layer type".into(),
            None if layer.clip == 1 && missing_clip_bases[depth] => {
                "its clipping base was skipped".into()
            }
            None => {
                if layer.is_folder() {
                    ancestors.push(entry);
                }
                kept.push((
                    entry,
                    ancestors
                        .iter()
                        .rev()
                        .find(|a| a.id != entry.id)
                        .map(|a| a.id),
                ));
                continue;
            }
        };
        let name = if layer.is_folder() {
            skipped_folder = Some(entry.depth);
            format!("{} (folder, with its contents)", name_of(entry.id))
        } else {
            name_of(entry.id)
        };
        match skips.iter_mut().find(|(r, _)| *r == reasons) {
            Some((_, names)) => names.push(name),
            None => skips.push((reasons, vec![name])),
        }
    }
    let toned = |dotted: bool| -> Vec<String> {
        kept.iter()
            .filter(|(entry, _)| {
                report
                    .tones
                    .get(&entry.id)
                    .is_some_and(|(tone, _)| tone.screen.is_some() == dotted)
                    && masks.contains_key(&entry.id)
            })
            .map(|(entry, _)| name_of(entry.id))
            .collect()
    };
    let (dotted, shaded) = (toned(true), toned(false));
    if !dotted.is_empty() {
        notes.push(format!(
            "Screentone drawn as its dots on an ordinary layer, through the layer's mask (Dudel has no screentone settings): {}.",
            name_list(&dotted)
        ));
    }
    if !shaded.is_empty() {
        notes.push(format!(
            "Screentone drawn as even shading at its dot density, through the layer's mask (Dudel draws the dots themselves only for round, square and line tones at a set density, spaced 2 px or more apart): {}.",
            name_list(&shaded)
        ));
    }
    let drawn: Vec<String> = kept
        .iter()
        .filter(|(entry, _)| vectors.contains_key(&entry.id))
        .map(|(entry, _)| name_of(entry.id))
        .collect();
    if !drawn.is_empty() {
        notes.push(format!(
            "Vector {} drawn as raster pixels from {} strokes (lines cannot be edited as vectors here, and may differ slightly from CSP's): {}.",
            if drawn.len() == 1 { "layer" } else { "layers" },
            if drawn.len() == 1 { "its" } else { "their" },
            name_list(&drawn)
        ));
    }
    for (reasons, names) in skips {
        notes.push(format!(
            "Skipped {} {}: {reasons}.",
            if names.len() == 1 {
                "layer".to_string()
            } else {
                format!("{} layers", names.len())
            },
            name_list(&names)
        ));
    }
    if report.timeline_present {
        let frame = report
            .animation
            .current_frame
            .map_or(String::new(), |f| format!(" (frame {})", f as i64 + 1));
        details.push(format!("This file has a timeline. Dudel imported the picture on the frame it was saved on{frame}; other frames and keyframed layer changes are not imported."));
    }
    for (&folder, cel) in &report.animation.folders {
        if !kept.iter().any(|(entry, _)| entry.id == folder) {
            continue;
        }
        let cels = kept.iter().filter(|(_, parent)| *parent == Some(folder));
        let found = cels
            .clone()
            .any(|(entry, _)| Some(name_of(entry.id)) == *cel);
        if report.animation.uncertain.contains(&folder) {
            notes.push(format!(
                "Dudel could not read the keyframes of animation folder “{}”, so it uses the cel the folder stores; this can differ from the saved frame.",
                name_of(folder)
            ));
        }
        details.push(match cel {
            Some(cel) if found => format!(
                "Animation folder “{}” shows cel “{cel}”{}.",
                name_of(folder),
                match cels.count() - 1 {
                    0 => String::new(),
                    1 => "; its other cel is kept as a hidden layer".into(),
                    others => format!("; its other {others} cels are kept as hidden layers"),
                }
            ),
            Some(cel) => format!(
                "Animation folder “{}” is set to show cel “{cel}”, which is not inside it, so it shows nothing on the saved frame; its cels are kept as hidden layers.",
                name_of(folder)
            ),
            None => format!(
                "Animation folder “{}” shows no cel on the saved frame; its cels are kept as hidden layers.",
                name_of(folder)
            ),
        });
    }
    let mut entries = Vec::new();
    let canvas = [report.canvas.width as usize, report.canvas.height as usize];
    for &(order, parent) in &kept {
        let layer = layer_of(order.id);
        // An animation folder shows only the cel for the current frame.
        let cel_shown = match parent.and_then(|p| report.animation.folders.get(&p)) {
            Some(cel) => cel.as_deref() == Some(name_of(order.id).as_str()),
            None => true,
        };
        let content = match layer.layer_type {
            1 | 3 => crate::clip_document::Content::Raster,
            0 if layer.is_image() || layer.is_vector() => crate::clip_document::Content::Raster,
            2 if layer.is_tone() => crate::clip_document::Content::Raster,
            4098 if layer.is_correction() => {
                crate::clip_document::Content::Adjustment(report.corrections[&layer.id].clone())
            }
            0 | 256 if layer.is_folder() => crate::clip_document::Content::Folder,
            1584 => {
                let Some(color) = &layer.paper_draw_color else {
                    return Err(format!("Layer {}: missing paper color", layer.id));
                };
                let full = report
                    .full_scale
                    .iter()
                    .find(|full| full.layer == layer.id)
                    .unwrap();
                if color.enabled != 1
                    || !full.attributes.paper_default()
                    || [
                        full.attributes.width as usize,
                        full.attributes.height as usize,
                    ] != canvas
                    || [
                        layer.offset_x,
                        layer.offset_y,
                        layer.render_offscreen_offset_x,
                        layer.render_offscreen_offset_y,
                    ] != [0; 4]
                {
                    return Err(format!(
                        "Layer {}: unsupported paper rendering mode",
                        layer.id
                    ));
                }
                crate::clip_document::Content::Paper(color.high_byte_rgb)
            }
            _ => unreachable!("unsupported content is skipped above"),
        };
        entries.push(crate::clip_document::Entry {
            source_id: layer.id,
            name: name_of(layer.id),
            depth: order.depth,
            visible: layer.visibility & 1 != 0 && cel_shown,
            opacity: layer.opacity as u16,
            clipped: layer.clip == 1,
            content,
        });
    }
    // CSP's selected layer may have been skipped; select an imported raster.
    let active = if entries.iter().any(|e| e.source_id == report.canvas.current) {
        report.canvas.current
    } else {
        entries
            .iter()
            .find(|e| !matches!(e.content, crate::clip_document::Content::Folder))
            .or(entries.first())
            .map_or(report.canvas.current, |e| e.source_id)
    };
    let mut budget = crate::clip_blocks::Budget::default();
    let mut doc =
        crate::clip_document::assemble(canvas, entries, active, &issues, cancelled, &mut |id| {
            let layer = report.layers.iter().find(|layer| layer.id == id).unwrap();
            if let Some(strokes) = vectors.get(&id) {
                return crate::clip_vector::render(strokes, canvas, cancelled);
            }
            // Tone dots are drawn as CSP draws them when Dudel can, otherwise
            // as their average: black at the dot density. Both are shown
            // through the layer mask.
            if let Some((tone, _)) = report.tones.get(&id) {
                if let Some(screen) = &tone.screen {
                    return crate::screentone::render(screen, tone.percent, canvas, cancelled);
                }
                let alpha = (tone.percent as f32 / 100.0 * 255.0).round() as u8;
                return Ok(crate::pixels::Pixels::solid(
                    canvas[0],
                    canvas[1],
                    [0, 0, 0, alpha],
                ));
            }
            if let Some(image) = report.images.get(&id) {
                let (offset, bytes) = inventory
                    .external_body_bounds(image.external_chunk_offset, image.external_id.len())?;
                source
                    .seek(SeekFrom::Start(offset))
                    .map_err(|error| error.to_string())?;
                let size = [
                    image.attributes.width as usize,
                    image.attributes.height as usize,
                ];
                let original = crate::clip_raster::decode(
                    &mut source,
                    bytes,
                    &image.attributes,
                    crate::clip_raster::Placement {
                        canvas: size,
                        layer_offset: [0, 0],
                        render_offset: [0, 0],
                    },
                    &mut budget,
                    cancelled,
                )?;
                return crate::clip_raster::place(
                    &original,
                    size,
                    image.corners,
                    canvas,
                    cancelled,
                );
            }
            let full = report
                .full_scale
                .iter()
                .find(|full| full.layer == id)
                .unwrap();
            let (Some(external), Some(chunk)) = (&full.external_id, full.external_chunk_offset)
            else {
                return Err(format!("Layer {id}: unresolved raster reference"));
            };
            let (offset, bytes) = inventory.external_body_bounds(chunk, external.len())?;
            source
                .seek(SeekFrom::Start(offset))
                .map_err(|error| error.to_string())?;
            crate::clip_raster::decode(
                &mut source,
                bytes,
                &full.attributes,
                crate::clip_raster::Placement {
                    canvas,
                    layer_offset: [layer.offset_x, layer.offset_y],
                    render_offset: [
                        layer.render_offscreen_offset_x,
                        layer.render_offscreen_offset_y,
                    ],
                },
                &mut budget,
                cancelled,
            )
        })?;
    doc.dpi = report.canvas.resolution;
    crate::document::validate_dpi(doc.dpi)?;
    // Assembly stores children before their folder; map source identities using
    // the same postorder rather than names (which need not be unique).
    let mut order = Vec::new();
    let mut pending = Vec::new();
    for &(entry, _) in &kept {
        while pending
            .last()
            .is_some_and(|e: &&Entry| e.depth >= entry.depth)
        {
            order.push(pending.pop().unwrap().id);
        }
        if report
            .layers
            .iter()
            .find(|l| l.id == entry.id)
            .unwrap()
            .is_folder()
        {
            pending.push(entry);
        } else {
            order.push(entry.id);
        }
    }
    while let Some(entry) = pending.pop() {
        order.push(entry.id);
    }
    for (layer, id) in doc.layers.iter_mut().zip(order) {
        let source = report.layers.iter().find(|l| l.id == id).unwrap();
        layer.blend = clip_blend(source.composite).ok_or("Unsupported CSP blend mode")?;
        let lock = report.locks.get(&id).copied().unwrap_or(0);
        layer.locked = lock & 1 != 0;
        layer.alpha_locked = !layer.folder && lock & 16 != 0;
        if let Some(coverage) = masks.remove(&id) {
            layer.mask = Some(crate::layer_mask::LayerMask {
                id: crate::document::LayerId::new(),
                enabled: true,
                linked: true,
                coverage,
            });
        }
    }
    if doc.layers.iter().any(|layer| layer.mask.is_some()) {
        doc.version = 2;
    }
    doc.validate()?;
    Ok((doc, notes, details))
}
/// One vector layer's strokes, if they only use plain round brushes.
fn read_vector(
    source: &mut File,
    inventory: &clip_container::Inventory,
    data: &ExternalData,
    plain_styles: &BTreeMap<u32, u8>,
) -> Result<Vec<crate::clip_vector::Stroke>, String> {
    const MAX_VECTOR_BYTES: u64 = 512 * 1024 * 1024;
    let (offset, bytes) = inventory.external_body_bounds(data.offset, data.id.len())?;
    if bytes > MAX_VECTOR_BYTES {
        return Err("vector data too large".into());
    }
    source
        .seek(SeekFrom::Start(offset))
        .map_err(|error| error.to_string())?;
    let mut body = vec![0; bytes as usize];
    source
        .read_exact(&mut body)
        .map_err(|error| error.to_string())?;
    let mut strokes = crate::clip_vector::parse(&body)?;
    for stroke in &mut strokes {
        stroke.antialias = *plain_styles
            .get(&stroke.style)
            .ok_or_else(|| format!("brush style {}", stroke.style))?;
    }
    Ok(strokes)
}
/// A short, user-facing reason for skipping a layer.
fn skip_reason(issues: &[String]) -> String {
    let mut reasons = BTreeSet::new();
    for issue in issues {
        reasons.insert(match () {
            _ if issue.contains("vector strokes") => {
                "vector layer with brush features Dudel cannot draw yet"
            }
            _ if issue.contains("Vector") => {
                "vector layer Dudel cannot read (CSP saves vector strokes, not their pixels)"
            }
            _ if issue.contains("TextLayer") => "text layer",
            _ if issue.contains("Balloon") => "speech balloon",
            _ if issue.contains("FilterLayer") => "correction layer",
            _ if issue.contains("Gradation") => "gradient fill",
            _ if issue.contains("LayerEffect") => "layer effect",
            _ if issue.contains("image material") => "image layer Dudel cannot place",
            _ if issue.contains("Material") => "material layer",
            _ if issue.contains("lock/mask") => "layer mask or lock",
            _ if issue.contains("monochrome") => "monochrome fill",
            _ if issue.contains("type/folder/visibility/blend") => {
                "unsupported layer type or blend mode"
            }
            _ => "unsupported CSP feature",
        });
    }
    // A vector or text layer also reports its render metadata; name it once.
    let generic = [
        "unsupported CSP feature",
        "unsupported layer type or blend mode",
    ];
    if reasons.iter().any(|reason| !generic.contains(reason)) {
        for reason in generic {
            reasons.remove(reason);
        }
    }
    reasons.into_iter().collect::<Vec<_>>().join(", ")
}

pub(crate) fn compare_rendering(input: &Path, output: Option<&Path>) -> Result<(), String> {
    #[derive(serde::Serialize)]
    struct Comparison {
        scope: &'static str,
        import_ready: bool,
        omissions: Vec<String>,
        canvas: [usize; 2],
        preview: crate::clip_preview::Comparison,
        quantized_leaf_opacity_hypothesis: crate::clip_preview::Comparison,
    }
    let (mut doc, mut omissions, details) = prepare_document_mode(input, &|| false, true)?;
    omissions.extend(details);
    let mut source = File::open(input).map_err(|error| error.to_string())?;
    let length = source.metadata().map_err(|error| error.to_string())?.len();
    let inventory = clip_container::scan(&mut source, length)?;
    let (offset, size) = inventory.selected_sql_payload();
    source
        .seek(SeekFrom::Start(offset))
        .map_err(|error| error.to_string())?;
    let db = open_snapshot((&mut source).take(size), size as usize)?;
    let (kind, prefix): (String, String) = db
        .query_row(
            "SELECT type,substr(sql,1,128) FROM sqlite_schema WHERE name='CanvasPreview'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(db_error)?;
    if kind != "table"
        || !prefix
            .split_whitespace()
            .take(2)
            .collect::<Vec<_>>()
            .join(" ")
            .eq_ignore_ascii_case("CREATE TABLE")
    {
        return Err("Expected ordinary CanvasPreview table".into());
    }
    let previews = rows(
        &db,
        "SELECT ImageType,ImageWidth,ImageHeight,ImageData FROM CanvasPreview",
        1,
        |row| {
            let data = row.get_ref(3)?.as_blob()?;
            if data.len() > 16 * 1024 * 1024 {
                return Err(rusqlite::Error::InvalidQuery);
            }
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, u32>(1)?,
                row.get::<_, u32>(2)?,
                data.to_vec(),
            ))
        },
    )?;
    let (kind, width, height, encoded) = previews
        .into_iter()
        .next()
        .ok_or("Missing stored preview")?;
    if kind != 1 {
        return Err("Unsupported stored preview image type".into());
    }
    let preview = crate::clip_preview::compare(&encoded, [width, height], &doc)?;
    crate::clip_preview::bake_leaf_opacity_hypothesis(&mut doc);
    let quantized_leaf_opacity_hypothesis =
        crate::clip_preview::compare(&encoded, [width, height], &doc)?;
    clip_container::write_report(
        &Comparison {
            scope: "Read-only rendering study with explicitly omitted editor overlays. Not eligible for import; no document publication or source modification.",
            import_ready: false,
            omissions,
            canvas: [doc.width, doc.height],
            preview,
            quantized_leaf_opacity_hypothesis,
        },
        output,
    )
}

/// Compare unpublished native group hypotheses with saved full-scale group caches.
/// A cache may be stale; matching it does not establish complete CSP semantics.
pub(crate) fn compare_folder_caches(input: &Path, output: Option<&Path>) -> Result<(), String> {
    #[derive(serde::Serialize)]
    struct Group {
        source_layer: i64,
        source_opacity: i64,
        source_visibility: i64,
        entries: usize,
        intrinsic_group: crate::clip_preview::RgbaComparison,
        with_group_opacity: crate::clip_preview::RgbaComparison,
    }
    #[derive(serde::Serialize)]
    struct Study {
        scope: &'static str,
        import_ready: bool,
        omissions: Vec<String>,
        canvas: [usize; 2],
        groups: Vec<Group>,
    }
    let (mut source, inventory, report) = read_source_mode(input, true)?;
    if !report.import_issues.is_empty() {
        return Err(format!(
            "Unsupported clip content: {}",
            report.import_issues.join("; ")
        ));
    }
    let canvas = [report.canvas.width as usize, report.canvas.height as usize];
    let mut budget = crate::clip_blocks::Budget::default();
    let mut decode = |id| {
        let layer = report
            .layers
            .iter()
            .find(|layer| layer.id == id)
            .ok_or("Missing source layer")?;
        let full = report
            .full_scale
            .iter()
            .find(|full| full.layer == id)
            .ok_or("Missing full-scale folder/raster cache")?;
        let (Some(external), Some(chunk)) = (&full.external_id, full.external_chunk_offset) else {
            return Err("Unresolved folder/raster cache reference".into());
        };
        let (offset, bytes) = inventory.external_body_bounds(chunk, external.len())?;
        source
            .seek(SeekFrom::Start(offset))
            .map_err(|error| error.to_string())?;
        crate::clip_raster::decode(
            &mut source,
            bytes,
            &full.attributes,
            crate::clip_raster::Placement {
                canvas,
                layer_offset: [layer.offset_x, layer.offset_y],
                render_offset: [
                    layer.render_offscreen_offset_x,
                    layer.render_offscreen_offset_y,
                ],
            },
            &mut budget,
            &|| false,
        )
    };
    let mut groups = Vec::new();
    for (start, parent) in report.root_child_next_order.iter().enumerate() {
        let folder = report
            .layers
            .iter()
            .find(|layer| layer.id == parent.id)
            .unwrap();
        if !folder.is_folder() {
            continue;
        }
        if folder.clip != 0 {
            return Err("Folder cache study cannot isolate a clipped folder".into());
        }
        let mut entries = Vec::new();
        for (relative, item) in report.root_child_next_order[start..].iter().enumerate() {
            if relative != 0 && item.depth <= parent.depth {
                break;
            }
            let layer = report
                .layers
                .iter()
                .find(|layer| layer.id == item.id)
                .unwrap();
            let content = if layer.is_folder() {
                crate::clip_document::Content::Folder
            } else if layer.is_raster() {
                crate::clip_document::Content::Raster
            } else {
                return Err("Folder cache study supports only raster descendants".into());
            };
            entries.push(crate::clip_document::Entry {
                source_id: layer.id,
                name: String::new(),
                depth: item.depth - parent.depth,
                // Group visibility/opacity may be applied outside its stored cache.
                visible: relative == 0 || layer.visibility & 1 != 0,
                opacity: if relative == 0 {
                    256
                } else {
                    layer.opacity as u16
                },
                clipped: layer.clip == 1,
                content,
            });
        }
        let mut doc = crate::clip_document::assemble(
            canvas,
            entries,
            folder.id,
            &[],
            &|| false,
            &mut decode,
        )?;
        let cached = decode(folder.id)?;
        let native = crate::pixels::Pixels::from(doc.composite_transparent());
        let intrinsic_group = crate::clip_preview::compare_pixels(&native, &cached, true)?;
        drop(native);
        doc.layers[doc.active].opacity = folder.opacity as f32 / 256.;
        let native = crate::pixels::Pixels::from(doc.composite_transparent());
        let with_group_opacity = crate::clip_preview::compare_pixels(&native, &cached, true)?;
        groups.push(Group {
            source_layer: folder.id,
            source_opacity: folder.opacity,
            source_visibility: folder.visibility,
            entries: doc.layers.len(),
            intrinsic_group,
            with_group_opacity,
        });
    }
    if groups.is_empty() {
        return Err("No ordinary folders available for cache comparison".into());
    }
    clip_container::write_report(
        &Study {
            scope: "Read-only isolated folder hypotheses versus saved CSP group caches; hidden RGB normalized for metrics only. Cache freshness and complete CSP rendering semantics unproven. No import eligibility or source modification.",
            import_ready: false,
            omissions: report.rendering_study_omissions,
            canvas,
            groups,
        },
        output,
    )
}

/// User-selected full-size PNG reference. File pairing/export state is external
/// evidence and cannot be inferred from matching geometry or pixel metrics.
pub(crate) fn compare_export(
    input: &Path,
    reference: &Path,
    output: Option<&Path>,
) -> Result<(), String> {
    #[derive(serde::Serialize)]
    struct Comparison {
        scope: &'static str,
        import_ready: bool,
        omissions: Vec<String>,
        canvas: [usize; 2],
        full_size_png: crate::clip_preview::Comparison,
    }
    let file = File::open(reference).map_err(|error| error.to_string())?;
    if file.metadata().map_err(|error| error.to_string())?.len() > 16 * 1024 * 1024 {
        return Err("PNG reference exceeds comparison bounds".into());
    }
    let mut encoded = Vec::new();
    file.take(16 * 1024 * 1024 + 1)
        .read_to_end(&mut encoded)
        .map_err(|error| error.to_string())?;
    let (doc, mut omissions, details) = prepare_document_mode(input, &|| false, true)?;
    omissions.extend(details);
    let comparison =
        crate::clip_preview::compare(&encoded, [doc.width as u32, doc.height as u32], &doc)?;
    clip_container::write_report(
        &Comparison {
            scope: "Read-only native composition versus supplied full-size PNG. Pair correspondence, export state and complete CSP semantics require external validation. Explicit study omissions remain unsupported by import; no source modification or document publication.",
            import_ready: false,
            omissions,
            canvas: [doc.width, doc.height],
            full_size_png: comparison,
        },
        output,
    )
}

pub(crate) fn verify_document(input: &Path, output: Option<&Path>) -> Result<(), String> {
    #[derive(serde::Serialize)]
    struct Verification {
        scope: &'static str,
        canvas: [usize; 2],
        entries: usize,
        active: usize,
        composite: crate::clip_raster::Statistics,
    }
    let doc = prepare_document(input, &|| false)?;
    let rgba = crate::pixels::Pixels::from(doc.composite_transparent());
    clip_container::write_report(
        &Verification {
            scope: "Unpublished candidate document assembly and native composite; no complete CSP feature/rounding validation or GUI import.",
            canvas: [doc.width, doc.height],
            entries: doc.layers.len(),
            active: doc.active,
            composite: crate::clip_raster::statistics(&rgba),
        },
        output,
    )
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    const ID: &str = "extrnlid0123456789ABCDEF0123456789ABCDEF";
    #[test]
    fn reduced_preview_dpi_preserves_physical_size() {
        use image::ImageEncoder;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("preview.clip");
        let mut png = Vec::new();
        image::codecs::png::PngEncoder::new(&mut png)
            .write_image(&[255; 16 * 24 * 4], 16, 24, image::ExtendedColorType::Rgba8)
            .unwrap();
        let hex: String = png.iter().map(|b| format!("{b:02x}")).collect();
        for (geometry, dpi) in [
            ("CanvasWidth=32,CanvasHeight=48,CanvasResolution=600", 300.0),
            (
                "CanvasWidth=0.32,CanvasHeight=0.48,CanvasUnit=3,CanvasResolution=100",
                50.0,
            ),
            ("CanvasWidth=32,CanvasHeight=48,CanvasResolution=0", 72.0),
        ] {
            let sql = format!(
                "UPDATE Canvas SET {geometry}; CREATE TABLE CanvasPreview (ImageType INTEGER, ImageWidth INTEGER,ImageHeight INTEGER,ImageData BLOB); INSERT INTO CanvasPreview VALUES (1,16,24,x'{hex}');"
            );
            std::fs::write(&path, worker_fixture_with_sql(&sql)).unwrap();
            let doc = import_preview(&path, &|| false).unwrap().document;
            assert_eq!(doc.dpi, dpi);
            assert_eq!([doc.width, doc.height], [16, 24]);
        }
    }
    #[test]
    fn imported_dpi_comes_from_canvas_without_resizing_pixels() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("dpi.clip");
        for dpi in [72.0, 300.0, 600.5, 1200.0] {
            std::fs::write(
                &path,
                worker_fixture_with_sql(&format!("UPDATE Canvas SET CanvasResolution={dpi}")),
            )
            .unwrap();
            let doc = import_document(&path, &|| false).unwrap().document;
            assert_eq!(doc.dpi, dpi);
            assert_eq!([doc.width, doc.height], [32, 48]);
        }
    }
    pub(crate) fn worker_fixture() -> Vec<u8> {
        worker_fixture_with_sql("")
    }
    pub(crate) fn worker_fixture_with_sql(mutation: &str) -> Vec<u8> {
        worker_fixture_with_externals(mutation, &[])
    }
    /// The worker fixture plus extra external chunks (id, body), each
    /// registered in ExternalChunk after the raster chunk.
    pub(crate) fn worker_fixture_with_externals(
        mutation: &str,
        extra: &[(&str, Vec<u8>)],
    ) -> Vec<u8> {
        let db = fixture();
        db.execute_batch("UPDATE Layer SET LayerType=1 WHERE MainId=40; ALTER TABLE Layer ADD COLUMN LayerName TEXT; ALTER TABLE Layer ADD COLUMN LayerLock INTEGER; ALTER TABLE Layer ADD COLUMN LayerMasking INTEGER; ALTER TABLE Layer ADD COLUMN LayerLayerMaskMipmap INTEGER; ALTER TABLE Layer ADD COLUMN LayerUsePaletteColor INTEGER; UPDATE Layer SET LayerName='Fixture',LayerLock=0,LayerMasking=0,LayerLayerMaskMipmap=0,LayerUsePaletteColor=0; UPDATE Layer SET LayerMasking=32 WHERE MainId=20;").unwrap();
        db.execute_batch(mutation).unwrap();
        let chunk = |id: &str, body: &[u8]| {
            let mut ext = b"CHNKExta".to_vec();
            ext.extend((8 + id.len() as u64 + body.len() as u64).to_be_bytes());
            ext.extend((id.len() as u64).to_be_bytes());
            ext.extend(id.as_bytes());
            ext.extend(body);
            ext
        };
        let mut ext = chunk(ID, &crate::clip_blocks::tests::fixture(&[None]));
        for (id, body) in extra {
            db.execute(
                "INSERT INTO ExternalChunk VALUES (?1,?2)",
                rusqlite::params![id.as_bytes(), 80 + ext.len() as i64],
            )
            .unwrap();
            ext.extend(chunk(id, body));
        }
        let mut data = container(&db.serialize("main").unwrap());
        data[48..56].copy_from_slice(&(80 + ext.len() as u64).to_be_bytes());
        data.splice(80..80, ext);
        let size = data.len() as u64;
        data[8..16].copy_from_slice(&size.to_be_bytes());
        data
    }
    /// A track value map holding `ImageCelName`, as CSP stores it.
    fn cel_map(cel: &str) -> Vec<u8> {
        let utf16 = |text: &str| {
            let mut out = (text.encode_utf16().count() as u32).to_be_bytes().to_vec();
            out.extend(text.encode_utf16().flat_map(u16::to_be_bytes));
            out
        };
        let mut entry = utf16("ImageCelName");
        entry.extend(utf16(cel));
        entry.extend([0, 0, 0, 2, 0, 0, 0, 0]);
        let mut map = vec![0, 0, 0, 8, 0, 0, 0, 1];
        map.extend((entry.len() as u32 + 4).to_be_bytes());
        map.extend(entry);
        map
    }
    #[test]
    fn track_value_maps_name_the_current_cel_and_reject_bad_bounds() {
        assert_eq!(cel_name(&cel_map("1")).as_deref(), Some("1"));
        assert_eq!(
            cel_name(&cel_map("1 Copy 20")).as_deref(),
            Some("1 Copy 20")
        );
        let map = cel_map("12");
        for end in 0..map.len() - 8 {
            assert_eq!(cel_name(&map[..end]), None, "{end}");
        }
        let mut huge = map.clone();
        huge[8..12].copy_from_slice(&u32::MAX.to_be_bytes());
        assert_eq!(cel_name(&huge), None);
        assert_eq!(cel_name(&[0, 0, 0, 8, 0, 0, 0, 0]), None);
    }
    #[test]
    fn animation_folders_show_only_the_saved_frames_cel() {
        // Layer 40 becomes an animation folder whose only cel is layer 30,
        // saved on frame 5 (index 4) of a 6 fps timeline: time 40 in 1/60 s.
        let tree = "UPDATE Layer SET LayerType=0,LayerFolder=1,LayerRenderMipmap=0,LayerFirstChildIndex=30 WHERE MainId=40; UPDATE Layer SET LayerNextIndex=0 WHERE MainId=30; UPDATE Layer SET LayerFirstChildIndex=40 WHERE MainId=20; UPDATE Layer SET LayerName='Cel A' WHERE MainId=30; ALTER TABLE Layer ADD COLUMN AnimationFolder INTEGER; UPDATE Layer SET AnimationFolder=1 WHERE MainId=40; ALTER TABLE Layer ADD COLUMN LayerUuid TEXT; UPDATE Layer SET LayerUuid='00112233-4455-6677-8899-aabbccddeeff' WHERE MainId=40; CREATE TABLE TimeLine (MainId INTEGER, CurrentFrame REAL, FrameRate REAL); INSERT INTO TimeLine VALUES (1, 4.0, 6.0); CREATE TABLE Track (MainId INTEGER, TrackKind INTEGER, TrackValueMap BLOB, LayerUuidWithTrack BLOB, TrackActionMixer BLOB);";
        let uuid = "x'00112233445566778899aabbccddeeff'";
        let mixer_id = "extrnlidFEDCBA9876543210FEDCBA9876543210";
        let mixer = |cels: &[&str]| crate::clip_timeline::tests::mixer(&[0., 30., 50.], cels, 60.0);
        // (keyframes, stored cel, cel shown?, expected note)
        let cases: [(Option<Vec<u8>>, &str, bool, &str); 3] = [
            // Keyframes win over a stale stored cel.
            (
                Some(mixer(&["Cel B", "Cel A", "Cel B"])),
                "Cel B",
                true,
                "shows cel “Cel A”",
            ),
            (
                Some(mixer(&["Cel A", "Cel B", "Cel A"])),
                "Cel A",
                false,
                "cel “Cel B”, which is not inside it",
            ),
            // Without keyframes the stored cel is used, with a caveat.
            (None, "Cel A", true, "could not read the keyframes"),
        ];
        for (keys, stored, shown, note) in cases {
            let hex: String = cel_map(stored).iter().map(|b| format!("{b:02x}")).collect();
            let mixer_ref = if keys.is_some() {
                format!("CAST('{mixer_id}' AS BLOB)")
            } else {
                "NULL".into()
            };
            let extra: Vec<(&str, Vec<u8>)> = keys.into_iter().map(|k| (mixer_id, k)).collect();
            let source = worker_fixture_with_externals(
                &format!(
                    "{tree} INSERT INTO Track VALUES (1, 2000, x'{hex}', {uuid}, {mixer_ref}); INSERT INTO Track VALUES (2, 2001, x'00000008', NULL, NULL);"
                ),
                &extra,
            );
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("animation.clip");
            std::fs::write(&path, &source).unwrap();
            let imported = import_document(&path, &|| false).unwrap();
            let doc = imported.document;
            assert_eq!(doc.layers.len(), 2);
            assert!(doc.layers[1].folder && !doc.layers[0].folder);
            assert_eq!(doc.layers[0].visible, shown, "{note}");
            let notes = [imported.notes, imported.details].concat().join("\n");
            assert!(notes.contains("frame 5"), "{notes}");
            assert!(notes.contains(note), "{notes}");
            // Strict inspection still refuses timelines.
            assert!(prepare_document_mode(&path, &|| false, false).is_err());
        }
    }
    #[test]
    fn editable_import_accepts_supported_folders_and_clipping() {
        for mutation in [
            "UPDATE Layer SET LayerType=256,LayerFolder=1,LayerRenderMipmap=0 WHERE MainId=40",
            "UPDATE Layer SET LayerType=0,LayerFolder=1,LayerRenderMipmap=0 WHERE MainId=40",
            "UPDATE Layer SET LayerType=0,LayerFolder=17,LayerRenderMipmap=0 WHERE MainId=40",
            "UPDATE Layer SET LayerClip=1 WHERE MainId=30",
        ] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("generated.clip");
            std::fs::write(&path, worker_fixture_with_sql(mutation)).unwrap();
            let doc = prepare_document(&path, &|| false).unwrap();
            doc.validate().unwrap();
        }
    }
    #[test]
    fn skipped_clipping_bases_do_not_reattach_dependents_or_cross_folder_boundaries() {
        let folder = |id, next, child, clipped| {
            format!(
                "INSERT INTO Layer (MainId,LayerType,LayerFolder,LayerVisibility,LayerOpacity,LayerComposite,LayerClip,LayerNextIndex,LayerFirstChildIndex,LayerRenderMipmap,LayerOffsetX,LayerOffsetY,LayerRenderOffscrOffsetX,LayerRenderOffscrOffsetY,LayerName,LayerLock,LayerMasking,LayerLayerMaskMipmap,LayerUsePaletteColor) VALUES ({id},0,1,1,256,0,{clipped},{next},{child},0,0,0,0,0,'{id}',0,32,0,0);"
            )
        };
        let flat = format!(
            "{}{} UPDATE Layer SET LayerFirstChildIndex=100 WHERE MainId=20; UPDATE Layer SET LayerNextIndex=110,LayerClip=1 WHERE MainId=40; UPDATE Layer SET LayerLock=2 WHERE MainId=30;",
            folder(100, 30, 0, 0),
            folder(110, 0, 0, 0)
        );
        let cases = [
            // An unsupported base removes its clipped chain, even with an
            // earlier surviving base. The next unclipped sibling resets it.
            (flat.clone(), vec!["100", "110"], true),
            // An unsupported clipped member does not remove its valid base
            // or the later members of the same chain.
            (
                format!("{flat} UPDATE Layer SET LayerClip=1 WHERE MainId=30;"),
                vec!["100", "40", "110"],
                false,
            ),
            // A missing base inside a retained folder does not affect the
            // folder's own clipping chain in its parent.
            (
                format!(
                    "{}{} UPDATE Layer SET LayerFirstChildIndex=100 WHERE MainId=20; UPDATE Layer SET LayerLock=2 WHERE MainId=30; UPDATE Layer SET LayerClip=1 WHERE MainId=40;",
                    folder(100, 110, 30, 0),
                    folder(110, 0, 0, 1)
                ),
                vec!["100", "110"],
                true,
            ),
            // Skipped base folders and dependent clipped folders both omit
            // their descendants. Neither descendant can become a new base.
            (
                format!(
                    "{}{}{} UPDATE Layer SET LayerFirstChildIndex=100 WHERE MainId=20; UPDATE Layer SET LayerNextIndex=0 WHERE MainId=30; UPDATE Layer SET LayerLock=2 WHERE MainId=100;",
                    folder(100, 110, 30, 0),
                    folder(110, 120, 40, 1),
                    folder(120, 0, 0, 0)
                ),
                vec!["120"],
                true,
            ),
        ];
        for (mutation, expected, missing_base_note) in cases {
            let source = worker_fixture_with_sql(&format!(
                "UPDATE Layer SET LayerName=CAST(MainId AS TEXT); {mutation}"
            ));
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("clipped.clip");
            std::fs::write(&path, &source).unwrap();
            let imported = import_document(&path, &|| false).unwrap();
            assert_eq!(
                imported
                    .document
                    .layers
                    .iter()
                    .map(|layer| layer.name.as_str())
                    .collect::<Vec<_>>(),
                expected
            );
            assert_eq!(
                imported
                    .notes
                    .iter()
                    .any(|note| note.contains("its clipping base was skipped")),
                missing_base_note
            );
            imported.document.validate().unwrap();
            assert_eq!(std::fs::read(&path).unwrap(), source);
        }
    }
    #[test]
    fn ordinary_open_and_closed_folders_keep_children_in_unpublished_study() {
        for folder in [1, 17] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("generated.clip");
            let sql = format!(
                "UPDATE Layer SET LayerType=0,LayerFolder={folder},LayerRenderMipmap=0,LayerNextIndex=0,LayerFirstChildIndex=40,LayerMasking=32 WHERE MainId=30"
            );
            std::fs::write(&path, worker_fixture_with_sql(&sql)).unwrap();
            let (doc, notes, _) = prepare_document_mode(&path, &|| false, true).unwrap();
            // The folder-masking detail is internal: no note for the user.
            assert!(notes.is_empty(), "{notes:?}");
            assert_eq!(doc.layers.len(), 2);
            assert!(!doc.layers[0].folder);
            assert_eq!(doc.layers[0].depth, 1);
            assert!(doc.layers[1].folder);
            assert_eq!(doc.layers[1].depth, 0);
            assert_eq!(doc.active, 1);
        }
        let db = fixture();
        let mut report = check(&db).unwrap();
        let layer = &mut report.layers[1];
        for (kind, folder, supported) in [
            (0, 0, false),
            (0, 16, false),
            (0, 17, true),
            (1, 17, false),
            (0, 33, false),
            (512, 1, false),
            (256, 17, false),
        ] {
            layer.layer_type = kind;
            layer.folder = folder;
            assert_eq!(layer.supported_kind(), supported);
        }
    }
    #[test]
    fn folder_cache_study_stays_numeric_unpublished_and_requires_resolved_cache() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("generated.clip");
        let output = dir.path().join("numeric.json");
        let mutation = "UPDATE Layer SET LayerType=0,LayerFolder=17,LayerNextIndex=0,LayerFirstChildIndex=40,LayerMasking=32 WHERE MainId=30";
        let source = worker_fixture_with_sql(mutation);
        std::fs::write(&path, &source).unwrap();
        compare_folder_caches(&path, Some(&output)).unwrap();
        let report: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&output).unwrap()).unwrap();
        assert_eq!(report["import_ready"], false);
        assert_eq!(report["groups"].as_array().unwrap().len(), 1);
        assert_eq!(report["groups"][0]["entries"], 2);
        assert_eq!(
            report["groups"][0]["intrinsic_group"]["exact_pixels"],
            32 * 48
        );
        assert_eq!(
            report["groups"][0]["with_group_opacity"]["hidden_rgb_normalized"],
            true
        );
        assert_eq!(report["omissions"].as_array().unwrap().len(), 1);
        assert_eq!(std::fs::read(&path).unwrap(), source);
        assert!(compare_folder_caches(&path, Some(&output)).is_err());
        assert!(prepare_document(&path, &|| false).is_ok());
        let missing = worker_fixture_with_sql(&format!(
            "{mutation}; UPDATE Layer SET LayerRenderMipmap=0 WHERE MainId=30"
        ));
        std::fs::write(&path, &missing).unwrap();
        let unavailable = dir.path().join("unavailable.json");
        assert!(
            compare_folder_caches(&path, Some(&unavailable))
                .unwrap_err()
                .contains("Missing full-scale")
        );
        assert!(!unavailable.exists());
        assert_eq!(std::fs::read(&path).unwrap(), missing);
    }
    #[test]
    fn legacy_effects_and_modern_special_payloads_cannot_bypass_raster_preflight() {
        for (field, payload) in [
            ("FilterLayerInfo", "x'010203'"),
            ("LayerEffectInfo", "x'010203'"),
            ("GradationFillInfo", "x'010203'"),
            ("TextLayerString", "'private source text'"),
            ("TextLayerAttributes", "x'010203'"),
            ("VectorNormalStrokeIndex", "2"),
            ("MaterialContentType", "110"),
            ("UsePreviewMaskColorType", "1"),
        ] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("generated.clip");
            let sql = format!(
                "ALTER TABLE Layer ADD COLUMN {field}; UPDATE Layer SET {field}={payload} WHERE MainId=30"
            );
            let inactive =
                worker_fixture_with_sql(&format!("{sql}; UPDATE Layer SET {field}=NULL"));
            std::fs::write(&path, inactive).unwrap();
            let complete = prepare_document(&path, &|| false).unwrap().layers.len();
            let source = worker_fixture_with_sql(&sql);
            std::fs::write(&path, &source).unwrap();
            // Strict inspection rejects the file.
            let error = prepare_document_mode(&path, &|| false, false)
                .err()
                .unwrap();
            assert!(error.contains(&format!("unsupported {field}")), "{error}");
            assert!(!error.contains("private source text"));
            // Import leaves the layer out entirely and says so; its payload is
            // never read as raster pixels and its text never reaches a note.
            let (doc, notes, _) = prepare_document_mode(&path, &|| false, true).unwrap();
            assert_eq!(doc.layers.len(), complete - 1, "{field}");
            assert!(notes.iter().any(|n| n.starts_with("Skipped")), "{notes:?}");
            assert!(notes.iter().all(|n| !n.contains("private source text")));
            assert_eq!(std::fs::read(&path).unwrap(), source);
        }
    }
    #[test]
    fn external_png_comparison_is_full_size_numeric_and_never_overwrites_inputs() {
        use image::ImageEncoder;
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.clip");
        let reference = dir.path().join("reference.png");
        let report = dir.path().join("report.json");
        let clip = worker_fixture();
        std::fs::write(&source, &clip).unwrap();
        let encode = |w, h, rgba: &[u8]| {
            let mut png = Vec::new();
            image::codecs::png::PngEncoder::new(&mut png)
                .write_image(rgba, w, h, image::ExtendedColorType::Rgba8)
                .unwrap();
            png
        };
        let png = encode(32, 48, &vec![0; 32 * 48 * 4]);
        std::fs::write(&reference, &png).unwrap();
        compare_export(&source, &reference, Some(&report)).unwrap();
        let metrics: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&report).unwrap()).unwrap();
        assert_eq!(metrics["import_ready"], false);
        assert_eq!(metrics["full_size_png"]["compared_full_size"], true);
        assert_eq!(metrics["full_size_png"]["exact_pixels"], 32 * 48);
        for existing in [&source, &reference, &report] {
            assert!(compare_export(&source, &reference, Some(existing)).is_err());
        }
        assert_eq!(std::fs::read(&source).unwrap(), clip);
        assert_eq!(std::fs::read(&reference).unwrap(), png);
        let unavailable = dir.path().join("unavailable.json");
        std::fs::write(&reference, encode(1, 1, &[0; 4])).unwrap();
        assert!(compare_export(&source, &reference, Some(&unavailable)).is_err());
        assert!(!unavailable.exists());
        std::fs::write(&reference, b"not PNG").unwrap();
        assert!(compare_export(&source, &reference, Some(&unavailable)).is_err());
        assert!(!unavailable.exists());
    }
    fn fixture() -> Connection {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE Canvas (MainId INTEGER,CanvasWidth REAL,CanvasHeight REAL,CanvasChannelBytes INTEGER,CanvasDefaultChannelOrder INTEGER,CanvasRootFolder INTEGER,CanvasCurrentLayer INTEGER,CanvasSrcProfile BLOB,CanvasUnit INTEGER,CanvasResolution REAL);
            CREATE TABLE Layer (_PW_ID INTEGER PRIMARY KEY,MainId INTEGER,LayerType INTEGER,LayerFolder INTEGER,LayerVisibility INTEGER,LayerOpacity INTEGER,LayerComposite INTEGER,LayerClip INTEGER,LayerNextIndex INTEGER,LayerFirstChildIndex INTEGER,LayerRenderMipmap INTEGER,LayerOffsetX INTEGER,LayerOffsetY INTEGER,LayerRenderOffscrOffsetX INTEGER,LayerRenderOffscrOffsetY INTEGER);
            CREATE TABLE Mipmap (MainId INTEGER,CanvasId INTEGER,LayerId INTEGER,MipmapCount INTEGER,BaseMipmapInfo INTEGER);
            CREATE TABLE MipmapInfo (MainId INTEGER,LayerId INTEGER,ThisScale REAL,Offscreen INTEGER,NextIndex INTEGER);
            CREATE TABLE Offscreen (MainId INTEGER,CanvasId INTEGER,LayerId INTEGER,Attribute BLOB,BlockData BLOB);
            CREATE TABLE ExternalChunk (ExternalID BLOB,Offset INTEGER);
            INSERT INTO Canvas VALUES (7,32,48,1,33,20,30,NULL,0,300);
            INSERT INTO Layer VALUES (1,20,256,1,1,256,0,0,0,30,50,0,0,0,0);
            INSERT INTO Layer VALUES (2,30,1,0,5,256,0,0,40,0,60,-7,-9,7,9);
            INSERT INTO Layer VALUES (3,40,1584,0,1,128,0,0,0,0,70,0,0,0,0);
            INSERT INTO Mipmap VALUES (50,7,20,1,80),(60,7,30,2,90),(70,7,40,1,100);
            INSERT INTO MipmapInfo VALUES (80,20,100,110,0),(90,30,100,120,91),(91,30,50,121,0),(100,40,100,130,0);").unwrap();
        for (id, owner) in [(110, 20), (120, 30), (121, 30), (130, 40)] {
            db.execute(
                "INSERT INTO Offscreen VALUES (?1,7,?2,?3,?4)",
                rusqlite::params![
                    id,
                    owner,
                    crate::clip_attributes::fixture(32, 48, &[20, 0, 0, 0, 4], &[104]),
                    ID.as_bytes()
                ],
            )
            .unwrap();
        }
        db.execute("INSERT INTO ExternalChunk VALUES (?1,80)", [ID.as_bytes()])
            .unwrap();
        db
    }
    fn check(db: &Connection) -> Result<Report, String> {
        metadata(db, &[(ID.into(), 80)])
    }

    #[test]
    fn main_id_graph_mipmap_offsets_and_uninterpreted_fields() {
        let db = fixture();
        let report = check(&db).unwrap();
        assert_eq!(
            report
                .root_child_next_order
                .iter()
                .map(|entry| entry.id)
                .collect::<Vec<_>>(),
            [30, 40]
        );
        let full = report
            .full_scale
            .iter()
            .find(|full| full.layer == 30)
            .unwrap();
        assert_eq!((full.mipmap, full.info, full.offscreen), (60, 90, 120));
        assert_eq!(full.external_chunk_offset, Some(80));
        assert_eq!(
            (report.layers[1].offset_x, report.layers[1].offset_y),
            (-7, -9)
        );
        assert_eq!(
            (
                report.layers[1].render_offscreen_offset_x,
                report.layers[1].render_offscreen_offset_y
            ),
            (7, 9)
        );
        assert!(
            report
                .diagnostics
                .iter()
                .any(|message| message.contains("separate render-offscreen offsets"))
        );
        assert!(
            report
                .diagnostics
                .iter()
                .any(|message| message.contains("Layer 30 has uninterpreted"))
        );
        assert!(
            report
                .diagnostics
                .iter()
                .any(|message| message.contains("Layer 40 has uninterpreted"))
        );
        db.execute(
            "UPDATE ExternalChunk SET ExternalID=CAST(ExternalID AS TEXT)",
            [],
        )
        .unwrap();
        assert_eq!(
            check(&db).unwrap().full_scale[1].external_chunk_offset,
            Some(80)
        );
        db.execute("DELETE FROM ExternalChunk", []).unwrap();
        let unresolved = check(&db).unwrap();
        assert!(
            unresolved
                .full_scale
                .iter()
                .all(|full| full.external_chunk_offset.is_none())
        );
        assert!(
            unresolved
                .diagnostics
                .iter()
                .any(|message| message.contains("must not be assumed blank"))
        );
    }

    #[test]
    fn reject_damaged_graph_mip_and_external_references() {
        for mutation in [
            "UPDATE Layer SET LayerNextIndex=30 WHERE MainId=40",
            "UPDATE Layer SET LayerNextIndex=99 WHERE MainId=30",
            "UPDATE Layer SET LayerNextIndex=0 WHERE MainId=30",
            "UPDATE Layer SET MainId=30 WHERE MainId=40",
            "UPDATE Layer SET LayerFirstChildIndex=40 WHERE MainId=30",
            "UPDATE Canvas SET CanvasCurrentLayer=99",
            "UPDATE Layer SET LayerFolder=0 WHERE MainId=20",
            "UPDATE Layer SET LayerOpacity=257 WHERE MainId=30",
            "UPDATE Layer SET LayerRenderMipmap=99 WHERE MainId=30",
            "UPDATE Mipmap SET LayerId=40 WHERE MainId=60",
            "UPDATE Mipmap SET MipmapCount=1 WHERE MainId=60",
            "UPDATE Mipmap SET MipmapCount=3 WHERE MainId=60",
            "UPDATE MipmapInfo SET ThisScale=50 WHERE MainId=90",
            "UPDATE MipmapInfo SET NextIndex=90 WHERE MainId=91",
            "UPDATE MipmapInfo SET MainId=90 WHERE MainId=91",
            "UPDATE Offscreen SET LayerId=40 WHERE MainId=120",
            "UPDATE MipmapInfo SET Offscreen=99 WHERE MainId=90",
            "UPDATE ExternalChunk SET Offset=81",
            "UPDATE ExternalChunk SET ExternalID=123",
            "INSERT INTO ExternalChunk SELECT * FROM ExternalChunk",
            "UPDATE Canvas SET CanvasWidth=16385",
            "UPDATE Canvas SET CanvasWidth=10001,CanvasHeight=10000",
            "UPDATE Canvas SET CanvasWidth=32.5",
            "UPDATE Canvas SET CanvasUnit=1,CanvasWidth=2,CanvasHeight=3",
            "UPDATE Canvas SET CanvasResolution=0",
            "INSERT INTO Canvas SELECT * FROM Canvas",
            "UPDATE Offscreen SET Attribute=zeroblob(65537)",
            "UPDATE Offscreen SET BlockData=zeroblob(1025)",
            "DROP TABLE Layer; CREATE VIEW Layer AS SELECT 1",
            "UPDATE Layer SET LayerVisibility='wrong-type' WHERE MainId=30",
            "UPDATE Layer SET LayerRenderOffscrOffsetX='wrong-type' WHERE MainId=30",
        ] {
            let db = fixture();
            db.execute_batch(mutation).unwrap();
            assert!(check(&db).is_err(), "accepted {mutation}");
        }
        let db = fixture();
        assert!(metadata(&db, &[(ID.into(), 80), (ID.into(), 90)]).is_err());
    }

    #[test]
    fn nested_folders_have_bounded_depth_and_no_shared_children() {
        let db = fixture();
        db.execute_batch("UPDATE Layer SET LayerFolder=1,LayerType=256,LayerFirstChildIndex=40,LayerNextIndex=0 WHERE MainId=30").unwrap();
        let report = check(&db).unwrap();
        assert_eq!(
            report
                .root_child_next_order
                .iter()
                .map(|entry| (entry.id, entry.depth))
                .collect::<Vec<_>>(),
            [(30, 0), (40, 1)]
        );
        db.execute_batch("UPDATE Layer SET LayerNextIndex=40 WHERE MainId=30")
            .unwrap();
        assert!(check(&db).unwrap_err().contains("multiply referenced"));
        let canvas = report.canvas;
        let mut layers = report.layers;
        layers[1].child = 1000;
        for depth in 0..=MAX_DEPTH {
            layers.push(Layer {
                id: 1000 + depth as i64,
                layer_type: 256,
                folder: 1,
                visibility: 1,
                opacity: 256,
                composite: 0,
                clip: 0,
                next: 0,
                child: if depth == MAX_DEPTH {
                    40
                } else {
                    1001 + depth as i64
                },
                mipmap: 1,
                offset_x: 0,
                offset_y: 0,
                render_offscreen_offset_x: 0,
                render_offscreen_offset_y: 0,
                paper_draw_color: None,
                image: false,
                vector: false,
                tone: false,
                correction: false,
            });
        }
        assert!(
            layer_order(&canvas, &layers)
                .unwrap_err()
                .contains("depth limit")
        );
    }
    #[test]
    fn structural_folders_do_not_require_a_cached_raster_mipmap() {
        let db = fixture();
        db.execute("UPDATE Layer SET LayerRenderMipmap=0 WHERE MainId=20", [])
            .unwrap();
        let report = check(&db).unwrap();
        assert_eq!(report.root_child_next_order.len(), 2);
        assert!(!report.full_scale.iter().any(|level| level.layer == 20));
        db.execute("UPDATE Layer SET LayerRenderMipmap=0 WHERE MainId=30", [])
            .unwrap();
        assert!(check(&db).is_err());
    }
    #[test]
    fn document_feature_preflight_preserves_names_privately_and_rejects_masks_effects_and_profiles()
    {
        fn source() -> Connection {
            let db = fixture();
            db.execute_batch("UPDATE Layer SET LayerType=1 WHERE MainId=40; ALTER TABLE Layer ADD COLUMN LayerName TEXT; ALTER TABLE Layer ADD COLUMN LayerLock INTEGER; ALTER TABLE Layer ADD COLUMN LayerMasking INTEGER; ALTER TABLE Layer ADD COLUMN LayerLayerMaskMipmap INTEGER; ALTER TABLE Layer ADD COLUMN LayerUsePaletteColor INTEGER; UPDATE Layer SET LayerName='Paint private name',LayerLock=0,LayerMasking=0,LayerLayerMaskMipmap=0,LayerUsePaletteColor=0; UPDATE Layer SET LayerMasking=32 WHERE MainId=20;").unwrap();
            db
        }
        let db = source();
        let mut report = check(&db).unwrap();
        document_features(&db, &mut report).unwrap();
        assert!(report.import_issues.is_empty());
        assert_eq!(report.document_properties[&30], "Paint private name");
        // A Layer palette color label does not change the picture.
        db.execute_batch("UPDATE Layer SET LayerUsePaletteColor=1 WHERE MainId=30")
            .unwrap();
        let mut labeled = check(&db).unwrap();
        document_features(&db, &mut labeled).unwrap();
        assert!(labeled.import_issues.is_empty());
        assert!(
            !serde_json::to_string(&report)
                .unwrap()
                .contains("Paint private name")
        );
        for mutation in [
            "UPDATE Layer SET LayerLayerMaskMipmap=1 WHERE MainId=30",
            "UPDATE Layer SET LayerLock=1 WHERE MainId=30",
            "ALTER TABLE Layer ADD COLUMN FilterLayerV132 BLOB; UPDATE Layer SET FilterLayerV132=x'01' WHERE MainId=30",
            "CREATE TABLE Track (MainId INTEGER); INSERT INTO Track VALUES (1)",
            "ALTER TABLE Canvas ADD COLUMN CanvasDstProfile BLOB; UPDATE Canvas SET CanvasDstProfile=x'01'",
        ] {
            let db = source();
            db.execute_batch(mutation).unwrap();
            let mut report = check(&db).unwrap();
            document_features(&db, &mut report).unwrap();
            assert!(!report.import_issues.is_empty(), "accepted {mutation}");
        }
        let db = source();
        db.execute_batch("ALTER TABLE Layer ADD COLUMN SpecialRulerManager INTEGER; UPDATE Layer SET SpecialRulerManager=9 WHERE MainId=30").unwrap();
        let mut strict = check(&db).unwrap();
        document_features(&db, &mut strict).unwrap();
        assert!(!strict.import_issues.is_empty());
        assert!(strict.rendering_study_omissions.is_empty());
        let mut study = check(&db).unwrap();
        document_features_mode(&db, &mut study, true).unwrap();
        assert!(study.import_issues.is_empty());
        assert_eq!(study.rendering_study_omissions.len(), 1);
        db.execute(
            "UPDATE Layer SET LayerLayerMaskMipmap=1 WHERE MainId=30",
            [],
        )
        .unwrap();
        let mut study = check(&db).unwrap();
        document_features_mode(&db, &mut study, true).unwrap();
        // Import never renders the mask without it: the layer is skipped.
        assert!(study.import_issues.is_empty());
        assert!(
            study.layer_issues.contains_key(&30),
            "import must never bypass rendering features"
        );
    }
    #[test]
    fn vector_layers_with_plain_brushes_are_drawn_not_skipped() {
        const VECTOR: &str = "extrnlidFEDCBA9876543210FEDCBA9876543210";
        let with_vector = |db: &Connection| metadata(db, &[(ID.into(), 80), (VECTOR.into(), 500)]);
        let db = fixture();
        db.execute_batch(
            "ALTER TABLE Layer ADD COLUMN LayerName TEXT; ALTER TABLE Layer ADD COLUMN LayerLock INTEGER; ALTER TABLE Layer ADD COLUMN LayerMasking INTEGER; ALTER TABLE Layer ADD COLUMN LayerLayerMaskMipmap INTEGER; ALTER TABLE Layer ADD COLUMN LayerUsePaletteColor INTEGER;
             ALTER TABLE Layer ADD COLUMN VectorNormalStrokeIndex INTEGER;
             ALTER TABLE Layer ADD COLUMN DrawToRenderOffscreenType INTEGER;
             UPDATE Layer SET LayerType=1 WHERE MainId=40;
             UPDATE Layer SET LayerName='',LayerLock=0,LayerMasking=0,LayerLayerMaskMipmap=0,LayerUsePaletteColor=0; UPDATE Layer SET LayerMasking=32 WHERE MainId=20;
             UPDATE Layer SET LayerType=0,VectorNormalStrokeIndex=1,DrawToRenderOffscreenType=10 WHERE MainId=30;
             CREATE TABLE VectorObjectList (MainId INTEGER,CanvasId INTEGER,LayerId INTEGER,VectorData BLOB);
             INSERT INTO VectorObjectList VALUES (1,7,30,CAST('extrnlidFEDCBA9876543210FEDCBA9876543210' AS BLOB));
             INSERT INTO ExternalChunk VALUES ('extrnlidFEDCBA9876543210FEDCBA9876543210',500);
             CREATE TABLE BrushStyle (MainId INTEGER,TexturePattern INTEGER,SprayFlag INTEGER);
             INSERT INTO BrushStyle VALUES (1,0,0),(2,3,0);",
        )
        .unwrap();
        let mut study = with_vector(&db).unwrap();
        document_features_mode(&db, &mut study, true).unwrap();
        assert!(study.layers.iter().any(|l| l.id == 30 && l.is_vector()));
        assert!(
            !study.layer_issues.contains_key(&30),
            "{:?}",
            study.layer_issues
        );
        assert_eq!(study.vectors[&30].offset, 500);
        // Textured styles are not plain; strokes using them skip the layer.
        assert_eq!(study.plain_brush_styles, BTreeMap::from([(1, 0)]));
        // Strict inspection still refuses vector layers.
        let mut strict = with_vector(&db).unwrap();
        document_features(&db, &mut strict).unwrap();
        assert!(!strict.import_issues.is_empty());
        // A render type other than the observed one is not drawn.
        db.execute_batch("UPDATE Layer SET DrawToRenderOffscreenType=20 WHERE MainId=30")
            .unwrap();
        let mut study = with_vector(&db).unwrap();
        document_features_mode(&db, &mut study, true).unwrap();
        assert!(study.layer_issues.contains_key(&30));
    }
    #[test]
    fn tone_effects_give_their_density_only_when_nothing_else_is_on() {
        fn name(out: &mut Vec<u8>, text: &str) {
            out.extend((text.len() as u32).to_be_bytes());
            out.extend(text.encode_utf16().flat_map(u16::to_be_bytes));
        }
        let info = |edge: u32, tone: u32, density: u32, line: u32| {
            let mut body = Vec::new();
            name(&mut body, "EffectEdge");
            body.extend(edge.to_be_bytes());
            body.extend(3.0f64.to_be_bytes());
            name(&mut body, "EffectTone");
            for word in [tone, 0x4052_0000, 0, 1, 2, density] {
                body.extend(word.to_be_bytes());
            }
            name(&mut body, "EffectLine");
            body.extend(8u32.to_be_bytes());
            body.extend(line.to_be_bytes());
            body.extend(0u32.to_be_bytes());
            let mut out = ((body.len() + 8) as u32).to_be_bytes().to_vec();
            out.extend(2u32.to_be_bytes());
            out.extend(body);
            out
        };
        let percent = |info: &[u8]| tone_effect(info).map(|tone| tone.percent);
        assert_eq!(percent(&info(0, 1, 30, 0)), Some(30));
        assert_eq!(percent(&info(1, 1, 30, 0)), None);
        assert_eq!(percent(&info(0, 0, 30, 0)), None);
        assert_eq!(percent(&info(0, 1, 30, 1)), None);
        assert_eq!(percent(&info(0, 1, 101, 0)), None);
        let mut short = info(0, 1, 30, 0);
        short.pop();
        assert_eq!(percent(&short), None);
        // Too short for the grid: shaded evenly.
        assert_eq!(tone_effect(&info(0, 1, 30, 0)).unwrap().screen, None);
    }
    #[test]
    fn correction_layers_convert_only_checked_settings() {
        use crate::adjustment::{Adjustment, Settings};
        let words =
            |values: &[i32]| -> Vec<u8> { values.iter().flat_map(|v| v.to_be_bytes()).collect() };
        // Brightness +30, contrast +40: CSP's (in + 30 - 128) / 0.6 + 128.
        let settings = correction(&words(&[1, 8, 30, 40])).unwrap();
        let line = Adjustment::try_from(settings).unwrap();
        for (input, output) in [(40.0, 31.3), (100.0, 131.3), (128.0, 178.0), (150.0, 214.7)] {
            assert!((line.apply([input; 3])[0] - output).abs() < 0.1, "{input}");
        }
        assert!(correction(&words(&[1, 8, 30, -40])).is_err());
        // Hue/saturation/luminosity in CSP's own model: +20 luminosity takes
        // black to 50 (not 51), and +60 / -40 / +20 turns pure red into
        // CSP's (212, 215, 111) up to one level.
        let hsl = |values: [i32; 3]| {
            Adjustment::try_from(
                correction(&words(&[4, 12, values[0], values[1], values[2]])).unwrap(),
            )
            .unwrap()
        };
        assert_eq!(hsl([0, 0, 20]).apply([0.0; 3]), [50.0; 3]);
        assert_eq!(
            hsl([0, 0, 0]).apply([12.0, 200.0, 99.0]),
            [12.0, 200.0, 99.0]
        );
        let red = hsl([60, -40, 20]).apply([255.0, 6.0, 0.0]);
        assert!(
            red.iter()
                .zip([212.0, 215.0, 111.0])
                .all(|(a, b)| (a - b).abs() <= 1.0),
            "{red:?}"
        );
        assert!(correction(&words(&[4, 12, 400, 0, 0])).is_err());
        // Tone curve: 32 curves of 65 words, the first for all channels.
        let curve = |master: &[[u16; 2]], red: &[[u16; 2]]| {
            let mut info = words(&[3, 4160]);
            for index in 0..32 {
                let points: &[[u16; 2]] = match index {
                    0 => master,
                    1 => red,
                    _ => &[[0, 0], [u16::MAX, u16::MAX]],
                };
                let mut block = vec![points.len() as u16];
                block.extend(points.iter().flatten());
                block.resize(65, 0);
                info.extend(block.iter().flat_map(|v| v.to_be_bytes()));
            }
            correction(&info)
        };
        let identity = [[0, 0], [u16::MAX, u16::MAX]];
        assert_eq!(
            curve(&[[0, 0], [25368, 48914], [u16::MAX, u16::MAX]], &identity).unwrap(),
            Settings::Curves {
                channels: vec![(0, vec![[0, 0], [99, 190], [255, 255]])],
                bezier: true,
            }
        );
        let four = [[0, 0], [9000, 20000], [40000, 50000], [u16::MAX, u16::MAX]];
        assert!(curve(&four, &identity).is_err());
        assert!(curve(&identity, &[[0, 0], [30000, 40000], [u16::MAX, u16::MAX]]).is_err());
        assert!(correction(&words(&[2, 0])).is_err());
    }
    #[test]
    fn round_dot_tones_at_a_set_density_get_their_grid() {
        fn name(out: &mut Vec<u8>, text: &str) {
            out.extend((text.len() as u32).to_be_bytes());
            out.extend(text.encode_utf16().flat_map(u16::to_be_bytes));
        }
        // CSP's full record: 300 dpi, round dots, set density 40%, 85
        // lines, 45°, no offset.
        let info = |shape: u32, offset: u32| {
            let mut body = Vec::new();
            name(&mut body, "EffectEdge");
            body.extend(0u32.to_be_bytes());
            name(&mut body, "EffectTone");
            body.extend(1u32.to_be_bytes());
            body.extend(300.0f64.to_be_bytes());
            for word in [shape, 2, 40] {
                body.extend(word.to_be_bytes());
            }
            body.extend(85.0f64.to_be_bytes());
            for word in [1, 0, 45, 10, 0, offset, 0] {
                body.extend(word.to_be_bytes());
            }
            body.extend([0.0f64, 0.0].map(f64::to_be_bytes).concat());
            let mut out = ((body.len() + 8) as u32).to_be_bytes().to_vec();
            out.extend(2u32.to_be_bytes());
            out.extend(body);
            out
        };
        let screen = crate::screentone::Screen {
            lines: 85.0,
            resolution: 300.0,
            angle: 45,
            shape: crate::screentone::Shape::Round,
        };
        assert_eq!(
            tone_effect(&info(1, 0)),
            Some(ToneEffect {
                percent: 40,
                screen: Some(screen)
            })
        );
        assert_eq!(
            tone_effect(&info(4, 0)).unwrap().screen,
            Some(crate::screentone::Screen {
                shape: crate::screentone::Shape::Line,
                ..screen
            })
        );
        // Other dot shapes and moved grids are shaded evenly.
        assert_eq!(tone_effect(&info(7, 0)).unwrap().screen, None);
        assert_eq!(tone_effect(&info(1, 3)).unwrap().screen, None);
    }
    #[test]
    fn switched_off_proofing_and_inch_canvases_open() {
        let db = fixture();
        // Proofing and canvas color adjustment keep stored settings while off.
        db.execute_batch(
            "ALTER TABLE Canvas ADD COLUMN CanvasDoSimulateColor INTEGER;
             ALTER TABLE Canvas ADD COLUMN CanvasUseColorAdjustment INTEGER;
             ALTER TABLE Canvas ADD COLUMN CanvasSimulateSrcProfile BLOB;
             ALTER TABLE Canvas ADD COLUMN CanvasColorAdjustmentLevel BLOB;
             ALTER TABLE Layer ADD COLUMN LayerName TEXT; ALTER TABLE Layer ADD COLUMN LayerLock INTEGER; ALTER TABLE Layer ADD COLUMN LayerMasking INTEGER; ALTER TABLE Layer ADD COLUMN LayerLayerMaskMipmap INTEGER; ALTER TABLE Layer ADD COLUMN LayerUsePaletteColor INTEGER;
             UPDATE Layer SET LayerType=1 WHERE MainId=40;
             UPDATE Layer SET LayerName='',LayerLock=0,LayerMasking=0,LayerLayerMaskMipmap=0,LayerUsePaletteColor=0; UPDATE Layer SET LayerMasking=32 WHERE MainId=20;
             UPDATE Canvas SET CanvasDoSimulateColor=0,CanvasUseColorAdjustment=0,CanvasSimulateSrcProfile=x'01',CanvasColorAdjustmentLevel=x'01'",
        )
        .unwrap();
        let mut report = check(&db).unwrap();
        document_features(&db, &mut report).unwrap();
        assert!(
            report.import_issues.is_empty(),
            "{:?}",
            report.import_issues
        );
        for on in [
            "UPDATE Canvas SET CanvasDoSimulateColor=1",
            "UPDATE Canvas SET CanvasUseColorAdjustment=1",
        ] {
            db.execute_batch(on).unwrap();
            let mut report = check(&db).unwrap();
            document_features(&db, &mut report).unwrap();
            assert!(!report.import_issues.is_empty(), "accepted {on}");
        }
        // 0.32 x 0.48 inches at 100 dpi is the fixture's 32 x 48 px.
        let db = fixture();
        db.execute_batch(
            "UPDATE Canvas SET CanvasUnit=3,CanvasWidth=0.32,CanvasHeight=0.48,CanvasResolution=100",
        )
        .unwrap();
        let report = check(&db).unwrap();
        assert_eq!([report.canvas.width, report.canvas.height], [32.0, 48.0]);
        db.execute_batch("UPDATE Canvas SET CanvasWidth=0.325")
            .unwrap();
        assert!(check(&db).is_err());
        // Millimetres round to the nearest pixel: 8.2 x 12.1 mm at 100 dpi
        // is 32.3 x 47.6 px, the fixture's 32 x 48.
        db.execute_batch(
            "UPDATE Canvas SET CanvasUnit=2,CanvasWidth=8.2,CanvasHeight=12.1,CanvasResolution=100",
        )
        .unwrap();
        let report = check(&db).unwrap();
        assert_eq!([report.canvas.width, report.canvas.height], [32.0, 48.0]);
        db.execute_batch("UPDATE Canvas SET CanvasUnit=3,CanvasWidth=0.32,CanvasHeight=0.48")
            .unwrap();
        db.execute_batch("UPDATE Canvas SET CanvasWidth=200,CanvasHeight=0.48")
            .unwrap();
        assert!(check(&db).unwrap_err().contains("16,384 px per side"));
    }

    #[test]
    fn snapshot_is_read_only_bounded_and_interruptible() {
        let source = fixture();
        let columns = (0..68)
            .map(|index| format!("column{index} INTEGER"))
            .collect::<Vec<_>>()
            .join(",");
        source
            .execute_batch(&format!("CREATE TABLE Project ({columns})"))
            .unwrap();
        let bytes = source.serialize("main").unwrap();
        let db = open_snapshot(&bytes[..], bytes.len()).unwrap();
        let temporary_path = db._file.path().to_owned();
        assert!(db.is_readonly("main").unwrap());
        db.execute_batch("PRAGMA query_only=OFF").unwrap();
        assert!(db.execute("DELETE FROM Layer", []).is_err());
        db.execute_batch("PRAGMA query_only=ON").unwrap();
        assert_eq!(check(&db).unwrap().layers.len(), 3);
        assert_eq!(db.limit(Limit::SQLITE_LIMIT_COLUMN).unwrap(), 128);
        assert!(open_snapshot(&bytes[..bytes.len() - 1], bytes.len()).is_err());
        assert!(open_snapshot(std::io::empty(), MAX_SQL + 1).is_err());
        let error = db.query_row("WITH RECURSIVE count(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM count WHERE x<100000000) SELECT sum(x) FROM count", [], |row| row.get::<_,i64>(0)).unwrap_err();
        assert!(
            matches!(error,rusqlite::Error::SqliteFailure(code,_) if code.code == rusqlite::ErrorCode::OperationInterrupted)
        );
        drop(db);
        assert!(!temporary_path.exists());
    }

    #[test]
    fn bounded_rows_and_raster_counts() {
        let db = fixture();
        assert!(
            rows(&db, "SELECT MainId FROM Layer", 2, |row| row
                .get::<_, i64>(0))
            .is_err()
        );
        let report = check(&db).unwrap();
        let mut layers = report.layers;
        layers[2].next = 1000;
        for index in 0..MAX_RASTER_LAYERS {
            layers.push(Layer {
                id: 1000 + index as i64,
                layer_type: 1,
                folder: 0,
                visibility: 1,
                opacity: 256,
                composite: 0,
                clip: 0,
                next: if index + 1 == MAX_RASTER_LAYERS {
                    0
                } else {
                    1001 + index as i64
                },
                child: 0,
                mipmap: 1,
                offset_x: 0,
                offset_y: 0,
                render_offscreen_offset_x: 0,
                render_offscreen_offset_y: 0,
                paper_draw_color: None,
                image: false,
                vector: false,
                tone: false,
                correction: false,
            });
        }
        assert!(
            layer_order(&report.canvas, &layers)
                .unwrap_err()
                .contains("raster count")
        );
    }
    #[test]
    fn optional_paper_colors_preserve_unsigned_words_without_defaulting_old_schemas() {
        let db = fixture();
        assert!(
            check(&db)
                .unwrap()
                .layers
                .iter()
                .all(|layer| layer.paper_draw_color.is_none())
        );
        db.execute_batch("ALTER TABLE Layer ADD COLUMN DrawColorEnable INTEGER; ALTER TABLE Layer ADD COLUMN DrawColorMainRed INTEGER; ALTER TABLE Layer ADD COLUMN DrawColorMainGreen INTEGER; ALTER TABLE Layer ADD COLUMN DrawColorMainBlue INTEGER; UPDATE Layer SET DrawColorEnable=1,DrawColorMainRed=4294967295,DrawColorMainGreen=2394721980,DrawColorMainBlue=0 WHERE MainId=40;").unwrap();
        let report = check(&db).unwrap();
        let color = report.layers[2].paper_draw_color.as_ref().unwrap();
        assert_eq!(color.enabled, 1);
        assert_eq!(color.raw_rgb, [u32::MAX, 2394721980, 0]);
        assert_eq!(color.high_byte_rgb, [255, 142, 0]);
        db.execute(
            "UPDATE Layer SET DrawColorMainRed=4294967296 WHERE MainId=40",
            [],
        )
        .unwrap();
        assert!(check(&db).is_err());
        db.execute("UPDATE Layer SET DrawColorMainRed=-1 WHERE MainId=40", [])
            .unwrap();
        assert!(check(&db).is_err());
    }
    #[test]
    fn nonempty_timeline_is_diagnosed_and_unknown_track_views_are_never_queried() {
        let db = fixture();
        db.execute_batch("CREATE TABLE Track (MainId INTEGER)")
            .unwrap();
        assert!(
            !check(&db)
                .unwrap()
                .diagnostics
                .iter()
                .any(|message| message.contains("Timeline/track content"))
        );
        db.execute("INSERT INTO Track VALUES (1)", []).unwrap();
        assert!(
            check(&db)
                .unwrap()
                .diagnostics
                .iter()
                .any(|message| message.contains("must not silently discard animation"))
        );
        db.execute_batch("DROP TABLE Track; CREATE VIEW Track AS SELECT unavailable_function()")
            .unwrap();
        assert!(
            check(&db)
                .unwrap()
                .diagnostics
                .iter()
                .any(|message| message.contains("unsupported schema"))
        );
    }

    fn container(sql: &[u8]) -> Vec<u8> {
        let mut bytes = b"CSFCHUNK".to_vec();
        bytes.extend([0; 8]);
        bytes.extend(24u64.to_be_bytes());
        bytes.extend(b"CHNKHead");
        bytes.extend(40u64.to_be_bytes());
        bytes.extend(256u64.to_be_bytes());
        bytes.extend(80u64.to_be_bytes());
        bytes.extend(16u64.to_be_bytes());
        bytes.extend([0; 16]);
        bytes.extend(b"CHNKSQLi");
        bytes.extend((sql.len() as u64).to_be_bytes());
        bytes.extend(sql);
        bytes.extend(b"CHNKFoot");
        bytes.extend(0u64.to_be_bytes());
        let len = bytes.len() as u64;
        bytes[8..16].copy_from_slice(&len.to_be_bytes());
        bytes
    }

    #[test]
    fn reports_preserve_input_and_existing_files_even_on_bad_metadata() {
        let db = fixture();
        db.execute("DELETE FROM ExternalChunk", []).unwrap();
        let source = container(&db.serialize("main").unwrap());
        let temp = tempfile::tempdir().unwrap();
        let input = temp.path().join("sample.clip");
        let output = temp.path().join("report.json");
        std::fs::write(&input, &source).unwrap();
        inspect(&input, Some(&output)).unwrap();
        let original_report = std::fs::read(&output).unwrap();
        assert!(inspect(&input, Some(&output)).is_err());
        assert!(inspect(&input, Some(&input)).is_err());
        assert_eq!(std::fs::read(&input).unwrap(), source);
        assert_eq!(std::fs::read(&output).unwrap(), original_report);
        db.execute_batch("UPDATE Layer SET LayerNextIndex=30 WHERE MainId=40")
            .unwrap();
        std::fs::write(&input, container(&db.serialize("main").unwrap())).unwrap();
        let bad_report = temp.path().join("bad.json");
        assert!(inspect(&input, Some(&bad_report)).is_err());
        assert!(!bad_report.exists());
    }
    #[test]
    fn metadata_inspection_cross_checks_external_sizes_before_creating_report() {
        let db = fixture();
        let body = crate::clip_blocks::tests::fixture(&[None]);
        fn with_external(sql: &[u8], body: &[u8]) -> Vec<u8> {
            let mut ext = b"CHNKExta".to_vec();
            ext.extend((48 + body.len() as u64).to_be_bytes());
            ext.extend(40u64.to_be_bytes());
            ext.extend(ID.as_bytes());
            ext.extend(body);
            let mut data = container(sql);
            data[48..56].copy_from_slice(&(80 + ext.len() as u64).to_be_bytes());
            data.splice(80..80, ext);
            let size = data.len() as u64;
            data[8..16].copy_from_slice(&size.to_be_bytes());
            data
        }
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("fixture.clip");
        let output = dir.path().join("report.json");
        let source = with_external(&db.serialize("main").unwrap(), &body);
        std::fs::write(&input, &source).unwrap();
        inspect(&input, Some(&output)).unwrap();
        let json: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&output).unwrap()).unwrap();
        assert!(
            json["full_scale"]
                .as_array()
                .unwrap()
                .iter()
                .all(|level| level["block_layout_validated"] == true)
        );
        assert_eq!(std::fs::read(&input).unwrap(), source);
        // Structurally valid attributes with a wrong record size must fail only
        // when crossed against the actual external record, with no new report.
        let wrong = crate::clip_attributes::fixture(32, 48, &[20, 0, 0, 0, 4], &[108]);
        db.execute(
            "UPDATE Offscreen SET Attribute=?1 WHERE MainId=120",
            [wrong],
        )
        .unwrap();
        let damaged = with_external(&db.serialize("main").unwrap(), &body);
        std::fs::write(&input, &damaged).unwrap();
        let rejected = dir.path().join("rejected.json");
        assert!(
            inspect(&input, Some(&rejected))
                .unwrap_err()
                .contains("does not match")
        );
        assert!(!rejected.exists());
        assert_eq!(std::fs::read(&input).unwrap(), damaged);
    }
}
