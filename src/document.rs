use crate::brush::Brush;
use serde::{Deserialize, Serialize};

/// Shared by live dabs and rebuilding a finalized prefix under a tapered tail.
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
    flow_lookup: Option<crate::brush_edge::FlowLookup>,
    sin: f32,
    cos: f32,
    map: crate::symmetry::Map,
    pub left: i32,
    pub right: i32,
    pub top: i32,
    pub bottom: i32,
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
        let radius = (brush.size * brush.size_factor(&sample) / 2.0).max(0.5);
        let opacity = brush.opacity
            * brush.flow
            * brush.opacity_factor(&sample)
            * brush
                .dynamics
                .flow
                .as_ref()
                .map_or(1.0, |d| d.evaluate(&sample));
        let interval_ratio = (brush.adjust_flow_by_spacing && brush.opacity > 0.0).then(|| {
            let diameter = brush.size * brush.size_factor(&sample) * brush.aspect;
            let reference = (diameter * 0.1).max(0.5);
            sample
                .spacing
                .unwrap_or_else(|| (diameter * brush.spacing).max(0.5))
                / reference
        });
        let flow_lookup = interval_ratio.and_then(|ratio| {
            (opacity > 0.0 && opacity <= brush.opacity * 0.95)
                .then(|| crate::brush_edge::FlowLookup::new(opacity / brush.opacity, ratio))
        });
        let angle = brush.angle + sample.rotation.unwrap_or(0.0);
        let (sin, cos) = angle.to_radians().sin_cos();
        let extent = 0.5
            + if matches!(
                brush.tip,
                crate::brush::Tip::Square | crate::brush::Tip::Image
            ) {
                radius * std::f32::consts::SQRT_2
            } else {
                radius
            };
        Self {
            position: [x, y],
            radius,
            opacity,
            interval_ratio,
            flow_lookup,
            sin,
            cos,
            map,
            left: ((x - extent).floor() as i32).max(0),
            right: ((x + extent).ceil() as i32).min(width as i32 - 1),
            top: ((y - extent).floor() as i32).max(0),
            bottom: ((y + extent).ceil() as i32).min(height as i32 - 1),
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
    pub pixels: crate::pixels::Pixels,
    /// Only injected into canvas display snapshots, never live artwork or files.
    #[serde(skip)]
    pub(crate) display_reference: Option<std::sync::Arc<crate::reference_stack::Scene>>,
}

/// Largest canvas side. GPU textures and tiles are validated at this size.
pub const MAX_SIDE: usize = 4096;
/// Largest canvas area: a full 4096 x 4096 (16.7 MP), enough for 4K UHD artwork.
/// Raster/mask payload still shares the 512 MiB document budget.
pub const MAX_PIXELS: usize = MAX_SIDE * MAX_SIDE;
pub const MAX_RASTER_LAYERS: usize = 32;
pub const MAX_ENTRIES: usize = 64;
pub const MAX_DEPTH: u8 = 6;

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

impl Default for Layer {
    fn default() -> Self {
        Self {
            id: LayerId::new(),
            name: String::new(),
            visible: true,
            opacity: 1.0,
            csp_byte_opacity: false,
            blend: crate::blend::BlendMode::Normal,
            clipped: false,
            locked: false,
            alpha_locked: false,
            folder: false,
            depth: 0,
            collapsed: false,
            mask: None,
            pixels: Default::default(),
            display_reference: None,
        }
    }
}

impl Layer {
    pub(crate) fn display_key(&self) -> usize {
        self.display_reference.as_ref().map_or(0, |r| r.key)
    }
    pub(crate) fn display_region_key(&self, rect: [usize; 4]) -> usize {
        self.display_reference
            .as_ref()
            .filter(|r| r.intersects(rect))
            .map_or(0, |_| self.display_key())
    }
    pub(crate) fn has_display_payload(&self, rect: [usize; 4]) -> bool {
        self.display_reference.as_ref().map_or_else(
            || self.pixels.region_has_payload(rect),
            |r| r.intersects(rect),
        )
    }
    pub(crate) fn mask_coverage(&self, index: usize, width: usize) -> f32 {
        self.mask.as_ref().filter(|m| m.enabled).map_or(1.0, |m| {
            f32::from(m.coverage.at(index % width, index / width)) / 255.0
        })
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
        if !clipped {
            commit(&mut self.out, self.group, self.mode);
            self.group = [color[0] * a, color[1] * a, color[2] * a, a];
            self.mode = blend;
        } else if a > 0.0 && self.group[3] > 0.0 {
            // Clipping edits group color while retaining the base alpha.
            let alpha = self.group[3];
            let base = [0, 1, 2].map(|c| self.group[c] / alpha);
            let blended = blend.pixel(base, color);
            for c in 0..3 {
                self.group[c] = ((1.0 - a) * base[c] + a * blended[c]) * alpha;
            }
        }
    }
    fn finish(mut self) -> [f32; 4] {
        commit(&mut self.out, self.group, self.mode);
        self.out
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Document {
    pub version: u32,
    pub width: usize,
    pub height: usize,
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
            canvas_frame: crate::canvas_size::Frame::identity(),
            layers: vec![Layer::raster("Layer 1".into(), width, height)],
            active: 0,
            selection: None,
            selection_mask: None,
            guides: None,
            references: Vec::new(),
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        self.validate_metadata()?;
        if self.version == 2
            && self
                .layers
                .iter()
                .map(|l| {
                    l.pixels.payload_bound()
                        + l.mask.as_ref().map_or(0, |m| m.coverage.payload_bytes())
                })
                .sum::<usize>()
                > 512 * 1024 * 1024
        {
            return Err(
                "Raster and mask coverage exceed the 512 MiB document payload budget".into(),
            );
        }
        if self
            .layers
            .iter()
            .filter_map(|l| l.mask.as_ref())
            .any(|m| m.coverage.dimensions() != [self.width, self.height])
        {
            return Err("Layer mask geometry differs from document".into());
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
    pub fn validate_metadata(&self) -> Result<(), String> {
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
            if layer.name.len() > 4096
                || (layer.folder && layer.csp_byte_opacity)
                || !layer.opacity.is_finite()
                || !(0.0..=1.0).contains(&layer.opacity)
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
        let Some(layer) = self.layers.get(index) else {
            return Some("Selected layer no longer exists");
        };
        if layer.folder {
            Some("A folder is selected — select a layer inside it to draw")
        } else if layer.locked {
            Some("Layer is locked — unlock it to draw")
        } else if !self.shown(index) {
            Some("Layer is hidden — show it (and its folder) to draw")
        } else {
            None
        }
    }

    /// Composite one pixel. Folders are isolated groups: their contents are
    /// combined first, then blended into the parent level with the folder
    /// visibility, opacity and blend mode. `levels` is caller-owned scratch.
    fn composite_pixel(&self, index: usize, levels: &mut [Level]) -> [f32; 4] {
        levels[0] = EMPTY_LEVEL;
        self.composite_layers(index, levels, 0, &self.layers);
        levels[0].finish()
    }
    /// `composite_pixel` for a pixel where `empty[i]` marks raster layers that
    /// contribute zero alpha (hidden, transparent or nothing stored). Those add
    /// a zero source without reading pixels, which matches reading a
    /// transparent pixel exactly while still closing clipping groups.
    fn composite_pixel_sparse(
        &self,
        index: usize,
        levels: &mut [Level],
        empty: &[bool],
    ) -> [f32; 4] {
        levels[0] = EMPTY_LEVEL;
        let mut top = 0;
        for (layer, &empty) in self.layers.iter().zip(empty) {
            if empty {
                let depth = layer.depth as usize;
                while top < depth {
                    top += 1;
                    levels[top] = EMPTY_LEVEL;
                }
                levels[depth].add([0.0; 3], 0.0, layer.clipped, layer.blend);
            } else {
                top = self.composite_layers(index, levels, top, std::slice::from_ref(layer));
            }
        }
        levels[0].finish()
    }
    fn composite_layers(
        &self,
        index: usize,
        levels: &mut [Level],
        mut top: usize,
        layers: &[Layer],
    ) -> usize {
        for layer in layers {
            let depth = layer.depth as usize;
            while top < depth {
                top += 1;
                levels[top] = EMPTY_LEVEL;
            }
            if layer.folder {
                let content = if top > depth {
                    top -= 1;
                    levels[top + 1].finish()
                } else {
                    [0.0; 4]
                };
                let a = if layer.visible {
                    content[3] * layer.opacity * layer.mask_coverage(index, self.width)
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
                let pixel = layer.display_reference.as_ref().map_or_else(
                    || layer.pixels.rgba(index),
                    |r| r.pixel(index % self.width, index / self.width),
                );
                let a = if layer.visible {
                    layer.pixel_alpha(pixel[3]) * layer.mask_coverage(index, self.width)
                } else {
                    0.0
                };
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
        let mut state = CompositeState {
            levels: [EMPTY_LEVEL; MAX_DEPTH as usize + 1],
            top: 0,
        };
        state.top = self.composite_layers(index, &mut state.levels, 0, &self.layers[..self.active]);
        state
    }
    pub(crate) fn prefix_states(&self, rect: [usize; 4]) -> Vec<CompositeState> {
        // Keep empty bases: they terminate previous clipping groups. Only merge
        // consecutive empty rasters at the same depth; folders always execute.
        let mut plan: Vec<(&Layer, bool)> = Vec::with_capacity(self.active);
        for layer in &self.layers[..self.active] {
            let empty = !layer.folder
                && (!layer.visible || layer.opacity == 0.0 || !layer.has_display_payload(rect));
            if empty
                && let Some((previous, previous_empty)) = plan.last_mut()
                && *previous_empty
                && previous.depth == layer.depth
            {
                if !layer.clipped {
                    *previous = layer;
                }
                continue;
            }
            plan.push((layer, empty));
        }
        let mut states = Vec::with_capacity((rect[2] - rect[0]) * (rect[3] - rect[1]));
        for y in rect[1]..rect[3] {
            for x in rect[0]..rect[2] {
                let index = y * self.width + x;
                if plan.len() == self.active {
                    states.push(self.prefix_state(index));
                    continue;
                }
                let mut state = CompositeState {
                    levels: [EMPTY_LEVEL; MAX_DEPTH as usize + 1],
                    top: 0,
                };
                for &(layer, empty) in &plan {
                    if empty {
                        let depth = layer.depth as usize;
                        while state.top < depth {
                            state.top += 1;
                            state.levels[state.top] = EMPTY_LEVEL;
                        }
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
                        );
                    }
                }
                states.push(state);
            }
        }
        states
    }
    pub(crate) fn composite_from_prefix(
        &self,
        index: usize,
        mut state: CompositeState,
        transparent: bool,
    ) -> [u8; 4] {
        self.composite_layers(
            index,
            &mut state.levels,
            state.top,
            &self.layers[self.active..],
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
        check()?;
        let [left, top, right, bottom] = rect;
        let width = right.saturating_sub(left);
        let height = bottom.saturating_sub(top);
        let mut out = vec![0; width * height * 4];
        if width == 0 || height == 0 {
            return Ok(out);
        }
        // Per 256px storage block, which raster layers add nothing there.
        // Sparse layers then cost no pixel reads outside the blocks they paint.
        const BLOCK: usize = crate::pixels::SIDE;
        let (first_column, first_row_block) = (left / BLOCK, top / BLOCK);
        let block_columns = (right - 1) / BLOCK - first_column + 1;
        let block_rows = (bottom - 1) / BLOCK - first_row_block + 1;
        let empty: Vec<Vec<bool>> = (0..block_rows * block_columns)
            .map(|block| {
                let bx = (first_column + block % block_columns) * BLOCK;
                let by = (first_row_block + block / block_columns) * BLOCK;
                let area = [
                    bx.max(left),
                    by.max(top),
                    (bx + BLOCK).min(right),
                    (by + BLOCK).min(bottom),
                ];
                self.layers
                    .iter()
                    .map(|layer| {
                        !layer.folder
                            && (!layer.visible
                                || layer.opacity == 0.0
                                || !layer.has_display_payload(area))
                    })
                    .collect()
            })
            .collect();
        let sparse = empty.iter().any(|block| block.iter().any(|skip| *skip));
        let rows = |chunk: &mut [u8], first_row: usize| {
            let mut levels = [EMPTY_LEVEL; MAX_DEPTH as usize + 1];
            for (offset, pixel) in chunk.chunks_exact_mut(4).enumerate() {
                if offset & 1023 == 0 {
                    check()?;
                }
                let (x, y) = (left + offset % width, top + first_row + offset / width);
                let index = y * self.width + x;
                let value = if sparse {
                    let block =
                        (y / BLOCK - first_row_block) * block_columns + x / BLOCK - first_column;
                    self.composite_pixel_sparse(index, &mut levels, &empty[block])
                } else {
                    self.composite_pixel(index, &mut levels)
                };
                for c in 0..3 {
                    pixel[c] = if transparent {
                        if value[3] > 0.0 {
                            (value[c] / value[3]).round() as u8
                        } else {
                            0
                        }
                    } else {
                        (value[c] + 255.0 * (1.0 - value[3])).round() as u8
                    };
                }
                pixel[3] = if transparent {
                    (value[3] * 255.0).round() as u8
                } else {
                    255
                };
            }
            Ok::<(), String>(())
        };
        // Small brush regions stay on the calling thread; full refreshes
        // (undo, layer edits, export) split rows across cores.
        let threads = if width * height >= 1 << 16 {
            std::thread::available_parallelism().map_or(1, |n| n.get().min(16))
        } else {
            1
        };
        if threads <= 1 {
            rows(&mut out, 0)?;
        } else {
            let band = height.div_ceil(threads);
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
        let value = self.composite_pixel(index, &mut [EMPTY_LEVEL; MAX_DEPTH as usize + 1]);
        output_pixel(value, transparent)
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
        let value = self.composite_pixel(
            y * self.width + x,
            &mut [EMPTY_LEVEL; MAX_DEPTH as usize + 1],
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
        let reference = match source {
            crate::fill::Source::Active => self.layers[self.active].pixels.clone(),
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
                && equivalent(&reference.rgba(index))
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
        let mask = crate::fill::region_mask(
            [self.width, self.height],
            seed / 4,
            options,
            eligible,
            cancel,
        )?;
        let edges = if options.fill_edges && options.expand >= 0 {
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
        let [x, y] = raster.position;
        let radius = raster.radius;
        let opacity = raster.opacity;
        let interval_ratio = raster.interval_ratio;
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
            && raster.right >= raster.left
            && raster.right - raster.left < 512
            && buffer.as_ref().is_some_and(|b| b.is_prefix_pass())
        {
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
                    let (dx, dy) = (px as f32 + 0.5 - x, py as f32 + 0.5 - y);
                    let (dx, dy) = (map[0] * dx + map[1] * dy, map[2] * dx + map[3] * dy);
                    let u = (dx * cos + dy * sin) / radius;
                    let v = (-dx * sin + dy * cos) / (radius * brush.aspect);
                    if brush.tip == crate::brush::Tip::Image && u.abs().max(v.abs()) >= 1.0 {
                        continue;
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
                        continue;
                    }
                    let mut a = opacity * coverage * brush.grain_coverage(px as usize, py as usize);
                    if let Some(ratio) = interval_ratio {
                        a = brush.opacity
                            * flow_lookup.as_ref().map_or_else(
                                || {
                                    crate::brush_edge::compensated_delivery(
                                        a / brush.opacity,
                                        ratio,
                                    )
                                },
                                |lookup| lookup.evaluate(a / opacity),
                            );
                    }
                    if a <= 0.0 {
                        continue;
                    }
                    let (px, py) = (px as usize, py as usize);
                    let selected = self
                        .selection_mask
                        .as_ref()
                        .map_or(1.0, |m| f32::from(m.at(px, py)) / 255.0);
                    if selected == 0.0 {
                        continue;
                    }
                    a *= selected;
                    if prefix_pass {
                        if let Some(buffer) = buffer.as_deref_mut() {
                            buffer.commit_coverage_at(
                                self.active,
                                px,
                                py,
                                a,
                                brush.opacity * selected,
                            );
                        }
                        continue;
                    }
                    if scalar_writes {
                        writer.as_mut().unwrap().edit(px, py, |pixels| {
                            if let Some(buffer) = buffer.as_deref_mut() {
                                a = buffer.accumulate_at(
                                    active,
                                    px,
                                    py,
                                    pixels,
                                    a,
                                    brush.opacity * selected,
                                );
                            }
                            paint_pixel(pixels, a, color, erase, alpha_locked);
                        });
                    } else {
                        delivery[(px - chunk_left as usize) * 2] = a;
                        delivery[(px - chunk_left as usize) * 2 + 1] = brush.opacity * selected;
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
                            if let Some(buffer) = buffer.as_deref_mut() {
                                a = buffer.accumulate_at(
                                    active,
                                    chunk_left as usize + index,
                                    py as usize,
                                    pixels,
                                    a,
                                    delivery[index * 2 + 1],
                                );
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
            || a.blend != b.blend
            || a.clipped != b.clipped
            || a.locked != b.locked
            || a.alpha_locked != b.alpha_locked
            || a.folder != b.folder
            || a.depth != b.depth
            || !crate::layer_mask::same_snapshot(&a.mask, &b.mask)
        {
            return None;
        }
        if !a.pixels.shares(&b.pixels) {
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
    Some(Patch {
        layer,
        regions,
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
        for mode in crate::blend::BlendMode::ALL {
            for layer in &mut doc.layers {
                layer.blend = mode;
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
                            doc.composite_from_prefix(index, state, false),
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
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
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
                    let value =
                        doc.composite_pixel(index, &mut [EMPTY_LEVEL; MAX_DEPTH as usize + 1]);
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
        for mode in crate::blend::BlendMode::ALL {
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
