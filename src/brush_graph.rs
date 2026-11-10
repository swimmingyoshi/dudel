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
const MAX_NODES: usize = 64;
const MAX_LINKS: usize = 128;

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
            | Self::Constant { .. } => 0,
            Self::Curve(_)
            | Self::Remap { .. }
            | Self::Invert
            | Self::SizeOutput
            | Self::OpacityOutput
            | Self::FlowOutput
            | Self::SpacingOutput => 1,
            Self::Multiply | Self::Add | Self::Minimum | Self::Maximum => 2,
            Self::Mix => 3,
        }
    }
    /// Which brush output this node drives, if it is an output node.
    pub fn output(&self) -> Option<Output> {
        match self {
            Self::SizeOutput => Some(Output::Size),
            Self::OpacityOutput => Some(Output::Opacity),
            Self::FlowOutput => Some(Output::Flow),
            Self::SpacingOutput => Some(Output::Spacing),
            _ => None,
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
        }
    }
    fn validate(&self) -> Result<(), String> {
        let amount = |v: f32| v.is_finite() && (0.0..=1.0).contains(&v);
        let ok = match self {
            Self::Distance { length } => length.is_finite() && (1.0..=100_000.0).contains(length),
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

/// The brush settings a graph can drive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Output {
    Size,
    Opacity,
    Flow,
    Spacing,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Node {
    pub id: NodeId,
    pub kind: NodeKind,
}

/// A wire from a node's output to input `port` of another node.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Link {
    pub from: NodeId,
    pub to: NodeId,
    pub port: u8,
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
    /// Change the graph; the compiled form is rebuilt on next use.
    #[allow(dead_code, reason = "the Studio's graph editor, phase 2")]
    pub fn edit(&mut self) -> (&mut Vec<Node>, &mut Vec<Link>) {
        self.compiled = OnceLock::new();
        (&mut self.nodes, &mut self.links)
    }
    pub fn validate(&self) -> Result<(), String> {
        self.compiled().map(|_| ())
    }
    /// Whether a connected output node drives `output`.
    #[allow(dead_code, reason = "the Studio greys out driven settings, phase 2")]
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
        self.links.push(Link { from, to, port });
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
    Curve(u8, Curve),
    Remap(u8, [f32; 4]),
    Invert(u8),
    Multiply([u8; 2]),
    Add([u8; 2]),
    Minimum([u8; 2]),
    Maximum([u8; 2]),
    Mix([u8; 3]),
}

/// Register 0 is always missing; unconnected inputs read it.
const MISSING: u8 = 0;

/// A validated graph lowered to a flat list of operations.
#[derive(Clone, Debug)]
pub struct Compiled {
    ops: Vec<Op>,
    /// Register feeding each output (size, opacity, flow, spacing).
    outputs: [Option<u8>; 4],
    shaped_pressure: bool,
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
}

/// Graph results; `None` where the graph does not drive the setting.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Values {
    pub size: Option<f32>,
    pub opacity: Option<f32>,
    pub flow: Option<f32>,
    pub spacing: Option<f32>,
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
        Values {
            size: value(self.outputs[0]),
            opacity: value(self.outputs[1]),
            flow: value(self.outputs[2]),
            spacing: value(self.outputs[3]),
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
    let index_of = |id: NodeId| graph.nodes.iter().position(|n| n.id == id);
    let mut seen_outputs = [false; 4];
    for (i, node) in graph.nodes.iter().enumerate() {
        if graph.nodes[..i].iter().any(|n| n.id == node.id) {
            return Err("Two brush graph nodes share an id".into());
        }
        node.kind.validate()?;
        if let Some(output) = node.kind.output()
            && std::mem::replace(&mut seen_outputs[output as usize], true)
        {
            return Err(format!("The graph has two {} outputs", node.kind.name()));
        }
    }
    // Each input port's source, by node index.
    let mut sources: Vec<[Option<usize>; 3]> = vec![[None; 3]; graph.nodes.len()];
    for link in &graph.links {
        let (Some(from), Some(to)) = (index_of(link.from), index_of(link.to)) else {
            return Err("A brush graph link points to a missing node".into());
        };
        if graph.nodes[from].kind.output().is_some() {
            return Err(format!(
                "{} is an output and has nothing to connect from",
                graph.nodes[from].kind.name()
            ));
        }
        let kind = &graph.nodes[to].kind;
        if link.port >= kind.inputs() {
            return Err(format!("{} has no input {}", kind.name(), link.port + 1));
        }
        let slot = &mut sources[to][usize::from(link.port)];
        if slot.is_some() {
            return Err(format!("{} has two links into one input", kind.name()));
        }
        *slot = Some(from);
    }
    // Topological order (Kahn), keeping the saved order among ready nodes so
    // compiling is deterministic.
    let count = graph.nodes.len();
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
    let mut outputs = [None; 4];
    let mut shaped_pressure = false;
    for &index in &order {
        let port = |p: usize| sources[index][p].map_or(MISSING, |s| register[s]);
        let op = match &graph.nodes[index].kind {
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
            kind => {
                let output = kind.output().expect("only outputs remain");
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
        let crowded = graph((0..65).map(|_| Speed).collect(), &[]);
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
