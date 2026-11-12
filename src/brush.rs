use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Default, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Tip {
    #[default]
    Round,
    Square,
    Image,
}

#[derive(Clone, Copy, Default, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum StabilizationSpace {
    #[default]
    Document,
    Screen,
}

/// How the stabilizer follows the pen.
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum StabilizerMode {
    /// The line eases toward the pen and glides onto it when the pen rests.
    #[default]
    Smooth,
    /// The line trails the pen on a string of fixed screen length: it holds
    /// still until the string is taut, then follows with a slight delay.
    Rope,
}
impl StabilizerMode {
    pub fn is_smooth(&self) -> bool {
        *self == Self::Smooth
    }
}

#[derive(Clone, Copy, Default, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SoftnessProfile {
    #[default]
    LegacyLinear,
    Eased,
}

/// Asset bytes retain their serialized representation; cached hashes are runtime-only.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TipAlpha {
    bytes: Vec<u8>,
    #[serde(skip)]
    fingerprint: std::sync::OnceLock<u64>,
}
impl PartialEq for TipAlpha {
    fn eq(&self, other: &Self) -> bool {
        self.bytes == other.bytes
    }
}
impl PartialEq<Vec<u8>> for TipAlpha {
    fn eq(&self, other: &Vec<u8>) -> bool {
        self.bytes == *other
    }
}
impl From<Vec<u8>> for TipAlpha {
    fn from(bytes: Vec<u8>) -> Self {
        Self {
            bytes,
            fingerprint: Default::default(),
        }
    }
}
impl std::ops::Deref for TipAlpha {
    type Target = Vec<u8>;
    fn deref(&self) -> &Self::Target {
        &self.bytes
    }
}
impl std::ops::DerefMut for TipAlpha {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.fingerprint.take();
        &mut self.bytes
    }
}
impl TipAlpha {
    pub(crate) fn fingerprint(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        *self.fingerprint.get_or_init(|| {
            let mut hash = std::collections::hash_map::DefaultHasher::new();
            self.bytes.hash(&mut hash);
            hash.finish()
        })
    }
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct TipImage {
    pub width: usize,
    pub height: usize,
    pub alpha: TipAlpha,
}
impl TipImage {
    pub fn validate(&self) -> Result<(), String> {
        if self.width == 0
            || self.height == 0
            || self.width > 128
            || self.height > 128
            || self.alpha.len() != self.width * self.height
        {
            return Err("Invalid embedded tip image (maximum 128 × 128)".into());
        }
        Ok(())
    }
    pub fn coverage(&self, u: f32, v: f32) -> f32 {
        if !(-1.0..1.0).contains(&u) || !(-1.0..1.0).contains(&v) {
            return 0.0;
        }
        // Pixel-center coordinates keep source texels exact and interpolate between them.
        let x = ((u + 1.0) * 0.5 * self.width as f32 - 0.5).clamp(0.0, (self.width - 1) as f32);
        let y = ((v + 1.0) * 0.5 * self.height as f32 - 0.5).clamp(0.0, (self.height - 1) as f32);
        let (x0, y0) = (x.floor() as usize, y.floor() as usize);
        let (x1, y1) = ((x0 + 1).min(self.width - 1), (y0 + 1).min(self.height - 1));
        let sample = |x, y| self.alpha[y * self.width + x] as f32 / 255.0;
        let top = sample(x0, y0) * (1.0 - x.fract()) + sample(x1, y0) * x.fract();
        let bottom = sample(x0, y1) * (1.0 - x.fract()) + sample(x1, y1) * x.fract();
        top * (1.0 - y.fract()) + bottom * y.fract()
    }
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct Brush {
    pub name: String,
    #[serde(default)]
    pub tags: String,
    pub size: f32,
    pub opacity: f32,
    /// Per-dab delivery; opacity becomes a stroke ceiling with buffered painting.
    #[serde(default = "linear_pressure")]
    pub flow: f32,
    pub hardness: f32,
    pub spacing: f32,
    pub pressure_size: bool,
    pub pressure_opacity: bool,
    #[serde(default = "linear_pressure")]
    pub pressure_gamma: f32,
    #[serde(default)]
    pub custom_pressure: bool,
    #[serde(default = "linear_curve")]
    pub pressure_curve: [f32; 5],
    #[serde(default)]
    pub stabilization: f32,
    #[serde(default)]
    pub stabilization_space: StabilizationSpace,
    #[serde(default)]
    pub stabilization_by_speed: bool,
    #[serde(default, skip_serializing_if = "StabilizerMode::is_smooth")]
    pub stabilizer_mode: StabilizerMode,
    /// Screen distance independent of follow smoothing; None retains legacy length.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rope_distance: Option<f32>,
    #[serde(default)]
    pub softness_profile: SoftnessProfile,
    #[serde(default)]
    pub dynamics: crate::dynamics::BrushDynamics,
    #[serde(default)]
    pub adjust_flow_by_spacing: bool,
    #[serde(default)]
    pub taper_in: f32,
    #[serde(default)]
    pub taper_out: f32,
    #[serde(default)]
    pub tip: Tip,
    #[serde(default = "linear_pressure")]
    pub aspect: f32,
    #[serde(default)]
    pub angle: f32,
    #[serde(default)]
    pub image: Option<TipImage>,
    #[serde(default)]
    pub grain: Option<TipImage>,
    #[serde(default)]
    pub grain_amount: f32,
    #[serde(default = "linear_pressure")]
    pub grain_scale: f32,
    #[serde(default)]
    pub scatter: f32,
    #[serde(default)]
    pub scatter_seed: u32,
    /// Starred in the library; does not affect how the brush paints.
    #[serde(default)]
    pub favorite: bool,
    /// What turns the tip: nothing, the stroke direction, or the pen's tilt.
    #[serde(default)]
    pub angle_source: AngleSource,
    /// Random turn per dab, up to this many degrees either way.
    #[serde(default)]
    pub angle_jitter: f32,
    /// Tool group the preset belongs to; None infers one from its name and tags.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<BrushGroup>,
    /// Node graph driving size, opacity, flow and spacing per dab. Each
    /// connected output replaces that setting's pressure toggle and dynamics.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub graph: Option<crate::brush_graph::BrushGraph>,
    /// Where the graph editor shows each node. Moving nodes never changes
    /// how the brush paints.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub graph_layout: Vec<crate::brush_graph::NodePosition>,
    /// Water a Wet brush leaves with its paint: more stays wet longer and
    /// spreads further.
    #[serde(default = "half")]
    pub wetness: f32,
    /// How much paint settles into the paper's valleys instead of lying
    /// evenly. Needs a paper on the canvas.
    #[serde(default)]
    pub granulation: f32,
    /// How much the brush feels the canvas's paper: 0 ignores it, 1 fully.
    #[serde(default = "linear_pressure")]
    pub paper_response: f32,
    /// How thick the paint stands on a Thick paint layer: 0 lies flat, 1
    /// the thickest.
    #[serde(default = "half")]
    pub thickness: f32,
    /// Paint with the color, or smudge or blur what is already there.
    #[serde(default, skip_serializing_if = "Blend::is_paint")]
    pub blend: Blend,
}

/// What a dab does to the layer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Blend {
    /// Lay down the current color.
    #[default]
    Paint,
    /// Drag the paint under the tip along the stroke, like a finger.
    Smudge,
    /// Soften the paint under the tip.
    Blur,
    /// Paint with water: blends into paint that is still wet, and keeps
    /// spreading until it dries.
    Wet,
}
impl Blend {
    pub const ALL: [Blend; 4] = [Self::Paint, Self::Wet, Self::Smudge, Self::Blur];
    /// Smudge and blur move paint that is already there.
    pub fn moves_paint(&self) -> bool {
        matches!(self, Self::Smudge | Self::Blur)
    }
    pub fn is_paint(&self) -> bool {
        *self == Self::Paint
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::Paint => "Paint",
            Self::Smudge => "Smudge",
            Self::Blur => "Blur",
            Self::Wet => "Wet",
        }
    }
    pub fn hint(self) -> &'static str {
        match self {
            Self::Paint => "Paint with the current color",
            Self::Smudge => {
                "Drag the paint under the brush along the stroke · opacity sets the strength · taper out does not apply"
            }
            Self::Blur => {
                "Soften the paint under the brush · opacity sets the strength · taper out does not apply"
            }
            Self::Wet => {
                "Paint with water: blends into paint that is still wet and keeps spreading until it dries · flow sets how fast paint builds · taper out does not apply"
            }
        }
    }
}

/// Paint tool groups shown as separate tools, like CSP's pencil, pen and
/// airbrush tools. Each group remembers its own preset.
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum BrushGroup {
    Pencil,
    Pen,
    #[default]
    Brush,
    Airbrush,
}
impl BrushGroup {
    pub const ALL: [BrushGroup; 4] = [Self::Pencil, Self::Pen, Self::Brush, Self::Airbrush];
    pub fn name(self) -> &'static str {
        match self {
            Self::Pencil => "Pencil",
            Self::Pen => "Pen",
            Self::Brush => "Brush",
            Self::Airbrush => "Airbrush",
        }
    }
    pub fn index(self) -> usize {
        Self::ALL.iter().position(|g| *g == self).unwrap()
    }
    pub fn next(self) -> Self {
        Self::ALL[(self.index() + 1) % Self::ALL.len()]
    }
    pub fn icon(self) -> crate::icons::Icon {
        match self {
            Self::Pencil => crate::icons::Icon::Pencil,
            Self::Pen => crate::icons::Icon::Pen,
            Self::Brush => crate::icons::Icon::Brush,
            Self::Airbrush => crate::icons::Icon::Airbrush,
        }
    }
    pub fn hint(self) -> &'static str {
        match self {
            Self::Pencil => "Textured, pressure-shaded sketching presets",
            Self::Pen => "Crisp inking presets for lineart and lettering",
            Self::Brush => "Painting presets: gouache, washes, bristles",
            Self::Airbrush => "Soft sprays for shading and blending",
        }
    }
}

/// What the tip's angle follows, added to the brush angle.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize,
)]
pub enum AngleSource {
    #[default]
    Fixed,
    /// Turns with the stroke, so flat tips draw like a calligraphy pen.
    Direction,
    /// Turns toward where the pen leans, where the tablet reports tilt.
    Tilt,
}

fn linear_pressure() -> f32 {
    1.0
}
fn half() -> f32 {
    0.5
}
fn linear_curve() -> [f32; 5] {
    [0.0, 0.25, 0.5, 0.75, 1.0]
}

impl Default for Brush {
    fn default() -> Self {
        Self {
            name: "Round ink".into(),
            tags: String::new(),
            size: 18.0,
            opacity: 1.0,
            flow: 1.0,
            hardness: 0.85,
            spacing: 0.15,
            pressure_size: true,
            pressure_opacity: false,
            pressure_gamma: 1.0,
            custom_pressure: false,
            pressure_curve: linear_curve(),
            stabilization: 0.0,
            stabilization_space: StabilizationSpace::Document,
            stabilization_by_speed: false,
            stabilizer_mode: StabilizerMode::Smooth,
            rope_distance: None,
            softness_profile: SoftnessProfile::LegacyLinear,
            dynamics: Default::default(),
            adjust_flow_by_spacing: false,
            taper_in: 0.0,
            taper_out: 0.0,
            tip: Tip::Round,
            aspect: 1.0,
            angle: 0.0,
            image: None,
            grain: None,
            grain_amount: 0.0,
            grain_scale: 1.0,
            scatter: 0.0,
            scatter_seed: 0,
            favorite: false,
            angle_source: AngleSource::Fixed,
            angle_jitter: 0.0,
            group: None,
            graph: None,
            graph_layout: Vec::new(),
            wetness: 0.5,
            granulation: 0.0,
            paper_response: 1.0,
            thickness: 0.5,
            blend: Blend::Paint,
        }
    }
}

/// Per-dab multipliers from pressure, dynamics or the brush graph.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Factors {
    /// Includes taper.
    pub size: f32,
    /// Includes taper.
    pub opacity: f32,
    pub flow: f32,
    pub spacing: f32,
    /// The dab's color change, when the brush graph sets color.
    pub color: Option<crate::brush_graph::ColorShift>,
}

impl Brush {
    /// The brush graph's results for this dab, when the brush has a graph.
    pub fn graph_values(
        &self,
        sample: &crate::stroke::Sample,
    ) -> Option<crate::brush_graph::Values> {
        let compiled = self.graph.as_ref()?.compiled().ok()?;
        Some(compiled.evaluate(&self.graph_inputs(compiled, sample)))
    }
    /// What the graph reads for one dab.
    pub fn graph_inputs(
        &self,
        compiled: &crate::brush_graph::Compiled,
        sample: &crate::stroke::Sample,
    ) -> crate::brush_graph::Inputs {
        crate::brush_graph::Inputs {
            pressure: sample.pressure,
            shaped_pressure: if compiled.needs_shaped_pressure() {
                self.response(sample.pressure)
            } else {
                sample.pressure
            },
            speed: sample.speed,
            tilt: sample.tilt,
            twist: sample.twist,
            random: sample.random,
            distance: sample.distance,
            heading: sample.heading,
        }
    }
    /// Size, opacity, flow and spacing factors for one dab, running the
    /// brush graph once. Each matches its single-factor method exactly.
    pub fn factors(&self, sample: &crate::stroke::Sample) -> Factors {
        let graph = self.graph_values(sample).unwrap_or_default();
        let taper = sample.taper.unwrap_or(1.0);
        Factors {
            size: taper * graph.size.unwrap_or_else(|| self.plain_size(sample)),
            opacity: taper * graph.opacity.unwrap_or_else(|| self.plain_opacity(sample)),
            flow: graph.flow.unwrap_or_else(|| self.plain_flow(sample)),
            spacing: graph.spacing.unwrap_or_else(|| self.plain_spacing(sample)),
            color: graph.color,
        }
    }
    pub fn size_factor(&self, sample: &crate::stroke::Sample) -> f32 {
        let graph = self.graph_values(sample).and_then(|v| v.size);
        sample.taper.unwrap_or(1.0) * graph.unwrap_or_else(|| self.plain_size(sample))
    }
    /// Size from the pressure toggle or size dynamics, before taper.
    fn plain_size(&self, sample: &crate::stroke::Sample) -> f32 {
        self.dynamics.size.as_ref().map_or_else(
            || {
                if self.pressure_size {
                    self.response(sample.pressure).max(0.05)
                } else {
                    1.0
                }
            },
            |d| d.evaluate(sample),
        )
    }
    fn plain_opacity(&self, sample: &crate::stroke::Sample) -> f32 {
        self.dynamics.opacity.as_ref().map_or_else(
            || {
                if self.pressure_opacity {
                    self.response(sample.pressure)
                } else {
                    1.0
                }
            },
            |d| d.evaluate(sample),
        )
    }
    fn plain_flow(&self, sample: &crate::stroke::Sample) -> f32 {
        self.dynamics
            .flow
            .as_ref()
            .map_or(1.0, |d| d.evaluate(sample))
    }
    fn plain_spacing(&self, sample: &crate::stroke::Sample) -> f32 {
        self.dynamics
            .spacing
            .as_ref()
            .map_or(1.0, |d| d.evaluate(sample))
    }
    /// How far back the end of a stroke is re-tapered after the pen lifts.
    /// Smudge and blur read the layer as they go, so they cannot be re-tapered.
    pub fn tail_length(&self) -> f32 {
        // Color brushes mix each dab's color into the stroke as it goes, and
        // re-tapering would have to replay those colors too.
        if self.blend.is_paint() && !self.graph.as_ref().is_some_and(|g| g.drives_color()) {
            self.taper_out
        } else {
            0.0
        }
    }
    /// Explicit group, or one inferred for presets saved before groups existed.
    pub fn group(&self) -> BrushGroup {
        if let Some(group) = self.group {
            return group;
        }
        let text = format!("{} {}", self.name, self.tags).to_lowercase();
        let has = |words: &[&str]| words.iter().any(|w| text.contains(w));
        if has(&["airbrush", "spray"]) || (self.hardness <= 0.1 && self.size >= 48.0) {
            BrushGroup::Airbrush
        } else if has(&["pencil", "sketch", "charcoal", "graphite", "crayon"]) {
            BrushGroup::Pencil
        } else if has(&["pen", "ink", "liner", "marker", "lettering", "calligraph"]) {
            BrushGroup::Pen
        } else {
            BrushGroup::Brush
        }
    }
    pub fn matches_search(&self, query: &str) -> bool {
        let searchable = format!("{} {}", self.name, self.tags).to_lowercase();
        query
            .split_whitespace()
            .all(|word| searchable.contains(&word.to_lowercase()))
    }
    /// Grain is anchored to document pixels, independent of dab position/angle.
    pub fn grain_coverage(&self, x: usize, y: usize) -> f32 {
        if self.grain_amount == 0.0 {
            return 1.0;
        }
        let x = (x as f32 / self.grain_scale).floor() as usize;
        let y = (y as f32 / self.grain_scale).floor() as usize;
        let value = if let Some(mask) = &self.grain {
            mask.alpha[(y % mask.height) * mask.width + x % mask.width] as f32 / 255.0
        } else {
            let mut hash =
                (x as u32).wrapping_mul(0x9e3779b9) ^ (y as u32).wrapping_mul(0x85ebca6b);
            hash ^= hash >> 16;
            hash = hash.wrapping_mul(0x7feb352d);
            hash ^= hash >> 15;
            (hash & 255) as f32 / 255.0
        };
        1.0 - self.grain_amount * (1.0 - value)
    }
    pub fn outline(&self) -> Vec<[f32; 2]> {
        let (sin, cos) = self.angle.to_radians().sin_cos();
        let raw: Vec<[f32; 2]> = match self.tip {
            Tip::Square | Tip::Image => vec![[-1.0, -1.0], [1.0, -1.0], [1.0, 1.0], [-1.0, 1.0]],
            Tip::Round => (0..48)
                .map(|i| {
                    let angle = i as f32 / 48.0 * std::f32::consts::TAU;
                    [angle.cos(), angle.sin()]
                })
                .collect(),
        };
        raw.into_iter()
            .map(|[x, y]| {
                let x = x * self.size * 0.5;
                let y = y * self.size * 0.5 * self.aspect;
                [x * cos - y * sin, x * sin + y * cos]
            })
            .collect()
    }
    pub fn response(&self, pressure: f32) -> f32 {
        let pressure = pressure.clamp(0.0, 1.0);
        if self.custom_pressure {
            let position = pressure * 4.0;
            let index = (position.floor() as usize).min(3);
            let t = position - index as f32;
            self.pressure_curve[index] * (1.0 - t) + self.pressure_curve[index + 1] * t
        } else {
            pressure.powf(self.pressure_gamma)
        }
    }
    pub fn validate(&self) -> Result<(), String> {
        self.dynamics.validate()?;
        if let Some(graph) = &self.graph {
            graph.validate()?;
        }
        if self.graph_layout.len() > 64
            || self
                .graph_layout
                .iter()
                .any(|p| !p.x.is_finite() || !p.y.is_finite() || p.x.abs().max(p.y.abs()) > 1e6)
        {
            return Err("Brush graph layout is damaged".into());
        }
        if self.pressure_curve[0] != 0.0
            || self.pressure_curve[4] != 1.0
            || self
                .pressure_curve
                .iter()
                .any(|value| !value.is_finite() || !(0.0..=1.0).contains(value))
            || self.pressure_curve.windows(2).any(|pair| pair[0] > pair[1])
        {
            return Err("Pressure curve must rise from 0 to 1 without descending".into());
        }
        if let Some(image) = &self.image {
            image.validate()?;
        }
        if let Some(grain) = &self.grain {
            grain.validate()?;
        }
        if self.tip == Tip::Image && self.image.is_none() {
            return Err("Image tip is missing its embedded image".into());
        }
        if self.name.len() > 200 || self.name.trim().is_empty() {
            return Err("Invalid brush name".into());
        }
        if !self.tags.trim().is_empty() {
            let tags: Vec<_> = self.tags.split(',').map(str::trim).collect();
            if tags.len() > 8
                || tags.iter().any(|tag| {
                    tag.is_empty() || tag.len() > 40 || tag.chars().any(char::is_control)
                })
            {
                return Err("Use up to 8 comma-separated tags, each 1–40 bytes".into());
            }
            let mut unique = std::collections::HashSet::new();
            if tags.iter().any(|tag| !unique.insert(tag.to_lowercase())) {
                return Err("Brush tags must be unique".into());
            }
        }
        if self
            .rope_distance
            .is_some_and(|v| !v.is_finite() || !(0.0..=500.0).contains(&v))
        {
            return Err("Rope distance must be between 0 and 500 screen pixels".into());
        }
        for (value, min, max) in [
            (self.size, 1.0, 300.0),
            (self.opacity, 0.0, 1.0),
            (self.flow, 0.0, 1.0),
            (self.hardness, 0.0, 1.0),
            (self.spacing, 0.02, 1.0),
            (self.pressure_gamma, 0.2, 5.0),
            (self.stabilization, 0.0, 50.0),
            (self.taper_in, 0.0, 300.0),
            (self.taper_out, 0.0, 300.0),
            (self.aspect, 0.05, 1.0),
            (self.angle, -180.0, 180.0),
            (self.grain_amount, 0.0, 1.0),
            (self.grain_scale, 0.25, 16.0),
            (self.scatter, 0.0, 2.0),
            (self.angle_jitter, 0.0, 180.0),
            (self.paper_response, 0.0, 1.0),
            (self.wetness, 0.0, 1.0),
            (self.granulation, 0.0, 1.0),
            (self.thickness, 0.0, 1.0),
        ] {
            if !value.is_finite() || value < min || value > max {
                return Err("Brush setting outside supported range".into());
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn asset_hash_cache_invalidates_on_mutation_and_keeps_wire_format() {
        let mut alpha: TipAlpha = vec![10, 20, 30].into();
        let first = alpha.fingerprint();
        assert_eq!(alpha.fingerprint.get(), Some(&first));
        assert_eq!(serde_json::to_string(&alpha).unwrap(), "[10,20,30]");
        let clone = alpha.clone();
        alpha[0] = 99;
        assert!(alpha.fingerprint.get().is_none());
        assert_ne!(alpha.fingerprint(), first);
        assert_eq!(clone.fingerprint(), first);
        alpha.copy_from_slice(&[10, 20, 30]);
        assert_eq!(alpha.fingerprint(), first);
        let decoded: TipAlpha = serde_json::from_str("[10,20,30]").unwrap();
        assert_eq!(decoded, clone);
        assert_eq!(decoded.fingerprint(), first);
    }
    #[test]
    fn custom_pressure_interpolates_monotonically_and_rejects_bad_curves() {
        let mut brush = Brush {
            custom_pressure: true,
            pressure_curve: [0.0, 0.1, 0.7, 0.9, 1.0],
            ..Brush::default()
        };
        brush.validate().unwrap();
        assert!((brush.response(0.375) - 0.4).abs() < 0.00001);
        assert_eq!(brush.response(-1.0), 0.0);
        assert_eq!(brush.response(2.0), 1.0);
        let restored: Brush =
            serde_json::from_str(&serde_json::to_string(&brush).unwrap()).unwrap();
        assert_eq!(restored.response(0.5), 0.7);
        for pair in (0..100).map(|i| {
            [
                brush.response(i as f32 / 100.0),
                brush.response((i + 1) as f32 / 100.0),
            ]
        }) {
            assert!(pair[0] <= pair[1]);
        }
        brush.pressure_curve[2] = 0.0;
        assert!(brush.validate().is_err());
        brush.pressure_curve[2] = f32::NAN;
        assert!(brush.validate().is_err());
    }
    #[test]
    fn image_coverage_interpolates_texel_centers_and_bounds() {
        let image = TipImage {
            width: 2,
            height: 2,
            alpha: (vec![255, 0, 0, 0]).into(),
        };
        assert_eq!(image.coverage(-0.5, -0.5), 1.0);
        assert_eq!(image.coverage(0.0, -0.5), 0.5);
        assert_eq!(image.coverage(0.0, 0.0), 0.25);
        assert_eq!(image.coverage(0.5, 0.5), 0.0);
        assert_eq!(image.coverage(-1.0, -1.0), 1.0);
        assert_eq!(image.coverage(1.0, 0.0), 0.0);
        assert_eq!(image.coverage(f32::NAN, 0.0), 0.0);
        let single = TipImage {
            width: 1,
            height: 1,
            alpha: (vec![128]).into(),
        };
        assert_eq!(single.coverage(0.8, -0.8), 128.0 / 255.0);
    }
    #[test]
    fn tag_search_matches_all_words_and_validates_imported_tags() {
        let mut brush = Brush {
            name: "Soft graphite".into(),
            tags: "Sketch, Pencil".into(),
            ..Brush::default()
        };
        brush.validate().unwrap();
        assert!(brush.matches_search("GRAPHITE sketch"));
        assert!(brush.matches_search("pencil"));
        assert!(!brush.matches_search("pencil ink"));
        assert!(brush.matches_search("  "));
        brush.tags = "Sketch, sketch".into();
        assert!(brush.validate().is_err());
        brush.tags = "sketch,".into();
        assert!(brush.validate().is_err());
    }
    #[test]
    fn grain_tiles_scales_and_preserves_legacy_coverage() {
        let mut brush = Brush::default();
        assert_eq!(brush.grain_coverage(5, 8), 1.0);
        brush.grain_amount = 1.0;
        brush.grain = Some(TipImage {
            width: 2,
            height: 1,
            alpha: (vec![0, 255]).into(),
        });
        assert_eq!(brush.grain_coverage(0, 0), 0.0);
        assert_eq!(brush.grain_coverage(1, 0), 1.0);
        assert_eq!(brush.grain_coverage(2, 10), 0.0);
        brush.grain_scale = 2.0;
        assert_eq!(brush.grain_coverage(1, 0), 0.0);
        assert_eq!(brush.grain_coverage(2, 0), 1.0);
        let restored: Brush =
            serde_json::from_str(&serde_json::to_string(&brush).unwrap()).unwrap();
        restored.validate().unwrap();
        assert_eq!(restored.grain_coverage(2, 0), 1.0);
        brush.grain_scale = f32::NAN;
        assert!(brush.validate().is_err());
    }
    #[test]
    fn pressure_response_and_legacy_defaults() {
        let legacy = r#"{"name":"Old brush","size":18.0,"opacity":1.0,"hardness":0.85,"spacing":0.15,"pressure_size":true,"pressure_opacity":false}"#;
        let brush: Brush = serde_json::from_str(legacy).unwrap();
        brush.validate().unwrap();
        assert_eq!(brush.response(0.5), 0.5);
        assert_eq!(brush.stabilization, 0.0);
        assert_eq!(brush.taper_in, 0.0);
        assert_eq!(brush.taper_out, 0.0);
        assert_eq!(brush.tip, Tip::Round);
        assert_eq!(brush.aspect, 1.0);
        assert_eq!(brush.angle, 0.0);
        let heavy = Brush {
            pressure_gamma: 2.0,
            ..brush.clone()
        };
        assert_eq!(heavy.response(0.5), 0.25);
        let light = Brush {
            pressure_gamma: 0.5,
            ..brush
        };
        assert!(light.response(0.5) > 0.5);
        assert_eq!(light.response(0.0), 0.0);
        assert_eq!(light.response(1.0), 1.0);
    }
}
