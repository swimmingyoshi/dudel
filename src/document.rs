use crate::brush::Brush;
use serde::{Deserialize, Serialize};

/// Shared by live dabs and rebuilding a finalized prefix under a tapered tail.
/// The share of what is there a clear-water dab lifts at full strength.
const WATER_LIFT: f32 = 0.06;

/// Lay paint over paint that is still wet, mixing the two as pigments.
fn paint_pigment(pixel: &mut [u8], a: f32, color: [u8; 3], alpha_locked: bool) {
    if a <= 0.0 {
        return;
    }
    let old_a = f32::from(pixel[3]) / 255.0;
    let new_a = if alpha_locked {
        old_a
    } else {
        a + old_a * (1.0 - a)
    };
    let share = if alpha_locked { a } else { a / new_a };
    let mixed = crate::pigment::mix([pixel[0], pixel[1], pixel[2]], color, share);
    pixel[..3].copy_from_slice(&mixed);
    pixel[3] = (new_a * 255.0).round() as u8;
}

pub(crate) fn paint_pixel(
    pixels: &mut [u8],
    a: f32,
    color: [u8; 3],
    erase: bool,
    alpha_locked: bool,
) {
    if a <= 0.0 {
        return;
    }
    if alpha_locked {
        if pixels[3] == 0 {
            return;
        }
        for (c, value) in color.iter().enumerate() {
            pixels[c] = (*value as f32 * a + pixels[c] as f32 * (1.0 - a)).round() as u8;
        }
    } else if erase {
        pixels[3] = (pixels[3] as f32 * (1.0 - a)).round() as u8;
    } else {
        let old_a = pixels[3] as f32 / 255.0;
        let new_a = a + old_a * (1.0 - a);
        if new_a > 0.0 {
            for (c, value) in color.iter().enumerate() {
                pixels[c] = ((*value as f32 * a + pixels[c] as f32 * old_a * (1.0 - a)) / new_a)
                    .round() as u8;
            }
            pixels[3] = (new_a * 255.0).round() as u8;
        }
    }
}

/// Prepared dab geometry and delivery, shared by whole-dab and resumable rows.
/// Expensive flow lookup and dynamics are evaluated once per dab, not per slice.
pub(crate) struct DabRaster {
    pub position: [f32; 2],
    radius: f32,
    opacity: f32,
    interval_ratio: Option<f32>,
    /// How many dabs this one stands for, when the brush is held to wider
    /// spacing than it asks for (`Brush::dab_spacing`).
    held: Option<f32>,
    flow_lookup: Option<crate::brush_edge::FlowLookup>,
    sin: f32,
    cos: f32,
    map: crate::symmetry::Map,
    pub left: i32,
    pub right: i32,
    pub top: i32,
    pub bottom: i32,
    /// Smudge: how far the tip moved since the previous dab.
    pub(crate) smear: [f32; 2],
    /// Brush graph color for this dab, and the stroke's secondary color.
    pub(crate) color_shift: Option<crate::brush_graph::ColorShift>,
    pub(crate) secondary: Option<[u8; 3]>,
    /// Stroke direction in degrees, once the stroke has moved: thick paint
    /// streaks run along it.
    pub(crate) heading: Option<f32>,
    /// The brush's Water, Pigment, Thickness, Paper texture and Granulation
    /// for this dab, after the brush graph.
    pub(crate) wetness: f32,
    pub(crate) pigment: f32,
    pub(crate) thickness: f32,
    pub(crate) paper_response: f32,
    pub(crate) granulation: f32,
}
/// How far a tip reaches left and right, and up and down, of its center:
/// the extent of the turned ellipse or rectangle with its soft outer pixel
/// (`brush_edge::coverage_with_profile` is zero past it). Infinite when it
/// cannot be worked out, and for tips of a pixel or less, which are sampled
/// around each pixel.
fn tip_extent(
    radius: f32,
    aspect: f32,
    square: bool,
    map: crate::symmetry::Map,
    sin: f32,
    cos: f32,
) -> [f32; 2] {
    if radius <= 1.0 {
        return [f32::INFINITY; 2];
    }
    // Canvas offsets to tip axes, in pixels: along the long side, across it.
    let a = [
        cos * map[0] + sin * map[2],
        cos * map[1] + sin * map[3],
        -sin * map[0] + cos * map[2],
        -sin * map[1] + cos * map[3],
    ];
    let det = a[0] * a[3] - a[1] * a[2];
    if det.abs() <= 1e-6 || det.is_nan() {
        return [f32::INFINITY; 2];
    }
    // Coverage ends half a pixel of the short side past the edge, in tip
    // units, which is further along the long side.
    let (long, short) = (radius + 0.5 / aspect, radius * aspect + 0.5);
    let reach = |along: f32, across: f32| {
        let (along, across) = (along * long, across * short);
        let reach = if square {
            along.abs() + across.abs()
        } else {
            along.hypot(across)
        };
        // Half a pixel spare for rounding.
        reach / det.abs() + 0.5
    };
    [reach(a[3], a[1]), reach(a[2], a[0])]
}
impl DabRaster {
    pub fn new(
        sample: crate::stroke::Sample,
        brush: &Brush,
        map: crate::symmetry::Map,
        width: usize,
        height: usize,
    ) -> Self {
        let (x, y) = (sample.x, sample.y);
        let factors = brush.factors(&sample);
        let radius = (brush.size * factors.size / 2.0).max(0.5);
        let opacity = brush.opacity * brush.flow * factors.opacity * factors.flow;
        let interval_ratio = (brush.adjust_flow_by_spacing && brush.opacity > 0.0).then(|| {
            let diameter = brush.size * factors.size * brush.aspect;
            let reference = (diameter * 0.1).max(0.5);
            sample
                .spacing
                .unwrap_or_else(|| (diameter * brush.spacing).max(0.5))
                / reference
        });
        // A wet or blur brush held to wider spacing than it asks for lays
        // fewer dabs; each delivers more, so the stroke builds up as it
        // would have.
        let [asked, used] = brush.dab_spacing(&factors);
        let held = (used > asked && brush.opacity > 0.0).then(|| used / asked);
        let flow_lookup = interval_ratio.and_then(|ratio| {
            (opacity > 0.0 && opacity <= brush.opacity * 0.95)
                .then(|| crate::brush_edge::FlowLookup::new(opacity / brush.opacity, ratio))
        });
        let angle = brush.angle + sample.rotation.unwrap_or(0.0);
        let (sin, cos) = angle.to_radians().sin_cos();
        let square = matches!(
            brush.tip,
            crate::brush::Tip::Square | crate::brush::Tip::Image
        );
        // The square around the long side, and inside it the box the turned
        // tip really reaches: a narrow tip covers a fraction of the square,
        // and a wet dab copies and blurs its whole box.
        let full = 0.5
            + if square {
                radius * std::f32::consts::SQRT_2
            } else {
                radius
            };
        let [extent_x, extent_y] =
            tip_extent(radius, brush.aspect, square, map, sin, cos).map(|tight| tight.min(full));
        Self {
            position: [x, y],
            radius,
            opacity,
            interval_ratio,
            held,
            flow_lookup,
            sin,
            cos,
            map,
            left: ((x - extent_x).floor() as i32).max(0),
            right: ((x + extent_x).ceil() as i32).min(width as i32 - 1),
            top: ((y - extent_y).floor() as i32).max(0),
            bottom: ((y + extent_y).ceil() as i32).min(height as i32 - 1),
            smear: [0.0; 2],
            color_shift: factors.color,
            secondary: None,
            heading: sample.heading,
            wetness: brush.wetness * factors.material.water,
            pigment: brush.pigment * factors.material.pigment,
            thickness: brush.thickness * factors.material.thickness,
            paper_response: brush.paper_response * factors.material.paper,
            granulation: brush.granulation * factors.material.granulation,
        }
    }
}

/// Process-local identity: snapshots retain it; loading and duplication mint new
/// identities. It is deliberately absent from persisted project data.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct LayerId(u64);
impl LayerId {
    pub(crate) fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        Self(
            NEXT.fetch_update(
                std::sync::atomic::Ordering::Relaxed,
                std::sync::atomic::Ordering::Relaxed,
                |id| id.checked_add(1),
            )
            .expect("Layer identity exhausted"),
        )
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Layer {
    #[serde(skip, default = "LayerId::new")]
    pub id: LayerId,
    pub name: String,
    pub visible: bool,
    pub opacity: f32,
    /// Imported CSP raster leaves quantize effective coverage to whole bytes.
    /// Native/legacy layers retain continuous opacity by default.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub csp_byte_opacity: bool,
    /// Photoshop's fill opacity: fades the layer's own pixels like opacity,
    /// but not its effects. Applied when the document is expanded.
    #[serde(default = "full_fill", skip_serializing_if = "is_full_fill")]
    pub fill: f32,
    #[serde(default)]
    pub blend: crate::blend::BlendMode,
    #[serde(default)]
    pub clipped: bool,
    #[serde(default)]
    pub locked: bool,
    #[serde(default)]
    pub alpha_locked: bool,
    /// Folder header: no pixels; its contents are the contiguous deeper
    /// entries directly below it in the stack.
    #[serde(default)]
    pub folder: bool,
    /// Nesting level; 0 is the document root.
    #[serde(default)]
    pub depth: u8,
    /// Folder rows hide their contents in the layer panel.
    #[serde(default)]
    pub collapsed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mask: Option<crate::layer_mask::LayerMask>,
    /// Layer effects, bottom to top, drawn whenever the document is
    /// composited (`layer_effects::expand`); a folder's follow its contents.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub effects: Vec<crate::layer_effects::LayerEffect>,
    #[serde(skip)]
    pub(crate) effect_cache: crate::layer_effects::Cache,
    /// An adjustment layer recolors what lies below it instead of adding
    /// pixels; its own pixels are unused.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adjustment: Option<crate::adjustment::Adjustment>,
    /// A vector layer keeps its strokes; its pixels are them painted in order
    /// (`vector_layer`), and other pixel edits are refused.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vector: Option<crate::vector_layer::VectorLayer>,
    /// A text layer keeps its words and layout; its pixels are that text
    /// drawn once (`text_layer`), and other pixel edits are refused.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) text: Option<crate::text_layer::TextLayer>,
    /// What kind of paint the layer holds (`material`): watercolor, dry
    /// media or thick paint; None is a plain layer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub material: Option<Box<crate::material::Material>>,
    pub pixels: crate::pixels::Pixels,
    /// Only injected into canvas display snapshots, never live artwork or files.
    #[serde(skip)]
    pub(crate) display_reference: Option<std::sync::Arc<crate::reference_stack::Scene>>,
}

/// Largest canvas side. The display draws 256-pixel tiles, so no GPU texture
/// approaches this size; long strips (webtoons) can use the full side.
pub const MAX_SIDE: usize = 16384;
/// Largest canvas area: 100 megapixels, Clip Studio Paint's 10,000 x 10,000.
/// Raster/mask payload still shares the document budgets.
pub const MAX_PIXELS: usize = 100_000_000;
/// The limits above, for messages.
pub const SIZE_LIMIT: &str = "up to 16,384 px per side and 100 megapixels in total";
/// Raster layers per document. Large illustration and VTuber rig files from
/// CSP, Photoshop (8,000 layer cap) and Affinity rarely exceed a few hundred.
pub const MAX_RASTER_LAYERS: usize = 1000;
/// Layers plus folders.
pub const MAX_ENTRIES: usize = 2000;
/// Stored raster pixels per document (only painted tiles count): the most the
/// former 32-layer cap allowed (32 full 4096 x 4096 layers). Mask-aware
/// documents share the same budget between raster and mask payload; at
/// 512 MiB a single painted 100-megapixel layer would nearly fill it.
pub const MAX_PAYLOAD: usize = 2 << 30;
pub const MAX_MASKED_PAYLOAD: usize = MAX_PAYLOAD;
/// Effects per layer (Photoshop allows ten of each kind).
pub const MAX_EFFECTS: usize = 32;
/// Folder nesting, matching Photoshop's ten levels.
pub const MAX_DEPTH: u8 = 10;

pub(crate) fn validate_folder_depths(
    entries: impl IntoIterator<Item = (bool, u8)>,
) -> Result<(), String> {
    let mut previous = None;
    for (folder, depth) in entries {
        if depth > MAX_DEPTH {
            return Err(format!("Folders nest at most {MAX_DEPTH} levels deep"));
        }
        if let Some(previous) = previous
            && depth < previous
            && !(folder && depth + 1 == previous)
        {
            return Err("Invalid folder structure".into());
        }
        previous = Some(depth);
    }
    if previous != Some(0) {
        return Err("Invalid folder structure".into());
    }
    Ok(())
}

fn full_fill() -> f32 {
    1.0
}
fn is_full_fill(fill: &f32) -> bool {
    *fill == 1.0
}

impl Default for Layer {
    fn default() -> Self {
        Self {
            id: LayerId::new(),
            name: String::new(),
            visible: true,
            opacity: 1.0,
            csp_byte_opacity: false,
            fill: 1.0,
            blend: crate::blend::BlendMode::Normal,
            clipped: false,
            locked: false,
            alpha_locked: false,
            folder: false,
            depth: 0,
            collapsed: false,
            mask: None,
            effects: Vec::new(),
            effect_cache: Default::default(),
            adjustment: None,
            vector: None,
            text: None,
            material: None,
            pixels: Default::default(),
            display_reference: None,
        }
    }
}

impl Layer {
    pub(crate) fn has_effects(&self) -> bool {
        self.effects.iter().any(|e| e.enabled)
    }
    pub(crate) fn display_key(&self) -> usize {
        self.display_reference.as_ref().map_or(0, |r| r.key)
    }
    pub(crate) fn display_region_key(&self, rect: [usize; 4]) -> usize {
        if let Some(adjustment) = &self.adjustment {
            return adjustment.key();
        }
        self.display_reference
            .as_ref()
            .filter(|r| r.intersects(rect))
            .map_or(0, |_| self.display_key())
    }
    pub(crate) fn has_display_payload(&self, rect: [usize; 4]) -> bool {
        // An adjustment acts wherever there is something below it.
        if self.adjustment.is_some() {
            return true;
        }
        self.display_reference.as_ref().map_or_else(
            || self.pixels.region_has_payload(rect) && !self.mask_hides(rect),
            |r| r.intersects(rect),
        )
    }
    /// Whether an enabled mask hides all of `rect`, so the layer's pixels
    /// show nowhere there.
    pub(crate) fn mask_hides(&self, rect: [usize; 4]) -> bool {
        self.mask
            .as_ref()
            .is_some_and(|mask| mask.enabled && mask.coverage.region_is_clear(rect))
    }
    pub(crate) fn mask_coverage(&self, index: usize, width: usize) -> f32 {
        self.mask_coverage_at(index % width, index / width)
    }
    #[inline]
    fn mask_coverage_at(&self, x: usize, y: usize) -> f32 {
        self.mask
            .as_ref()
            .filter(|m| m.enabled)
            .map_or(1.0, |m| f32::from(m.coverage.at(x, y)) / 255.0)
    }
    pub(crate) fn pixel_alpha(&self, alpha: u8) -> f32 {
        if self.csp_byte_opacity {
            (alpha as f32 * self.opacity).floor() / 255.0
        } else {
            alpha as f32 / 255.0 * self.opacity
        }
    }
    pub fn raster(name: String, width: usize, height: usize) -> Self {
        Self {
            name,
            pixels: crate::pixels::Pixels::sparse(width, height),
            ..Self::default()
        }
    }
    pub fn new_folder(name: String) -> Self {
        Self {
            name,
            folder: true,
            ..Self::default()
        }
    }
}

/// Per-nesting-level compositing state: the finished backdrop plus the open
/// clipping group (base layer and the layers clipped to it).
#[derive(Clone, Copy)]
struct Level {
    out: [f32; 4],
    group: [f32; 4],
    mode: crate::blend::BlendMode,
    /// Alpha of this level's own contents, as if composited alone. Blend
    /// modes never change alpha, so this holds even for a pass-through level
    /// whose `out` also contains everything below it.
    coverage: f32,
    /// After an unclipped adjustment layer, layers clipped to it act through
    /// its mask and opacity (this strength) on everything below.
    adjust_base: Option<f32>,
}
#[derive(Clone, Copy)]
pub(crate) struct CompositeState {
    levels: [Level; MAX_DEPTH as usize + 1],
    top: usize,
}
const EMPTY_LEVEL: Level = Level {
    out: [0.0; 4],
    group: [0.0; 4],
    mode: crate::blend::BlendMode::Normal,
    coverage: 0.0,
    adjust_base: None,
};

fn commit(out: &mut [f32; 4], group: [f32; 4], mode: crate::blend::BlendMode) {
    let a = group[3];
    if a == 0.0 {
        return;
    }
    let backdrop_alpha = out[3];
    let source = [0, 1, 2].map(|c| group[c] / a);
    let backdrop = if backdrop_alpha > 0.0 {
        [0, 1, 2].map(|c| out[c] / backdrop_alpha)
    } else {
        [0.0; 3]
    };
    if let Some(base) = mode.glow_base() {
        // Glow: blend the alpha-weighted source at full strength over the
        // backdrop, rather than fading the blended result by alpha.
        let weighted = source.map(|c| c * a);
        let blended = base.pixel(backdrop, weighted);
        for c in 0..3 {
            out[c] = blended[c] * backdrop_alpha + weighted[c] * (1.0 - backdrop_alpha);
        }
        out[3] = a + backdrop_alpha * (1.0 - a);
        return;
    }
    let blended = mode.pixel(backdrop, source);
    for c in 0..3 {
        out[c] = out[c] * (1.0 - a)
            + source[c] * a * (1.0 - backdrop_alpha)
            + blended[c] * a * backdrop_alpha;
    }
    out[3] = a + backdrop_alpha * (1.0 - a);
}

impl Level {
    /// Add one straight-color source with effective alpha `a`.
    fn add(&mut self, color: [f32; 3], a: f32, clipped: bool, blend: crate::blend::BlendMode) {
        if clipped && let Some(strength) = self.adjust_base {
            let a = a * strength;
            commit(
                &mut self.out,
                [color[0] * a, color[1] * a, color[2] * a, a],
                blend,
            );
            self.coverage = a + self.coverage * (1.0 - a);
            return;
        }
        if !clipped {
            self.adjust_base = None;
            commit(&mut self.out, self.group, self.mode);
            self.coverage = self.contents_alpha();
            self.group = [color[0] * a, color[1] * a, color[2] * a, a];
            self.mode = blend;
        } else if a > 0.0 && self.group[3] > 0.0 {
            // Clipping edits group color while retaining the base alpha.
            let alpha = self.group[3];
            let base = [0, 1, 2].map(|c| self.group[c] / alpha);
            if let Some(glow) = blend.glow_base() {
                let blended = glow.pixel(base, color.map(|c| c * a));
                for (group, value) in self.group.iter_mut().zip(blended) {
                    *group = value * alpha;
                }
                return;
            }
            let blended = blend.pixel(base, color);
            for c in 0..3 {
                self.group[c] = ((1.0 - a) * base[c] + a * blended[c]) * alpha;
            }
        }
    }
    /// An adjustment layer: recolor the open clipping group when clipped,
    /// else everything composited so far, faded in by `t`.
    fn adjust(
        &mut self,
        adjustment: &crate::adjustment::Adjustment,
        t: f32,
        clipped: bool,
        blend: crate::blend::BlendMode,
    ) {
        let t = match (clipped, self.adjust_base) {
            (true, Some(strength)) => t * strength,
            (true, None) => t,
            (false, _) => {
                // Like any unclipped layer, it ends the open clipping group.
                commit(&mut self.out, self.group, self.mode);
                self.coverage = self.contents_alpha();
                self.group = [0.0; 4];
                self.mode = crate::blend::BlendMode::Normal;
                self.adjust_base = Some(t);
                t
            }
        };
        let target = if clipped && self.adjust_base.is_none() {
            &mut self.group
        } else {
            &mut self.out
        };
        let alpha = target[3];
        if t <= 0.0 || alpha <= 0.0 {
            return;
        }
        let color: [f32; 3] = std::array::from_fn(|c| target[c] / alpha);
        let adjusted = blend.pixel(color, adjustment.apply(color));
        for c in 0..3 {
            target[c] = (color[c] + (adjusted[c] - color[c]) * t) * alpha;
        }
    }
    fn finish(mut self) -> [f32; 4] {
        commit(&mut self.out, self.group, self.mode);
        self.out
    }
    /// `coverage` with the open group included.
    fn contents_alpha(&self) -> f32 {
        self.group[3] + self.coverage * (1.0 - self.group[3])
    }
    /// A pass-through folder's contents start from everything below them.
    fn passing_through(parent: &Level) -> Level {
        Level {
            out: parent.finish(),
            ..EMPTY_LEVEL
        }
    }
}
/// For each layer, bit `k` is set when the folder that owns nesting level
/// `k` above it is Pass through. Empty when the document has none, which
/// keeps ordinary compositing on its original path.
pub(crate) fn pass_plan(layers: &[Layer]) -> Vec<u16> {
    if !layers
        .iter()
        .any(|l| l.folder && l.blend == crate::blend::BlendMode::PassThrough)
    {
        return Vec::new();
    }
    // Folders follow their contents, so walk down from the top: a folder at
    // depth d owns level d + 1 for the entries below it until the next
    // entry at depth d or less.
    let mut owner = [false; MAX_DEPTH as usize + 1];
    let mut plan = vec![0u16; layers.len()];
    for (i, layer) in layers.iter().enumerate().rev() {
        plan[i] = (1..=layer.depth as usize)
            .filter(|k| owner[k - 1])
            .fold(0, |bits, k| bits | 1 << k);
        if layer.folder {
            owner[layer.depth as usize] = layer.blend == crate::blend::BlendMode::PassThrough;
        }
    }
    plan
}
/// How the strip compositor treats a layer inside one storage block.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Skip {
    /// Read and composite its pixels.
    Draw,
    /// Nothing here: add a zero source, which still closes a clipping group.
    Zero,
    /// Nothing here, and a zero source would change nothing: a clipped
    /// layer (clipping a zero source is a no-op), or an unclipped one right
    /// after another empty unclipped layer at its depth (the group is
    /// already zero, and the mode of a zero group is never used).
    Nothing,
}
fn skip_plan(layers: &[Layer], empty: impl Fn(&Layer) -> bool) -> Vec<Skip> {
    let mut plan = Vec::with_capacity(layers.len());
    // Depth of the previous entry when it was empty and left its level's
    // group zero.
    let mut zero_group: Option<u8> = None;
    let mut previous_depth: Option<u8> = None;
    for layer in layers {
        let skip = if layer.folder || !empty(layer) {
            zero_group = None;
            Skip::Draw
        } else if previous_depth == Some(layer.depth)
            && (layer.clipped || zero_group == Some(layer.depth))
        {
            Skip::Nothing
        } else {
            zero_group = (!layer.clipped).then_some(layer.depth);
            Skip::Zero
        };
        previous_depth = (!layer.folder).then_some(layer.depth);
        plan.push(skip);
    }
    plan
}
/// Open nesting levels up to `depth` for the layer at `global` index.
fn open_levels(levels: &mut [Level], top: &mut usize, depth: usize, plan: &[u16], global: usize) {
    while *top < depth {
        *top += 1;
        levels[*top] = if plan.get(global).is_some_and(|bits| bits & 1 << *top != 0) {
            Level::passing_through(&levels[*top - 1])
        } else {
            EMPTY_LEVEL
        };
    }
}
/// Dissolve's per-pixel threshold: a fixed hash of the pixel index.
fn dissolve(index: usize, a: f32) -> f32 {
    let mut h = index as u64 ^ 0x9e37_79b9_7f4a_7c15;
    h = (h ^ (h >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    h = (h ^ (h >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    h ^= h >> 31;
    let threshold = (h >> 40) as f32 / (1u64 << 24) as f32;
    // A hidden pixel keeps the signed zero of `a`, matching skipped empty layers.
    if a > threshold { 1.0 } else { a * 0.0 }
}

pub(crate) fn default_dpi() -> f64 {
    72.0
}
pub(crate) fn validate_dpi(dpi: f64) -> Result<(), String> {
    if dpi.is_finite() && (1.0..=12000.0).contains(&dpi) {
        Ok(())
    } else {
        Err("Resolution must be between 1 and 12,000 DPI".into())
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Document {
    pub version: u32,
    pub width: usize,
    pub height: usize,
    /// Pixels per inch; changing this does not resample artwork.
    #[serde(default = "default_dpi")]
    pub dpi: f64,
    #[serde(skip, default = "crate::canvas_size::Frame::identity")]
    pub(crate) canvas_frame: std::sync::Arc<crate::canvas_size::Frame>,
    pub layers: Vec<Layer>,
    pub active: usize,
    #[serde(default)]
    pub selection: Option<crate::selection::Selection>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selection_mask: Option<crate::selection_mask::Mask>,
    /// Guides carried by saved files only. The app owns the live guides and
    /// fills this on the copy it saves, so undo never reverts guide edits.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guides: Option<Box<crate::guides::Guides>>,
    /// Reference layers carried by saved files only, like the guides. Their
    /// images travel in the project trailer, never in serialized metadata.
    #[serde(default, skip_serializing)]
    pub references: Vec<crate::reference_layers::ReferenceLayer>,
    /// The surface brushes paint on; None is a smooth canvas.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paper: Option<crate::paper::Paper>,
    /// Where paint is still wet. Never saved.
    #[serde(skip)]
    pub(crate) wet: crate::wet::Field,
}

/// Final 8-bit output of a premultiplied composite value: on white paper, or
/// straight RGBA matching `composite_bounds(.., transparent = true)`.
fn output_pixel(value: [f32; 4], transparent: bool) -> [u8; 4] {
    if transparent {
        let alpha = value[3];
        let color = |c: f32| {
            if alpha > 0.0 {
                (c / alpha).round() as u8
            } else {
                0
            }
        };
        [
            color(value[0]),
            color(value[1]),
            color(value[2]),
            (alpha * 255.0).round() as u8,
        ]
    } else {
        [
            (value[0] + 255.0 * (1.0 - value[3])).round() as u8,
            (value[1] + 255.0 * (1.0 - value[3])).round() as u8,
            (value[2] + 255.0 * (1.0 - value[3])).round() as u8,
            255,
        ]
    }
}
impl Document {
    pub fn new(width: usize, height: usize) -> Self {
        Self {
            version: 1,
            width,
            height,
            dpi: 300.0,
            canvas_frame: crate::canvas_size::Frame::identity(),
            layers: vec![Layer::raster("Layer 1".into(), width, height)],
            active: 0,
            selection: None,
            selection_mask: None,
            guides: None,
            references: Vec::new(),
            paper: None,
            wet: Default::default(),
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        self.validate_metadata()?;
        let budget = if self.version == 2 {
            MAX_MASKED_PAYLOAD
        } else {
            MAX_PAYLOAD
        };
        if self
            .layers
            .iter()
            .map(|l| {
                l.pixels.payload_bound()
                    + l.mask.as_ref().map_or(0, |m| m.coverage.payload_bytes())
                    + l.material
                        .as_ref()
                        .and_then(|m| m.relief.as_ref())
                        .map_or(0, |r| r.payload_bytes())
            })
            .sum::<usize>()
            > budget
        {
            return Err(format!(
                "Raster and mask coverage exceed the {} MiB document payload budget",
                budget >> 20
            ));
        }
        if self
            .layers
            .iter()
            .filter_map(|l| l.mask.as_ref())
            .any(|m| m.coverage.dimensions() != [self.width, self.height])
        {
            return Err("Layer mask geometry differs from document".into());
        }
        for layer in &self.layers {
            if let Some(material) = &layer.material {
                crate::material::validate(material, [self.width, self.height], layer.folder)?;
            }
        }
        if self.layers.iter().any(|layer| {
            layer.pixels.len()
                != if layer.folder {
                    0
                } else {
                    self.width * self.height * 4
                }
        }) {
            return Err("Invalid layer pixel length".into());
        }
        Ok(())
    }
    /// Folder contents sit directly below their folder, one level deeper;
    /// nesting may only close at a folder row, and the top entry is at the root.
    pub fn validate_structure(&self) -> Result<(), String> {
        validate_folder_depths(self.layers.iter().map(|layer| (layer.folder, layer.depth)))?;
        for (index, layer) in self.layers.iter().enumerate() {
            if self.layers[..index]
                .iter()
                .any(|other| other.id == layer.id)
            {
                return Err("Duplicate layer identity".into());
            }
        }
        if self.raster_count() > MAX_RASTER_LAYERS {
            return Err(format!("A document holds up to {MAX_RASTER_LAYERS} layers"));
        }
        Ok(())
    }
    /// Whether compositing goes through `layer_effects::expand`: some layer
    /// draws effects or has fill opacity.
    pub(crate) fn needs_expansion(&self) -> bool {
        self.layers.iter().any(|l| {
            l.has_effects() || l.fill != 1.0 || l.material.as_ref().is_some_and(|m| m.lit())
        })
    }
    pub fn validate_metadata(&self) -> Result<(), String> {
        validate_dpi(self.dpi)?;
        if let Some(paper) = &self.paper {
            paper.validate()?;
        }
        if self.layers.iter().any(|l| l.display_reference.is_some()) {
            return Err("Reference display snapshots cannot be saved as artwork".into());
        }
        if !matches!(self.version, 1 | 2)
            || self.width == 0
            || self.height == 0
            || self.width > MAX_SIDE
            || self.height > MAX_SIDE
            || self.width * self.height > MAX_PIXELS
            || self.layers.is_empty()
            || self.layers.len() > MAX_ENTRIES
            || self.active >= self.layers.len()
        {
            return Err("Unsupported document dimensions, version or layer count".into());
        }
        if self.version == 1 && self.layers.iter().any(|l| l.mask.is_some()) {
            return Err("Layer masks require document version 2".into());
        }
        for layer in &self.layers {
            if layer.effects.len() > MAX_EFFECTS {
                return Err("Invalid layer effects".into());
            }
            for effect in &layer.effects {
                effect.validate()?;
            }
            if let Some(adjustment) = &layer.adjustment {
                if layer.folder {
                    return Err("Folders cannot be adjustment layers".into());
                }
                adjustment.validate()?;
            }
            if let Some(vector) = &layer.vector {
                if layer.folder || layer.adjustment.is_some() {
                    return Err("Only raster layers can be vector layers".into());
                }
                vector.validate()?;
            }
            if let Some(text) = &layer.text {
                if layer.folder || layer.adjustment.is_some() || layer.vector.is_some() {
                    return Err("Only raster layers can be text layers".into());
                }
                text.validate()?;
            }
            if let Some(material) = &layer.material
                && (!crate::Dudel::takes_material(layer) || material.settings.validate().is_err())
            {
                return Err("Invalid layer material".into());
            }
            if layer.name.len() > 4096
                || (layer.folder && layer.csp_byte_opacity)
                || !layer.opacity.is_finite()
                || !(0.0..=1.0).contains(&layer.opacity)
                || !(0.0..=1.0).contains(&layer.fill)
            {
                return Err("Invalid layer data".into());
            }
        }
        if self
            .selection
            .is_some_and(|s| !s.valid(self.width, self.height))
        {
            return Err("Invalid selection bounds".into());
        }
        if let Some(mask) = &self.selection_mask
            && (mask.dimensions() != [self.width, self.height] || mask.bounds() != self.selection)
        {
            return Err("Selection mask geometry or bounds differ from document".into());
        }
        self.validate_structure()
    }

    pub fn add_layer(&mut self) {
        let number = self.raster_count() + 1;
        self.layers.push(Layer::raster(
            format!("Layer {number}"),
            self.width,
            self.height,
        ));
        self.active = self.layers.len() - 1;
    }

    pub fn raster_count(&self) -> usize {
        self.layers.iter().filter(|l| !l.folder).count()
    }
    /// Room for `rasters` more layers and `entries` more stack entries.
    pub fn has_room(&self, rasters: usize, entries: usize) -> bool {
        self.raster_count() + rasters <= MAX_RASTER_LAYERS
            && self.layers.len() + entries <= MAX_ENTRIES
    }
    /// Index range of an entry and, for folders, everything inside it.
    pub fn block(&self, index: usize) -> std::ops::Range<usize> {
        let mut start = index;
        if self.layers[index].folder {
            let depth = self.layers[index].depth;
            while start > 0 && self.layers[start - 1].depth > depth {
                start -= 1;
            }
        }
        start..index + 1
    }
    /// The folder directly containing an entry.
    pub fn parent(&self, index: usize) -> Option<usize> {
        let depth = self.layers[index].depth;
        if depth == 0 {
            return None;
        }
        (index + 1..self.layers.len()).find(|&i| self.layers[i].depth < depth)
    }
    /// Visible itself and inside visible folders.
    pub fn shown(&self, index: usize) -> bool {
        let mut current = Some(index);
        while let Some(i) = current {
            if !self.layers[i].visible {
                return false;
            }
            current = self.parent(i);
        }
        true
    }
    /// Why the active entry cannot receive pixels, if it cannot.
    pub fn paint_blocker(&self) -> Option<&'static str> {
        self.paint_blocker_for(self.active)
    }
    pub fn paint_blocker_for(&self, index: usize) -> Option<&'static str> {
        self.raster_blocker_for(index).or_else(|| {
            if self.layers[index].text.is_some() {
                return Some(crate::text_layer::BLOCKER);
            }
            self.layers[index].vector.as_ref().map(|_| {
                "This is a vector layer — draw on it with the brush, or rasterize it (Layer menu) to edit its pixels"
            })
        })
    }
    /// `paint_blocker_for` without refusing vector layers, for operations
    /// that move their strokes along (Move).
    pub fn raster_blocker_for(&self, index: usize) -> Option<&'static str> {
        let Some(layer) = self.layers.get(index) else {
            return Some("Selected layer no longer exists");
        };
        if layer.folder {
            Some("A folder is selected — select a layer inside it to draw")
        } else if layer.locked {
            Some("Layer is locked — unlock it to draw")
        } else if layer.adjustment.is_some() {
            Some("An adjustment layer has no pixels — paint on its mask or another layer")
        } else if !self.shown(index) {
            Some("Layer is hidden — show it (and its folder) to draw")
        } else {
            None
        }
    }

    /// Composite one pixel. Folders are isolated groups: their contents are
    /// combined first, then blended into the parent level with the folder
    /// visibility, opacity and blend mode. `levels` is caller-owned scratch.
    fn composite_pixel(&self, index: usize, levels: &mut [Level], plan: &[u16]) -> [f32; 4] {
        levels[0] = EMPTY_LEVEL;
        self.composite_layers(index, levels, 0, &self.layers, 0, plan);
        levels[0].finish()
    }
    /// Composite `layers`, which start at index `start` of the document's
    /// layers, onto the open levels. `plan` is `pass_plan` for the document.
    fn composite_layers(
        &self,
        index: usize,
        levels: &mut [Level],
        top: usize,
        layers: &[Layer],
        start: usize,
        plan: &[u16],
    ) -> usize {
        let at = [index % self.width, index / self.width];
        self.composite_layers_at(index, at, levels, top, layers, start, plan)
    }
    /// `composite_layers` with the pixel's coordinates already known, so
    /// each layer reads its pixel and mask without dividing the index again.
    #[allow(clippy::too_many_arguments)]
    fn composite_layers_at(
        &self,
        index: usize,
        [x, y]: [usize; 2],
        levels: &mut [Level],
        mut top: usize,
        layers: &[Layer],
        start: usize,
        plan: &[u16],
    ) -> usize {
        for (offset, layer) in layers.iter().enumerate() {
            let depth = layer.depth as usize;
            open_levels(levels, &mut top, depth, plan, start + offset);
            if layer.folder && layer.blend == crate::blend::BlendMode::PassThrough {
                // The contents were composited onto everything below; fade
                // that result in by the folder's visibility, opacity and mask.
                let parent = levels[depth];
                let below = parent.finish();
                let (content, coverage) = if top > depth {
                    top -= 1;
                    let inner = levels[top + 1];
                    (inner.finish(), inner.contents_alpha())
                } else {
                    (below, 0.0)
                };
                let t = if layer.visible {
                    layer.opacity * layer.mask_coverage_at(x, y)
                } else {
                    0.0
                };
                // Express the faded result as a normal layer over `below` whose
                // alpha is the folder's own coverage, so layers clipped to the
                // folder clip to its contents. Normal compositing of this
                // group reproduces below + (content - below) * t exactly.
                let alpha = coverage * t;
                let faded: [f32; 4] =
                    std::array::from_fn(|c| below[c] + (content[c] - below[c]) * t);
                levels[depth] = if alpha > 0.0 {
                    let mut group: [f32; 4] =
                        std::array::from_fn(|c| faded[c] - below[c] * (1.0 - alpha));
                    group[3] = alpha;
                    Level {
                        out: below,
                        group,
                        mode: crate::blend::BlendMode::Normal,
                        coverage: parent.contents_alpha(),
                        adjust_base: None,
                    }
                } else {
                    // Nothing of its own to clip to (say, only adjustment
                    // layers): keep the recolored result as the backdrop.
                    Level {
                        out: faded,
                        coverage: parent.contents_alpha(),
                        ..EMPTY_LEVEL
                    }
                };
                continue;
            }
            if let Some(adjustment) = &layer.adjustment {
                let t = if layer.visible {
                    layer.opacity * layer.mask_coverage_at(x, y)
                } else {
                    0.0
                };
                levels[depth].adjust(adjustment, t, layer.clipped, layer.blend);
                continue;
            }
            if layer.folder {
                let content = if top > depth {
                    top -= 1;
                    levels[top + 1].finish()
                } else {
                    [0.0; 4]
                };
                let a = if layer.visible {
                    content[3] * layer.opacity * layer.mask_coverage_at(x, y)
                } else {
                    0.0
                };
                let color = if content[3] > 0.0 {
                    [0, 1, 2].map(|c| content[c] / content[3])
                } else {
                    [0.0; 3]
                };
                levels[depth].add(color, a, layer.clipped, layer.blend);
            } else {
                let pixel = layer
                    .display_reference
                    .as_ref()
                    .map_or_else(|| layer.pixels.rgba_at(self.width, x, y), |r| r.pixel(x, y));
                let mut a = if layer.visible {
                    layer.pixel_alpha(pixel[3]) * layer.mask_coverage_at(x, y)
                } else {
                    0.0
                };
                if layer.blend == crate::blend::BlendMode::Dissolve {
                    a = dissolve(index, a);
                }
                levels[depth].add(
                    [pixel[0] as f32, pixel[1] as f32, pixel[2] as f32],
                    a,
                    layer.clipped,
                    layer.blend,
                );
            }
        }
        top
    }
    pub(crate) fn prefix_state(&self, index: usize) -> CompositeState {
        self.prefix_state_planned(index, &pass_plan(&self.layers))
    }
    fn prefix_state_planned(&self, index: usize, plan: &[u16]) -> CompositeState {
        let mut state = CompositeState {
            levels: [EMPTY_LEVEL; MAX_DEPTH as usize + 1],
            top: 0,
        };
        state.top = self.composite_layers(
            index,
            &mut state.levels,
            0,
            &self.layers[..self.active],
            0,
            plan,
        );
        state
    }
    pub(crate) fn prefix_states(&self, rect: [usize; 4]) -> Vec<CompositeState> {
        // Keep empty bases: they terminate previous clipping groups. Only merge
        // consecutive empty rasters at the same depth; folders always execute.
        let pass = pass_plan(&self.layers);
        let mut plan: Vec<(usize, &Layer, bool)> = Vec::with_capacity(self.active);
        for (global, layer) in self.layers[..self.active].iter().enumerate() {
            let empty = !layer.folder
                && (!layer.visible || layer.opacity == 0.0 || !layer.has_display_payload(rect));
            if empty
                && let Some((previous_global, previous, previous_empty)) = plan.last_mut()
                && *previous_empty
                && previous.depth == layer.depth
                && pass.get(*previous_global) == pass.get(global)
            {
                if !layer.clipped {
                    *previous = layer;
                    *previous_global = global;
                }
                continue;
            }
            plan.push((global, layer, empty));
        }
        let mut states = Vec::with_capacity((rect[2] - rect[0]) * (rect[3] - rect[1]));
        for y in rect[1]..rect[3] {
            for x in rect[0]..rect[2] {
                let index = y * self.width + x;
                if plan.len() == self.active {
                    states.push(self.prefix_state_planned(index, &pass));
                    continue;
                }
                let mut state = CompositeState {
                    levels: [EMPTY_LEVEL; MAX_DEPTH as usize + 1],
                    top: 0,
                };
                for &(global, layer, empty) in &plan {
                    if empty {
                        let depth = layer.depth as usize;
                        open_levels(&mut state.levels, &mut state.top, depth, &pass, global);
                        let alpha = if layer.visible {
                            0.0 * layer.opacity
                        } else {
                            0.0
                        };
                        state.levels[depth].add([0.0; 3], alpha, layer.clipped, layer.blend);
                    } else {
                        state.top = self.composite_layers(
                            index,
                            &mut state.levels,
                            state.top,
                            std::slice::from_ref(layer),
                            global,
                            &pass,
                        );
                    }
                }
                states.push(state);
            }
        }
        states
    }
    /// Finish a pixel from its prefix state. `plan` is `pass_plan` for this
    /// document, computed once per region by the caller.
    pub(crate) fn composite_from_prefix(
        &self,
        index: usize,
        mut state: CompositeState,
        transparent: bool,
        plan: &[u16],
    ) -> [u8; 4] {
        self.composite_layers(
            index,
            &mut state.levels,
            state.top,
            &self.layers[self.active..],
            self.active,
            plan,
        );
        output_pixel(state.levels[0].finish(), transparent)
    }
    fn composite_output(&self, transparent: bool) -> Vec<u8> {
        self.composite_bounds(0, 0, self.width, self.height, transparent)
    }
    /// Row-packed RGBA for a clamped, half-open document rectangle.
    pub fn composite_region(
        &self,
        left: usize,
        top: usize,
        right: usize,
        bottom: usize,
    ) -> Vec<u8> {
        self.composite_bounds(
            left.min(self.width),
            top.min(self.height),
            right.min(self.width),
            bottom.min(self.height),
            false,
        )
    }
    fn composite_bounds(
        &self,
        left: usize,
        top: usize,
        right: usize,
        bottom: usize,
        transparent: bool,
    ) -> Vec<u8> {
        self.composite_bounds_checked([left, top, right, bottom], transparent, || Ok(()))
            .expect("Uncancelled composite cannot fail")
    }
    pub(crate) fn composite_bounds_checked(
        &self,
        rect: [usize; 4],
        transparent: bool,
        check: impl Fn() -> Result<(), String> + Sync,
    ) -> Result<Vec<u8>, String> {
        self.composite_grid(rect, 1, transparent, check)
    }
    /// A quick, blocky stand-in for `composite_bounds_checked`: one pixel
    /// composited per `step` square (squares aligned to the canvas), spread
    /// over its square. Reducing it by `step` gives exactly those pixels.
    pub(crate) fn composite_sampled_cancelable(
        &self,
        rect: [usize; 4],
        step: usize,
        cancel: &std::sync::atomic::AtomicBool,
        transparent: bool,
    ) -> Result<Vec<u8>, String> {
        self.composite_grid(rect, step.max(1), transparent, || {
            crate::fill::check_cancel(Some(cancel))
        })
    }
    fn composite_grid(
        &self,
        rect: [usize; 4],
        step: usize,
        transparent: bool,
        check: impl Fn() -> Result<(), String> + Sync,
    ) -> Result<Vec<u8>, String> {
        check()?;
        if self.needs_expansion() {
            let expanded = crate::layer_effects::expand(self, false, &|| check().is_err())?;
            return expanded.composite_grid(rect, step, transparent, check);
        }
        let [left, top, right, bottom] = rect;
        let width = right.saturating_sub(left);
        let height = bottom.saturating_sub(top);
        if width == 0 || height == 0 {
            return Ok(vec![0; width * height * 4]);
        }
        // The middle of each step square, kept inside the rectangle.
        let samples = |from: usize, to: usize| -> Vec<usize> {
            if step == 1 {
                return (from..to).collect();
            }
            (from / step * step..to)
                .step_by(step)
                .map(|start| (start + step / 2).clamp(from, to - 1))
                .collect()
        };
        let (xs, ys) = (samples(left, right), samples(top, bottom));
        let values = self.composite_samples(rect, &xs, &ys, transparent, &check)?;
        if step == 1 {
            return Ok(values);
        }
        let mut out = vec![0; width * height * 4];
        for (row, y) in out.chunks_exact_mut(width * 4).zip(top..) {
            let source = &values[(y / step - top / step) * xs.len() * 4..][..xs.len() * 4];
            for (pixel, x) in row.chunks_exact_mut(4).zip(left..) {
                pixel.copy_from_slice(&source[(x / step - left / step) * 4..][..4]);
            }
        }
        Ok(out)
    }
    /// The composite at each (x, y) of `xs` by `ys` (ascending, inside
    /// `rect`), row by row.
    fn composite_samples(
        &self,
        rect: [usize; 4],
        xs: &[usize],
        ys: &[usize],
        transparent: bool,
        check: &(impl Fn() -> Result<(), String> + Sync),
    ) -> Result<Vec<u8>, String> {
        let [left, top, right, bottom] = rect;
        let width = xs.len();
        let mut out = vec![0; width * ys.len() * 4];
        // Per 256px storage block, which raster layers add nothing there.
        // Sparse layers then cost no pixel reads outside the blocks they paint.
        const BLOCK: usize = crate::pixels::SIDE;
        let (first_column, first_row_block) = (left / BLOCK, top / BLOCK);
        let block_columns = (right - 1) / BLOCK - first_column + 1;
        let block_rows = (bottom - 1) / BLOCK - first_row_block + 1;
        let empty: Vec<Vec<Skip>> = (0..block_rows * block_columns)
            .map(|block| {
                let bx = (first_column + block % block_columns) * BLOCK;
                let by = (first_row_block + block / block_columns) * BLOCK;
                let area = [
                    bx.max(left),
                    by.max(top),
                    (bx + BLOCK).min(right),
                    (by + BLOCK).min(bottom),
                ];
                skip_plan(&self.layers, |layer| {
                    !layer.visible || layer.opacity == 0.0 || !layer.has_display_payload(area)
                })
            })
            .collect();
        let sparse = empty
            .iter()
            .any(|block| block.iter().any(|skip| *skip != Skip::Draw));
        let pass = pass_plan(&self.layers);
        // Short strips of a row go through the layers one layer at a time,
        // so each layer's pixels and mask are read in order instead of every
        // layer being visited for each pixel in turn. Each pixel keeps its own
        // level stack and sees exactly the steps it would on its own.
        const STRIP: usize = 64;
        let rows = |chunk: &mut [u8], first_row: usize| {
            let mut stacks = vec![([EMPTY_LEVEL; MAX_DEPTH as usize + 1], 0); STRIP];
            for (out, &y) in chunk.chunks_exact_mut(width * 4).zip(&ys[first_row..]) {
                check()?;
                let mut start = 0;
                while start < width {
                    // Strips never cross a storage block.
                    let mut end = start + 1;
                    while end < width && end - start < STRIP && xs[end] / BLOCK == xs[start] / BLOCK
                    {
                        end += 1;
                    }
                    let strip = &xs[start..end];
                    let stacks = &mut stacks[..strip.len()];
                    let skip = sparse.then(|| {
                        &empty[(y / BLOCK - first_row_block) * block_columns + strip[0] / BLOCK
                            - first_column]
                    });
                    for (levels, open) in stacks.iter_mut() {
                        levels[0] = EMPTY_LEVEL;
                        *open = 0;
                    }
                    for (global, layer) in self.layers.iter().enumerate() {
                        let depth = layer.depth as usize;
                        // A layer with nothing here adds a zero source without reading
                        // pixels: the same as a transparent pixel, and it still closes
                        // clipping groups.
                        let skip = skip.map_or(Skip::Draw, |skip| skip[global]);
                        if skip == Skip::Nothing {
                            continue;
                        }
                        if skip == Skip::Zero {
                            for (levels, open) in stacks.iter_mut() {
                                open_levels(levels, open, depth, &pass, global);
                                levels[depth].add([0.0; 3], 0.0, layer.clipped, layer.blend);
                            }
                            continue;
                        }
                        for ((levels, open), &x) in stacks.iter_mut().zip(strip) {
                            *open = self.composite_layers_at(
                                y * self.width + x,
                                [x, y],
                                levels,
                                *open,
                                std::slice::from_ref(layer),
                                global,
                                &pass,
                            );
                        }
                    }
                    for (offset, (levels, _)) in stacks.iter().enumerate() {
                        let value = levels[0].finish();
                        let pixel = &mut out[(start + offset) * 4..][..4];
                        pixel.copy_from_slice(&output_pixel(value, transparent));
                    }
                    start = end;
                }
            }
            Ok::<(), String>(())
        };
        // Small brush regions stay on the calling thread; full refreshes
        // (undo, layer edits, export) and quick passes over large areas
        // split rows across cores.
        let height = ys.len();
        let threads = if (right - left) * (bottom - top) >= 1 << 16 && width * height >= 64 {
            std::thread::available_parallelism().map_or(1, |n| n.get().min(16))
        } else {
            1
        };
        if threads <= 1 {
            rows(&mut out, 0)?;
        } else {
            // Several bands a core, taken as cores free up: artwork is rarely
            // spread evenly, and a band of busy rows would hold up the rest.
            let band = height.div_ceil(threads * 8).max(16);
            let bands: Vec<_> = out
                .chunks_mut(band * width * 4)
                .map(std::sync::Mutex::new)
                .collect();
            let results = crate::parallel::run(bands.len(), |index| {
                rows(&mut bands[index].lock().unwrap(), index * band)
            });
            drop(bands);
            for result in results {
                result.expect("Composite worker panicked")?;
            }
        }
        check()?;
        Ok(out)
    }
    pub fn composite(&self) -> Vec<u8> {
        self.composite_region(0, 0, self.width, self.height)
    }
    pub(crate) fn composite_rgba_pixel(&self, index: usize) -> [u8; 4] {
        self.composite_display_pixel(index, false)
    }
    /// One composited pixel: opaque on white paper, or straight RGBA when
    /// `transparent` (as the display shows it over a checkerboard).
    pub(crate) fn composite_display_pixel(&self, index: usize, transparent: bool) -> [u8; 4] {
        let value = self.composite_pixel(
            index,
            &mut [EMPTY_LEVEL; MAX_DEPTH as usize + 1],
            &pass_plan(&self.layers),
        );
        output_pixel(value, transparent)
    }
    /// Red, green and blue counts of what shows below layer `index`, on
    /// white paper, from at most about `samples` evenly spread pixels.
    /// Folders the layer sits in are left open, so their contents below it
    /// count as if outside them.
    pub(crate) fn histogram_below(
        &self,
        index: usize,
        samples: usize,
        cancel: &dyn Fn() -> bool,
    ) -> Result<[[u32; 256]; 3], String> {
        let mut below = self.clone();
        below.layers.truncate(index.min(self.layers.len()));
        below.active = 0;
        // Headers follow their contents, so walking down from the top tells
        // which of the remaining layers are still inside a remaining folder.
        let mut open: Vec<u8> = Vec::new();
        for layer in below.layers.iter_mut().rev() {
            while open.last().is_some_and(|d| *d >= layer.depth) {
                open.pop();
            }
            let depth = layer.depth;
            layer.depth = open.len() as u8;
            if layer.folder {
                open.push(depth);
            }
        }
        let mut counts = [[0u32; 256]; 3];
        if below.layers.is_empty() {
            counts
                .iter_mut()
                .for_each(|c| c[255] = samples.max(1) as u32);
            return Ok(counts);
        }
        let below = if below.needs_expansion() {
            crate::layer_effects::expand(&below, false, cancel)?
        } else {
            below
        };
        let step = ((below.width * below.height) as f64 / samples.max(1) as f64)
            .sqrt()
            .max(1.0) as usize;
        let plan = pass_plan(&below.layers);
        let mut levels = [EMPTY_LEVEL; MAX_DEPTH as usize + 1];
        for y in (step / 2..below.height).step_by(step) {
            if cancel() {
                return Err("Canceled".into());
            }
            for x in (step / 2..below.width).step_by(step) {
                let value = below.composite_pixel(y * below.width + x, &mut levels, &plan);
                let pixel = output_pixel(value, false);
                for (channel, count) in counts.iter_mut().enumerate() {
                    count[usize::from(pixel[channel])] += 1;
                }
            }
        }
        Ok(counts)
    }
    /// Display region composite; `transparent` keeps alpha instead of paper.
    pub(crate) fn composite_display_region(&self, rect: [usize; 4], transparent: bool) -> Vec<u8> {
        self.composite_bounds(
            rect[0].min(self.width),
            rect[1].min(self.height),
            rect[2].min(self.width),
            rect[3].min(self.height),
            transparent,
        )
    }
    pub(crate) fn composite_region_cancelable(
        &self,
        rect: [usize; 4],
        cancel: &std::sync::atomic::AtomicBool,
        transparent: bool,
    ) -> Result<Vec<u8>, String> {
        self.composite_bounds_checked(rect, transparent, || {
            crate::fill::check_cancel(Some(cancel))
        })
    }
    pub fn composite_transparent(&self) -> Vec<u8> {
        self.composite_output(true)
    }
    pub fn pick_color(&self, x: usize, y: usize) -> Option<[u8; 3]> {
        if x >= self.width || y >= self.height {
            return None;
        }
        if self.needs_expansion() {
            return crate::layer_effects::expand(self, false, &|| false)
                .ok()?
                .pick_color(x, y);
        }
        let value = self.composite_pixel(
            y * self.width + x,
            &mut [EMPTY_LEVEL; MAX_DEPTH as usize + 1],
            &pass_plan(&self.layers),
        );
        Some([0, 1, 2].map(|c| (value[c] + 255.0 * (1.0 - value[3])).round() as u8))
    }

    /// Four-connected fill against active-layer pixels. Comparing premultiplied
    /// color makes fully transparent pixels equivalent regardless of hidden RGB.
    pub fn fill(
        &mut self,
        x: usize,
        y: usize,
        color: [u8; 3],
        opacity: f32,
        tolerance: u8,
    ) -> Result<bool, String> {
        self.fill_using(x, y, color, opacity, tolerance, crate::fill::Source::Active)
    }

    pub fn fill_using(
        &mut self,
        x: usize,
        y: usize,
        color: [u8; 3],
        opacity: f32,
        tolerance: u8,
        source: crate::fill::Source,
    ) -> Result<bool, String> {
        self.fill_request(
            crate::fill::Request {
                point: [x, y],
                color,
                opacity,
                tolerance,
                source,
                options: crate::fill::Options::default(),
            },
            None,
        )
    }
    pub(crate) fn fill_request(
        &mut self,
        request: crate::fill::Request,
        cancel: Option<&std::sync::atomic::AtomicBool>,
    ) -> Result<bool, String> {
        let crate::fill::Request {
            point: [x, y],
            color,
            opacity,
            tolerance,
            source,
            options,
        } = request;
        crate::fill::check_cancel(cancel)?;
        options.validate()?;
        if !opacity.is_finite() {
            return Err("Fill opacity must be finite".into());
        }
        if x >= self.width || y >= self.height {
            return Ok(false);
        }
        if self.selection.is_some_and(|s| !s.contains(x, y)) {
            return Ok(false);
        }
        if self
            .selection_mask
            .as_ref()
            .is_some_and(|m| m.at(x, y) == 0)
        {
            return Ok(false);
        }
        if let Some(blocker) = self.paint_blocker() {
            return Err(blocker.into());
        }
        let opacity = opacity.clamp(0.0, 1.0);
        let reference_layer = if let crate::fill::Source::Layer(id) = source {
            let layer = self
                .layers
                .iter()
                .find(|layer| layer.id == id)
                .ok_or("Fill reference layer no longer exists")?;
            if layer.folder {
                return Err("Choose a raster fill reference".into());
            }
            Some(layer)
        } else {
            None
        };
        if opacity == 0.0 {
            return Ok(false);
        }
        // A vector layer chosen as the reference is all the walls there are.
        let wall_layer = reference_layer.filter(|layer| layer.vector.is_some());
        let lines_only = options.line_centres && wall_layer.is_some();
        let walls = if options.line_centres {
            let mut walls = vec![0u64; (self.width * self.height).div_ceil(64)];
            for (index, layer) in self.layers.iter().enumerate() {
                let chosen = wall_layer.map_or(self.shown(index), |wall| wall.id == layer.id);
                if let Some(vector) = layer.vector.as_ref().filter(|_| chosen) {
                    crate::fill::check_cancel(cancel)?;
                    vector.mark_centres(&mut walls, [self.width, self.height]);
                }
            }
            if crate::fill::contains(&walls, y * self.width + x) {
                return Ok(false);
            }
            Some(walls)
        } else {
            None
        };
        let reference = match source {
            crate::fill::Source::Active => self.layers[self.active].pixels.clone(),
            crate::fill::Source::Visible if options.line_centres => {
                // The lines' own pixels must not stop the fill short of their middles.
                let mut plain = self.clone();
                for layer in plain.layers.iter_mut().filter(|l| l.vector.is_some()) {
                    layer.visible = false;
                }
                plain
                    .composite_bounds_checked([0, 0, self.width, self.height], true, || {
                        crate::fill::check_cancel(cancel)
                    })?
                    .into()
            }
            crate::fill::Source::Visible => self
                .composite_bounds_checked([0, 0, self.width, self.height], true, || {
                    crate::fill::check_cancel(cancel)
                })?
                .into(),
            crate::fill::Source::Layer(_) => reference_layer.unwrap().pixels.clone(),
        };
        crate::fill::check_cancel(cancel)?;
        let same_reference = source == crate::fill::Source::Active
            || source == crate::fill::Source::Layer(self.layers[self.active].id);
        let layer = &mut self.layers[self.active];
        let alpha_locked = layer.alpha_locked;
        let seed = (y * self.width + x) * 4;
        let target = reference.rgba(seed / 4);
        let equivalent = |pixel: &[u8]| {
            if pixel[3].abs_diff(target[3]) > tolerance {
                return false;
            }
            (0..3).all(|c| {
                let a = (u16::from(pixel[c]) * u16::from(pixel[3]) / 255) as u8;
                let b = (u16::from(target[c]) * u16::from(target[3]) / 255) as u8;
                a.abs_diff(b) <= tolerance
            })
        };
        let eligible = |index: usize| {
            self.selection
                .is_none_or(|s| s.contains(index % self.width, index / self.width))
                && self
                    .selection_mask
                    .as_ref()
                    .is_none_or(|m| m.at(index % self.width, index / self.width) > 0)
                && walls
                    .as_ref()
                    .is_none_or(|walls| !crate::fill::contains(walls, index))
                && (lines_only || equivalent(&reference.rgba(index)))
        };
        if options == crate::fill::Options::default() {
            let mut writer = layer.pixels.writer();
            let mut changed = false;
            crate::fill::connected_spans(
                [self.width, self.height],
                seed / 4,
                eligible,
                |y, left, right| {
                    writer.edit_span(self.width, left, y, right - left, |offset, pixel| {
                        let selected = self
                            .selection_mask
                            .as_ref()
                            .map_or(255, |m| m.at(left + offset, y));
                        let before: [u8; 4] = pixel.try_into().unwrap();
                        crate::fill::paint_pixel(
                            pixel,
                            color,
                            opacity * f32::from(selected) / 255.0,
                            alpha_locked,
                        );
                        changed |= pixel != before;
                    });
                },
                cancel,
            )?;
            return Ok(changed);
        }
        let mut mask = crate::fill::region_mask(
            [self.width, self.height],
            seed / 4,
            options,
            eligible,
            cancel,
        )?;
        if let Some(walls) = &walls {
            crate::fill::reach_walls(&mut mask, walls, [self.width, self.height], cancel)?;
        }
        let edges = if options.fill_edges && options.expand >= 0 && walls.is_none() {
            Some(crate::fill::soft_edges(
                [self.width, self.height],
                &mask,
                target,
                tolerance,
                |at| reference.rgba(at),
                |at| {
                    self.selection
                        .is_none_or(|s| s.contains(at % self.width, at / self.width))
                        && self
                            .selection_mask
                            .as_ref()
                            .is_none_or(|m| m.at(at % self.width, at / self.width) > 0)
                },
                cancel,
            )?)
        } else {
            None
        };
        let mut writer = layer.pixels.writer();
        let mut changed = false;
        let selected = |x, y| {
            if self.selection.is_some_and(|s| !s.contains(x, y)) {
                0
            } else {
                self.selection_mask.as_ref().map_or(255, |m| m.at(x, y))
            }
        };
        // Paint the solid component in runs. Soft edges are disjoint from the
        // core and retain their individual coverage calculation below.
        for y in 0..self.height {
            crate::fill::check_cancel(cancel)?;
            let mut x = 0;
            while x < self.width {
                if !crate::fill::contains(&mask, y * self.width + x) || selected(x, y) == 0 {
                    x += 1;
                    continue;
                }
                let left = x;
                x += 1;
                while x < self.width
                    && crate::fill::contains(&mask, y * self.width + x)
                    && selected(x, y) > 0
                {
                    x += 1;
                }
                writer.edit_span(self.width, left, y, x - left, |offset, pixel| {
                    let opacity = opacity * f32::from(selected(left + offset, y)) / 255.0;
                    let before: [u8; 4] = pixel.try_into().unwrap();
                    if !alpha_locked && options.fill_edges && same_reference && target[3] == 0 {
                        let coverage = 1.0 - f32::from(pixel[3]) / 255.0;
                        crate::fill::replace_edge(pixel, target, color, opacity, coverage);
                    } else {
                        crate::fill::paint_pixel(pixel, color, opacity, alpha_locked);
                    }
                    changed |= pixel != before;
                });
            }
        }
        if let Some(edges) = edges.filter(|_| !alpha_locked) {
            for (base, word) in edges.iter().enumerate() {
                if base & 15 == 0 {
                    crate::fill::check_cancel(cancel)?;
                }
                let mut bits = *word;
                while bits != 0 {
                    let index = base * 64 + bits.trailing_zeros() as usize;
                    bits &= bits - 1;
                    let selected = selected(index % self.width, index / self.width);
                    if selected == 0 {
                        continue;
                    }
                    let coverage = crate::fill::edge_coverage(
                        [self.width, self.height],
                        index,
                        target,
                        |at| reference.rgba(at),
                    );
                    if coverage <= 0.0 {
                        continue;
                    }
                    let pixel = writer.rgba_mut(index);
                    let before: [u8; 4] = pixel.try_into().unwrap();
                    let opacity = opacity * f32::from(selected) / 255.0;
                    if same_reference {
                        crate::fill::replace_edge(pixel, target, color, opacity, coverage);
                    } else {
                        crate::fill::paint_pixel(pixel, color, opacity, false);
                    }
                    changed |= pixel != before;
                }
            }
        }
        crate::fill::check_cancel(cancel)?;
        Ok(changed)
    }

    pub fn dab(
        &mut self,
        x: f32,
        y: f32,
        pressure: f32,
        brush: &Brush,
        color: [u8; 3],
        erase: bool,
    ) {
        self.dab_mapped(
            x,
            y,
            pressure,
            brush,
            color,
            erase,
            crate::symmetry::IDENTITY,
        );
    }

    /// A dab whose tip is looked up through `map` (an orthogonal 2 x 2
    /// matrix), used by symmetry to mirror or rotate the tip.
    #[allow(clippy::too_many_arguments)]
    pub fn dab_mapped(
        &mut self,
        x: f32,
        y: f32,
        pressure: f32,
        brush: &Brush,
        color: [u8; 3],
        erase: bool,
        map: crate::symmetry::Map,
    ) {
        self.dab_buffered(x, y, pressure, brush, color, erase, map, None);
    }
    #[allow(clippy::too_many_arguments)]
    pub fn dab_buffered(
        &mut self,
        x: f32,
        y: f32,
        pressure: f32,
        brush: &Brush,
        color: [u8; 3],
        erase: bool,
        map: crate::symmetry::Map,
        buffer: Option<&mut crate::stroke_buffer::StrokeBuffer>,
    ) {
        self.dab_sample_buffered(
            crate::stroke::Sample {
                x,
                y,
                pressure,
                ..Default::default()
            },
            brush,
            color,
            erase,
            map,
            buffer,
        );
    }
    pub fn dab_sample_buffered(
        &mut self,
        sample: crate::stroke::Sample,
        brush: &Brush,
        color: [u8; 3],
        erase: bool,
        map: crate::symmetry::Map,
        buffer: Option<&mut crate::stroke_buffer::StrokeBuffer>,
    ) {
        let raster = DabRaster::new(sample, brush, map, self.width, self.height);
        self.paint_dab_rows(
            &raster,
            brush,
            color,
            erase,
            buffer,
            raster.top,
            raster.bottom,
        );
    }
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn paint_dab_rows(
        &mut self,
        raster: &DabRaster,
        brush: &Brush,
        color: [u8; 3],
        erase: bool,
        mut buffer: Option<&mut crate::stroke_buffer::StrokeBuffer>,
        first_row: i32,
        last_row: i32,
    ) {
        let paper = self.paper;
        let [x, y] = raster.position;
        let radius = raster.radius;
        let opacity = raster.opacity;
        let interval_ratio = raster.interval_ratio;
        let held = raster.held;
        let flow_lookup = &raster.flow_lookup;
        let (sin, cos) = (raster.sin, raster.cos);
        let map = raster.map;
        let target = buffer
            .as_ref()
            .map_or(crate::paint_target::Target::Raster, |b| b.target);
        let active = self.active;
        let layer = &mut self.layers[self.active];
        if !buffer.as_ref().map_or_else(
            || target.valid(layer, [self.width, self.height]),
            |b| b.target_valid(layer, [self.width, self.height]),
        ) || layer.locked
            || !layer.visible
            || (target.is_raster() && erase && layer.alpha_locked)
        {
            return;
        }
        // Materials change how paint lands on raster pixels (`material`).
        let material = layer
            .material
            .as_ref()
            .map(|m| m.settings)
            .filter(|_| target.is_raster());
        let kind = material.map(|m| m.kind);
        // Dry media always feels a tooth: the canvas's paper, or its own.
        let dry_tooth = material
            .filter(|m| m.kind == crate::material::Kind::Dry)
            .map(|m| m.tooth);
        let paper = match dry_tooth {
            Some(tooth) => Some(crate::material::dry_paper(paper, tooth)),
            None => paper,
        };
        let paper_response = dry_tooth.map_or(raster.paper_response, |tooth| {
            raster.paper_response.max(tooth.sqrt())
        });
        // A bounded pure-coverage row separates analytic evaluation from the
        // dependent accumulation stores. Keep all other brush cases on the
        // existing general pixel path, including selection and tiny-tip AA.
        if radius > 1.0
            && brush.tip == crate::brush::Tip::Round
            && brush.grain.is_none()
            && brush.grain_amount == 0.0
            && interval_ratio.is_none()
            && self.selection.is_none()
            && self.selection_mask.is_none()
            && paper.is_none()
            && kind != Some(crate::material::Kind::Paint)
            && raster.right >= raster.left
            && raster.right - raster.left < 512
            && buffer.as_ref().is_some_and(|b| b.is_prefix_pass())
        {
            let _timed = crate::stage_timer::scope(crate::stage_timer::Stage::Prefix);
            let buffer = buffer.unwrap();
            let mut delivery = [0.0f32; 512];
            let width = (raster.right - raster.left + 1) as usize;
            for py in first_row.max(raster.top)..=last_row.min(raster.bottom) {
                for (offset, value) in delivery[..width].iter_mut().enumerate() {
                    let px = raster.left + offset as i32;
                    let (dx, dy) = (px as f32 + 0.5 - x, py as f32 + 0.5 - y);
                    let (dx, dy) = (map[0] * dx + map[1] * dy, map[2] * dx + map[3] * dy);
                    let u = (dx * cos + dy * sin) / radius;
                    let v = (-dx * sin + dy * cos) / (radius * brush.aspect);
                    let coverage = crate::brush_edge::coverage_with_profile(
                        u,
                        v,
                        radius,
                        brush.aspect,
                        brush.hardness,
                        brush.tip,
                        brush.softness_profile,
                    );
                    *value = if coverage <= 0.0 {
                        0.0
                    } else {
                        opacity * coverage
                    };
                }
                buffer.commit_coverage_row(
                    self.active,
                    raster.left as usize,
                    py as usize,
                    &delivery[..width],
                    brush.opacity,
                );
            }
            return;
        }
        // The eraser end still erases with a smudge or blur brush.
        let blending = !brush.blend.is_paint() && !erase;
        // Watercolor and thick paint mix as pigments where paint meets paint.
        let pigment = matches!(
            kind,
            Some(crate::material::Kind::Watercolor | crate::material::Kind::Paint)
        );
        let wet = brush.blend == crate::brush::Blend::Wet && !erase;
        // Paper tooth holds back paint laid on artwork, and granulating
        // paint settles in its valleys; erasing, smudging and mask painting
        // ignore the paper.
        let paper = paper
            .filter(|_| target.is_raster() && !erase && !brush.blend.moves_paint())
            .map(|paper| paper.surface());
        // A graph that sets color gives this dab its own color, mixed into
        // the stroke pixel by pixel.
        let tracking = raster.color_shift.is_some() && !erase;
        let color = raster
            .color_shift
            .map_or(color, |shift| shift.apply(color, raster.secondary));
        let (color, erase, alpha_locked) = target.params(color, erase, layer.alpha_locked);
        let prefix_pass = buffer.as_ref().is_some_and(|b| b.is_prefix_pass());
        // Clip the iteration domain before evaluating coverage or touching COW
        // storage. Work outside a rectangular selection cannot affect artwork.
        let (mut left, mut right) = (raster.left, raster.right);
        let (mut top, mut bottom) = (first_row.max(raster.top), last_row.min(raster.bottom));
        if let Some(selection) = self.selection {
            left = left.max(selection.left as i32);
            right = right.min(selection.right as i32 - 1);
            top = top.max(selection.top as i32);
            bottom = bottom.min(selection.bottom as i32 - 1);
        }
        if left > right || top > bottom {
            return;
        }
        let selection_mask = &self.selection_mask;
        // The dab's alpha and coverage ceiling at a pixel, if it paints there.
        let coverage_at = |px: i32, py: i32| -> Option<(f32, f32)> {
            let (dx, dy) = (px as f32 + 0.5 - x, py as f32 + 0.5 - y);
            let (dx, dy) = (map[0] * dx + map[1] * dy, map[2] * dx + map[3] * dy);
            let u = (dx * cos + dy * sin) / radius;
            let v = (-dx * sin + dy * cos) / (radius * brush.aspect);
            if brush.tip == crate::brush::Tip::Image && u.abs().max(v.abs()) >= 1.0 {
                return None;
            }
            let coverage = if brush.tip == crate::brush::Tip::Image {
                brush
                    .image
                    .as_ref()
                    .map_or(0.0, |image| image.coverage(u, v))
            } else if radius <= 1.0 {
                let mut sum = 0.0;
                for sy in 0..8 {
                    for sx in 0..8 {
                        let ox = (sx as f32 + 0.5) / 8.0 - 0.5;
                        let oy = (sy as f32 + 0.5) / 8.0 - 0.5;
                        let mx = map[0] * ox + map[1] * oy;
                        let my = map[2] * ox + map[3] * oy;
                        let su = u + (mx * cos + my * sin) / radius;
                        let sv = v + (-mx * sin + my * cos) / (radius * brush.aspect);
                        let d = if brush.tip == crate::brush::Tip::Round {
                            (su * su + sv * sv).sqrt()
                        } else {
                            su.abs().max(sv.abs())
                        };
                        if d < 1.0 {
                            sum += crate::brush_edge::interior(
                                d,
                                brush.hardness,
                                brush.softness_profile,
                                0.001,
                            );
                        }
                    }
                }
                sum / 64.0
            } else {
                crate::brush_edge::coverage_with_profile(
                    u,
                    v,
                    radius,
                    brush.aspect,
                    brush.hardness,
                    brush.tip,
                    brush.softness_profile,
                )
            };
            if coverage <= 0.0 {
                return None;
            }
            let mut a = opacity * coverage * brush.grain_coverage(px as usize, py as usize);
            if let Some(ratio) = interval_ratio {
                a = brush.opacity
                    * flow_lookup.as_ref().map_or_else(
                        || crate::brush_edge::compensated_delivery(a / brush.opacity, ratio),
                        |lookup| lookup.evaluate(a / opacity),
                    );
            }
            if let Some(surface) = &paper
                && (paper_response > 0.0 || raster.granulation > 0.0)
            {
                // Once per pixel for both.
                let height = surface.height(px as usize, py as usize);
                let paper = &surface.paper;
                if paper_response > 0.0 {
                    a *= paper.hold_at(
                        height,
                        a / brush.opacity.max(f32::MIN_POSITIVE),
                        paper_response,
                    );
                }
                if raster.granulation > 0.0 {
                    a = (a * paper.pool_at(height, raster.granulation)).min(brush.opacity);
                }
            }
            // After the paper, so each of the dabs this one stands for
            // would have met the same tooth.
            if let Some(ratio) = held {
                a = brush.opacity
                    * crate::brush_edge::compensated_delivery(a / brush.opacity, ratio);
            }
            if a <= 0.0 {
                return None;
            }
            let selected = selection_mask
                .as_ref()
                .map_or(1.0, |m| f32::from(m.at(px as usize, py as usize)) / 255.0);
            if selected == 0.0 {
                return None;
            }
            Some((a * selected, brush.opacity * selected))
        };
        // Smudge, blur, wet and thick paint dabs go pixel by pixel on this
        // thread, and thick paint asks for each pixel's coverage twice (the
        // relief, then the paint). For a large dab the coverage, the costly
        // part, is worked out once here, in bands across cores.
        let (span, rows) = ((right - left + 1) as usize, (bottom - top + 1) as usize);
        let blending_now = !brush.blend.is_paint() && !erase && !prefix_pass;
        let thick = kind == Some(crate::material::Kind::Paint) && !prefix_pass;
        let area = [
            left as usize,
            top as usize,
            right as usize + 1,
            bottom as usize + 1,
        ];
        let drawn =
            ((blending_now || thick) && crate::dab_bands::worthwhile(span, rows)).then(|| {
                let _timed = crate::stage_timer::scope(crate::stage_timer::Stage::Coverage);
                crate::dab_bands::Drawn::new(area, |px, py| coverage_at(px as i32, py as i32))
            });
        let coverage_at = |px: i32, py: i32| -> Option<(f32, f32)> {
            match &drawn {
                Some(drawn) => drawn.at(px as usize, py as usize),
                None => coverage_at(px, py),
            }
        };
        // Thick paint: the dab builds, wears or carries the relief where it
        // paints (once per dab; a prefix pass repeats dabs already painted).
        if kind == Some(crate::material::Kind::Paint)
            && !prefix_pass
            && let Some(material) = layer.material.as_mut()
        {
            let _timed = crate::stage_timer::scope(crate::stage_timer::Stage::Relief);
            crate::material::touch_relief(
                material,
                [self.width, self.height],
                &crate::material::Touch {
                    rect: [left, top, right, bottom],
                    center: [x, y],
                    radius,
                    // Streaks follow the stroke once it has a direction.
                    heading: raster.heading.map(|h| h.to_radians().sin_cos()),
                    smear: raster.smear,
                    blend: brush.blend,
                    thickness: raster.thickness,
                    erase,
                },
                &coverage_at,
            );
        }
        if blending && !prefix_pass {
            let width = self.width;
            let mut scratch = buffer
                .as_deref_mut()
                .map(|b| std::mem::take(&mut b.blend_scratch))
                .unwrap_or_default();
            // Rows of one dab may arrive in slices; the copy is made once.
            if first_row <= raster.top {
                crate::blend_brush::capture(
                    &mut scratch,
                    |px, py| target.read(layer, width, px, py),
                    [self.width, self.height],
                    [raster.left, raster.top, raster.right, raster.bottom],
                    brush.blend,
                    raster.smear,
                    radius,
                );
            }
            let layer_id = layer.id;
            // Dry media stays dry.
            let watering = wet && target.is_raster() && kind != Some(crate::material::Kind::Dry);
            let span = (right - left + 1) as usize;
            // Water already here, and water this dab leaves, per row.
            let (mut was_wet, mut water) = if wet {
                (vec![0.0f32; span], vec![0.0f32; span])
            } else {
                (Vec::new(), Vec::new())
            };
            // A large dab on a layer's tiles blends in row bands across
            // cores, with the arithmetic of the loop below.
            if let Some(drawn) = &drawn
                && target.is_raster()
                && layer.pixels.is_tiled()
                && let Some(buffer) = buffer.as_deref_mut()
            {
                let mut damp = Vec::new();
                if watering {
                    let _timed = crate::stage_timer::scope(crate::stage_timer::Stage::WaterRead);
                    damp = vec![0.0f32; span * rows];
                    for (row, values) in damp.chunks_exact_mut(span).enumerate() {
                        self.wet
                            .read_row(layer_id, top as usize + row, left as usize, values);
                    }
                }
                let (left, top) = (left as usize, top as usize);
                let load = raster.pigment;
                let timed = crate::stage_timer::scope(crate::stage_timer::Stage::Blend);
                let left_water = crate::dab_bands::apply(
                    &mut layer.pixels,
                    buffer,
                    active,
                    drawn,
                    watering,
                    |px, py, a, ceiling, entry, pixel| {
                        if entry.1 < 0.0 {
                            *entry = ((&*pixel).try_into().unwrap(), 0.0);
                        }
                        let source = scratch.source(brush.blend, raster.smear, px, py);
                        if !wet {
                            crate::blend_brush::mix(pixel, source, a, alpha_locked, pigment);
                            return 0.0;
                        }
                        let strength = a / ceiling.max(f32::MIN_POSITIVE);
                        let damp = damp
                            .get((py - top) * span + (px - left))
                            .copied()
                            .unwrap_or(0.0);
                        if damp > 0.0 {
                            crate::blend_brush::mix(
                                pixel,
                                source,
                                (damp * strength).min(1.0) * 0.7,
                                alpha_locked,
                                pigment,
                            );
                        }
                        if load < 1.0 && !alpha_locked {
                            let lift = strength.min(1.0) * (1.0 - load) * WATER_LIFT;
                            pixel[3] = (f32::from(pixel[3]) * (1.0 - lift)).round() as u8;
                        }
                        if pigment && damp > 0.0 && pixel[3] > 0 {
                            paint_pigment(pixel, a * load, color, alpha_locked);
                        } else {
                            paint_pixel(pixel, a * load, color, false, alpha_locked);
                        }
                        strength.min(1.0) * raster.wetness
                    },
                );
                drop(timed);
                let _timed = crate::stage_timer::scope(crate::stage_timer::Stage::WaterWrite);
                let rows = left_water.iter().flat_map(|band| band.chunks_exact(span));
                for (row, water) in rows.enumerate() {
                    self.wet.add_row(layer_id, top + row, left, water);
                }
                buffer.blend_scratch = scratch;
                return;
            }
            let _timed = crate::stage_timer::scope(crate::stage_timer::Stage::Serial);
            let mut writer = target.writer(layer, width);
            for py in top..=bottom {
                if watering {
                    let _timed = crate::stage_timer::scope(crate::stage_timer::Stage::SerialWater);
                    self.wet
                        .read_row(layer_id, py as usize, left as usize, &mut was_wet);
                    water.fill(0.0);
                }
                for px in left..=right {
                    let Some((a, ceiling)) = coverage_at(px, py) else {
                        continue;
                    };
                    let column = (px - left) as usize;
                    let (px, py) = (px as usize, py as usize);
                    let source = scratch.source(brush.blend, raster.smear, px, py);
                    writer.edit(px, py, |pixel| {
                        if let Some(buffer) = buffer.as_deref_mut() {
                            buffer.remember_at(active, px, py, pixel);
                        }
                        if wet {
                            // Wet-on-wet: paint still wet here runs together
                            // before the new paint lands. Dry paint stays put.
                            let strength = a / ceiling.max(f32::MIN_POSITIVE);
                            let damp = was_wet.get(column).copied().unwrap_or(0.0);
                            if damp > 0.0 {
                                crate::blend_brush::mix(
                                    pixel,
                                    source,
                                    (damp * strength).min(1.0) * 0.7,
                                    alpha_locked,
                                    pigment,
                                );
                            }
                            // Clear water lifts a little of what is there.
                            let load = raster.pigment;
                            if load < 1.0 && !alpha_locked {
                                let lift = strength.min(1.0) * (1.0 - load) * WATER_LIFT;
                                pixel[3] = (f32::from(pixel[3]) * (1.0 - lift)).round() as u8;
                            }
                            if pigment && damp > 0.0 && pixel[3] > 0 {
                                paint_pigment(pixel, a * load, color, alpha_locked);
                            } else {
                                paint_pixel(pixel, a * load, color, false, alpha_locked);
                            }
                            if watering {
                                water[column] = strength.min(1.0) * raster.wetness;
                            }
                        } else {
                            crate::blend_brush::mix(pixel, source, a, alpha_locked, pigment);
                        }
                    });
                }
                if watering {
                    let _timed = crate::stage_timer::scope(crate::stage_timer::Stage::SerialWater);
                    self.wet
                        .add_row(layer_id, py as usize, left as usize, &water);
                }
            }
            if let Some(buffer) = buffer.as_deref_mut() {
                buffer.blend_scratch = scratch;
            }
            return;
        }
        let _timed = crate::stage_timer::scope(if prefix_pass {
            crate::stage_timer::Stage::Prefix
        } else {
            crate::stage_timer::Stage::Paint
        });
        // Large dabs on raster tiles blend in row bands across cores.
        if !prefix_pass
            && !tracking
            && target.is_raster()
            && layer.pixels.is_tiled()
            && let Some(buffer) = buffer.as_deref_mut()
            && crate::dab_bands::worthwhile(
                (right - left + 1) as usize,
                (bottom - top + 1) as usize,
            )
        {
            let made;
            let drawn = match &drawn {
                Some(drawn) => drawn,
                None => {
                    made = crate::dab_bands::Drawn::new(area, |px, py| {
                        coverage_at(px as i32, py as i32)
                    });
                    &made
                }
            };
            crate::dab_bands::paint(
                &mut layer.pixels,
                buffer,
                active,
                drawn,
                color,
                erase,
                alpha_locked,
            );
            return;
        }
        let mut writer = (!prefix_pass).then(|| target.writer(layer, self.width));
        let scalar_writes = writer
            .as_ref()
            .is_some_and(|writer| writer.prefers_scalar_dabs((right - left + 1) as usize));
        // Fixed scratch keeps each resumable row bounded without allocating.
        // Separate coverage from writes so tile ownership is checked per span,
        // while retaining pixel order and the existing blending arithmetic.
        let mut delivery = [0.0f32; 1024];
        for py in top..=bottom {
            for chunk_left in (left..=right).step_by(512) {
                let chunk_right = right.min(chunk_left + 511);
                let count = (chunk_right - chunk_left + 1) as usize;
                if !prefix_pass && !scalar_writes {
                    delivery[..count * 2].fill(0.0);
                }
                for px in chunk_left..=chunk_right {
                    let Some((mut a, ceiling)) = coverage_at(px, py) else {
                        continue;
                    };
                    let (px, py) = (px as usize, py as usize);
                    if prefix_pass {
                        if let Some(buffer) = buffer.as_deref_mut() {
                            buffer.commit_coverage_at(self.active, px, py, a, ceiling);
                        }
                        continue;
                    }
                    if scalar_writes {
                        writer.as_mut().unwrap().edit(px, py, |pixels| {
                            let mut color = color;
                            if let Some(buffer) = buffer.as_deref_mut() {
                                if tracking {
                                    (a, color) = buffer.accumulate_color_at(
                                        active, px, py, pixels, a, ceiling, color,
                                    );
                                } else {
                                    a = buffer.accumulate_at(active, px, py, pixels, a, ceiling);
                                }
                            }
                            paint_pixel(pixels, a, color, erase, alpha_locked);
                        });
                    } else {
                        delivery[(px - chunk_left as usize) * 2] = a;
                        delivery[(px - chunk_left as usize) * 2 + 1] = ceiling;
                    }
                }
                if prefix_pass || scalar_writes {
                    continue;
                }
                let mut at = 0;
                while at < count {
                    if delivery[at * 2] <= 0.0 {
                        at += 1;
                        continue;
                    }
                    let start = at;
                    while at < count && delivery[at * 2] > 0.0 {
                        at += 1;
                    }
                    writer.as_mut().unwrap().edit_span(
                        chunk_left as usize + start,
                        py as usize,
                        at - start,
                        |offset, pixels| {
                            let index = start + offset;
                            let mut a = delivery[index * 2];
                            let mut color = color;
                            if let Some(buffer) = buffer.as_deref_mut() {
                                let (x, ceiling) =
                                    (chunk_left as usize + index, delivery[index * 2 + 1]);
                                if tracking {
                                    (a, color) = buffer.accumulate_color_at(
                                        active,
                                        x,
                                        py as usize,
                                        pixels,
                                        a,
                                        ceiling,
                                        color,
                                    );
                                } else {
                                    a = buffer.accumulate_at(
                                        active,
                                        x,
                                        py as usize,
                                        pixels,
                                        a,
                                        ceiling,
                                    );
                                }
                            }
                            paint_pixel(pixels, a, color, erase, alpha_locked);
                        },
                    );
                }
            }
        }
    }

    pub fn clear_selected(&mut self) -> Result<bool, String> {
        let layer = &mut self.layers[self.active];
        if layer.folder {
            return Err("Select a layer inside the folder to clear it".into());
        }
        if layer.locked || layer.alpha_locked {
            return Err("Unlock pixels and transparency before clearing".into());
        }
        let rect = self
            .selection
            .map_or([0, 0, self.width, self.height], |selection| {
                [
                    selection.left,
                    selection.top,
                    selection.right,
                    selection.bottom,
                ]
            });
        if let Some(mask) = &self.selection_mask {
            let mut changed = false;
            let source = layer.pixels.clone();
            let mut cleared = source.clone();
            {
                let mut writer = cleared.writer();
                for y in rect[1]..rect[3] {
                    for x in rect[0]..rect[2] {
                        let coverage = mask.at(x, y);
                        if coverage == 0 {
                            continue;
                        }
                        let before = source.rgba(y * self.width + x);
                        if before[3] == 0 {
                            continue;
                        }
                        let alpha = ((u16::from(before[3]) * (255 - u16::from(coverage)) + 127)
                            / 255) as u8;
                        if alpha != before[3] {
                            changed = true;
                            let pixel = writer.rgba_mut(y * self.width + x);
                            if coverage == 255 {
                                pixel.fill(0);
                            } else {
                                pixel[3] = alpha;
                            }
                        }
                    }
                }
                if changed {
                    writer.reclaim_region(self.width, rect);
                }
            }
            if changed {
                layer.pixels = cleared;
            }
            Ok(changed)
        } else {
            Ok(layer.pixels.clear_visible(self.width, rect))
        }
    }
}

/// Changed regions within individual sparse tiles, plus action state.
/// Applying a patch swaps it with the document, so the same record serves as
/// the matching redo (or undo) entry afterwards.
struct Patch {
    layer: usize,
    regions: Vec<RegionPatch>,
    /// The layer's thick paint relief on the other side of the action,
    /// when the action changed it. A relief shares every tile the action
    /// left alone, so this holds only what changed.
    relief: Option<Option<crate::layer_mask::Mask>>,
    active: usize,
    selection: Option<crate::selection::Selection>,
    selection_mask: Option<crate::selection_mask::Mask>,
}

struct RegionPatch {
    rect: [usize; 4],
    pixels: crate::history_pixels::Payload,
}

enum Entry {
    /// Copy-on-write snapshot; only buffers the action replaced cost memory.
    Full(Document),
    /// Completed action whose cost planner chose a snapshot over pixel patches.
    /// Preserve that choice when later checkpoints settle or invert history.
    RetainedFull(Document),
    Patch(Patch),
}

impl Entry {
    fn prepare(&mut self) -> Result<(), String> {
        if let Entry::Patch(patch) = self {
            // Decode all regions before modifying artwork, so a late decode
            // failure leaves both the document and the history action intact.
            for region in &mut patch.regions {
                let [left, top, right, bottom] = region.rect;
                region.pixels.prepare((right - left) * (bottom - top) * 4)?;
            }
        }
        Ok(())
    }
    fn apply(self, doc: &mut Document) -> Entry {
        match self {
            Entry::Full(snapshot) => Entry::Full(std::mem::replace(doc, snapshot)),
            Entry::RetainedFull(snapshot) => Entry::RetainedFull(std::mem::replace(doc, snapshot)),
            Entry::Patch(mut patch) => {
                let width = doc.width;
                if !patch.regions.is_empty() {
                    let mut target = doc.layers[patch.layer].pixels.writer();
                    for region in &mut patch.regions {
                        target.swap_region(width, region.rect, region.pixels.raw_mut());
                    }
                }
                if let Some(relief) = &mut patch.relief
                    && let Some(material) = &mut doc.layers[patch.layer].material
                {
                    std::mem::swap(&mut material.relief, relief);
                }
                std::mem::swap(&mut doc.active, &mut patch.active);
                std::mem::swap(&mut doc.selection, &mut patch.selection);
                std::mem::swap(&mut doc.selection_mask, &mut patch.selection_mask);
                Entry::Patch(patch)
            }
        }
    }
}

/// Bounding rectangle of differing pixels. Columns are found in 32-pixel
/// chunks with slice comparisons, so the rectangle may be slightly larger.
pub(crate) fn changed_rect(
    before: &[u8],
    after: &[u8],
    width: usize,
    height: usize,
) -> Option<[usize; 4]> {
    let stride = width * 4;
    let row = |y: usize| y * stride..(y + 1) * stride;
    let top = (0..height).find(|&y| before[row(y)] != after[row(y)])?;
    let bottom = (top..height).rfind(|&y| before[row(y)] != after[row(y)])? + 1;
    let differs = |x: usize| {
        let end = (x + 32).min(width);
        (top..bottom).any(|y| {
            let span = (y * width + x) * 4..(y * width + end) * 4;
            before[span.clone()] != after[span]
        })
    };
    let chunks: Vec<usize> = (0..width).step_by(32).collect();
    let left = *chunks.iter().find(|&&x| differs(x))?;
    let right = chunks
        .iter()
        .rev()
        .find(|&&x| differs(x))
        .map_or(width, |x| (x + 32).min(width));
    Some([left, top, right, bottom])
}

/// Reduce a snapshot to changed regions within tiles,
/// when the action edited the pixels of at most one layer.
fn compact_layer(before: &Document, after: &Document) -> Option<Option<usize>> {
    if before.version != after.version
        || before.width != after.width
        || before.height != after.height
        || before.dpi != after.dpi
        || before.paper != after.paper
        || before.canvas_frame != after.canvas_frame
        || before.layers.len() != after.layers.len()
    {
        return None;
    }
    let mut edited = None;
    for (index, (a, b)) in before.layers.iter().zip(&after.layers).enumerate() {
        if a.id != b.id
            || a.name != b.name
            || a.collapsed != b.collapsed
            || a.visible != b.visible
            || a.opacity != b.opacity
            || a.csp_byte_opacity != b.csp_byte_opacity
            || a.fill != b.fill
            || a.blend != b.blend
            || a.clipped != b.clipped
            || a.locked != b.locked
            || a.alpha_locked != b.alpha_locked
            || a.folder != b.folder
            || a.depth != b.depth
            || a.effects != b.effects
            || a.adjustment != b.adjustment
            || !crate::vector_layer::same(&a.vector, &b.vector)
            || a.text != b.text
            || !crate::layer_mask::same_snapshot(&a.mask, &b.mask)
            || !crate::material::same_settings(&a.material, &b.material)
        {
            return None;
        }
        if !a.pixels.shares(&b.pixels) || !crate::material::same(&a.material, &b.material) {
            if edited.is_some() {
                return None;
            }
            edited = Some(index);
        }
    }
    Some(edited)
}

fn compact(before: &Document, after: &Document) -> Option<Patch> {
    let edited = compact_layer(before, after)?;
    let layer = edited.unwrap_or(0);
    let source = &before.layers[layer].pixels;
    let regions = edited.map_or_else(Vec::new, |_| {
        source
            .changed_regions(&after.layers[layer].pixels, after.width, after.height)
            .into_iter()
            .flat_map(|[left, top, right, bottom]| {
                let side = crate::pixels::SIDE;
                let mut rects = Vec::new();
                for by in top / side..bottom.div_ceil(side) {
                    for bx in left / side..right.div_ceil(side) {
                        rects.push([
                            left.max(bx * side),
                            top.max(by * side),
                            right.min((bx + 1) * side),
                            bottom.min((by + 1) * side),
                        ]);
                    }
                }
                rects
            })
            .map(|rect| {
                let pixels = source.region(after.width, rect);
                RegionPatch {
                    rect,
                    pixels: crate::history_pixels::Payload::new(pixels),
                }
            })
            .collect()
    });
    // Thick paint: the relief before the action, if the action changed it.
    let relief = edited
        .filter(|_| {
            !crate::material::same(
                &before.layers[layer].material,
                &after.layers[layer].material,
            )
        })
        .map(|_| {
            before.layers[layer]
                .material
                .as_ref()
                .and_then(|m| m.relief.clone())
        });
    Some(Patch {
        layer,
        regions,
        relief,
        active: before.active,
        selection: before.selection,
        selection_mask: before.selection_mask.clone(),
    })
}

#[path = "history_settlement.rs"]
mod history_settlement;
pub use history_settlement::Settlement;
pub(crate) use history_settlement::same_snapshot;

pub struct History {
    undo: Vec<Entry>,
    redo: Vec<Entry>,
    pending_redo: Option<Vec<Entry>>,
    bytes: usize,
    budget: usize,
    limit: usize,
    prefetch_budget: usize,
    cooling: Option<crate::history_pixels::Cooling>,
    error: Option<String>,
}
impl History {
    pub const BUDGET_MIB: usize = 512;
    pub const PREFETCH_MIB: usize = 32;
    pub fn new() -> Self {
        Self {
            undo: vec![],
            redo: vec![],
            pending_redo: None,
            bytes: 0,
            budget: Self::BUDGET_MIB * 1024 * 1024,
            limit: 100,
            prefetch_budget: Self::PREFETCH_MIB * 1024 * 1024,
            cooling: None,
            error: None,
        }
    }
    /// Exclude the nearest two actions on each stack from cooling. Older patches
    /// compress and nearby packed regions prefetch through one bounded worker.
    /// Prefetch stays within both its cache budget and available history budget.
    /// Identity checks
    /// discard a worker result if undo, redo, eviction or cancellation changed it.
    pub fn cool_idle(&mut self, doc: &Document) -> bool {
        if self
            .cooling
            .as_ref()
            .is_some_and(|job| job.worker.is_finished())
        {
            let job = self.cooling.take().unwrap();
            let output = job
                .worker
                .join()
                .unwrap_or(crate::history_pixels::Output::Failed);
            let decoded = output.prefetched_bytes();
            self.bytes = self.measure(doc);
            let warm_fits = decoded == 0
                || (self.prefetched_bytes().saturating_add(decoded) <= self.prefetch_budget
                    && self
                        .bytes
                        .saturating_sub(crate::history_pixels::raw_capacity(job.source.bytes()))
                        .saturating_add(decoded)
                        <= self.budget);
            if warm_fits
                && let Some(region) = self
                    .undo
                    .iter_mut()
                    .chain(&mut self.redo)
                    .chain(self.pending_redo.iter_mut().flatten())
                    .filter_map(|entry| {
                        if let Entry::Patch(patch) = entry {
                            Some(patch)
                        } else {
                            None
                        }
                    })
                    .flat_map(|patch| &mut patch.regions)
                    .find(|region| region.pixels.matches(&job.source))
            {
                region.pixels.accept_job(&job.source, output);
            }
            if self.has_pending_stroke() {
                // Accept identity-checked finished jobs, but never evict the
                // checkpoint or suspended history of a discardable gesture.
                self.bytes = self.measure(doc);
            } else {
                self.trim(doc, true);
            }
        }
        if self.cooling.is_none() && !self.has_pending_stroke() {
            let warm_available = self.prefetch_budget.saturating_sub(self.prefetched_bytes());
            let budget_available = self.budget.saturating_sub(self.bytes);
            let warm = self
                .undo
                .iter()
                .rev()
                .take(2)
                .chain(self.redo.iter().rev().take(2))
                .filter_map(|entry| {
                    if let Entry::Patch(patch) = entry {
                        Some(patch)
                    } else {
                        None
                    }
                })
                .flat_map(|patch| &patch.regions)
                .filter_map(|region| region.pixels.warm_candidate())
                .find(|(bytes, len)| {
                    let decoded =
                        len + 1 + std::mem::size_of::<Vec<u8>>() + 2 * std::mem::size_of::<usize>();
                    decoded <= warm_available
                        && decoded.saturating_sub(crate::history_pixels::raw_capacity(bytes))
                            <= budget_available
                })
                .map(|(bytes, len)| (bytes.clone(), len));
            if let Some((bytes, len)) = warm {
                self.cooling = crate::history_pixels::Cooling::prefetch(bytes, len);
            }
        }
        if self.cooling.is_none() && !self.has_pending_stroke() {
            let undo_cold = self.undo.len().saturating_sub(2);
            let redo_cold = self.redo.len().saturating_sub(2);
            let source = self
                .undo
                .iter()
                .take(undo_cold)
                .chain(self.redo.iter().take(redo_cold))
                .filter_map(|entry| {
                    if let Entry::Patch(patch) = entry {
                        Some(patch)
                    } else {
                        None
                    }
                })
                .flat_map(|patch| &patch.regions)
                .find_map(|region| region.pixels.cold_candidate())
                .cloned();
            if let Some(source) = source {
                self.cooling = crate::history_pixels::Cooling::start(source);
            }
        }
        self.cooling.is_some()
    }
    pub fn compressed_regions(&self) -> usize {
        self.undo
            .iter()
            .chain(&self.redo)
            .chain(self.pending_redo.iter().flatten())
            .filter_map(|entry| {
                if let Entry::Patch(patch) = entry {
                    Some(patch)
                } else {
                    None
                }
            })
            .flat_map(|patch| &patch.regions)
            .filter(|region| matches!(region.pixels, crate::history_pixels::Payload::Packed { .. }))
            .count()
    }
    pub fn prefetched_bytes(&self) -> usize {
        self.undo
            .iter()
            .chain(&self.redo)
            .chain(self.pending_redo.iter().flatten())
            .filter_map(|entry| {
                if let Entry::Patch(patch) = entry {
                    Some(patch)
                } else {
                    None
                }
            })
            .flat_map(|patch| &patch.regions)
            .map(|region| region.pixels.prefetched_bytes())
            .sum()
    }
    pub fn take_error(&mut self) -> Option<String> {
        self.error.take()
    }
    /// Memory retained by history beyond the live document: patch bytes plus
    /// snapshot buffers that are no longer shared with the document.
    fn measure(&self, doc: &Document) -> usize {
        let mut registry = crate::snapshot_memory::Registry::new(doc);
        self.register_allocations(&mut registry);
        registry.additional_bytes()
    }
    pub(crate) fn register_allocations(&self, registry: &mut crate::snapshot_memory::Registry) {
        for entry in self
            .undo
            .iter()
            .chain(&self.redo)
            .chain(self.pending_redo.iter().flatten())
        {
            match entry {
                Entry::Patch(patch) => {
                    if let Some(mask) = &patch.selection_mask {
                        let (id, bytes) = mask.allocation();
                        registry.add_buffer(id, bytes);
                    }
                    registry.add_owned(
                        std::mem::size_of::<Patch>()
                            + patch.regions.capacity() * std::mem::size_of::<RegionPatch>(),
                    );
                    if let Some(Some(relief)) = &patch.relief {
                        relief.visit_allocations(|id, bytes| registry.add_pixels(id, bytes));
                    }
                    for region in &patch.regions {
                        registry.add_buffer(
                            region.pixels.allocation_identity(),
                            region.pixels.capacity(),
                        );
                    }
                }
                Entry::Full(snapshot) | Entry::RetainedFull(snapshot) => {
                    registry.add_document(snapshot)
                }
            }
        }
        if let Some(job) = &self.cooling {
            registry.add_buffer(
                job.source.identity(),
                crate::history_pixels::raw_capacity(job.source.bytes()),
            );
        }
    }
    fn trim(&mut self, doc: &Document, preserve_undo: bool) {
        self.bytes = self.measure(doc);
        // Retain one nearest action even if that entry alone exceeds budget.
        while self.undo.len() + self.redo.len() > 1
            && (self.bytes > self.budget || self.undo.len() + self.redo.len() > self.limit)
        {
            let remove_undo = if self.undo.len() > 1 {
                true
            } else if self.redo.len() > 1 {
                false
            } else {
                !preserve_undo
            };
            if remove_undo {
                self.undo.remove(0);
            } else {
                self.redo.remove(0);
            }
            self.bytes = self.measure(doc);
        }
    }
    /// Convert the newest snapshot into tile-region patches once its action is
    /// complete. Cost-planned retained snapshots have already settled and keep
    /// their storage choice; only unfinished full snapshots are examined.
    pub fn settle(&mut self, doc: &Document) {
        self.pending_redo = None;
        if let Some(Entry::Full(before)) = self.undo.last()
            && let Some(patch) = compact(before, doc)
        {
            *self.undo.last_mut().unwrap() = Entry::Patch(patch);
        }
        // Full snapshots can gain exclusive tiles even when metadata prevents
        // compaction. Refresh and enforce the budget for every finished action.
        self.trim(doc, true);
    }
    pub fn begin_stroke_checkpoint(&mut self, doc: &Document) {
        self.begin_gesture_checkpoint(doc);
    }
    /// Suspend redo and defer eviction until a paint or metadata gesture
    /// commits. A no-op can discard this checkpoint without losing redo.
    pub fn begin_gesture_checkpoint(&mut self, doc: &Document) {
        self.settle(doc);
        self.pending_redo = Some(std::mem::take(&mut self.redo));
        self.undo.push(Entry::Full(doc.clone()));
        // Defer eviction until commit so cancellation preserves older history.
        self.bytes = self.measure(doc);
    }
    pub fn has_pending_stroke(&self) -> bool {
        self.pending_redo.is_some()
    }
    /// Open the newest action again, so that what changes next joins it:
    /// paint drying after a wet stroke then undoes and redoes with that
    /// stroke. Only for an action that painted `layer` (the wet one), when
    /// nothing waits to be redone; false otherwise.
    pub(crate) fn reopen_last(&mut self, doc: &Document, layer: usize) -> bool {
        if !self.redo.is_empty() || self.pending_redo.as_ref().is_some_and(|r| !r.is_empty()) {
            return false;
        }
        if !matches!(
            self.undo.last(),
            Some(Entry::Patch(patch)) if patch.layer == layer && !patch.regions.is_empty()
        ) {
            return false;
        }
        let mut entry = self.undo.pop().unwrap();
        if let Err(error) = entry.prepare() {
            self.undo.push(entry);
            self.error = Some(error);
            return false;
        }
        // The document from before the action, as an open snapshot that
        // settles again once what joins it is done.
        let mut before = doc.clone();
        entry.apply(&mut before);
        self.undo.push(Entry::Full(before));
        self.bytes = self.measure(doc);
        true
    }
    pub fn checkpoint(&mut self, doc: &Document) {
        self.settle(doc);
        self.redo.clear();
        self.undo.push(Entry::Full(doc.clone()));
        self.trim(doc, true);
    }
    pub fn discard_pending_checkpoint(&mut self, doc: &Document) -> bool {
        if !matches!(self.undo.last(), Some(Entry::Full(_))) {
            return false;
        }
        self.undo.pop();
        if let Some(redo) = self.pending_redo.take() {
            self.redo = redo;
        }
        self.bytes = self.measure(doc);
        true
    }
    pub fn undo(&mut self, doc: &mut Document) -> bool {
        self.error = None;
        self.settle(doc);
        if let Some(mut entry) = self.undo.pop() {
            if let Err(error) = entry.prepare() {
                self.undo.push(entry);
                self.error = Some(error);
                self.bytes = self.measure(doc);
                return false;
            }
            let inverse = entry.apply(doc);
            self.redo.push(inverse);
            self.trim(doc, false);
            true
        } else {
            false
        }
    }
    pub fn redo(&mut self, doc: &mut Document) -> bool {
        self.error = None;
        if let Some(mut entry) = self.redo.pop() {
            if let Err(error) = entry.prepare() {
                self.redo.push(entry);
                self.error = Some(error);
                self.bytes = self.measure(doc);
                return false;
            }
            let inverse = entry.apply(doc);
            self.undo.push(inverse);
            self.trim(doc, true);
            true
        } else {
            false
        }
    }
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }
    pub fn over_budget(&self) -> bool {
        self.bytes > self.budget
    }
    /// Refresh accounting after a rollback without evicting restored history.
    pub fn refresh_memory(&mut self, doc: &Document) {
        self.bytes = self.measure(doc);
    }
    pub fn retained_bytes(&self) -> usize {
        self.bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sparse_composite_matches_reading_every_layer() {
        use crate::blend::BlendMode;
        let (w, h) = (600, 300);
        let mut doc = Document::new(w, h);
        let raster = |rect: [usize; 4], rgba: [u8; 4]| {
            let mut layer = Layer::raster("layer".into(), w, h);
            let count = (rect[2] - rect[0]) * (rect[3] - rect[1]);
            layer
                .pixels
                .writer()
                .write_region(w, rect, &rgba.repeat(count));
            layer
        };
        let mut layers = vec![raster([0, 0, 200, 300], [200, 40, 40, 255])];
        // A hidden base still closes its clipping group: the clipped layer
        // above it must stay invisible, not attach to the layer below.
        let mut hidden = raster([150, 0, 350, 200], [20, 200, 40, 255]);
        hidden.visible = false;
        layers.push(hidden);
        let mut clipped = raster([0, 0, 600, 300], [30, 30, 220, 200]);
        clipped.clipped = true;
        layers.push(clipped);
        // An empty base in most blocks, with a clipped layer over it.
        layers.push(raster([500, 250, 510, 260], [10, 10, 10, 255]));
        let mut over_empty = raster([0, 100, 600, 160], [250, 250, 0, 180]);
        over_empty.clipped = true;
        layers.push(over_empty);
        // Folder contents sit directly below the folder header, one level deeper.
        let mut inner = raster([300, 50, 590, 250], [90, 160, 230, 255]);
        inner.depth = 1;
        layers.push(inner);
        let mut empty_inner = Layer::raster("empty".into(), w, h);
        empty_inner.depth = 1;
        layers.push(empty_inner);
        let mut folder = Layer::new_folder("folder".into());
        folder.opacity = 0.5;
        folder.blend = BlendMode::Multiply;
        layers.push(folder);
        let mut transparent = raster([0, 0, 600, 300], [255, 255, 255, 255]);
        transparent.opacity = 0.0;
        layers.push(transparent);
        let mut screen = raster([260, 260, 270, 300], [0, 120, 255, 128]);
        screen.blend = BlendMode::Screen;
        layers.push(screen);
        doc.layers = layers;
        doc.active = 0;
        let fast = doc.composite_region(0, 0, w, h);
        let reference: Vec<u8> = (0..w * h)
            .flat_map(|i| doc.composite_rgba_pixel(i))
            .collect();
        assert!(
            fast == reference,
            "sparse composite differs from per-layer reads"
        );
        let part = doc.composite_region(140, 90, 520, 280);
        for (row, y) in (90..280).enumerate() {
            let start = (y * w + 140) * 4;
            assert_eq!(
                &part[row * 380 * 4..(row + 1) * 380 * 4],
                &reference[start..start + 380 * 4]
            );
        }
    }
    #[test]
    fn strip_composite_matches_per_pixel_compositing_on_random_documents() {
        use crate::blend::BlendMode;
        let (w, h) = (300, 140);
        let mut seed = 0x2545_f491_4f6c_dd1du64;
        let mut next = move |n: usize| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed % n as u64) as usize
        };
        let mut checked = 0;
        for _ in 0..150 {
            // Children come first, one level deeper, then their folder.
            fn group(
                depth: u8,
                next: &mut impl FnMut(usize) -> usize,
                w: usize,
                h: usize,
            ) -> Vec<Layer> {
                let mut layers = Vec::new();
                for _ in 0..1 + next(4) {
                    if depth < 3 && next(4) == 0 {
                        layers.extend(group(depth + 1, next, w, h));
                        let mut folder = Layer::new_folder("folder".into());
                        folder.depth = depth;
                        folder.blend = [
                            BlendMode::Normal,
                            BlendMode::PassThrough,
                            BlendMode::Multiply,
                        ][next(3)];
                        folder.opacity = [1.0, 0.6][next(2)];
                        folder.visible = next(6) != 0;
                        layers.push(folder);
                        continue;
                    }
                    let mut layer = Layer::raster("layer".into(), w, h);
                    layer.depth = depth;
                    let rect = [next(w), next(h), 0, 0];
                    let rect = [
                        rect[0],
                        rect[1],
                        rect[0] + 1 + next(w - rect[0]),
                        rect[1] + 1 + next(h - rect[1]),
                    ];
                    let rgba = [
                        next(256) as u8,
                        next(256) as u8,
                        next(256) as u8,
                        [255, 128, 0][next(3)],
                    ];
                    let count = (rect[2] - rect[0]) * (rect[3] - rect[1]);
                    layer
                        .pixels
                        .writer()
                        .write_region(w, rect, &rgba.repeat(count));
                    layer.blend = BlendMode::ALL[next(BlendMode::ALL.len())];
                    layer.opacity = [1.0, 0.7, 0.0][next(3)];
                    layer.visible = next(5) != 0;
                    layer.clipped = next(3) == 0;
                    if next(3) == 0 {
                        let mut coverage =
                            crate::layer_mask::Mask::new(w, h, [0, 255][next(2)]).unwrap();
                        let mut writer = coverage.writer();
                        for _ in 0..2_000 {
                            writer.set(next(w), next(h), next(256) as u8);
                        }
                        layer.mask = Some(crate::layer_mask::LayerMask {
                            id: LayerId::new(),
                            enabled: next(4) != 0,
                            linked: true,
                            coverage,
                        });
                    }
                    layers.push(layer);
                }
                layers
            }
            let mut doc = Document::new(w, h);
            doc.layers = group(0, &mut next, w, h);
            doc.active = 0;
            if doc.validate().is_err() {
                continue;
            }
            checked += 1;
            for transparent in [false, true] {
                let fast = doc.composite_display_region([0, 0, w, h], transparent);
                for (i, pixel) in fast.chunks_exact(4).enumerate() {
                    assert_eq!(
                        pixel,
                        doc.composite_display_pixel(i, transparent),
                        "pixel {i}"
                    );
                }
            }
            // The quick version holds the true composite at each square's middle.
            let cancel = std::sync::atomic::AtomicBool::new(false);
            let rect = [3, 5, 290, 131];
            let quick = doc
                .composite_sampled_cancelable(rect, 4, &cancel, false)
                .unwrap();
            for y in rect[1]..rect[3] {
                for x in rect[0]..rect[2] {
                    let at = |v: usize, from: usize, to: usize| (v / 4 * 4 + 2).clamp(from, to - 1);
                    let sample = at(y, rect[1], rect[3]) * w + at(x, rect[0], rect[2]);
                    let index = ((y - rect[1]) * (rect[2] - rect[0]) + x - rect[0]) * 4;
                    assert_eq!(
                        &quick[index..index + 4],
                        doc.composite_display_pixel(sample, false)
                    );
                }
            }
        }
        assert!(checked >= 40, "only {checked} random documents were valid");
    }
    #[test]
    fn strip_composite_skips_runs_of_empty_layers_exactly() {
        // Many small layers spread over several storage blocks, so most
        // blocks see long runs of empty clipped and unclipped layers.
        use crate::blend::BlendMode;
        let (w, h) = (600, 300);
        let mut seed = 0x9e37_79b9_7f4a_7c15u64;
        let mut next = move |n: usize| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed % n as u64) as usize
        };
        for _ in 0..12 {
            let mut layers = Vec::new();
            for _ in 0..60 {
                let depth = if next(4) == 0 { 1 } else { 0 };
                if depth == 0 && layers.last().is_some_and(|l: &Layer| l.depth == 1) {
                    let mut folder = Layer::new_folder("folder".into());
                    folder.blend = [BlendMode::Normal, BlendMode::PassThrough][next(2)];
                    layers.push(folder);
                }
                let mut layer = Layer::raster("layer".into(), w, h);
                layer.depth = depth;
                let (x, y) = (next(w - 20), next(h - 20));
                let rect = [x, y, x + 1 + next(20), y + 1 + next(20)];
                let count = (rect[2] - rect[0]) * (rect[3] - rect[1]);
                let rgba = [next(256) as u8, next(256) as u8, next(256) as u8, 200];
                if next(5) != 0 {
                    layer
                        .pixels
                        .writer()
                        .write_region(w, rect, &rgba.repeat(count));
                }
                layer.blend = [BlendMode::Normal, BlendMode::Multiply, BlendMode::Screen][next(3)];
                layer.clipped = next(2) == 0;
                layers.push(layer);
            }
            if layers.last().is_some_and(|l| l.depth == 1) {
                layers.push(Layer::new_folder("folder".into()));
            }
            let mut doc = Document::new(w, h);
            doc.layers = layers;
            doc.active = 0;
            doc.validate().unwrap();
            let fast = doc.composite_display_region([0, 0, w, h], true);
            for (i, pixel) in fast.chunks_exact(4).enumerate() {
                assert_eq!(pixel, doc.composite_display_pixel(i, true), "pixel {i}");
            }
        }
    }
    #[test]
    fn regional_prefix_plan_matches_scalar_states_and_pixels_across_empty_groups() {
        let mut doc = Document::new(257, 35);
        doc.layers[0].pixels = [17, 83, 201, 128].repeat(257 * 35).into();
        doc.layers[0].depth = 1;
        for _ in 0..5 {
            doc.add_layer();
        }
        for layer in &mut doc.layers[1..] {
            layer.depth = 1;
        }
        doc.layers[1].visible = false;
        doc.layers[1]
            .pixels
            .writer()
            .rgba_mut(0)
            .copy_from_slice(&[201, 17, 83, 255]);
        doc.layers[2].clipped = true;
        doc.layers[3].opacity = 0.0;
        doc.layers[3]
            .pixels
            .writer()
            .rgba_mut(0)
            .copy_from_slice(&[19, 211, 73, 255]);
        doc.layers[4].opacity = -0.0;
        doc.layers[5].clipped = true;
        doc.layers[5]
            .pixels
            .writer()
            .rgba_mut(0)
            .copy_from_slice(&[201, 17, 83, 255]);
        doc.layers.push(Layer::new_folder("Group".into()));
        doc.layers[6].opacity = 0.6;
        doc.add_layer();
        doc.layers[7]
            .pixels
            .writer()
            .rgba_mut(34 * 257 + 256)
            .copy_from_slice(&[9, 11, 13, 0]); // Stored hidden RGB must stay conservative.
        doc.add_layer();
        doc.layers[8].depth = 1;
        doc.layers[8]
            .pixels
            .writer()
            .rgba_mut(34 * 257 + 256)
            .copy_from_slice(&[31, 157, 201, 128]);
        doc.add_layer();
        doc.layers[9].depth = 2;
        let mut folder = Layer::new_folder("Nested".into());
        folder.depth = 1;
        doc.layers.push(folder);
        doc.layers.push(Layer::new_folder("Outer".into()));
        doc.add_layer();
        doc.validate().unwrap();
        let modes = crate::blend::BlendMode::ALL
            .into_iter()
            .flat_map(|mode| [(mode, false), (mode, true)]);
        for (mode, pass_folders) in modes {
            for layer in &mut doc.layers {
                layer.blend = if layer.folder && pass_folders {
                    crate::blend::BlendMode::PassThrough
                } else {
                    mode
                };
            }
            for active in 0..doc.layers.len() {
                doc.active = active;
                for rect in [[0, 0, 16, 16], [240, 16, 256, 32], [256, 32, 257, 35]] {
                    let states = doc.prefix_states(rect);
                    assert_eq!(states.len(), (rect[2] - rect[0]) * (rect[3] - rect[1]));
                    for (offset, state) in states.into_iter().enumerate() {
                        let index = (rect[1] + offset / (rect[2] - rect[0])) * doc.width
                            + rect[0]
                            + offset % (rect[2] - rect[0]);
                        let expected = doc.prefix_state(index);
                        assert_eq!(state.top, expected.top);
                        for (actual, expected) in state.levels.into_iter().zip(expected.levels) {
                            assert_eq!(actual.mode, expected.mode);
                            assert_eq!(
                                actual.out.map(f32::to_bits),
                                expected.out.map(f32::to_bits)
                            );
                            assert_eq!(
                                actual.group.map(f32::to_bits),
                                expected.group.map(f32::to_bits)
                            );
                        }
                        assert_eq!(
                            doc.composite_from_prefix(index, state, false, &pass_plan(&doc.layers)),
                            doc.composite_rgba_pixel(index)
                        );
                    }
                }
            }
        }
    }
    #[test]
    fn combined_registry_deduplicates_history_and_frozen_source_without_changing_undo() {
        for flat in [false, true] {
            let mut doc = Document::new(513, 257);
            if flat {
                doc.layers[0].pixels = vec![0; 513 * 257 * 4].into();
            }
            doc.layers[0]
                .pixels
                .writer()
                .rgba_mut(0)
                .copy_from_slice(&[9, 11, 13, 0]);
            let frozen = doc.clone();
            let descriptor = crate::snapshot_memory::Snapshot::capture(&frozen);
            let mut history = History::new();
            history.checkpoint(&doc);
            doc.layers[0].name = "Changed".into();
            doc.layers[0]
                .pixels
                .writer()
                .rgba_mut(0)
                .copy_from_slice(&[17, 83, 201, 255]);
            history.settle(&doc);
            let mut registry = crate::snapshot_memory::Registry::new(&doc);
            history.register_allocations(&mut registry);
            assert_eq!(registry.additional_bytes(), history.retained_bytes());
            let before = registry.additional_bytes();
            let separate = crate::snapshot_memory::beyond_live(&doc, [&descriptor]);
            assert!(separate.pixel_bytes_beyond_live > 0);
            registry.add_snapshot(&descriptor);
            assert_eq!(
                registry.additional_bytes() - before,
                separate.metadata_bytes
            );
            assert!(history.undo(&mut doc));
            assert_eq!(doc.layers[0].pixels, frozen.layers[0].pixels);
            assert_eq!(doc.layers[0].name, frozen.layers[0].name);
            assert!(history.redo(&mut doc));
            assert_eq!(doc.layers[0].pixels.rgba(0), [17, 83, 201, 255]);
            assert_eq!(doc.layers[0].name, "Changed");
        }
    }
    #[test]
    fn registry_counts_shared_patch_and_cooling_input_once_and_restores_both_regions() {
        let mut doc = Document::new(128, 64);
        doc.layers[0].pixels = [201, 17, 83, 255].repeat(128 * 64).into();
        let shared = std::sync::Arc::new([17, 83, 201, 128].repeat(64 * 64));
        let regions: Vec<_> = [[0, 0, 64, 64], [64, 0, 128, 64]]
            .into_iter()
            .map(|rect| RegionPatch {
                rect,
                pixels: crate::history_pixels::Payload::Raw {
                    bytes: shared.clone(),
                    checked: false,
                    prefetched: false,
                },
            })
            .collect();
        let expected = std::mem::size_of::<Patch>()
            + regions.capacity() * std::mem::size_of::<RegionPatch>()
            + crate::history_pixels::raw_capacity(&shared);
        let mut history = History::new();
        history.undo.push(Entry::Patch(Patch {
            layer: 0,
            relief: None,
            regions,
            active: 0,
            selection: None,
            selection_mask: None,
        }));
        history.cooling = crate::history_pixels::Cooling::start(shared);
        let mut registry = crate::snapshot_memory::Registry::new(&doc);
        history.register_allocations(&mut registry);
        assert_eq!(registry.additional_bytes(), expected);
        history.refresh_memory(&doc);
        assert_eq!(history.retained_bytes(), expected);
        assert!(history.undo(&mut doc));
        assert!(
            doc.layers[0]
                .pixels
                .chunks_exact(4)
                .all(|rgba| rgba == [17, 83, 201, 128])
        );
        assert!(history.redo(&mut doc));
        assert!(
            doc.layers[0]
                .pixels
                .chunks_exact(4)
                .all(|rgba| rgba == [201, 17, 83, 255])
        );
    }
    #[test]
    fn idle_maintenance_does_not_trim_a_discardable_gesture() {
        let mut doc = Document::new(37, 39);
        let mut history = History::new();
        for opacity in [0.8, 0.6, 0.4] {
            history.checkpoint(&doc);
            doc.layers[0].opacity = opacity;
            history.settle(&doc);
        }
        assert!(history.undo(&mut doc));
        let undo = history.undo.len();
        let redo = history.redo.len();
        history.begin_gesture_checkpoint(&doc);
        history.budget = 0;
        assert!(!history.cool_idle(&doc));
        assert_eq!(history.undo.len(), undo + 1);
        assert_eq!(history.pending_redo.as_ref().unwrap().len(), redo);
        assert!(history.discard_pending_checkpoint(&doc));
        assert_eq!(history.undo.len(), undo);
        assert_eq!(history.redo.len(), redo);
    }
    #[test]
    fn persistent_layer_metadata_round_trips_without_copying_pixel_payloads() {
        let edits: [fn(&mut Layer); 8] = [
            |layer| layer.csp_byte_opacity = true,
            |layer| layer.name = "Updated name".into(),
            |layer| layer.visible = false,
            |layer| layer.opacity = 0.4,
            |layer| layer.blend = crate::blend::BlendMode::Multiply,
            |layer| layer.clipped = true,
            |layer| layer.locked = true,
            |layer| layer.alpha_locked = true,
        ];
        let assert_same = |a: &Document, b: &Document| {
            assert_eq!(a.active, b.active);
            assert_eq!(a.selection, b.selection);
            assert_eq!(a.layers.len(), b.layers.len());
            for (a, b) in a.layers.iter().zip(&b.layers) {
                assert_eq!(a.name, b.name);
                assert_eq!(a.visible, b.visible);
                assert_eq!(a.opacity, b.opacity);
                assert_eq!(a.blend, b.blend);
                assert_eq!(a.clipped, b.clipped);
                assert_eq!(a.locked, b.locked);
                assert_eq!(a.alpha_locked, b.alpha_locked);
                assert_eq!(a.collapsed, b.collapsed);
                assert!(a.pixels.shares(&b.pixels));
            }
        };
        for edit in edits {
            let mut doc = Document::new(517, 259);
            doc.add_layer();
            doc.layers[0]
                .pixels
                .writer()
                .rgba_mut(0)
                .copy_from_slice(&[1, 2, 3, 255]);
            doc.layers[1]
                .pixels
                .writer()
                .rgba_mut(517 * 259 - 1)
                .copy_from_slice(&[9, 8, 7, 0]);
            let before = doc.clone();
            let mut history = History::new();
            history.checkpoint(&doc);
            edit(&mut doc.layers[1]);
            history.settle(&doc);
            let after = doc.clone();
            assert!(matches!(history.undo.last(), Some(Entry::Full(_))));
            assert!(history.retained_bytes() < 4096);
            assert!(history.undo(&mut doc));
            assert_same(&doc, &before);
            assert!(history.redo(&mut doc));
            assert_same(&doc, &after);
            assert!(doc.validate().is_ok());
        }
    }
    #[test]
    fn distant_flat_and_mixed_edits_use_small_exact_history_patches() {
        let (width, height) = (517, 259);
        let before_bytes = vec![255; width * height * 4];
        let mut after_bytes = before_bytes.clone();
        after_bytes[..4].copy_from_slice(&[7, 8, 9, 0]);
        let last = after_bytes.len() - 4;
        after_bytes[last..].copy_from_slice(&[10, 20, 30, 0]);
        let make = |bytes: &[u8], flat| {
            if flat {
                crate::pixels::Pixels::from(bytes.to_vec())
            } else {
                crate::pixels::Pixels::read_rgba(&mut &bytes[..], width, height).unwrap()
            }
        };
        for before_flat in [false, true] {
            for after_flat in [false, true] {
                let mut doc = Document::new(width, height);
                doc.layers[0].pixels = make(&before_bytes, before_flat);
                let mut history = History::new();
                history.checkpoint(&doc);
                doc.layers[0].pixels = make(&after_bytes, after_flat);
                history.settle(&doc);
                assert!(
                    history.retained_bytes() < 4096,
                    "distant edits retained a broad gap"
                );
                let Some(Entry::Patch(patch)) = history.undo.last() else {
                    panic!("edit was not compacted")
                };
                assert_eq!(patch.regions.len(), 2);
                assert_eq!(patch.regions[0].rect, [0, 0, 1, 1]);
                assert_eq!(patch.regions[1].rect, [516, 258, 517, 259]);
                assert!(history.undo(&mut doc));
                assert_eq!(doc.layers[0].pixels, before_bytes);
                assert!(history.redo(&mut doc));
                assert_eq!(doc.layers[0].pixels, after_bytes);
            }
        }
    }
    fn finish_cooling(history: &mut History, doc: &Document) {
        // Only a hang guard: a loaded machine can take far longer than usual.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(300);
        while history.cool_idle(doc) {
            assert!(
                std::time::Instant::now() < deadline,
                "Undo compression did not finish"
            );
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }
    #[test]
    fn cold_history_compresses_older_patches_and_preserves_complete_undo_redo_cycles() {
        let mut doc = Document::new(517, 259);
        let mut history = History::new();
        let mut states = vec![doc.clone()];
        for step in 1..=6 {
            history.checkpoint(&doc);
            {
                let mut writer = doc.layers[0].pixels.writer();
                for index in 0..517 * 259 {
                    writer.rgba_mut(index).copy_from_slice(&[
                        step * 11,
                        90,
                        140,
                        if index % 7 == 0 { 0 } else { 128 },
                    ]);
                }
            }
            doc.selection = Some(crate::selection::Selection {
                left: step as usize,
                top: 0,
                right: 516,
                bottom: 258,
            });
            history.settle(&doc);
            states.push(doc.clone());
        }
        let raw_bytes = history.retained_bytes();
        finish_cooling(&mut history, &doc);
        assert!(history.retained_bytes() < raw_bytes * 3 / 5);
        assert!(history.compressed_regions() > 0);
        assert!(history.undo.iter().rev().take(2).all(|entry| matches!(entry, Entry::Patch(p) if
            p.regions.iter().all(|r| matches!(r.pixels, crate::history_pixels::Payload::Raw { .. })) )));
        for _ in 0..2 {
            for state in states[..6].iter().rev() {
                assert!(history.undo(&mut doc));
                assert_eq!(doc.layers[0].pixels, state.layers[0].pixels);
                assert_eq!(doc.selection, state.selection);
                assert_eq!(history.bytes, history.measure(&doc));
            }
            assert!(!history.can_undo());
            finish_cooling(&mut history, &doc);
            assert!(history.compressed_regions() > 0);
            for state in &states[1..] {
                assert!(history.redo(&mut doc));
                assert_eq!(doc.layers[0].pixels, state.layers[0].pixels);
                assert_eq!(doc.selection, state.selection);
            }
            finish_cooling(&mut history, &doc);
        }
        history.begin_stroke_checkpoint(&doc);
        assert!(history.cooling.is_none());
        assert!(!history.cool_idle(&doc));
        assert!(history.discard_pending_checkpoint(&doc));
        history.refresh_memory(&doc);
        assert_eq!(history.bytes, history.measure(&doc));
    }
    #[test]
    fn compression_results_cannot_replace_an_inverse_created_during_worker_execution() {
        let mut doc = Document::new(256, 256);
        let mut history = History::new();
        history.checkpoint(&doc);
        doc.layers[0].pixels = vec![87; 256 * 256 * 4].into();
        history.settle(&doc);
        let painted = doc.layers[0].pixels.clone();
        let Entry::Patch(patch) = &history.undo[0] else {
            unreachable!()
        };
        let source = patch.regions[0].pixels.cold_candidate().unwrap().clone();
        history.cooling = crate::history_pixels::Cooling::start(source);
        assert!(history.cooling.is_some());
        assert!(history.undo(&mut doc));
        finish_cooling(&mut history, &doc);
        assert_eq!(history.compressed_regions(), 0);
        assert!(history.redo(&mut doc));
        assert_eq!(doc.layers[0].pixels, painted);
    }
    fn prefetch_fixture() -> (Document, History) {
        let mut doc = Document::new(128, 128);
        let mut history = History::new();
        for step in 1..=6 {
            history.checkpoint(&doc);
            doc.layers[0].pixels = vec![step * 20; 128 * 128 * 4].into();
            history.settle(&doc);
        }
        finish_cooling(&mut history, &doc);
        assert_eq!(history.compressed_regions(), 4);
        for _ in 0..2 {
            assert!(history.undo(&mut doc));
        }
        (doc, history)
    }
    #[test]
    fn nearby_prefetch_obeys_cache_and_history_budgets_without_evicting_actions() {
        let (mut doc, mut history) = prefetch_fixture();
        history.prefetch_budget = 128 * 128 * 4 + 41;
        let total_actions = history.undo.len() + history.redo.len();
        finish_cooling(&mut history, &doc);
        assert_eq!(history.prefetched_bytes(), history.prefetch_budget);
        assert_eq!(history.undo.len() + history.redo.len(), total_actions);
        let Entry::Patch(last) = history.undo.last().unwrap() else {
            unreachable!()
        };
        assert!(matches!(
            last.regions[0].pixels,
            crate::history_pixels::Payload::Raw {
                prefetched: true,
                ..
            }
        ));
        assert!(history.undo(&mut doc));
        assert_eq!(history.prefetched_bytes(), 0);
        assert_eq!(doc.layers[0].pixels.rgba(0), [60; 4]);
        finish_cooling(&mut history, &doc);
        assert!(history.prefetched_bytes() <= history.prefetch_budget);
        let (doc, mut history) = prefetch_fixture();
        history.budget = history.retained_bytes();
        finish_cooling(&mut history, &doc);
        assert_eq!(history.prefetched_bytes(), 0);
        assert_eq!(history.undo.len() + history.redo.len(), 6);
    }
    #[test]
    fn stale_prefetch_after_undo_or_eviction_cannot_replace_inverse_data() {
        let (mut doc, mut history) = prefetch_fixture();
        let Entry::Patch(patch) = history.undo.last().unwrap() else {
            unreachable!()
        };
        let (bytes, len) = patch.regions[0].pixels.warm_candidate().unwrap();
        history.cooling = crate::history_pixels::Cooling::prefetch(bytes.clone(), len);
        assert!(history.undo(&mut doc));
        finish_cooling(&mut history, &doc);
        let Entry::Patch(last) = history.redo.last().unwrap() else {
            unreachable!()
        };
        assert!(matches!(
            last.regions[0].pixels,
            crate::history_pixels::Payload::Raw {
                prefetched: false,
                ..
            }
        ));
        for _ in 0..3 {
            assert!(history.redo(&mut doc));
        }
        assert_eq!(doc.layers[0].pixels.rgba(0), [120; 4]);
        let (doc, mut history) = prefetch_fixture();
        let Entry::Patch(patch) = &history.undo[0] else {
            unreachable!()
        };
        let (bytes, len) = patch.regions[0].pixels.warm_candidate().unwrap();
        history.cooling = crate::history_pixels::Cooling::prefetch(bytes.clone(), len);
        history.budget = 1;
        history.settle(&doc);
        finish_cooling(&mut history, &doc);
        assert_eq!(history.prefetched_bytes(), 0);
        assert_eq!(history.undo.len() + history.redo.len(), 1);
    }
    #[test]
    fn prefetch_rechecks_reduced_budget_and_decode_failure_never_changes_artwork() {
        let (mut doc, mut history) = prefetch_fixture();
        let before = doc.layers[0].pixels.clone();
        let Entry::Patch(patch) = history.undo.last().unwrap() else {
            unreachable!()
        };
        let (bytes, len) = patch.regions[0].pixels.warm_candidate().unwrap();
        history.cooling = crate::history_pixels::Cooling::prefetch(bytes.clone(), len);
        history.prefetch_budget = 0;
        finish_cooling(&mut history, &doc);
        assert_eq!(history.prefetched_bytes(), 0);
        assert_eq!(doc.layers[0].pixels, before);
        history.prefetch_budget = History::PREFETCH_MIB * 1024 * 1024;
        let Entry::Patch(patch) = history.undo.last_mut().unwrap() else {
            unreachable!()
        };
        patch.regions[0].pixels =
            crate::history_pixels::Payload::packed(Box::new([0, 1, 2]), 128 * 128 * 4);
        finish_cooling(&mut history, &doc);
        assert_eq!(doc.layers[0].pixels, before);
        assert!(!history.cool_idle(&doc));
        assert!(!history.undo(&mut doc));
        assert!(history.take_error().is_some());
        assert_eq!(doc.layers[0].pixels, before);
        assert_eq!(history.undo.len() + history.redo.len(), 6);
    }
    #[test]
    fn compression_during_cancelable_stroke_preserves_pending_redo_and_eviction_discards_stale_results()
     {
        let mut doc = Document::new(64, 64);
        let mut history = History::new();
        for step in 1..=4 {
            history.checkpoint(&doc);
            doc.layers[0].pixels = vec![step * 20; 64 * 64 * 4].into();
            history.settle(&doc);
        }
        let painted = doc.layers[0].pixels.clone();
        for _ in 0..4 {
            assert!(history.undo(&mut doc));
        }
        let Entry::Patch(patch) = &history.redo[0] else {
            unreachable!()
        };
        history.cooling = crate::history_pixels::Cooling::start(
            patch.regions[0].pixels.cold_candidate().unwrap().clone(),
        );
        history.begin_stroke_checkpoint(&doc);
        let budget = history.budget;
        history.budget = 1;
        finish_cooling(&mut history, &doc);
        assert!(history.has_pending_stroke());
        assert!(history.compressed_regions() > 0);
        assert_eq!(history.pending_redo.as_ref().unwrap().len(), 4);
        history.budget = budget;
        assert!(history.discard_pending_checkpoint(&doc));
        for _ in 0..4 {
            assert!(history.redo(&mut doc));
        }
        assert_eq!(doc.layers[0].pixels, painted);
        assert_eq!(history.undo.len(), 4);
        let Entry::Patch(patch) = &history.undo[0] else {
            unreachable!()
        };
        history.cooling = crate::history_pixels::Cooling::start(
            patch.regions[0].pixels.cold_candidate().unwrap().clone(),
        );
        history.budget = 1;
        history.settle(&doc);
        assert_eq!(history.undo.len(), 1);
        finish_cooling(&mut history, &doc);
        assert_eq!(history.undo.len(), 1);
        assert_eq!(history.compressed_regions(), 0);
        assert!(history.undo(&mut doc));
        assert_eq!(doc.layers[0].pixels.rgba(0), [60; 4]);
        assert!(!history.can_undo());
    }
    #[test]
    fn late_decode_failure_preserves_artwork_and_keeps_the_action_available() {
        let mut doc = Document::new(512, 256);
        let mut history = History::new();
        history.checkpoint(&doc);
        for index in 0..512 * 256 {
            doc.layers[0]
                .pixels
                .writer()
                .rgba_mut(index)
                .copy_from_slice(&[1, 2, 3, 128]);
        }
        history.settle(&doc);
        let painted = doc.layers[0].pixels.clone();
        let Entry::Patch(patch) = &mut history.undo[0] else {
            unreachable!()
        };
        assert_eq!(patch.regions.len(), 2);
        let len = patch.regions[0].pixels.len();
        patch.regions[0].pixels = crate::history_pixels::Payload::packed(
            crate::history_pixels::compress(&vec![0; len]).unwrap(),
            len,
        );
        patch.regions[1].pixels = crate::history_pixels::Payload::packed(Box::new([0, 1, 2]), len);
        assert!(!history.undo(&mut doc));
        assert!(history.take_error().is_some());
        assert_eq!(doc.layers[0].pixels, painted);
        assert_eq!(history.undo.len(), 1);
        assert!(!history.can_redo());
        let Entry::Patch(patch) = &mut history.undo[0] else {
            unreachable!()
        };
        patch.regions[1].pixels = crate::history_pixels::Payload::new(vec![0; len]);
        assert!(history.undo(&mut doc));
        assert_eq!(doc.layers[0].pixels.capacity(), 0);
        assert!(history.redo(&mut doc));
        assert_eq!(doc.layers[0].pixels, painted);
    }
    #[test]
    fn partial_composite_matches_full_rows_and_clamps_bounds() {
        let mut doc = Document::new(8, 8);
        doc.dab(4.0, 4.0, 1.0, &Brush::default(), [78, 132, 204], false);
        doc.add_layer();
        doc.layers[1].blend = crate::blend::BlendMode::Multiply;
        doc.dab(5.0, 5.0, 0.5, &Brush::default(), [211, 78, 91], false);
        let full = doc.composite();
        let region = doc.composite_region(2, 3, 6, 7);
        for row in 0..4 {
            assert_eq!(
                &region[row * 16..(row + 1) * 16],
                &full[((row + 3) * 8 + 2) * 4..((row + 3) * 8 + 6) * 4]
            );
        }
        assert_eq!(doc.composite_region(0, 0, 99, 99), full);
        assert!(doc.composite_region(7, 7, 2, 2).is_empty());
    }
    #[test]
    fn grain_holes_stay_fixed_between_overlapping_dabs() {
        let brush = Brush {
            size: 16.0,
            hardness: 1.0,
            pressure_size: false,
            grain_amount: 1.0,
            grain: Some(crate::brush::TipImage {
                width: 2,
                height: 1,
                alpha: (vec![0, 255]).into(),
            }),
            ..Brush::default()
        };
        let mut doc = Document::new(16, 16);
        doc.dab(7.0, 8.0, 1.0, &brush, [0; 3], false);
        doc.dab(9.0, 8.0, 1.0, &brush, [0; 3], false);
        assert_eq!(doc.layers[0].pixels[(8 * 16 + 8) * 4 + 3], 0);
        assert_eq!(doc.layers[0].pixels[(8 * 16 + 9) * 4 + 3], 255);
    }
    #[test]
    fn image_tip_paints_only_its_mask_and_validation_rejects_bad_assets() {
        let mut brush = Brush {
            size: 16.0,
            pressure_size: false,
            tip: crate::brush::Tip::Image,
            image: Some(crate::brush::TipImage {
                width: 2,
                height: 2,
                alpha: (vec![255, 0, 0, 0]).into(),
            }),
            ..Brush::default()
        };
        brush.validate().unwrap();
        let mut doc = Document::new(32, 32);
        doc.dab(16.0, 16.0, 1.0, &brush, [255, 0, 0], false);
        assert_eq!(doc.layers[0].pixels[(10 * 32 + 10) * 4 + 3], 255);
        assert!(doc.layers[0].pixels[(15 * 32 + 15) * 4 + 3] > 0);
        assert!(doc.layers[0].pixels[(15 * 32 + 15) * 4 + 3] < 255);
        assert_eq!(doc.layers[0].pixels[(20 * 32 + 20) * 4 + 3], 0);
        brush.image.as_mut().unwrap().alpha.pop();
        assert!(brush.validate().is_err());
    }
    #[test]
    fn tip_shape_aspect_and_rotation_change_the_dab_footprint() {
        let ink = Brush {
            size: 16.0,
            hardness: 1.0,
            pressure_size: false,
            ..Brush::default()
        };
        let mut round = Document::new(32, 32);
        round.dab(16.0, 16.0, 1.0, &ink, [0; 3], false);
        let square = Brush {
            tip: crate::brush::Tip::Square,
            ..ink.clone()
        };
        let mut block = Document::new(32, 32);
        block.dab(16.0, 16.0, 1.0, &square, [0; 3], false);
        let corner = (22 * 32 + 22) * 4 + 3;
        assert_eq!(round.layers[0].pixels[corner], 0);
        assert_eq!(block.layers[0].pixels[corner], 255);
        let ellipse = Brush {
            aspect: 0.25,
            ..ink
        };
        let mut narrow = Document::new(32, 32);
        narrow.dab(16.0, 16.0, 1.0, &ellipse, [0; 3], false);
        let rotated = Brush {
            angle: 90.0,
            ..ellipse
        };
        let mut vertical = Document::new(32, 32);
        vertical.dab(16.0, 16.0, 1.0, &rotated, [0; 3], false);
        assert!(narrow.layers[0].pixels[(16 * 32 + 22) * 4 + 3] > 0);
        assert_eq!(vertical.layers[0].pixels[(16 * 32 + 22) * 4 + 3], 0);
        assert!(vertical.layers[0].pixels[(22 * 32 + 16) * 4 + 3] > 0);
    }
    #[test]
    fn a_turned_narrow_tip_paints_the_same_pixels_from_its_tight_box() {
        use crate::brush::Tip;
        let maps = [
            crate::symmetry::IDENTITY,
            [-1.0, 0.0, 0.0, 1.0],
            [0.0, -1.0, 1.0, 0.0],
        ];
        let sample = crate::stroke::Sample {
            x: 80.3,
            y: 77.6,
            pressure: 1.0,
            ..Default::default()
        };
        let area = |r: &DabRaster| (r.right - r.left + 1) * (r.bottom - r.top + 1);
        let (mut smaller, mut painted) = (0, 0);
        for tip in [Tip::Round, Tip::Square] {
            for aspect in [1.0, 0.3, 0.05] {
                for angle in [0.0, 30.0, 45.0, 90.0, 137.0] {
                    for (size, hardness) in [(1.5, 1.0), (3.0, 0.2), (40.0, 1.0), (120.0, 0.2)] {
                        for map in maps {
                            let brush = Brush {
                                size,
                                aspect,
                                angle,
                                tip,
                                hardness,
                                pressure_size: false,
                                ..Brush::default()
                            };
                            let tight = DabRaster::new(sample, &brush, map, 160, 160);
                            // The square around the long side: the box every
                            // dab had before.
                            let mut full = DabRaster::new(sample, &brush, map, 160, 160);
                            let extent = 0.5
                                + size / 2.0
                                    * if tip == Tip::Square {
                                        std::f32::consts::SQRT_2
                                    } else {
                                        1.0
                                    };
                            full.left = ((sample.x - extent).floor() as i32).max(0);
                            full.right = ((sample.x + extent).ceil() as i32).min(159);
                            full.top = ((sample.y - extent).floor() as i32).max(0);
                            full.bottom = ((sample.y + extent).ceil() as i32).min(159);
                            assert!(
                                tight.left >= full.left
                                    && tight.right <= full.right
                                    && tight.top >= full.top
                                    && tight.bottom <= full.bottom
                            );
                            smaller += usize::from(area(&tight) < area(&full));
                            let paint = |raster: &DabRaster| {
                                let mut doc = Document::new(160, 160);
                                doc.paint_dab_rows(
                                    raster,
                                    &brush,
                                    [20, 60, 200],
                                    false,
                                    None,
                                    raster.top,
                                    raster.bottom,
                                );
                                doc.layers[0].pixels.as_slice().to_vec()
                            };
                            let (a, b) = (paint(&tight), paint(&full));
                            painted += usize::from(a.iter().any(|v| *v != 0));
                            assert!(
                                a == b,
                                "{tip:?} {size} px, aspect {aspect}, {angle} degrees, {map:?}"
                            );
                        }
                    }
                }
            }
        }
        assert!(smaller > 100 && painted > 300, "{smaller} {painted}");
        // The tip of the lag report: a third as wide as long, at 45 degrees.
        let chisel = Brush {
            size: 300.0,
            aspect: 0.3,
            angle: 45.0,
            pressure_size: false,
            ..Brush::default()
        };
        let at = crate::stroke::Sample {
            x: 400.0,
            y: 400.0,
            pressure: 1.0,
            ..Default::default()
        };
        let raster = DabRaster::new(at, &chisel, crate::symmetry::IDENTITY, 800, 800);
        assert!(area(&raster) * 100 < 302 * 302 * 60, "{}", area(&raster));
        let level = Brush {
            angle: 0.0,
            ..chisel
        };
        let raster = DabRaster::new(at, &level, crate::symmetry::IDENTITY, 800, 800);
        assert!(raster.right - raster.left >= 300 && raster.bottom - raster.top <= 94);
    }
    #[test]
    fn a_wet_dab_held_to_wider_spacing_delivers_more() {
        let at = crate::stroke::Sample {
            x: 400.0,
            y: 400.0,
            pressure: 1.0,
            ..Default::default()
        };
        let paint = Brush {
            size: 300.0,
            aspect: 0.3,
            spacing: 0.05,
            pressure_size: false,
            ..Brush::default()
        };
        let wet = Brush {
            blend: crate::brush::Blend::Wet,
            ..paint.clone()
        };
        let ratio =
            |brush: &Brush| DabRaster::new(at, brush, crate::symmetry::IDENTITY, 800, 800).held;
        assert_eq!(ratio(&paint), None);
        // Asked 4.5 px, held to 6 px.
        assert!((ratio(&wet).unwrap() - 6.0 / 4.5).abs() < 1e-4);
        // A round wet brush at its usual spacing is left alone.
        let round = Brush {
            aspect: 1.0,
            spacing: 0.08,
            ..wet
        };
        assert_eq!(ratio(&round), None);
    }
    #[test]
    fn history_accounts_for_both_stacks_and_variable_layer_sizes() {
        let mut doc = Document::new(64, 64);
        let mut history = History::new();
        history.budget = 64 * 64 * 4 * 3;
        for _ in 0..8 {
            history.checkpoint(&doc);
            doc.add_layer();
        }
        // Adding layers shares every existing buffer: snapshots cost no pixels.
        assert_eq!(history.bytes, history.measure(&doc));
        assert!(history.bytes < 64 * 64 * 4);
        for _ in 0..6 {
            history.checkpoint(&doc);
            doc.layers[0].pixels.fill(7);
            doc.layers[0].pixels[0] = history.undo.len() as u8;
        }
        history.settle(&doc);
        assert_eq!(history.bytes, history.measure(&doc));
        assert!(history.bytes <= history.budget || history.undo.len() + history.redo.len() == 1);
        assert!(history.undo(&mut doc));
        assert_eq!(history.bytes, history.measure(&doc));
        assert!(history.redo(&mut doc));
        assert_eq!(history.bytes, history.measure(&doc));
        history.undo(&mut doc);
        history.checkpoint(&doc);
        assert!(history.redo.is_empty());
        assert_eq!(history.bytes, history.measure(&doc));
    }
    #[test]
    fn whole_sparse_clear_restores_pixels_through_history() {
        let mut doc = Document::new(517, 259);
        doc.layers[0]
            .pixels
            .writer()
            .rgba_mut(0)
            .copy_from_slice(&[1, 2, 3, 255]);
        doc.layers[0]
            .pixels
            .writer()
            .rgba_mut(258 * 517 + 516)
            .copy_from_slice(&[4, 5, 6, 128]);
        let original = doc.layers[0].pixels.clone();
        let mut history = History::new();
        history.checkpoint(&doc);
        assert!(doc.clear_selected().unwrap());
        assert_eq!(doc.layers[0].pixels.capacity(), 0);
        assert!(history.undo(&mut doc));
        for index in 0..517 * 259 {
            assert_eq!(doc.layers[0].pixels.rgba(index), original.rgba(index));
        }
        assert!(history.redo(&mut doc));
        for index in 0..517 * 259 {
            assert_eq!(doc.layers[0].pixels.rgba(index), [0; 4]);
        }
        assert_eq!(doc.layers[0].pixels.capacity(), 0);
        assert!(history.undo(&mut doc));
        assert_eq!(doc.layers[0].pixels.rgba(258 * 517 + 516), [4, 5, 6, 128]);
        assert!(history.redo(&mut doc));
        assert_eq!(doc.layers[0].pixels.capacity(), 0);
    }
    #[test]
    fn shared_tile_payloads_do_not_evict_full_snapshot_history_early() {
        let mut doc = Document::new(768, 256);
        {
            let mut writer = doc.layers[0].pixels.writer();
            writer.rgba_mut(0).copy_from_slice(&[5, 0, 0, 255]);
            writer.rgba_mut(512).copy_from_slice(&[7, 0, 0, 255]);
        }
        let mut history = History::new();
        history.budget = 850 * 1024;
        for step in 0..3 {
            history.checkpoint(&doc);
            // Metadata forces a full snapshot, while only one of two tiles changes.
            doc.layers[0].opacity = 0.9 - step as f32 * 0.1;
            doc.layers[0].pixels.writer().rgba_mut(0)[0] = 11 + step;
        }
        history.settle(&doc);
        assert_eq!(history.undo.len(), 3);
        assert!(
            history
                .undo
                .iter()
                .all(|entry| matches!(entry, Entry::Full(_)))
        );
        // Three historical versions of tile zero are owned by history. The
        // other tile is shared with live pixels and must never be charged.
        assert!(history.bytes >= 3 * 256 * 256 * 4);
        assert!(history.bytes < 3 * 256 * 256 * 4 + 16 * 1024);
        for expected in [12, 11, 5] {
            assert!(history.undo(&mut doc));
            assert_eq!(doc.layers[0].pixels.rgba(0)[0], expected);
            assert_eq!(doc.layers[0].pixels.rgba(512), [7, 0, 0, 255]);
            assert!(history.bytes < history.budget);
        }
        for expected in [11, 12, 13] {
            assert!(history.redo(&mut doc));
            assert_eq!(doc.layers[0].pixels.rgba(0)[0], expected);
            assert!(history.bytes < history.budget);
        }
        // Finished full snapshots must enforce a reduced budget immediately,
        // even when metadata prevents pixel-only patch compaction.
        history.budget = 400 * 1024;
        history.settle(&doc);
        assert_eq!(history.undo.len(), 1);
        assert!(history.bytes < history.budget);
        assert!(history.undo(&mut doc));
        assert_eq!(doc.layers[0].pixels.rgba(0)[0], 12);
        assert!(!history.can_undo());
    }
    #[test]
    fn strokes_compact_to_region_patches_that_undo_and_redo_exactly() {
        let mut doc = Document::new(256, 256);
        doc.add_layer();
        doc.active = 0;
        let mut history = History::new();
        let brush = Brush {
            size: 10.0,
            pressure_size: false,
            ..Brush::default()
        };
        let mut states = vec![doc.clone()];
        for step in 0..3 {
            history.checkpoint(&doc);
            doc.dab(
                40.0 + step as f32 * 60.0,
                50.0,
                1.0,
                &brush,
                [200, 10, 10],
                false,
            );
            history.settle(&doc);
            states.push(doc.clone());
        }
        doc.active = 1; // selecting a layer is not an action and must not block compaction
        history.checkpoint(&doc);
        doc.selection = Some(crate::selection::Selection {
            left: 0,
            top: 0,
            right: 8,
            bottom: 8,
        });
        history.settle(&doc);
        assert!(history.undo.iter().all(|e| matches!(e, Entry::Patch(_))));
        // Four small patches, far below one 256 KiB layer copy.
        assert!(history.bytes < 256 * 256 * 4 / 4, "{}", history.bytes);
        assert!(history.undo(&mut doc));
        assert_eq!(doc.selection, None);
        for expected in states.iter().rev().skip(1) {
            assert!(history.undo(&mut doc));
            assert_eq!(doc.layers[0].pixels, expected.layers[0].pixels);
            assert_eq!(doc.active, 0);
        }
        assert!(!history.undo(&mut doc));
        for expected in states.iter().skip(1) {
            assert!(history.redo(&mut doc));
            assert_eq!(doc.layers[0].pixels, expected.layers[0].pixels);
        }
        assert!(history.redo(&mut doc));
        assert!(doc.selection.is_some());
        assert_eq!(doc.active, 1);
    }
    #[test]
    fn sparse_tile_history_avoids_empty_diagonal_bounds_and_restores_edges() {
        let mut doc = Document::new(1025, 1031);
        let original = doc.layers[0].pixels.clone();
        let mut history = History::new();
        history.checkpoint(&doc);
        {
            let mut writer = doc.layers[0].pixels.writer();
            for x in 0..1025 {
                writer
                    .rgba_mut(x * 1025 + x)
                    .copy_from_slice(&[71, 93, 128, 255]);
            }
            // Last row and last column exercise partial edge tiles and hidden RGB.
            writer
                .rgba_mut(1030 * 1025 + 1024)
                .copy_from_slice(&[9, 8, 7, 0]);
        }
        let painted = doc.layers[0].pixels.clone();
        history.settle(&doc);
        let Entry::Patch(patch) = &history.undo[0] else {
            panic!("Expected compact patch")
        };
        assert_eq!(patch.regions.len(), 5);
        let payload: usize = patch.regions.iter().map(|r| r.pixels.len()).sum();
        assert_eq!(payload, 4 * 256 * 256 * 4 + 7 * 4);
        assert!(history.retained_bytes() < 1025 * 1031 * 4 / 3);
        for _ in 0..3 {
            assert!(history.undo(&mut doc));
            for i in 0..1025 * 1031 {
                assert_eq!(doc.layers[0].pixels.rgba(i), original.rgba(i));
            }
            assert!(history.redo(&mut doc));
            for i in 0..1025 * 1031 {
                assert_eq!(doc.layers[0].pixels.rgba(i), painted.rgba(i));
            }
        }
    }
    #[test]
    fn changed_rect_bounds_every_difference() {
        let before = vec![0u8; 100 * 50 * 4];
        let mut after = before.clone();
        after[(10 * 100 + 70) * 4] = 1;
        after[(30 * 100 + 5) * 4 + 3] = 1;
        let [left, top, right, bottom] = changed_rect(&before, &after, 100, 50).unwrap();
        assert!(left <= 5 && right > 70 && top == 10 && bottom == 31);
        assert_eq!(changed_rect(&before, &before, 100, 50), None);
    }
    #[test]
    fn reference_composite_cancellation_stops_bands_without_exposing_partial_pixels() {
        use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
        for (width, height) in [(73, 71), (513, 257)] {
            let mut doc = Document::new(width, height);
            doc.layers[0].pixels = [17, 83, 201, 128].repeat(width * height).into();
            let original = doc.clone();
            let cancel = AtomicBool::new(false);
            let checks = AtomicUsize::new(0);
            let result = doc.composite_bounds_checked([0, 0, width, height], true, || {
                if checks.fetch_add(1, Ordering::Relaxed) >= 2 {
                    cancel.store(true, Ordering::Relaxed);
                }
                crate::fill::check_cancel(Some(&cancel))
            });
            assert_eq!(result.unwrap_err(), "Fill canceled");
            assert!(checks.load(Ordering::Relaxed) >= 3);
            assert!(
                checks.load(Ordering::Relaxed) <= 20,
                "All bands should stop at their next check"
            );
            assert!(same_snapshot(&doc, &original));
            assert_eq!(doc.layers[0].pixels, original.layers[0].pixels);
            let edit = crate::document_edits::Edit::FillTarget {
                target: doc.layers[0].id,
                source: crate::fill::Source::Visible,
                point: [0, 0],
                color: [201, 83, 17],
                opacity: 1.0,
                tolerance: 0,
                options: crate::fill::Options::default(),
            };
            assert_eq!(
                edit.prepare_control(&doc, Some(&cancel)).err().unwrap(),
                "Fill canceled"
            );
            assert!(same_snapshot(&doc, &original));
        }
    }

    #[test]
    fn checked_reference_composite_matches_pixel_oracle_across_bands_and_layer_styles() {
        for (width, height) in [(73, 71), (513, 257)] {
            let mut doc = Document::new(width, height);
            let pixels: Vec<u8> = (0..width * height)
                .flat_map(|index| {
                    [
                        (index % 251) as u8,
                        (index % 193) as u8,
                        83,
                        (index % 255) as u8,
                    ]
                })
                .collect();
            doc.layers[0].pixels = pixels.into();
            doc.layers[0].opacity = 0.7;
            doc.group_active().unwrap();
            doc.layers[doc.active].opacity = 0.6;
            doc.add_layer();
            doc.layers[doc.active].pixels = [17, 83, 201, 127].repeat(width * height).into();
            doc.layers[doc.active].clipped = true;
            doc.layers[doc.active].blend = crate::blend::BlendMode::Multiply;
            doc.validate().unwrap();
            for transparent in [false, true] {
                let rect = [2, 3, width - 2, height - 3];
                let out = doc
                    .composite_bounds_checked(rect, transparent, || Ok(()))
                    .unwrap();
                for (offset, actual) in out.chunks_exact(4).enumerate() {
                    let row_width = rect[2] - rect[0];
                    let index =
                        (rect[1] + offset / row_width) * width + rect[0] + offset % row_width;
                    let value = doc.composite_pixel(
                        index,
                        &mut [EMPTY_LEVEL; MAX_DEPTH as usize + 1],
                        &pass_plan(&doc.layers),
                    );
                    let rgb = [0, 1, 2].map(|channel| {
                        if transparent {
                            if value[3] > 0.0 {
                                (value[channel] / value[3]).round() as u8
                            } else {
                                0
                            }
                        } else {
                            (value[channel] + 255.0 * (1.0 - value[3])).round() as u8
                        }
                    });
                    let alpha = if transparent {
                        (value[3] * 255.0).round() as u8
                    } else {
                        255
                    };
                    assert_eq!(actual, [rgb[0], rgb[1], rgb[2], alpha]);
                }
            }
        }
    }

    #[test]
    fn parallel_composite_matches_single_thread_rows() {
        let mut doc = Document::new(300, 300);
        doc.dab(
            150.0,
            150.0,
            1.0,
            &Brush {
                size: 200.0,
                ..Brush::default()
            },
            [10, 200, 90],
            false,
        );
        let full = doc.composite();
        let band = doc.composite_region(0, 120, 300, 121);
        assert_eq!(&full[120 * 300 * 4..121 * 300 * 4], band.as_slice());
    }
    #[test]
    fn clipping_chains_preserve_base_alpha_and_hidden_bases_hide_the_chain() {
        let mut doc = Document::new(2, 1);
        doc.layers[0].pixels = vec![255, 0, 0, 128, 0, 0, 0, 0].into();
        doc.add_layer();
        doc.layers[1].clipped = true;
        doc.layers[1].pixels = vec![0, 255, 0, 255, 0, 255, 0, 255].into();
        assert_eq!(
            doc.composite_transparent(),
            vec![0, 255, 0, 128, 0, 0, 0, 0]
        );
        doc.add_layer();
        doc.layers[2].clipped = true;
        doc.layers[2].pixels = vec![0, 0, 255, 255, 0, 0, 255, 255].into();
        assert_eq!(
            doc.composite_transparent(),
            vec![0, 0, 255, 128, 0, 0, 0, 0]
        );
        doc.layers[0].visible = false;
        assert_eq!(doc.composite_transparent(), vec![0; 8]);
        doc.layers[0].visible = true;
        doc.layers[0].opacity = 0.5;
        assert_eq!(doc.composite_transparent()[3], 64);
        assert_eq!(doc.layers[1].pixels[7], 255); // outside-base marks remain nondestructive
    }
    #[test]
    fn pass_through_glow_and_dissolve_follow_their_definitions() {
        use crate::blend::BlendMode;
        let pixel = |doc: &Document| doc.composite_transparent()[..4].to_vec();
        let near = |actual: Vec<u8>, expected: &[f32]| {
            assert!(
                actual
                    .iter()
                    .zip(expected.iter().copied())
                    .all(|(a, e)| (f32::from(*a) - e).abs() <= 1.0),
                "{actual:?} vs {expected:?}"
            );
        };
        // Gray backdrop, then a Multiply layer inside a folder.
        let mut doc = Document::new(1, 1);
        doc.layers[0].pixels = vec![128, 128, 128, 255].into();
        doc.add_layer();
        doc.layers[1].pixels = vec![100, 100, 100, 255].into();
        doc.layers[1].blend = BlendMode::Multiply;
        let flat = pixel(&doc);
        doc.layers[1].depth = 1;
        doc.layers.push(Layer::new_folder("Folder".into()));
        doc.validate().unwrap();
        // Isolated: Multiply over nothing is the plain color.
        near(pixel(&doc), &[100.0, 100.0, 100.0, 255.0]);
        doc.layers[2].blend = BlendMode::PassThrough;
        assert_eq!(pixel(&doc), flat);
        near(flat.clone(), &[50.2, 50.2, 50.2, 255.0]);
        doc.layers[2].opacity = 0.5;
        near(pixel(&doc), &[89.1, 89.1, 89.1, 255.0]);
        doc.layers[2].visible = false;
        assert_eq!(pixel(&doc), vec![128, 128, 128, 255]);
        // A layer clipped to a pass-through folder clips to its contents.
        doc.layers[2].visible = true;
        doc.layers[2].opacity = 1.0;
        doc.add_layer();
        doc.layers[3].pixels = vec![0, 200, 0, 255].into();
        doc.layers[3].clipped = true;
        assert_eq!(pixel(&doc), vec![0, 200, 0, 255]);
        doc.layers[1].pixels = vec![0, 0, 0, 0].into();
        assert_eq!(pixel(&doc), vec![128, 128, 128, 255]);

        // Glow modes are brighter than their plain forms at partial alpha.
        let over_gray = |mode| {
            let mut doc = Document::new(1, 1);
            doc.layers[0].pixels = vec![128, 128, 128, 255].into();
            doc.add_layer();
            doc.layers[1].pixels = vec![200, 200, 200, 128].into();
            doc.layers[1].blend = mode;
            pixel(&doc)[0]
        };
        let a = 128.0 / 255.0;
        near(vec![over_gray(BlendMode::AddGlow)], &[128.0 + 200.0 * a]);
        near(
            vec![over_gray(BlendMode::LinearDodge)],
            &[(1.0 - a) * 128.0 + a * 255.0],
        );
        near(
            vec![over_gray(BlendMode::GlowDodge)],
            &[128.0 * 255.0 / (255.0 - 200.0 * a)],
        );
        near(
            vec![over_gray(BlendMode::ColorDodge)],
            &[(1.0 - a) * 128.0 + a * 255.0],
        );
        // Over nothing, glow keeps the layer's own color and alpha.
        let mut alone = Document::new(1, 1);
        alone.layers[0].pixels = vec![200, 200, 200, 128].into();
        alone.layers[0].blend = BlendMode::AddGlow;
        near(pixel(&alone), &[200.0, 200.0, 200.0, 128.0]);

        // Dissolve shows a share of pixels matching alpha, each fully opaque.
        let mut doc = Document::new(64, 64);
        doc.layers[0].pixels = [10, 20, 30, 128].repeat(64 * 64).into();
        doc.layers[0].blend = BlendMode::Dissolve;
        let out = doc.composite_transparent();
        let shown = out.chunks_exact(4).filter(|p| p[3] == 255).count();
        assert!(
            out.chunks_exact(4)
                .all(|p| p[3] == 0 || p == [10, 20, 30, 255])
        );
        assert!((1800..2300).contains(&shown), "{shown}");
        assert_eq!(doc.composite_transparent(), out, "dissolve is stable");
    }
    #[test]
    fn blend_modes_handle_opaque_and_transparent_backdrops() {
        use crate::blend::BlendMode;
        let mut doc = Document::new(1, 1);
        doc.layers[0].pixels = vec![128, 128, 128, 255].into();
        doc.add_layer();
        doc.layers[1].pixels = vec![128, 128, 128, 255].into();
        doc.layers[1].blend = BlendMode::Multiply;
        assert_eq!(doc.composite(), vec![64, 64, 64, 255]);
        doc.layers[1].blend = BlendMode::Screen;
        assert_eq!(doc.composite(), vec![192, 192, 192, 255]);
        doc.layers[0].visible = false;
        doc.layers[1].pixels = vec![128, 128, 128, 128].into();
        assert_eq!(doc.composite_transparent(), vec![128, 128, 128, 128]);
        assert_eq!(
            doc.pick_color(0, 0).unwrap().as_slice(),
            &doc.composite()[..3]
        );
        let serialized = serde_json::to_vec(&doc).unwrap();
        let restored: Document = serde_json::from_slice(&serialized).unwrap();
        assert_eq!(restored.layers[1].blend, BlendMode::Screen);
    }
    #[test]
    fn every_blend_mode_preserves_transparent_source_color_and_project_metadata() {
        // Dissolve exists to make each pixel fully shown or hidden; see
        // pass_through_glow_and_dissolve_follow_their_definitions.
        let modes = crate::blend::BlendMode::ALL
            .into_iter()
            .filter(|mode| *mode != crate::blend::BlendMode::Dissolve);
        for mode in modes {
            let mut doc = Document::new(1, 1);
            doc.add_layer();
            doc.layers[1].blend = mode;
            doc.layers[1].pixels = vec![38, 132, 204, 128].into();
            assert_eq!(
                doc.composite_transparent(),
                vec![38, 132, 204, 128],
                "{}",
                mode.name()
            );
            let restored: Document =
                serde_json::from_slice(&serde_json::to_vec(&doc).unwrap()).unwrap();
            assert_eq!(restored.layers[1].blend, mode);
            doc.layers[0].pixels = vec![128, 128, 128, 128].into();
            assert_eq!(doc.composite_transparent()[3], 192);
        }
    }
    #[test]
    fn selections_constrain_paint_fill_and_clear() {
        let mut doc = Document::new(8, 8);
        doc.selection = Some(crate::selection::Selection {
            left: 2,
            top: 2,
            right: 4,
            bottom: 4,
        });
        doc.dab(3.0, 3.0, 1.0, &Brush::default(), [255, 0, 0], false);
        for y in 0..8 {
            for x in 0..8 {
                if !doc.selection.unwrap().contains(x, y) {
                    assert_eq!(doc.layers[0].pixels[(y * 8 + x) * 4 + 3], 0);
                }
            }
        }
        assert!(!doc.fill(0, 0, [0; 3], 1.0, 255).unwrap());
        assert!(doc.fill(2, 2, [0, 255, 0], 1.0, 255).unwrap());
        assert_eq!(
            doc.layers[0]
                .pixels
                .chunks_exact(4)
                .filter(|p| p[3] > 0)
                .count(),
            4
        );
        let mut history = History::new();
        history.checkpoint(&doc);
        assert!(doc.clear_selected().unwrap());
        assert!(history.undo(&mut doc));
        assert_eq!(
            doc.layers[0]
                .pixels
                .chunks_exact(4)
                .filter(|p| p[3] > 0)
                .count(),
            4
        );
        doc.selection.as_mut().unwrap().right = 100;
        assert!(doc.validate().is_err());
    }
    #[test]
    fn fill_is_contiguous_and_obeys_locks() {
        let mut doc = Document::new(3, 3);
        for y in 0..3 {
            doc.layers[0].pixels[(y * 3 + 1) * 4..(y * 3 + 1) * 4 + 4]
                .copy_from_slice(&[0, 0, 0, 255]);
        }
        assert!(doc.fill(0, 0, [255, 0, 0], 1.0, 0).unwrap());
        assert_eq!(&doc.layers[0].pixels[0..4], &[255, 0, 0, 255]);
        assert_eq!(doc.layers[0].pixels[11], 0);
        let before = doc.layers[0].pixels.clone();
        doc.layers[0].locked = true;
        assert!(doc.fill(0, 0, [0; 3], 1.0, 0).is_err());
        assert_eq!(doc.layers[0].pixels, before);
        doc.layers[0].locked = false;
        doc.layers[0].alpha_locked = true;
        doc.fill(2, 0, [0, 255, 0], 1.0, 0).unwrap();
        assert_eq!(doc.layers[0].pixels, before);
        doc.fill(0, 0, [0, 255, 0], 1.0, 0).unwrap();
        assert_eq!(&doc.layers[0].pixels[0..4], &[0, 255, 0, 255]);
    }
    #[test]
    fn tolerance_and_picker_use_expected_colors() {
        let mut doc = Document::new(3, 1);
        doc.layers[0].pixels = vec![10, 10, 10, 255, 15, 15, 15, 255, 100, 100, 100, 255].into();
        doc.fill(0, 0, [255, 0, 0], 1.0, 10).unwrap();
        assert_eq!(&doc.layers[0].pixels[4..8], &[255, 0, 0, 255]);
        assert_eq!(doc.pick_color(2, 0), Some([100; 3]));
        assert_eq!(doc.pick_color(3, 0), None);
        doc.layers[0].visible = false;
        assert_eq!(doc.pick_color(0, 0), Some([255; 3]));
    }
    #[test]
    fn fill_undo_redo_restores_the_whole_operation() {
        let mut doc = Document::new(8, 8);
        let before = doc.layers[0].pixels.clone();
        let mut history = History::new();
        history.checkpoint(&doc);
        doc.fill(0, 0, [40, 100, 200], 0.5, 0).unwrap();
        let after = doc.layers[0].pixels.clone();
        assert!(history.undo(&mut doc));
        assert_eq!(doc.layers[0].pixels, before);
        assert!(history.redo(&mut doc));
        assert_eq!(doc.layers[0].pixels, after);
        assert_eq!(
            doc.pick_color(0, 0).unwrap().as_slice(),
            &doc.composite()[0..3]
        );
    }
    #[test]
    fn locks_protect_pixels_and_alpha_lock_preserves_silhouette() {
        let mut doc = Document::new(16, 16);
        let brush = Brush {
            size: 10.0,
            pressure_size: false,
            ..Brush::default()
        };
        doc.dab(8.0, 8.0, 1.0, &brush, [255, 0, 0], false);
        let original = doc.layers[0].pixels.clone();
        doc.layers[0].locked = true;
        doc.dab(8.0, 8.0, 1.0, &brush, [0, 0, 255], false);
        assert_eq!(doc.layers[0].pixels, original);
        doc.layers[0].locked = false;
        doc.layers[0].alpha_locked = true;
        doc.dab(10.0, 8.0, 1.0, &brush, [0, 0, 255], false);
        for (before, after) in original
            .chunks_exact(4)
            .zip(doc.layers[0].pixels.chunks_exact(4))
        {
            assert_eq!(before[3], after[3]);
        }
        assert_ne!(doc.layers[0].pixels, original);
        let colored = doc.layers[0].pixels.clone();
        doc.dab(8.0, 8.0, 1.0, &brush, [0; 3], true);
        assert_eq!(doc.layers[0].pixels, colored);
    }
    #[test]
    fn transparent_composite_preserves_straight_alpha() {
        let mut doc = Document::new(1, 1);
        assert_eq!(doc.composite_transparent(), [0, 0, 0, 0]);
        doc.layers[0].pixels = vec![255, 0, 0, 128].into();
        assert_eq!(doc.composite_transparent(), [255, 0, 0, 128]);
        doc.add_layer();
        doc.layers[1].pixels = vec![0, 0, 255, 128].into();
        let out = doc.composite_transparent();
        assert_eq!(out[3], 192);
        assert!((out[0] as i32 - 85).abs() <= 1);
        assert!((out[2] as i32 - 170).abs() <= 1);
    }
    #[test]
    fn legacy_projects_default_to_unlocked_layers() {
        let doc: Document=serde_json::from_str(r#"{"version":1,"width":1,"height":1,"active":0,"layers":[{"name":"Old","visible":true,"opacity":1.0,"pixels":[0,0,0,0]}]}"#).unwrap();
        doc.validate().unwrap();
        assert!(!doc.layers[0].locked);
        assert!(!doc.layers[0].alpha_locked);
    }
    #[test]
    fn painting_erasing_and_history() {
        let mut d = Document::new(32, 32);
        let mut h = History::new();
        h.checkpoint(&d);
        let b = Brush {
            pressure_size: false,
            ..Brush::default()
        };
        d.dab(16.0, 16.0, 1.0, &b, [255, 0, 0], false);
        let i = (16 * 32 + 16) * 4;
        assert_eq!(&d.composite()[i..i + 4], &[255, 0, 0, 255]);
        assert!(h.undo(&mut d));
        assert_eq!(d.layers[0].pixels[i + 3], 0);
        assert!(h.redo(&mut d));
        d.dab(16.0, 16.0, 1.0, &b, [0; 3], true);
        assert_eq!(d.layers[0].pixels[i + 3], 0);
    }
    #[test]
    fn pressure_changes_footprint_and_hidden_layers_do_not_composite() {
        let b = Brush::default();
        let mut low = Document::new(64, 64);
        let mut high = low.clone();
        low.dab(32.0, 32.0, 0.2, &b, [0; 3], false);
        high.dab(32.0, 32.0, 1.0, &b, [0; 3], false);
        let count = |d: &Document| {
            d.layers[0]
                .pixels
                .chunks_exact(4)
                .filter(|p| p[3] > 0)
                .count()
        };
        assert!(count(&high) > count(&low));
        high.layers[0].visible = false;
        assert!(high.composite().iter().all(|v| *v == 255));
    }
    #[test]
    fn malformed_document_rejected() {
        let mut d = Document::new(8, 8);
        d.layers[0].pixels.pop();
        assert!(d.validate().is_err());
    }
    #[test]
    fn project_round_trip_preserves_layer_compositing() {
        let mut d = Document::new(1, 1);
        d.layers[0].pixels = vec![255, 0, 0, 255].into();
        d.add_layer();
        d.layers[1].pixels = vec![0, 0, 255, 255].into();
        d.layers[1].opacity = 0.5;
        let encoded = serde_json::to_vec(&d).unwrap();
        let restored: Document = serde_json::from_slice(&encoded).unwrap();
        restored.validate().unwrap();
        assert_eq!(restored.active, 1);
        assert_eq!(restored.composite(), vec![128, 0, 128, 255]);
        assert_eq!(restored.layers[0].pixels, d.layers[0].pixels);
    }
}
