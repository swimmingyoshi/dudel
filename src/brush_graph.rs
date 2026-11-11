//! Brush graphs: small node networks that compute a brush's per-dab size,
//! opacity, flow and spacing from pen input. A graph is validated and
//! compiled once into a flat list of operations; each dab runs that list over
//! a fixed register array, with no allocation. See `docs/dev/BRUSH_GRAPH_PLAN.md`.
//!
//! Values are amounts from 0 to 1. A sensor the pen does not report (speed
//! without timestamps, tilt on a mouse) is *missing*: it travels through
//! shaping nodes as missing, combine nodes leave it out, and an output that
//! receives only a missing value stays at full for that dab.
//!
//! A connected output owns its setting: the brush's pressure toggles and
//! dynamics for that setting are ignored. Unconnected outputs leave them as
//! they are. Rotation and scatter stay plain brush settings for now.
use crate::dynamics::Curve;
use serde::{Deserialize, Serialize};
use std::sync::{Arc, OnceLock};

/// The graph format this Dudel writes and reads.
pub const VERSION: u32 = 1;
/// Most nodes, counting every node inside groups.
const MAX_NODES: usize = 128;
const MAX_LINKS: usize = 256;
/// Most inputs and outputs a group has, and how deep groups nest.
pub const GROUP_INPUTS: u8 = 8;
pub const GROUP_OUTPUTS: u8 = 8;
const GROUP_DEPTH: usize = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct NodeId(pub u32);

/// What a node does. Parameters live in the variant, so each kind checks its
/// own values.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum NodeKind {
    /// Pen pressure. `shaped` passes it through the brush's own pressure
    /// response (gamma or custom curve) first.
    Pressure {
        shaped: bool,
    },
    /// Pen speed, 1 at 1000 screen pixels per second.
    Speed,
    /// How upright the pen is: 1 straight up, 0 lying flat.
    Tilt,
    /// Barrel rotation, one turn from 0 to 1.
    Twist,
    /// Distance along the stroke, reaching 1 after `length` pixels.
    Distance {
        length: f32,
    },
    /// A repeatable random value per dab. Seed 0 is the dab's own random
    /// value, the one brush dynamics use; other seeds give independent streams.
    Random {
        seed: u32,
    },
    Constant {
        value: f32,
    },
    /// Which way the stroke is heading: 0.5 rightward, 0.75 down, 0.25 up,
    /// 0 or 1 leftward.
    Direction,
    /// Which way the pen leans, on the same scale as Direction.
    TiltDirection,
    /// Repeats along the stroke every `period` pixels.
    Wave {
        period: f32,
        shape: WaveShape,
    },
    /// Smooth, repeatable wandering along the stroke; `scale` pixels per bump.
    Noise {
        scale: f32,
        seed: u32,
    },
    Curve(Curve),
    /// Maps `in_low..in_high` onto `out_low..out_high`, clamping outside.
    Remap {
        in_low: f32,
        in_high: f32,
        out_low: f32,
        out_high: f32,
    },
    /// One minus the input.
    Invert,
    /// Snaps the input to `count` evenly spaced levels from 0 to 1.
    Steps {
        count: u8,
    },
    Multiply,
    Add,
    Minimum,
    Maximum,
    /// Blends from the first input to the second by the third.
    Mix,
    SizeOutput,
    OpacityOutput,
    FlowOutput,
    SpacingOutput,
    /// Turns the tip: 0 is `degrees` one way, 1 the other, 0.5 none.
    RotationOutput {
        degrees: f32,
    },
    /// Moves the dab off the stroke, up to two brush sizes at 1.
    ScatterOutput,
    /// Turns the dab's hue: 0 is `degrees` one way, 1 the other, 0.5 none.
    HueOutput {
        degrees: f32,
    },
    /// Scales the dab color's saturation: 1 keeps it, 0 is gray.
    SaturationOutput,
    /// Scales the dab color's brightness: 1 keeps it, 0 is black.
    BrightnessOutput,
    /// Blends the main color toward the secondary color: 0 main, 1 secondary.
    ColorMixOutput,
    /// Computes a value from inputs a, b and c with a written formula.
    Formula {
        expression: String,
    },
    /// Several nodes shown and shared as one.
    Group(Box<Group>),
    /// Inside a group: the value wired into the group's input `index`.
    GroupInput {
        index: u8,
    },
    /// Inside a group: the value the group gives at output `index`.
    GroupResult {
        index: u8,
    },
}

fn one() -> u8 {
    1
}

/// Nodes packed into one, with up to eight inputs and outputs. Its own
/// nodes use their own ids; a group is opened up (inlined) when compiled.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Group {
    pub name: String,
    pub inputs: u8,
    #[serde(default = "one")]
    pub outputs: u8,
    pub nodes: Vec<Node>,
    pub links: Vec<Link>,
    /// Where the editor shows the nodes inside.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub layout: Vec<NodePosition>,
}

/// The shape a Wave node repeats.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum WaveShape {
    /// Smooth rise and fall.
    #[default]
    Sine,
    Triangle,
    /// On for the first half of each period, off for the second: dashes.
    Square,
    /// Rises steadily, then drops.
    Saw,
}
impl WaveShape {
    pub const ALL: [WaveShape; 4] = [Self::Sine, Self::Triangle, Self::Square, Self::Saw];
    pub fn name(self) -> &'static str {
        match self {
            Self::Sine => "Smooth",
            Self::Triangle => "Triangle",
            Self::Square => "Dashes",
            Self::Saw => "Ramp",
        }
    }
    /// The value at `phase` periods along, from 0 at the start.
    fn at(self, phase: f64) -> f32 {
        let f = phase.rem_euclid(1.0) as f32;
        match self {
            Self::Sine => 0.5 - 0.5 * (f * std::f32::consts::TAU).cos(),
            Self::Triangle => 1.0 - (1.0 - 2.0 * f).abs(),
            Self::Square => {
                if f < 0.5 {
                    1.0
                } else {
                    0.0
                }
            }
            Self::Saw => f,
        }
    }
}

/// Smooth 1-D value noise in 0..=1: random heights at whole numbers, eased
/// between them.
fn smooth_noise(x: f64, seed: u32) -> f32 {
    let height = |i: i64| {
        let mut h = (i as u32).wrapping_mul(0x9E37_79B1) ^ seed.wrapping_mul(0x85EB_CA77);
        h ^= h >> 16;
        h = h.wrapping_mul(0x7FEB_352D);
        h ^= h >> 15;
        h = h.wrapping_mul(0x846C_A68B);
        h ^= h >> 16;
        (h & 0xFFFF) as f32 / 65535.0
    };
    let i = x.floor();
    let t = (x - i) as f32;
    let t = t * t * (3.0 - 2.0 * t);
    let (a, b) = (height(i as i64), height(i as i64 + 1));
    a + (b - a) * t
}

impl NodeKind {
    /// How many input ports the node has.
    pub fn inputs(&self) -> u8 {
        match self {
            Self::Pressure { .. }
            | Self::Speed
            | Self::Tilt
            | Self::Twist
            | Self::Distance { .. }
            | Self::Random { .. }
            | Self::Constant { .. }
            | Self::Direction
            | Self::TiltDirection
            | Self::Wave { .. }
            | Self::Noise { .. } => 0,
            Self::Curve(_)
            | Self::Remap { .. }
            | Self::Invert
            | Self::Steps { .. }
            | Self::RotationOutput { .. }
            | Self::ScatterOutput
            | Self::SizeOutput
            | Self::OpacityOutput
            | Self::FlowOutput
            | Self::SpacingOutput
            | Self::HueOutput { .. }
            | Self::SaturationOutput
            | Self::BrightnessOutput
            | Self::ColorMixOutput => 1,
            Self::Multiply | Self::Add | Self::Minimum | Self::Maximum => 2,
            Self::Mix | Self::Formula { .. } => 3,
            Self::Group(group) => group.inputs,
            Self::GroupInput { .. } => 0,
            Self::GroupResult { .. } => 1,
        }
    }
    /// The node's title: a group's own name, else the kind's name.
    pub fn title(&self) -> String {
        match self {
            Self::Group(group) => group.name.clone(),
            Self::GroupInput { index } => format!("Group input {}", index + 1),
            Self::GroupResult { index } => format!("Group result {}", index + 1),
            other => other.name().to_owned(),
        }
    }
    /// How many outputs other nodes can be wired from: none for brush
    /// outputs and group results, a group's own count, else one.
    pub fn outputs(&self) -> u8 {
        match self {
            Self::Group(group) => group.outputs,
            Self::GroupResult { .. } => 0,
            kind if kind.output().is_some() => 0,
            _ => 1,
        }
    }
    /// Names of the outputs when there are several.
    pub fn output_names(&self) -> &'static [&'static str] {
        const OUT: [&str; 8] = [
            "Out 1", "Out 2", "Out 3", "Out 4", "Out 5", "Out 6", "Out 7", "Out 8",
        ];
        match self {
            Self::Group(group) if group.outputs > 1 => {
                &OUT[..usize::from(group.outputs.min(GROUP_OUTPUTS))]
            }
            _ => &[],
        }
    }
    /// Whether other nodes can be wired from this one.
    pub fn has_output_port(&self) -> bool {
        self.outputs() > 0
    }
    /// Which brush output this node drives, if it is an output node.
    pub fn output(&self) -> Option<Output> {
        match self {
            Self::SizeOutput => Some(Output::Size),
            Self::OpacityOutput => Some(Output::Opacity),
            Self::FlowOutput => Some(Output::Flow),
            Self::SpacingOutput => Some(Output::Spacing),
            Self::RotationOutput { .. } => Some(Output::Rotation),
            Self::ScatterOutput => Some(Output::Scatter),
            Self::HueOutput { .. } => Some(Output::Hue),
            Self::SaturationOutput => Some(Output::Saturation),
            Self::BrightnessOutput => Some(Output::Brightness),
            Self::ColorMixOutput => Some(Output::ColorMix),
            _ => None,
        }
    }
    /// Every kind, grouped as the editor lists them, with default settings.
    pub fn catalog() -> [(&'static str, Vec<NodeKind>); 4] {
        [
            (
                "Inputs",
                vec![
                    Self::Pressure { shaped: true },
                    Self::Speed,
                    Self::Tilt,
                    Self::Twist,
                    Self::Distance { length: 200.0 },
                    Self::Random { seed: 1 },
                    Self::Constant { value: 0.5 },
                    Self::Direction,
                    Self::TiltDirection,
                    Self::Wave {
                        period: 40.0,
                        shape: WaveShape::Sine,
                    },
                    Self::Noise {
                        scale: 80.0,
                        seed: 1,
                    },
                ],
            ),
            (
                "Shaping",
                vec![
                    Self::Curve(Curve::default()),
                    Self::Remap {
                        in_low: 0.0,
                        in_high: 1.0,
                        out_low: 0.0,
                        out_high: 1.0,
                    },
                    Self::Invert,
                    Self::Steps { count: 3 },
                ],
            ),
            (
                "Combine",
                vec![
                    Self::Multiply,
                    Self::Add,
                    Self::Minimum,
                    Self::Maximum,
                    Self::Mix,
                    Self::Formula {
                        expression: "a * b".into(),
                    },
                ],
            ),
            (
                "Outputs",
                vec![
                    Self::SizeOutput,
                    Self::OpacityOutput,
                    Self::FlowOutput,
                    Self::SpacingOutput,
                    Self::RotationOutput { degrees: 180.0 },
                    Self::ScatterOutput,
                    Self::HueOutput { degrees: 30.0 },
                    Self::SaturationOutput,
                    Self::BrightnessOutput,
                    Self::ColorMixOutput,
                ],
            ),
        ]
    }
    /// Names of the input ports.
    pub fn port_names(&self) -> &'static [&'static str] {
        const GROUP_PORTS: [&str; 8] = [
            "In 1", "In 2", "In 3", "In 4", "In 5", "In 6", "In 7", "In 8",
        ];
        match self {
            Self::Formula { .. } => return &["a", "b", "c"],
            Self::GroupResult { .. } => return &["Value"],
            Self::Group(group) => {
                return &GROUP_PORTS[..usize::from(group.inputs.min(GROUP_INPUTS))];
            }
            _ => {}
        }
        match self.inputs() {
            0 => &[],
            1 if self.output().is_some() => &["Value"],
            1 => &["In"],
            2 => &["A", "B"],
            _ => &["From", "To", "Amount"],
        }
    }
    /// One line on what the node does.
    pub fn description(&self) -> &'static str {
        match self {
            Self::Pressure { .. } => "Pen pressure, 0 to 1",
            Self::Speed => "Pen speed: 1 at 1000 screen pixels per second",
            Self::Tilt => "How upright the pen is: 1 straight up, 0 flat",
            Self::Twist => "Barrel rotation: one turn from 0 to 1",
            Self::Distance { .. } => "Distance along the stroke, 1 after the set length",
            Self::Random { .. } => "A repeatable random value per dab",
            Self::Constant { .. } => "A fixed value",
            Self::Direction => "Which way the stroke heads: 0.5 right, 0.75 down, 0.25 up",
            Self::TiltDirection => "Which way the pen leans: 0.5 right, 0.75 down, 0.25 up",
            Self::Wave { .. } => "Repeats along the stroke: dashes, dots, rhythms",
            Self::Noise { .. } => "Smooth, repeatable wandering along the stroke",
            Self::Steps { .. } => "Snaps its input to a few levels",
            Self::Formula { .. } => "Computes a value from a, b and c with a formula",
            Self::Group(_) => "Several nodes as one; Open group to edit them",
            Self::GroupInput { .. } => "A value wired into the group from outside",
            Self::GroupResult { .. } => "A value the group gives",
            Self::Curve(_) => "Reshapes its input with a curve",
            Self::Remap { .. } => "Maps one range onto another",
            Self::Invert => "One minus its input",
            Self::Multiply => "A × B; a missing input counts as 1",
            Self::Add => "A + B; a missing input counts as 0",
            Self::Minimum => "The smaller of A and B",
            Self::Maximum => "The larger of A and B",
            Self::Mix => "Blends From toward To by Amount",
            Self::SizeOutput => "Brush size for each dab, before taper",
            Self::OpacityOutput => "Dab opacity, before taper",
            Self::FlowOutput => "Paint delivered per dab",
            Self::SpacingOutput => "Distance to the next dab",
            Self::RotationOutput { .. } => "Turns the tip: 0.5 keeps the brush angle",
            Self::ScatterOutput => "Moves dabs off the stroke: up to two sizes at 1",
            Self::HueOutput { .. } => "Turns each dab's hue: 0.5 keeps it",
            Self::SaturationOutput => "Each dab's color saturation: 1 keeps it, 0 is gray",
            Self::BrightnessOutput => "Each dab's brightness: 1 keeps it, 0 is black",
            Self::ColorMixOutput => "Blends toward the secondary color: 0 main, 1 secondary",
        }
    }
    pub fn name(&self) -> &'static str {
        match self {
            Self::Pressure { .. } => "Pressure",
            Self::Speed => "Speed",
            Self::Tilt => "Tilt",
            Self::Twist => "Twist",
            Self::Distance { .. } => "Distance",
            Self::Random { .. } => "Random",
            Self::Constant { .. } => "Constant",
            Self::Direction => "Direction",
            Self::TiltDirection => "Tilt direction",
            Self::Wave { .. } => "Wave",
            Self::Noise { .. } => "Noise",
            Self::Steps { .. } => "Steps",
            Self::Formula { .. } => "Formula",
            Self::Group(_) => "Group",
            Self::GroupInput { .. } => "Group input",
            Self::GroupResult { .. } => "Group result",
            Self::Curve(_) => "Curve",
            Self::Remap { .. } => "Remap",
            Self::Invert => "Invert",
            Self::Multiply => "Multiply",
            Self::Add => "Add",
            Self::Minimum => "Minimum",
            Self::Maximum => "Maximum",
            Self::Mix => "Mix",
            Self::SizeOutput => "Size",
            Self::OpacityOutput => "Opacity",
            Self::FlowOutput => "Flow",
            Self::SpacingOutput => "Spacing",
            Self::RotationOutput { .. } => "Rotation",
            Self::ScatterOutput => "Scatter",
            Self::HueOutput { .. } => "Hue shift",
            Self::SaturationOutput => "Saturation",
            Self::BrightnessOutput => "Brightness",
            Self::ColorMixOutput => "Color mix",
        }
    }
    fn validate(&self) -> Result<(), String> {
        let amount = |v: f32| v.is_finite() && (0.0..=1.0).contains(&v);
        let ok = match self {
            Self::Distance { length } => length.is_finite() && (1.0..=100_000.0).contains(length),
            Self::HueOutput { degrees } | Self::RotationOutput { degrees } => {
                degrees.is_finite() && (0.0..=180.0).contains(degrees)
            }
            Self::Wave { period, .. } => period.is_finite() && (1.0..=100_000.0).contains(period),
            Self::Noise { scale, .. } => scale.is_finite() && (1.0..=100_000.0).contains(scale),
            Self::Steps { count } => (2..=32).contains(count),
            Self::Formula { expression } => {
                return crate::brush_formula::Program::parse(expression)
                    .map(|_| ())
                    .map_err(|error| format!("Formula: {error}"));
            }
            Self::Group(group) => {
                let name = group.name.trim();
                !name.is_empty()
                    && name.chars().count() <= 40
                    && !name.chars().any(char::is_control)
                    && group.inputs <= GROUP_INPUTS
                    && (1..=GROUP_OUTPUTS).contains(&group.outputs)
            }
            Self::GroupInput { index } => *index < GROUP_INPUTS,
            Self::GroupResult { index } => *index < GROUP_OUTPUTS,
            Self::Constant { value } => amount(*value),
            Self::Curve(curve) => return curve.validate(),
            Self::Remap {
                in_low,
                in_high,
                out_low,
                out_high,
            } => {
                [*in_low, *in_high, *out_low, *out_high]
                    .into_iter()
                    .all(amount)
                    && in_low != in_high
            }
            _ => true,
        };
        if ok {
            Ok(())
        } else {
            Err(format!("{} node has a setting out of range", self.name()))
        }
    }
}

/// Where the editor draws a node; saved with the brush, never compiled.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct NodePosition {
    pub id: NodeId,
    pub x: f32,
    pub y: f32,
}

/// Columns by depth from the inputs, so wires run left to right.
pub fn auto_layout(graph: &BrushGraph) -> Vec<NodePosition> {
    let index_of = |id: NodeId| graph.nodes.iter().position(|n| n.id == id);
    let mut depth = vec![0usize; graph.nodes.len()];
    // Longest path from a source; bounded passes cover any acyclic graph.
    for _ in 0..graph.nodes.len() {
        let mut changed = false;
        for link in &graph.links {
            if let (Some(from), Some(to)) = (index_of(link.from), index_of(link.to))
                && depth[to] < depth[from] + 1
            {
                depth[to] = depth[from] + 1;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    // Each column stacks its nodes by their drawn height (header, a row per
    // input, a footer row) plus a gap.
    let mut tops = vec![0.0f32; graph.nodes.len() + 1];
    graph
        .nodes
        .iter()
        .zip(&depth)
        .map(|(node, &column)| {
            let y = tops[column];
            tops[column] += 22.0 + f32::from(node.kind.inputs().max(1) + 1) * 20.0 + 16.0;
            NodePosition {
                id: node.id,
                x: column as f32 * 160.0,
                y,
            }
        })
        .collect()
}

/// The brush settings a graph can drive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Output {
    Size,
    Opacity,
    Flow,
    Spacing,
    Rotation,
    Scatter,
    // Color outputs last: `Values` reads them as one range.
    Hue,
    Saturation,
    Brightness,
    ColorMix,
}
const OUTPUTS: usize = 10;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Node {
    pub id: NodeId,
    pub kind: NodeKind,
}

fn is_zero(value: &u8) -> bool {
    *value == 0
}

/// A wire from a node's output to input `port` of another node.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Link {
    pub from: NodeId,
    pub to: NodeId,
    pub port: u8,
    /// Which of the source's outputs; only groups have more than one.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub from_port: u8,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct BrushGraph {
    version: u32,
    nodes: Vec<Node>,
    links: Vec<Link>,
    /// Compiled on first use; never saved. Cloning keeps it.
    #[serde(skip)]
    compiled: OnceLock<Result<Arc<Compiled>, String>>,
}

impl PartialEq for BrushGraph {
    fn eq(&self, other: &Self) -> bool {
        self.version == other.version && self.nodes == other.nodes && self.links == other.links
    }
}

impl std::fmt::Debug for BrushGraph {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BrushGraph")
            .field("nodes", &self.nodes)
            .field("links", &self.links)
            .finish()
    }
}

impl BrushGraph {
    pub fn new(nodes: Vec<Node>, links: Vec<Link>) -> Self {
        Self {
            version: VERSION,
            nodes,
            links,
            compiled: OnceLock::new(),
        }
    }
    pub fn nodes(&self) -> &[Node] {
        &self.nodes
    }
    pub fn links(&self) -> &[Link] {
        &self.links
    }
    /// Change the graph; the compiled form is rebuilt on next use.
    pub fn edit(&mut self) -> (&mut Vec<Node>, &mut Vec<Link>) {
        self.compiled = OnceLock::new();
        (&mut self.nodes, &mut self.links)
    }
    pub fn validate(&self) -> Result<(), String> {
        self.compiled().map(|_| ())
    }
    /// Whether any color output is wired, so dabs vary in color.
    pub fn drives_color(&self) -> bool {
        [
            Output::Hue,
            Output::Saturation,
            Output::Brightness,
            Output::ColorMix,
        ]
        .into_iter()
        .any(|output| self.drives(output))
    }
    /// Whether a connected output node drives `output`.
    pub fn drives(&self, output: Output) -> bool {
        self.compiled()
            .is_ok_and(|c| c.outputs[output as usize].is_some())
    }
    pub fn compiled(&self) -> Result<&Arc<Compiled>, String> {
        self.compiled
            .get_or_init(|| compile(self).map(Arc::new))
            .as_ref()
            .map_err(Clone::clone)
    }
    /// Feed the graph's structure to a hasher, for preview caches.
    pub fn hash_into(&self, hasher: &mut impl std::hash::Hasher) {
        use std::hash::Hash;
        self.version.hash(hasher);
        self.nodes.len().hash(hasher);
        fn values(hasher: &mut impl std::hash::Hasher, values: &[f32]) {
            values.iter().for_each(|v| v.to_bits().hash(hasher));
        }
        for node in &self.nodes {
            node.id.hash(hasher);
            node.kind.name().hash(hasher);
            match &node.kind {
                NodeKind::Pressure { shaped } => shaped.hash(hasher),
                NodeKind::Distance { length } => values(hasher, &[*length]),
                NodeKind::HueOutput { degrees } | NodeKind::RotationOutput { degrees } => {
                    values(hasher, &[*degrees])
                }
                NodeKind::Wave { period, shape } => {
                    values(hasher, &[*period]);
                    (*shape as u8).hash(hasher);
                }
                NodeKind::Noise { scale, seed } => {
                    values(hasher, &[*scale]);
                    seed.hash(hasher);
                }
                NodeKind::Steps { count } => count.hash(hasher),
                NodeKind::Formula { expression } => expression.hash(hasher),
                NodeKind::GroupInput { index } | NodeKind::GroupResult { index } => {
                    index.hash(hasher)
                }
                NodeKind::Group(group) => {
                    // Positions do not change painting; the contents do.
                    group.name.hash(hasher);
                    group.inputs.hash(hasher);
                    group.outputs.hash(hasher);
                    let inner = BrushGraph::new(group.nodes.clone(), group.links.clone());
                    inner.hash_into(hasher);
                }
                NodeKind::Random { seed } => seed.hash(hasher),
                NodeKind::Constant { value } => values(hasher, &[*value]),
                NodeKind::Curve(curve) => {
                    curve.points.len().hash(hasher);
                    curve.points.iter().for_each(|p| values(hasher, p));
                }
                NodeKind::Remap {
                    in_low,
                    in_high,
                    out_low,
                    out_high,
                } => values(hasher, &[*in_low, *in_high, *out_low, *out_high]),
                _ => {}
            }
        }
        self.links.hash(hasher);
    }
    /// Approximate heap bytes, for memory accounting.
    pub fn heap_bytes(&self) -> usize {
        self.nodes.capacity() * std::mem::size_of::<Node>()
            + self.links.capacity() * std::mem::size_of::<Link>()
            + self
                .nodes
                .iter()
                .map(|n| match &n.kind {
                    NodeKind::Curve(c) => c.points.capacity() * 8,
                    _ => 0,
                })
                .sum::<usize>()
    }

    /// A graph that paints exactly like `brush`'s pressure toggles and
    /// dynamics: size, opacity, flow and spacing.
    pub fn from_brush(brush: &crate::brush::Brush) -> Self {
        let mut builder = Builder::default();
        let response = |builder: &mut Builder, floor: Option<f32>| {
            let pressure = builder.node(NodeKind::Pressure { shaped: true });
            match floor {
                Some(floor) => {
                    let floor = builder.node(NodeKind::Constant { value: floor });
                    builder.combine(NodeKind::Maximum, pressure, floor)
                }
                None => pressure,
            }
        };
        let size = match &brush.dynamics.size {
            Some(d) => Some(builder.dynamics(d)),
            None if brush.pressure_size => Some(response(&mut builder, Some(0.05))),
            None => None,
        };
        let opacity = match &brush.dynamics.opacity {
            Some(d) => Some(builder.dynamics(d)),
            None if brush.pressure_opacity => Some(response(&mut builder, None)),
            None => None,
        };
        let flow = brush.dynamics.flow.as_ref().map(|d| builder.dynamics(d));
        let spacing = brush.dynamics.spacing.as_ref().map(|d| builder.dynamics(d));
        for (source, kind) in [
            (size, NodeKind::SizeOutput),
            (opacity, NodeKind::OpacityOutput),
            (flow, NodeKind::FlowOutput),
            (spacing, NodeKind::SpacingOutput),
        ] {
            if let Some(source) = source {
                let output = builder.node(kind);
                builder.link(source, output, 0);
            }
        }
        Self::new(builder.nodes, builder.links)
    }
}

/// A ready-made graph showing what nodes can do, to start from.
pub struct Template {
    pub name: &'static str,
    pub description: &'static str,
    pub graph: BrushGraph,
    /// Shows best with a flat tip (aspect below 1).
    pub flat_tip: bool,
}

/// Starting graphs for effects the plain brush settings cannot make.
pub fn templates() -> Vec<Template> {
    use NodeKind::*;
    let remap = |low: f32, high: f32| Remap {
        in_low: 0.0,
        in_high: 1.0,
        out_low: low,
        out_high: high,
    };
    // Size from pressure, as most brushes have it.
    let pressure_size = |b: &mut Builder| {
        let pressure = b.node(Pressure { shaped: true });
        let floor = b.node(Constant { value: 0.05 });
        let size = b.combine(Maximum, pressure, floor);
        let output = b.node(SizeOutput);
        b.link(size, output, 0);
    };
    let wire = |b: &mut Builder, from: NodeKind, to: NodeKind| {
        let from = b.node(from);
        let to = b.node(to);
        b.link(from, to, 0);
        to
    };
    let chain = |b: &mut Builder, kinds: Vec<NodeKind>| {
        let mut previous: Option<NodeId> = None;
        for kind in kinds {
            let node = b.node(kind);
            if let Some(previous) = previous {
                b.link(previous, node, 0);
            }
            previous = Some(node);
        }
    };
    let make = |name, description, flat_tip, build: &dyn Fn(&mut Builder)| {
        let mut b = Builder::default();
        build(&mut b);
        Template {
            name,
            description,
            graph: BrushGraph::new(b.nodes, b.links),
            flat_tip,
        }
    };
    vec![
        make(
            "Downstroke ink",
            "Thick going down, thin going up, like a pointed pen",
            false,
            &|b| {
                let pressure = b.node(Pressure { shaped: true });
                let direction = b.node(Direction);
                // Up (0.25) thin, down (0.75) full, sideways between.
                let shape = b.node(Curve(crate::dynamics::Curve {
                    points: vec![[0.0, 0.35], [0.25, 0.3], [0.75, 1.0], [1.0, 0.35]],
                }));
                b.link(direction, shape, 0);
                let width = b.combine(Multiply, pressure, shape);
                let output = b.node(SizeOutput);
                b.link(width, output, 0);
            },
        ),
        make(
            "Twisting ribbon",
            "A flat tip that slowly twists along the stroke",
            true,
            &|b| {
                pressure_size(b);
                wire(
                    b,
                    Noise {
                        scale: 140.0,
                        seed: 2,
                    },
                    RotationOutput { degrees: 90.0 },
                );
            },
        ),
        make(
            "Dashed line",
            "Dashes along the stroke, every 28 pixels",
            false,
            &|b| {
                pressure_size(b);
                wire(
                    b,
                    Wave {
                        period: 28.0,
                        shape: WaveShape::Square,
                    },
                    OpacityOutput,
                );
            },
        ),
        make(
            "Wobbly ink",
            "The width wanders smoothly, like a hand-cut line",
            false,
            &|b| {
                let pressure = b.node(Pressure { shaped: true });
                let noise = b.node(Noise {
                    scale: 60.0,
                    seed: 1,
                });
                let range = b.node(remap(0.45, 1.0));
                b.link(noise, range, 0);
                let width = b.combine(Multiply, pressure, range);
                let output = b.node(SizeOutput);
                b.link(width, output, 0);
            },
        ),
        make(
            "Color jitter",
            "Each dab a slightly different hue and shade",
            false,
            &|b| {
                pressure_size(b);
                wire(b, Random { seed: 1 }, HueOutput { degrees: 20.0 });
                chain(
                    b,
                    vec![Random { seed: 2 }, remap(0.8, 1.0), BrightnessOutput],
                );
            },
        ),
        make(
            "Pressure color",
            "Pressing harder paints toward the secondary color",
            false,
            &|b| {
                pressure_size(b);
                wire(b, Pressure { shaped: false }, ColorMixOutput);
            },
        ),
        make(
            "Speed spray",
            "Dabs spread out as you draw faster, with uneven density",
            false,
            &|b| {
                pressure_size(b);
                wire(b, Speed, ScatterOutput);
                chain(b, vec![Random { seed: 3 }, remap(0.3, 1.0), OpacityOutput]);
            },
        ),
        make(
            "Stipple",
            "Scattered dots, each either on or off",
            false,
            &|b| {
                pressure_size(b);
                chain(
                    b,
                    vec![Random { seed: 4 }, Steps { count: 2 }, OpacityOutput],
                );
                chain(b, vec![Random { seed: 5 }, remap(0.0, 0.4), ScatterOutput]);
            },
        ),
        make(
            "Tilt shading",
            "Leaning the pen widens and softens the stroke; the tip points where it leans",
            true,
            &|b| {
                chain(b, vec![Tilt, Invert, remap(0.35, 1.0), SizeOutput]);
                chain(b, vec![Tilt, remap(0.25, 1.0), OpacityOutput]);
                wire(b, TiltDirection, RotationOutput { degrees: 180.0 });
            },
        ),
    ]
}

#[derive(Default)]
struct Builder {
    nodes: Vec<Node>,
    links: Vec<Link>,
}
impl Builder {
    fn node(&mut self, kind: NodeKind) -> NodeId {
        let id = NodeId(self.nodes.len() as u32 + 1);
        self.nodes.push(Node { id, kind });
        id
    }
    fn link(&mut self, from: NodeId, to: NodeId, port: u8) {
        self.links.push(Link {
            from,
            to,
            port,
            from_port: 0,
        });
    }
    fn combine(&mut self, kind: NodeKind, a: NodeId, b: NodeId) -> NodeId {
        let node = self.node(kind);
        self.link(a, node, 0);
        self.link(b, node, 1);
        node
    }
    /// Curves multiplied together, then the minimum applied, the way
    /// `Dynamics::evaluate` does it.
    fn dynamics(&mut self, dynamics: &crate::dynamics::Dynamics) -> NodeId {
        use crate::dynamics::{Input, MinimumMode};
        let mut product: Option<NodeId> = None;
        for effector in &dynamics.inputs {
            let input = self.node(match effector.input {
                Input::Pressure => NodeKind::Pressure { shaped: false },
                Input::Speed => NodeKind::Speed,
                Input::TiltAltitude => NodeKind::Tilt,
                Input::Random => NodeKind::Random { seed: 0 },
            });
            let curve = self.node(NodeKind::Curve(effector.curve.clone()));
            self.link(input, curve, 0);
            // An unconnected Multiply input counts as 1, as the fold starts.
            let multiply = self.node(NodeKind::Multiply);
            if let Some(product) = product {
                self.link(product, multiply, 0);
            }
            self.link(curve, multiply, 1);
            product = Some(multiply);
        }
        let product = product.unwrap_or_else(|| self.node(NodeKind::Constant { value: 1.0 }));
        match dynamics.minimum_mode {
            MinimumMode::Clamp => {
                let floor = self.node(NodeKind::Constant {
                    value: dynamics.minimum,
                });
                self.combine(NodeKind::Maximum, product, floor)
            }
            MinimumMode::Remap => {
                let remap = self.node(NodeKind::Remap {
                    in_low: 0.0,
                    in_high: 1.0,
                    out_low: dynamics.minimum,
                    out_high: 1.0,
                });
                self.link(product, remap, 0);
                remap
            }
        }
    }
}

/// One step of a compiled graph; each writes the register of its position.
#[derive(Clone, Debug)]
enum Op {
    Pressure,
    ShapedPressure,
    Speed,
    Tilt,
    Twist,
    Distance(f32),
    Random(u32),
    Constant(f32),
    Direction,
    TiltDirection,
    Wave(f32, WaveShape),
    Noise(f32, u32),
    Steps(u8, u8),
    Curve(u8, Curve),
    Remap(u8, [f32; 4]),
    Invert(u8),
    Multiply([u8; 2]),
    Add([u8; 2]),
    Minimum([u8; 2]),
    Maximum([u8; 2]),
    Mix([u8; 3]),
    /// Inputs a, b, c; an unwired one reads 0.
    Formula([Option<u8>; 3], crate::brush_formula::Program),
}

/// Register 0 is always missing; unconnected inputs read it.
const MISSING: u8 = 0;

/// A validated graph lowered to a flat list of operations.
#[derive(Clone, Debug)]
pub struct Compiled {
    ops: Vec<Op>,
    /// Register feeding each output, in `Output` order.
    outputs: [Option<u8>; OUTPUTS],
    shaped_pressure: bool,
    /// The Hue shift and Rotation outputs' ranges in degrees.
    hue_degrees: f32,
    rotation_degrees: f32,
    /// The graph reads the stroke's direction.
    heading: bool,
}

/// Per-dab values the graph reads.
#[derive(Clone, Copy, Debug)]
pub struct Inputs {
    pub pressure: f32,
    /// The brush's pressure response at `pressure`.
    pub shaped_pressure: f32,
    pub speed: Option<f32>,
    pub tilt: Option<[f32; 2]>,
    pub twist: Option<f32>,
    pub random: Option<f32>,
    pub distance: f64,
    /// Stroke direction in degrees, once the stroke has moved.
    pub heading: Option<f32>,
}

/// Graph results; `None` where the graph does not drive the setting.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Values {
    pub size: Option<f32>,
    pub opacity: Option<f32>,
    pub flow: Option<f32>,
    pub spacing: Option<f32>,
    /// Degrees to turn the tip, when Rotation is wired (missing: no turn).
    pub rotation: Option<f32>,
    /// Scatter distance from 0 to 1, when Scatter is wired (missing: none).
    pub scatter: Option<f32>,
    /// Per-dab color change, when any color output is wired.
    pub color: Option<ColorShift>,
}

/// How one dab's color differs from the stroke's main color.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ColorShift {
    /// Degrees to turn the hue.
    pub hue: f32,
    /// Saturation and brightness multipliers, 1 for none.
    pub saturation: f32,
    pub brightness: f32,
    /// Share of the secondary color, 0 for none.
    pub mix: f32,
}
impl ColorShift {
    /// The dab's color from the main and (when known) secondary colors.
    pub fn apply(&self, main: [u8; 3], secondary: Option<[u8; 3]>) -> [u8; 3] {
        let base = match secondary {
            Some(other) if self.mix > 0.0 => std::array::from_fn(|i| {
                let (a, b) = (f32::from(main[i]), f32::from(other[i]));
                (a + (b - a) * self.mix).round().clamp(0.0, 255.0) as u8
            }),
            _ => main,
        };
        if self.hue == 0.0 && self.saturation == 1.0 && self.brightness == 1.0 {
            return base;
        }
        let [h, s, v] = crate::color_wheel::rgb_to_hsv(base);
        crate::color_wheel::hsv_to_rgb([
            h + self.hue / 360.0,
            (s * self.saturation).clamp(0.0, 1.0),
            (v * self.brightness).clamp(0.0, 1.0),
        ])
    }
}

/// An angle in degrees as an amount: 0.5 for 0°, 0 and 1 for ±180°.
fn centered_turn(degrees: f32) -> f32 {
    ((degrees + 180.0).rem_euclid(360.0)) / 360.0
}

/// A deterministic stream per seed, derived from the dab's own random value.
fn random_stream(value: f32, seed: u32) -> f32 {
    let mut h = value.to_bits() ^ seed.wrapping_mul(0x9E37_79B9);
    h ^= h >> 16;
    h = h.wrapping_mul(0x7FEB_352D);
    h ^= h >> 15;
    h = h.wrapping_mul(0x846C_A68B);
    h ^= h >> 16;
    (h & 0xFFFF) as f32 / 65535.0
}

impl Compiled {
    pub fn needs_shaped_pressure(&self) -> bool {
        self.shaped_pressure
    }
    /// Whether the graph reads the stroke's direction.
    pub fn needs_heading(&self) -> bool {
        self.heading
    }
    /// Whether a wired output node drives `output`.
    pub fn drives(&self, output: Output) -> bool {
        self.outputs[output as usize].is_some()
    }
    /// Run the graph for one dab. No allocation: registers live on the stack.
    pub fn evaluate(&self, inputs: &Inputs) -> Values {
        // Most graphs are small; a short register file is cheaper to clear.
        if self.ops.len() < 16 {
            self.run::<16>(inputs)
        } else {
            self.run::<{ MAX_NODES + 1 }>(inputs)
        }
    }
    fn run<const REGISTERS: usize>(&self, inputs: &Inputs) -> Values {
        let mut r = [f32::NAN; REGISTERS];
        // Missing operands are left out of combinations.
        let or = |v: f32, neutral: f32| if v.is_nan() { neutral } else { v };
        let product = |a: f32, b: f32| or(a, 1.0) * or(b, 1.0);
        let sum = |a: f32, b: f32| or(a, 0.0) + or(b, 0.0);
        for (index, op) in self.ops.iter().enumerate() {
            let at = |i: u8| r[usize::from(i)];
            r[index + 1] = match op {
                Op::Pressure => inputs.pressure,
                Op::ShapedPressure => inputs.shaped_pressure,
                Op::Speed => inputs.speed.unwrap_or(f32::NAN),
                Op::Tilt => inputs.tilt.map_or(f32::NAN, |[x, y]| {
                    1.0 - x.abs().max(y.abs()).min(90.0) / 90.0
                }),
                Op::Twist => inputs
                    .twist
                    .map_or(f32::NAN, |t| t.rem_euclid(360.0) / 360.0),
                Op::Distance(length) => (inputs.distance / f64::from(*length)).min(1.0) as f32,
                // Centered so 0.5 is rightward: with a 180° Rotation range
                // the tip turns exactly with the stroke.
                Op::Direction => inputs.heading.map_or(f32::NAN, centered_turn),
                Op::TiltDirection => inputs
                    .tilt
                    .filter(|[x, y]| x.hypot(*y) > 1.0)
                    .map_or(f32::NAN, |[x, y]| centered_turn(y.atan2(x).to_degrees())),
                Op::Wave(period, shape) => shape.at(inputs.distance / f64::from(*period)),
                Op::Noise(scale, seed) => smooth_noise(inputs.distance / f64::from(*scale), *seed),
                Op::Steps(source, count) => {
                    let x = at(*source);
                    let n = f32::from(*count);
                    ((x.clamp(0.0, 1.0) * n).floor().min(n - 1.0) / (n - 1.0)).max(0.0)
                        + if x.is_nan() { f32::NAN } else { 0.0 }
                }
                Op::Random(0) => inputs.random.unwrap_or(f32::NAN),
                Op::Random(seed) => inputs.random.map_or(f32::NAN, |v| random_stream(v, *seed)),
                Op::Constant(value) => *value,
                Op::Curve(source, curve) => {
                    let x = at(*source);
                    if x.is_nan() { x } else { curve.evaluate(x) }
                }
                Op::Remap(source, [in_low, in_high, out_low, out_high]) => {
                    let x = at(*source);
                    let t = ((x - in_low) / (in_high - in_low)).clamp(0.0, 1.0);
                    out_low + (out_high - out_low) * t
                }
                Op::Invert(source) => 1.0 - at(*source),
                Op::Multiply([a, b]) => product(at(*a), at(*b)),
                Op::Add([a, b]) => sum(at(*a), at(*b)),
                Op::Minimum([a, b]) => at(*a).min(at(*b)),
                Op::Maximum([a, b]) => at(*a).max(at(*b)),
                Op::Formula(ports, program) => program.run(ports.map(|p| p.map_or(0.0, at))),
                Op::Mix([a, b, t]) => {
                    let (a, b, t) = (at(*a), at(*b), at(*t));
                    a + (b - a) * t.clamp(0.0, 1.0)
                }
            };
        }
        // A driven output whose value is missing for this dab stays at full.
        let value = |output: Option<u8>| {
            output.map(|i| {
                let v = r[usize::from(i)];
                if v.is_nan() { 1.0 } else { v.clamp(0.0, 1.0) }
            })
        };
        // Color outputs: a missing value leaves that part of the color alone.
        let raw = |output: Output| {
            self.outputs[output as usize]
                .map(|i| r[usize::from(i)])
                .filter(|v| !v.is_nan())
                .map(|v| v.clamp(0.0, 1.0))
        };
        let color = self.outputs[Output::Hue as usize..]
            .iter()
            .any(Option::is_some)
            .then(|| ColorShift {
                hue: raw(Output::Hue).map_or(0.0, |v| (v * 2.0 - 1.0) * self.hue_degrees),
                saturation: raw(Output::Saturation).unwrap_or(1.0),
                brightness: raw(Output::Brightness).unwrap_or(1.0),
                mix: raw(Output::ColorMix).unwrap_or(0.0),
            });
        let rotation = self.outputs[Output::Rotation as usize].map(|_| {
            raw(Output::Rotation).map_or(0.0, |v| (v * 2.0 - 1.0) * self.rotation_degrees)
        });
        let scatter =
            self.outputs[Output::Scatter as usize].map(|_| raw(Output::Scatter).unwrap_or(0.0));
        Values {
            size: value(self.outputs[0]),
            opacity: value(self.outputs[1]),
            flow: value(self.outputs[2]),
            spacing: value(self.outputs[3]),
            rotation,
            scatter,
            color,
        }
    }
}

fn compile(graph: &BrushGraph) -> Result<Compiled, String> {
    if graph.version > VERSION {
        return Err("This brush graph was made by a newer Dudel".into());
    }
    if graph.version != VERSION {
        return Err("Unsupported brush graph version".into());
    }
    if graph.nodes.len() > MAX_NODES || graph.links.len() > MAX_LINKS {
        return Err(format!(
            "Brush graphs hold at most {MAX_NODES} nodes and {MAX_LINKS} links"
        ));
    }
    check_level(&graph.nodes, &graph.links, None)?;
    let mut seen_outputs = [false; OUTPUTS];
    for node in &graph.nodes {
        if let Some(output) = node.kind.output()
            && std::mem::replace(&mut seen_outputs[output as usize], true)
        {
            return Err(format!("The graph has two {} outputs", node.kind.name()));
        }
    }
    let mut flat = Inlined::default();
    inline(&graph.nodes, &graph.links, &[], 0, &mut flat)?;
    if flat.nodes.len() > MAX_NODES {
        return Err(format!(
            "Brush graphs hold at most {MAX_NODES} nodes, counting those inside groups"
        ));
    }
    lower(&flat.nodes, &flat.links)
}

/// Check one level of a graph: the top (`group` is None), or the inside of
/// a group with this many inputs and outputs.
fn check_level(nodes: &[Node], links: &[Link], group: Option<(u8, u8)>) -> Result<(), String> {
    let index_of = |id: NodeId| nodes.iter().position(|n| n.id == id);
    let mut seen_inputs = [false; GROUP_INPUTS as usize];
    let mut seen_results = [false; GROUP_OUTPUTS as usize];
    for (i, node) in nodes.iter().enumerate() {
        if nodes[..i].iter().any(|n| n.id == node.id) {
            return Err("Two brush graph nodes share an id".into());
        }
        node.kind.validate()?;
        match (&node.kind, group) {
            (NodeKind::GroupResult { index }, Some((_, outputs))) => {
                if *index >= outputs
                    || std::mem::replace(&mut seen_results[usize::from(*index)], true)
                {
                    return Err("A group's results are numbered wrongly".into());
                }
            }
            (NodeKind::GroupInput { index }, Some((inputs, _))) => {
                if *index >= inputs
                    || std::mem::replace(&mut seen_inputs[usize::from(*index)], true)
                {
                    return Err("A group's inputs are numbered wrongly".into());
                }
            }
            (NodeKind::GroupResult { .. } | NodeKind::GroupInput { .. }, None) => {
                return Err("Group inputs and results belong inside a group".into());
            }
            (kind, Some(_)) if kind.output().is_some() => {
                return Err(format!(
                    "{} outputs belong in the main graph, not in a group",
                    kind.name()
                ));
            }
            _ => {}
        }
    }
    if let Some((_, outputs)) = group
        && seen_results[..usize::from(outputs.min(GROUP_OUTPUTS))]
            .iter()
            .any(|seen| !seen)
    {
        return Err("A group is missing one of its results".into());
    }
    let mut used = std::collections::HashSet::new();
    for link in links {
        let (Some(from), Some(to)) = (index_of(link.from), index_of(link.to)) else {
            return Err("A brush graph link points to a missing node".into());
        };
        let source = &nodes[from].kind;
        if link.from_port >= source.outputs() {
            return Err(if source.outputs() == 0 {
                format!("{} has nothing to connect from", source.title())
            } else {
                format!("{} has no output {}", source.title(), link.from_port + 1)
            });
        }
        let kind = &nodes[to].kind;
        if link.port >= kind.inputs() {
            return Err(format!("{} has no input {}", kind.title(), link.port + 1));
        }
        if !used.insert((link.to, link.port)) {
            return Err(format!("{} has two links into one input", kind.title()));
        }
    }
    Ok(())
}

/// A graph with every group opened up, under fresh ids.
#[derive(Default)]
struct Inlined {
    nodes: Vec<Node>,
    links: Vec<Link>,
    next: u32,
}

/// One level being opened up.
struct Opening<'a> {
    nodes: &'a [Node],
    links: &'a [Link],
    /// The flat nodes feeding this level's group inputs.
    inputs: &'a [Option<NodeId>],
    depth: usize,
    fresh: std::collections::HashMap<NodeId, NodeId>,
    /// What feeds each output of the groups opened so far.
    opened: std::collections::HashMap<NodeId, Vec<Option<NodeId>>>,
    /// Groups being opened, to catch groups feeding each other.
    busy: std::collections::HashSet<NodeId>,
}
impl Opening<'_> {
    /// The flat node behind output `port` of node `id` at this level.
    fn source(
        &mut self,
        id: NodeId,
        port: u8,
        out: &mut Inlined,
    ) -> Result<Option<NodeId>, String> {
        if let Some(id) = self.fresh.get(&id) {
            return Ok(Some(*id));
        }
        let nodes = self.nodes;
        let node = nodes.iter().find(|n| n.id == id).expect("checked link");
        match &node.kind {
            NodeKind::GroupInput { index } => {
                Ok(self.inputs.get(usize::from(*index)).copied().flatten())
            }
            NodeKind::Group(group) => Ok(self
                .open(id, group, out)?
                .get(usize::from(port))
                .copied()
                .flatten()),
            _ => Ok(None),
        }
    }
    fn open(
        &mut self,
        id: NodeId,
        group: &Group,
        out: &mut Inlined,
    ) -> Result<Vec<Option<NodeId>>, String> {
        if let Some(results) = self.opened.get(&id) {
            return Ok(results.clone());
        }
        if !self.busy.insert(id) {
            return Err("The brush graph has a loop".into());
        }
        let mut feeds = Vec::new();
        for port in 0..group.inputs {
            let link = self
                .links
                .iter()
                .find(|l| l.to == id && l.port == port)
                .copied();
            feeds.push(match link {
                Some(link) => self.source(link.from, link.from_port, out)?,
                None => None,
            });
        }
        check_level(
            &group.nodes,
            &group.links,
            Some((group.inputs, group.outputs)),
        )?;
        let results = inline(&group.nodes, &group.links, &feeds, self.depth + 1, out)?;
        self.opened.insert(id, results.clone());
        Ok(results)
    }
}

/// Copy one level into `out`, opening groups. `inputs` are the flat nodes
/// feeding this group's inputs. Returns what feeds each of its results.
fn inline(
    nodes: &[Node],
    links: &[Link],
    inputs: &[Option<NodeId>],
    depth: usize,
    out: &mut Inlined,
) -> Result<Vec<Option<NodeId>>, String> {
    if depth > GROUP_DEPTH {
        return Err(format!("Groups nest at most {GROUP_DEPTH} deep"));
    }
    if out.nodes.len() > MAX_NODES {
        return Err(format!(
            "Brush graphs hold at most {MAX_NODES} nodes, counting those inside groups"
        ));
    }
    // Plain nodes get fresh ids up front.
    let mut fresh = std::collections::HashMap::new();
    for node in nodes {
        if !matches!(
            node.kind,
            NodeKind::Group(_) | NodeKind::GroupInput { .. } | NodeKind::GroupResult { .. }
        ) {
            out.next += 1;
            let id = NodeId(out.next);
            fresh.insert(node.id, id);
            out.nodes.push(Node {
                id,
                kind: node.kind.clone(),
            });
        }
    }
    let outputs = nodes
        .iter()
        .filter_map(|n| match n.kind {
            NodeKind::GroupResult { index } => Some(usize::from(index) + 1),
            _ => None,
        })
        .max()
        .unwrap_or(0);
    let mut results = vec![None; outputs];
    let mut level = Opening {
        nodes,
        links,
        inputs,
        depth,
        fresh,
        opened: Default::default(),
        busy: Default::default(),
    };
    for link in links {
        let to = nodes
            .iter()
            .find(|n| n.id == link.to)
            .expect("checked link");
        match &to.kind {
            NodeKind::Group(_) | NodeKind::GroupInput { .. } => {}
            NodeKind::GroupResult { index } => {
                results[usize::from(*index)] = level.source(link.from, link.from_port, out)?;
            }
            _ => {
                if let Some(from) = level.source(link.from, link.from_port, out)? {
                    out.links.push(Link {
                        from,
                        to: level.fresh[&link.to],
                        port: link.port,
                        from_port: 0,
                    });
                }
            }
        }
    }
    // Unused groups are still checked.
    for node in nodes {
        if let NodeKind::Group(group) = &node.kind
            && !level.opened.contains_key(&node.id)
        {
            level.open(node.id, group, out)?;
        }
    }
    if out.links.len() > MAX_LINKS * 2 {
        return Err("The brush graph has too many links".into());
    }
    Ok(results)
}

/// Lower a flat, checked graph to operations.
fn lower(nodes: &[Node], links: &[Link]) -> Result<Compiled, String> {
    let index_of = |id: NodeId| nodes.iter().position(|n| n.id == id);
    // Each input port's source, by node index.
    let mut sources: Vec<[Option<usize>; 4]> = vec![[None; 4]; nodes.len()];
    for link in links {
        let (Some(from), Some(to)) = (index_of(link.from), index_of(link.to)) else {
            return Err("A brush graph link points to a missing node".into());
        };
        sources[to][usize::from(link.port)] = Some(from);
    }
    // Topological order (Kahn), keeping the saved order among ready nodes so
    // compiling is deterministic.
    let count = nodes.len();
    let mut waiting: Vec<usize> = sources
        .iter()
        .map(|s| s.iter().filter(|s| s.is_some()).count())
        .collect();
    let mut order = Vec::with_capacity(count);
    let mut placed = vec![false; count];
    while order.len() < count {
        let Some(next) = (0..count).find(|&i| !placed[i] && waiting[i] == 0) else {
            return Err("The brush graph has a loop".into());
        };
        placed[next] = true;
        order.push(next);
        for (to, ports) in sources.iter().enumerate() {
            waiting[to] -= ports.iter().filter(|s| **s == Some(next)).count();
        }
    }
    let mut register = vec![MISSING; count];
    let mut ops = Vec::new();
    let mut outputs = [None; OUTPUTS];
    let mut shaped_pressure = false;
    let mut hue_degrees = 0.0;
    let mut rotation_degrees = 0.0;
    let mut heading = false;
    for &index in &order {
        let port = |p: usize| sources[index][p].map_or(MISSING, |s| register[s]);
        let op = match &nodes[index].kind {
            NodeKind::Pressure { shaped: true } => {
                shaped_pressure = true;
                Op::ShapedPressure
            }
            NodeKind::Pressure { shaped: false } => Op::Pressure,
            NodeKind::Speed => Op::Speed,
            NodeKind::Tilt => Op::Tilt,
            NodeKind::Twist => Op::Twist,
            NodeKind::Distance { length } => Op::Distance(*length),
            NodeKind::Random { seed } => Op::Random(*seed),
            NodeKind::Constant { value } => Op::Constant(*value),
            NodeKind::Direction => {
                heading = true;
                Op::Direction
            }
            NodeKind::TiltDirection => Op::TiltDirection,
            NodeKind::Wave { period, shape } => Op::Wave(*period, *shape),
            NodeKind::Noise { scale, seed } => Op::Noise(*scale, *seed),
            NodeKind::Steps { count } => Op::Steps(port(0), *count),
            NodeKind::Curve(curve) => Op::Curve(port(0), curve.clone()),
            NodeKind::Remap {
                in_low,
                in_high,
                out_low,
                out_high,
            } => Op::Remap(port(0), [*in_low, *in_high, *out_low, *out_high]),
            NodeKind::Invert => Op::Invert(port(0)),
            NodeKind::Multiply => Op::Multiply([port(0), port(1)]),
            NodeKind::Add => Op::Add([port(0), port(1)]),
            NodeKind::Minimum => Op::Minimum([port(0), port(1)]),
            NodeKind::Maximum => Op::Maximum([port(0), port(1)]),
            NodeKind::Mix => Op::Mix([port(0), port(1), port(2)]),
            NodeKind::Formula { expression } => {
                let wired = |p: usize| sources[index][p].map(|s| register[s]);
                Op::Formula(
                    [wired(0), wired(1), wired(2)],
                    crate::brush_formula::Program::parse(expression)?,
                )
            }
            NodeKind::Group(_) | NodeKind::GroupInput { .. } | NodeKind::GroupResult { .. } => {
                unreachable!("groups are opened before lowering")
            }
            kind => {
                let output = kind.output().expect("only outputs remain");
                match kind {
                    NodeKind::HueOutput { degrees } => hue_degrees = *degrees,
                    NodeKind::RotationOutput { degrees } => rotation_degrees = *degrees,
                    _ => {}
                }
                if let Some(source) = sources[index][0] {
                    outputs[output as usize] = Some(register[source]);
                }
                continue;
            }
        };
        ops.push(op);
        register[index] = ops.len() as u8;
    }
    Ok(Compiled {
        ops,
        outputs,
        shaped_pressure,
        hue_degrees,
        rotation_degrees,
        heading,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn graph(kinds: Vec<NodeKind>, links: &[(u32, u32, u8)]) -> BrushGraph {
        BrushGraph::new(
            kinds
                .into_iter()
                .enumerate()
                .map(|(i, kind)| Node {
                    id: NodeId(i as u32 + 1),
                    kind,
                })
                .collect(),
            links
                .iter()
                .map(|&(from, to, port)| Link {
                    from: NodeId(from),
                    to: NodeId(to),
                    port,
                    from_port: 0,
                })
                .collect(),
        )
    }
    fn inputs(pressure: f32) -> Inputs {
        Inputs {
            pressure,
            shaped_pressure: pressure,
            speed: None,
            tilt: None,
            twist: None,
            random: Some(0.25),
            distance: 0.0,
            heading: None,
        }
    }

    #[test]
    fn malformed_graphs_are_refused_with_a_reason() {
        use NodeKind::*;
        let refused = |g: BrushGraph, words: &str| {
            let error = g.validate().unwrap_err();
            assert!(error.contains(words), "{error}");
        };
        refused(graph(vec![Invert, Invert], &[(1, 2, 0), (2, 1, 0)]), "loop");
        refused(graph(vec![Speed, SizeOutput], &[(1, 3, 0)]), "missing");
        refused(graph(vec![Speed, SizeOutput], &[(1, 2, 1)]), "no input 2");
        refused(
            graph(vec![Speed, Tilt, SizeOutput], &[(1, 3, 0), (2, 3, 0)]),
            "two links",
        );
        refused(graph(vec![SizeOutput, SizeOutput], &[]), "two Size");
        refused(
            graph(vec![SizeOutput, Invert], &[(1, 2, 0)]),
            "nothing to connect",
        );
        refused(graph(vec![Constant { value: 2.0 }], &[]), "out of range");
        refused(
            graph(
                vec![Remap {
                    in_low: 0.5,
                    in_high: 0.5,
                    out_low: 0.0,
                    out_high: 1.0,
                }],
                &[],
            ),
            "out of range",
        );
        refused(
            graph(
                vec![Curve(crate::dynamics::Curve {
                    points: vec![[0.0, 0.0], [0.0, 0.5], [1.0, 1.0]],
                })],
                &[],
            ),
            "curves",
        );
        let mut duplicate = graph(vec![Speed, Tilt], &[]);
        duplicate.edit().0[1].id = NodeId(1);
        refused(duplicate, "share an id");
        let mut newer = graph(vec![Speed], &[]);
        newer.version = VERSION + 1;
        refused(newer, "newer Dudel");
        let crowded = graph((0..129).map(|_| Speed).collect(), &[]);
        refused(crowded, "at most");
        // A refused graph never drives anything.
        assert!(!graph(vec![Invert, Invert], &[(1, 2, 0), (2, 1, 0)]).drives(Output::Size));
    }

    #[test]
    fn missing_sensors_drop_out_of_combinations_and_outputs() {
        use NodeKind::*;
        // Size = pressure × speed: without speed it is just pressure.
        let g = graph(
            vec![Pressure { shaped: false }, Speed, Multiply, SizeOutput],
            &[(1, 3, 0), (2, 3, 1), (3, 4, 0)],
        );
        let compiled = g.compiled().unwrap();
        assert_eq!(compiled.evaluate(&inputs(0.5)).size, Some(0.5));
        let fast = Inputs {
            speed: Some(0.5),
            ..inputs(0.5)
        };
        assert_eq!(compiled.evaluate(&fast).size, Some(0.25));
        // An output fed only by a missing sensor stays at full for the dab.
        let g = graph(vec![Tilt, Invert, OpacityOutput], &[(1, 2, 0), (2, 3, 0)]);
        assert_eq!(
            g.compiled().unwrap().evaluate(&inputs(1.0)).opacity,
            Some(1.0)
        );
        assert!(g.drives(Output::Opacity) && !g.drives(Output::Size));
    }

    #[test]
    fn nodes_compute_their_documented_values() {
        use NodeKind::*;
        let one = |kind: NodeKind, links: &[(u32, u32, u8)], extra: Vec<NodeKind>, i: Inputs| {
            let mut kinds = vec![kind];
            kinds.extend(extra);
            let output = kinds.len() as u32 + 1;
            kinds.push(FlowOutput);
            let mut links = links.to_vec();
            links.push((1, output, 0));
            graph(kinds, &links).compiled().unwrap().evaluate(&i).flow
        };
        let i = Inputs {
            twist: Some(450.0),
            distance: 50.0,
            tilt: Some([30.0, -45.0]),
            ..inputs(0.8)
        };
        assert_eq!(one(Twist, &[], vec![], i), Some(0.25));
        assert_eq!(one(Distance { length: 200.0 }, &[], vec![], i), Some(0.25));
        assert_eq!(one(Tilt, &[], vec![], i), Some(0.5));
        assert_eq!(one(Random { seed: 0 }, &[], vec![], i), Some(0.25));
        let other = one(Random { seed: 7 }, &[], vec![], i).unwrap();
        assert!(other != 0.25 && (0.0..=1.0).contains(&other));
        assert_eq!(one(Random { seed: 7 }, &[], vec![], i), Some(other));
        // Mix(0.2, 1.0, by 0.5) = 0.6.
        let mixed = one(
            Mix,
            &[(2, 1, 0), (3, 1, 1), (4, 1, 2)],
            vec![
                Constant { value: 0.2 },
                Constant { value: 1.0 },
                Constant { value: 0.5 },
            ],
            i,
        )
        .unwrap();
        assert!((mixed - 0.6).abs() < 1e-6);
        // Remap 0.25..0.75 → 1..0, so pressure 0.8 clamps to 0.
        let remapped = one(
            Remap {
                in_low: 0.25,
                in_high: 0.75,
                out_low: 1.0,
                out_high: 0.0,
            },
            &[(2, 1, 0)],
            vec![Pressure { shaped: false }],
            i,
        );
        assert_eq!(remapped, Some(0.0));
        // Add clamps at the output.
        let added = one(
            Add,
            &[(2, 1, 0), (3, 1, 1)],
            vec![Pressure { shaped: false }, Constant { value: 0.5 }],
            i,
        );
        assert_eq!(added, Some(1.0));
    }

    /// Paint one recorded stroke with pressure, tilt and timing (so speed),
    /// through the stroke buffer as live drawing does. Smudge and blur
    /// brushes work on stripes.
    fn paint(brush: &crate::brush::Brush) -> Vec<u8> {
        use crate::stroke::{Sample, Stroke};
        let mut doc = crate::document::Document::new(320, 200);
        if !brush.blend.is_paint() {
            crate::blend_brush::preview_stripes(&mut doc, [200, 60, 40], [0, 320]);
        }
        let start = doc.layers[0].pixels.region(320, [0, 0, 320, 200]).to_vec();
        let mut stroke = Stroke::default();
        let mut buffer = crate::stroke_buffer::StrokeBuffer::default();
        let settings = crate::stroke_buffer::Settings {
            target: crate::paint_target::Target::Raster,
            brush: brush.clone(),
            color: [38, 44, 59],
            secondary: [220, 60, 40],
            erase: false,
            layer: 0,
            symmetry: Default::default(),
        };
        let count = 90;
        for i in 0..count {
            let t = i as f32 / (count - 1) as f32;
            // Uneven timing so speed rises and falls along the stroke.
            let time = f64::from(t) * 0.6 + f64::from((t * 9.0).sin()) * 0.02;
            let sample = Sample {
                x: 30.0 + t * 260.0,
                y: 100.0 + (t * std::f32::consts::TAU).sin() * 60.0,
                pressure: (t * std::f32::consts::PI).sin() * 0.9 + 0.05,
                time: Some(time),
                tilt: Some([60.0 * (1.0 - t) - 10.0, 25.0 * t]),
                twist: Some(t * 300.0),
                pressure_known: true,
                ..Default::default()
            };
            let mut dabs = stroke.sample_at_zoom(sample, brush, 1.0);
            if i + 1 == count {
                dabs.extend(stroke.finish_at_zoom(sample, brush, 1.0));
            }
            for dab in dabs {
                buffer.paint(&mut doc, dab, &settings);
            }
        }
        buffer.finish_tail(&mut doc, &settings);
        let end = doc.layers[0].pixels.region(320, [0, 0, 320, 200]).to_vec();
        assert!(end != start, "{} changed the layer", brush.name);
        end
    }

    /// Distinct opaque colors the stroke left.
    fn colors(pixels: &[u8]) -> std::collections::HashSet<[u8; 3]> {
        pixels
            .chunks_exact(4)
            .filter(|p| p[3] == 255)
            .map(|p| [p[0], p[1], p[2]])
            .collect()
    }
    fn color_brush(output: NodeKind, input: NodeKind) -> crate::brush::Brush {
        crate::brush::Brush {
            name: "Color test".into(),
            size: 24.0,
            hardness: 1.0,
            spacing: 0.1,
            pressure_size: false,
            graph: Some(graph(vec![input, output], &[(1, 2, 0)])),
            ..Default::default()
        }
    }

    #[test]
    fn color_outputs_vary_each_dab_and_mix_toward_the_secondary_color() {
        use NodeKind::*;
        // Random hue jitter: many hues, all from the stroke's main color.
        let jitter = color_brush(HueOutput { degrees: 60.0 }, Random { seed: 3 });
        jitter.validate().unwrap();
        assert!(jitter.graph.as_ref().unwrap().drives_color());
        assert_eq!(
            jitter.tail_length(),
            0.0,
            "color brushes are not re-tapered"
        );
        let painted = paint(&jitter);
        assert!(colors(&painted).len() > 20, "{}", colors(&painted).len());
        // Pressure blends main (low) toward secondary (high): the stroke's
        // light middle is red-ish, its ends dark like the main color.
        let mix = color_brush(ColorMixOutput, Pressure { shaped: false });
        let painted = paint(&mix);
        let reddest = colors(&painted).into_iter().map(|c| c[0]).max().unwrap();
        assert!(reddest > 150, "{reddest}");
        let at = |x: usize, y: usize| &painted[(y * 320 + x) * 4..][..4];
        // The stroke starts near (30, 100) with low pressure.
        let start = (25..40)
            .flat_map(|x| (90..110).map(move |y| (x, y)))
            .map(|(x, y)| at(x, y))
            .find(|p| p[3] == 255)
            .unwrap();
        assert!(start[0] < 120, "{start:?}");
    }

    #[test]
    fn steady_color_graphs_match_plain_brushes_and_plain_brushes_keep_no_colors() {
        use NodeKind::*;
        let plain = crate::brush::Brush {
            size: 24.0,
            hardness: 0.6,
            opacity: 0.7,
            flow: 0.4,
            pressure_size: false,
            ..Default::default()
        };
        // Color mix fed a constant 0 keeps the main color but takes the
        // per-pixel color path; coverage is the same, color within rounding.
        let steady = crate::brush::Brush {
            graph: Some(graph(
                vec![Constant { value: 0.0 }, ColorMixOutput],
                &[(1, 2, 0)],
            )),
            ..plain.clone()
        };
        let (a, b) = (paint(&plain), paint(&steady));
        for (p, q) in a.chunks_exact(4).zip(b.chunks_exact(4)) {
            assert_eq!(p[3], q[3]);
            for c in 0..3 {
                assert!(p[c].abs_diff(q[c]) <= 1, "{p:?} {q:?}");
            }
        }
        // Without color outputs the buffer keeps no per-pixel colors.
        let mut doc = crate::document::Document::new(64, 64);
        let mut buffer = crate::stroke_buffer::StrokeBuffer::default();
        let settings = crate::stroke_buffer::Settings {
            target: crate::paint_target::Target::Raster,
            brush: plain,
            color: [10, 20, 30],
            secondary: [200, 0, 0],
            erase: false,
            layer: 0,
            symmetry: Default::default(),
        };
        let dab = crate::stroke::Sample {
            x: 32.0,
            y: 32.0,
            pressure: 1.0,
            ..Default::default()
        };
        buffer.paint(&mut doc, dab, &settings);
        let plain_bytes = buffer.payload_capacity_bytes();
        let mut colored = crate::stroke_buffer::StrokeBuffer::default();
        let color_settings = crate::stroke_buffer::Settings {
            brush: steady,
            ..settings
        };
        let mut other = crate::document::Document::new(64, 64);
        colored.paint(&mut other, dab, &color_settings);
        assert!(colored.payload_capacity_bytes() > plain_bytes);
        // Cancel restores the layer for color brushes too.
        colored.restore(&mut other);
        assert!(other.layers[0].pixels == crate::document::Document::new(64, 64).layers[0].pixels);
    }

    #[test]
    fn wave_noise_steps_and_direction_nodes() {
        use NodeKind::*;
        let flow = |kind: NodeKind, i: Inputs| {
            graph(vec![kind, FlowOutput], &[(1, 2, 0)])
                .compiled()
                .unwrap()
                .evaluate(&i)
                .flow
                .unwrap()
        };
        let at = |distance: f64| Inputs {
            distance,
            ..inputs(1.0)
        };
        // Dashes: on for the first half of each 20 px period.
        let dashes = Wave {
            period: 20.0,
            shape: WaveShape::Square,
        };
        assert_eq!(flow(dashes.clone(), at(5.0)), 1.0);
        assert_eq!(flow(dashes.clone(), at(15.0)), 0.0);
        assert_eq!(flow(dashes, at(25.0)), 1.0);
        let smooth = |d| {
            flow(
                Wave {
                    period: 20.0,
                    shape: WaveShape::Sine,
                },
                at(d),
            )
        };
        assert!(smooth(0.0) < 0.01 && smooth(10.0) > 0.99);
        // Noise: repeatable, within 0..1, and smooth from pixel to pixel.
        let noise = |d| {
            flow(
                Noise {
                    scale: 50.0,
                    seed: 9,
                },
                at(d),
            )
        };
        let values: Vec<f32> = (0..400).map(|d| noise(f64::from(d))).collect();
        assert!(values.iter().all(|v| (0.0..=1.0).contains(v)));
        assert!(values.windows(2).all(|w| (w[0] - w[1]).abs() < 0.05));
        assert!(
            values.iter().cloned().fold(0.0, f32::max) - values.iter().cloned().fold(1.0, f32::min)
                > 0.3
        );
        assert_eq!(noise(123.0), noise(123.0));
        // Steps: two levels make 0 or 1.
        let stepped = |x: f32| {
            graph(
                vec![Constant { value: x }, Steps { count: 2 }, FlowOutput],
                &[(1, 2, 0), (2, 3, 0)],
            )
            .compiled()
            .unwrap()
            .evaluate(&inputs(1.0))
            .flow
            .unwrap()
        };
        assert_eq!((stepped(0.3), stepped(0.7), stepped(1.0)), (0.0, 1.0, 1.0));
        // Direction: rightward is 0.5, down the screen 0.75, up 0.25.
        let heading = |h| {
            flow(
                Direction,
                Inputs {
                    heading: Some(h),
                    ..inputs(1.0)
                },
            )
        };
        assert_eq!(
            (heading(0.0), heading(90.0), heading(-90.0)),
            (0.5, 0.75, 0.25)
        );
        let lean = flow(
            TiltDirection,
            Inputs {
                tilt: Some([0.0, 30.0]),
                ..inputs(1.0)
            },
        );
        assert_eq!(lean, 0.75);
    }

    /// Dabs of a straight stroke from `from` to `to`.
    fn stroke_dabs(
        brush: &crate::brush::Brush,
        from: [f32; 2],
        to: [f32; 2],
    ) -> Vec<crate::stroke::Sample> {
        let mut stroke = crate::stroke::Stroke::default();
        let mut dabs = Vec::new();
        for i in 0..=20 {
            let t = i as f32 / 20.0;
            let sample = crate::stroke::Sample {
                x: from[0] + (to[0] - from[0]) * t,
                y: from[1] + (to[1] - from[1]) * t,
                pressure: 1.0,
                pressure_known: true,
                ..Default::default()
            };
            dabs.extend(if i == 20 {
                stroke.finish(sample, brush)
            } else {
                stroke.sample(sample, brush)
            });
        }
        dabs
    }

    #[test]
    fn rotation_follows_the_stroke_and_scatter_moves_dabs() {
        use NodeKind::*;
        let turning = crate::brush::Brush {
            aspect: 0.3,
            pressure_size: false,
            graph: Some(graph(
                vec![Direction, RotationOutput { degrees: 180.0 }],
                &[(1, 2, 0)],
            )),
            ..Default::default()
        };
        let rotations = |to: [f32; 2]| {
            stroke_dabs(&turning, [100.0, 100.0], to)
                .into_iter()
                .skip(2)
                .map(|d| d.rotation.unwrap())
                .collect::<Vec<_>>()
        };
        // Across, then down: the tip turns with the stroke.
        assert!(rotations([300.0, 100.0]).iter().all(|r| r.abs() < 0.5));
        assert!(
            rotations([100.0, 300.0])
                .iter()
                .all(|r| (r - 90.0).abs() < 0.5)
        );
        // Scatter: none at 0, off the line at 1, within two sizes.
        let scatter = |amount: f32| crate::brush::Brush {
            size: 10.0,
            pressure_size: false,
            graph: Some(graph(
                vec![Constant { value: amount }, ScatterOutput],
                &[(1, 2, 0)],
            )),
            ..Default::default()
        };
        let offsets = |brush| {
            stroke_dabs(&brush, [0.0, 100.0], [300.0, 100.0])
                .into_iter()
                .map(|d| (d.y - 100.0).abs())
                .collect::<Vec<_>>()
        };
        assert!(offsets(scatter(0.0)).iter().all(|o| *o < 1e-4));
        let spread = offsets(scatter(1.0));
        assert!(spread.iter().any(|o| *o > 5.0) && spread.iter().all(|o| *o <= 20.0 + 1e-3));
    }

    #[test]
    fn every_template_is_valid_and_paints_something_new() {
        let plain = crate::brush::Brush {
            name: "Template test".into(),
            size: 20.0,
            spacing: 0.08,
            ..Default::default()
        };
        let reference = paint(&plain);
        let templates = templates();
        assert!(templates.len() >= 8);
        for template in templates {
            let brush = crate::brush::Brush {
                aspect: if template.flat_tip { 0.35 } else { 1.0 },
                graph: Some(template.graph),
                ..plain.clone()
            };
            brush
                .validate()
                .unwrap_or_else(|e| panic!("{}: {e}", template.name));
            let painted = paint(&brush);
            assert!(
                painted != reference,
                "{} looks like the plain brush",
                template.name
            );
        }
    }

    #[test]
    fn converted_brushes_paint_identical_pixels() {
        use crate::dynamics::{Curve, Dynamics, Effector, Input, MinimumMode};
        let curve = |points: Vec<[f32; 2]>| Curve { points };
        let mut brushes = crate::workspace::default_brushes();
        // Dynamics on every setting, both minimum modes, every input, and a
        // custom pressure curve.
        let mut dynamic = crate::brush::Brush {
            name: "All dynamics".into(),
            custom_pressure: true,
            pressure_curve: [0.0, 0.1, 0.6, 0.85, 1.0],
            pressure_opacity: true,
            adjust_flow_by_spacing: true,
            opacity: 0.8,
            flow: 0.7,
            taper_in: 30.0,
            taper_out: 25.0,
            ..Default::default()
        };
        dynamic.dynamics.size = Some(Dynamics {
            minimum: 0.1,
            minimum_mode: MinimumMode::Remap,
            inputs: vec![
                Effector {
                    input: Input::Pressure,
                    curve: curve(vec![[0.0, 0.0], [0.4, 0.7], [1.0, 1.0]]),
                },
                Effector {
                    input: Input::TiltAltitude,
                    curve: curve(vec![[0.0, 0.3], [1.0, 1.0]]),
                },
            ],
        });
        dynamic.dynamics.flow = Some(Dynamics {
            minimum: 0.2,
            minimum_mode: MinimumMode::Clamp,
            inputs: vec![
                Effector {
                    input: Input::Speed,
                    curve: curve(vec![[0.0, 1.0], [1.0, 0.2]]),
                },
                Effector {
                    input: Input::Random,
                    curve: curve(vec![[0.0, 0.5], [1.0, 1.0]]),
                },
            ],
        });
        dynamic.dynamics.spacing = Some(Dynamics {
            minimum: 0.3,
            minimum_mode: MinimumMode::Remap,
            inputs: vec![Effector {
                input: Input::Speed,
                curve: Curve::default(),
            }],
        });
        brushes.push(dynamic.clone());
        // Opacity dynamics replace the pressure toggle; no inputs at all.
        dynamic.dynamics.opacity = Some(Dynamics {
            minimum: 0.4,
            minimum_mode: MinimumMode::Remap,
            inputs: vec![],
        });
        dynamic.name = "Opacity without inputs".into();
        brushes.push(dynamic);
        let mut driven = 0;
        for brush in brushes {
            let graph = BrushGraph::from_brush(&brush);
            graph.validate().unwrap();
            driven += usize::from(graph.drives(Output::Size) || graph.drives(Output::Opacity));
            let converted = crate::brush::Brush {
                graph: Some(graph),
                ..brush.clone()
            };
            converted.validate().unwrap();
            let (before, after) = (paint(&brush), paint(&converted));
            assert!(before == after, "{} changed when converted", brush.name);
        }
        assert!(driven >= 15, "conversions drive real settings: {driven}");
    }

    #[test]
    fn a_driven_output_owns_its_setting() {
        // The graph says size 0.5 whatever the pressure toggle says.
        use NodeKind::*;
        let brush = crate::brush::Brush {
            size: 40.0,
            pressure_size: true,
            graph: Some(graph(
                vec![Constant { value: 0.5 }, SizeOutput],
                &[(1, 2, 0)],
            )),
            ..Default::default()
        };
        let sample = crate::stroke::Sample {
            pressure: 0.1,
            ..Default::default()
        };
        assert_eq!(brush.size_factor(&sample), 0.5);
        // Opacity is not driven, so its plain setting still applies.
        assert_eq!(brush.factors(&sample).opacity, 1.0);
    }

    #[test]
    fn packs_with_graphs_are_version_3_and_preferences_drop_damaged_graphs() {
        use crate::brush_pack::BrushPack;
        let brush = crate::brush::Brush {
            graph: Some(BrushGraph::from_brush(&crate::brush::Brush::default())),
            ..Default::default()
        };
        let pack = BrushPack::new(vec![brush.clone()]);
        assert_eq!(pack.version, 3);
        let json = serde_json::to_string(&pack).unwrap();
        let back: BrushPack = serde_json::from_str(&json).unwrap();
        back.validate().unwrap();
        assert!(back.brushes[0] == brush);
        let mut old = BrushPack::new(vec![brush.clone()]);
        old.version = 2;
        assert!(old.validate().is_err());
        assert_eq!(BrushPack::new(vec![Default::default()]).version, 2);

        // Preferences: a good graph round-trips, a damaged one is dropped.
        let app = crate::Dudel::default();
        let mut prefs =
            crate::preferences::Preferences::decode(&app.encode_preferences().unwrap()).unwrap();
        let mut damaged = brush.clone();
        damaged.graph.as_mut().unwrap().edit().1.push(Link {
            from: NodeId(999),
            to: NodeId(1),
            port: 0,
            from_port: 0,
        });
        prefs.brushes = vec![brush.clone(), damaged];
        let json = serde_json::to_string(&prefs).unwrap();
        let loaded = crate::preferences::Preferences::decode(&json).unwrap();
        assert_eq!(loaded.graphs_dropped, 1);
        assert!(loaded.brushes[0] == brush);
        assert!(loaded.brushes[1].graph.is_none());
    }

    #[test]
    fn graphs_round_trip_and_editing_recompiles() {
        use NodeKind::*;
        let mut g = graph(vec![Pressure { shaped: true }, SizeOutput], &[(1, 2, 0)]);
        assert!(g.drives(Output::Size));
        let json = serde_json::to_string(&g).unwrap();
        assert!(!json.contains("compiled"));
        let back: BrushGraph = serde_json::from_str(&json).unwrap();
        assert_eq!(back, g);
        assert!(back.compiled().unwrap().needs_shaped_pressure());
        g.edit().1.clear();
        assert!(!g.drives(Output::Size));
    }
}

/// `DUDEL_TEMPLATE_SHEET=out.png cargo test template_sheet -- --ignored`
/// draws one stroke per template, top to bottom in menu order.
#[cfg(test)]
#[test]
#[ignore = "writes a picture for review"]
fn template_sheet() {
    let Ok(path) = std::env::var("DUDEL_TEMPLATE_SHEET") else {
        return;
    };
    let templates = templates();
    let (width, row) = (900usize, 90usize);
    let mut doc = crate::document::Document::new(width, row * templates.len());
    for (index, template) in templates.into_iter().enumerate() {
        let brush = crate::brush::Brush {
            size: 22.0,
            spacing: 0.06,
            aspect: if template.flat_tip { 0.3 } else { 1.0 },
            graph: Some(template.graph),
            ..Default::default()
        };
        let settings = crate::stroke_buffer::Settings {
            target: crate::paint_target::Target::Raster,
            brush: brush.clone(),
            color: [40, 90, 200],
            secondary: [235, 120, 40],
            erase: false,
            layer: 0,
            symmetry: Default::default(),
        };
        let mut stroke = crate::stroke::Stroke::default();
        let mut buffer = crate::stroke_buffer::StrokeBuffer::default();
        let top = (index * row) as f32;
        for i in 0..=120 {
            let t = i as f32 / 120.0;
            let sample = crate::stroke::Sample {
                x: 40.0 + t * (width as f32 - 80.0),
                y: top + row as f32 / 2.0 + (t * std::f32::consts::TAU * 1.5).sin() * 22.0,
                pressure: (t * std::f32::consts::PI).sin().max(0.1),
                time: Some(f64::from(i) * (0.004 + 0.012 * f64::from(t))),
                tilt: Some([50.0 * (1.0 - t), 40.0 * t]),
                pressure_known: true,
                ..Default::default()
            };
            let dabs = if i == 120 {
                stroke.finish(sample, &brush)
            } else {
                stroke.sample(sample, &brush)
            };
            for dab in dabs {
                buffer.paint(&mut doc, dab, &settings);
            }
        }
        buffer.finish_tail(&mut doc, &settings);
    }
    image::save_buffer(
        path,
        &doc.composite(),
        width as u32,
        doc.height as u32,
        image::ExtendedColorType::Rgba8,
    )
    .unwrap();
}
