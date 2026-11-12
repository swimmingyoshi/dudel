//! The brush graph editor in Brush Studio: a node canvas drawn with egui's
//! painter. Drag a node to move it (with the others selected), drag from an
//! output dot to an input dot to wire them, drag empty space to pan and use
//! the wheel to zoom. Ctrl- or Shift-click selects several nodes, Shift-drag
//! selects a box of them; right-click adds or removes nodes. Groups pack
//! nodes into one and open to show their inside.
//!
//! Every edit is made on a copy of the current level (the main graph or the
//! inside of an open group) and the whole brush graph is checked before it is
//! kept, so the graph a brush paints with is always valid. The editor keeps
//! its own undo steps, separate from the document's.
use crate::{
    Dudel,
    brush::Brush,
    brush_graph::{
        BrushGraph, GROUP_INPUTS, GROUP_OUTPUTS, Group, Link, Node, NodeId, NodeKind, NodePosition,
    },
    egui,
};
use std::collections::HashSet;

const NODE_WIDTH: f32 = 132.0;
const HEADER: f32 = 22.0;
const ROW: f32 = 20.0;
const PORT: f32 = 5.0;
const UNDO_LIMIT: usize = 100;

type Snapshot = (Option<BrushGraph>, Vec<NodePosition>);

enum Drag {
    /// Moving the selection: where the pointer started (graph space) and
    /// where each moving node started.
    Move {
        from: [f32; 2],
        start: Vec<(NodeId, [f32; 2])>,
    },
    Wire {
        from: NodeId,
        from_port: u8,
        /// Picked up off an input rather than drawn from an output.
        moved: bool,
    },
    Pan,
    /// A selection box from this screen point.
    Select {
        from: egui::Pos2,
    },
}

/// What a point on the canvas is over.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Hit {
    Output(NodeId, u8),
    Input(NodeId, u8),
    Node(NodeId),
    Empty,
}

pub(crate) struct GraphEditor {
    pan: egui::Vec2,
    zoom: f32,
    /// Selected nodes at the current level; the last one's settings show.
    selected: Vec<NodeId>,
    drag: Option<Drag>,
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    /// The library entry being edited; undo steps belong to it.
    owner: Option<usize>,
    /// Graph position where the context menu was opened, and what was there.
    context: Option<([f32; 2], Hit)>,
    /// Screen rectangle of the canvas, as last drawn.
    canvas: egui::Rect,
    /// The view is fitted to the nodes on the next frame.
    fit: bool,
    /// A node setting is being dragged; it is one undo step.
    editing_setting: bool,
    /// The open groups, outermost first; empty for the main graph.
    path: Vec<NodeId>,
    /// A Formula being typed, kept while it does not parse yet.
    formula: Option<(NodeId, String)>,
    /// The egui pass in which the pointer was last over the canvas; the
    /// editor's keys apply on the next pass.
    hovered_pass: Option<u64>,
    /// Pen input to show the nodes' values for, when the test pad is idle.
    probe: Probe,
    /// Draw each node's value on it.
    show_values: bool,
    /// Text typed into an Add node search.
    search: String,
    /// A wire let go over empty canvas, waiting for a node to connect to.
    dropped: Option<Dropped>,
}

/// A wire let go over empty canvas; a node search opens there.
#[derive(Clone, Copy)]
struct Dropped {
    from: NodeId,
    from_port: u8,
    /// Where the new node goes, in graph space, and where the search opens.
    at: [f32; 2],
    screen: egui::Pos2,
    /// The search field still needs the keyboard.
    focus: bool,
}

/// Pen input chosen in "Try inputs".
#[derive(Clone, Copy)]
struct Probe {
    pressure: f32,
    speed: f32,
    /// Degrees from upright.
    tilt: f32,
    /// Stroke direction, degrees.
    direction: f32,
    /// Pixels along the stroke.
    distance: f32,
    random: f32,
}
impl Default for Probe {
    fn default() -> Self {
        Self {
            pressure: 0.6,
            speed: 0.3,
            tilt: 20.0,
            direction: 0.0,
            distance: 120.0,
            random: 0.5,
        }
    }
}
impl Probe {
    fn sample(&self) -> crate::stroke::Sample {
        crate::stroke::Sample {
            pressure: self.pressure,
            pressure_known: true,
            speed: Some(self.speed),
            tilt: Some([self.tilt, 0.0]),
            random: Some(self.random),
            distance: f64::from(self.distance),
            heading: Some(self.direction),
            ..Default::default()
        }
    }
}

/// A node value for showing: two decimals, or a dash when missing.
fn shown_value(value: f32) -> String {
    if value.is_nan() {
        "–".into()
    } else {
        format!("{value:.2}")
    }
}

impl Default for GraphEditor {
    fn default() -> Self {
        Self {
            pan: egui::Vec2::ZERO,
            zoom: 1.0,
            selected: Vec::new(),
            drag: None,
            undo: Vec::new(),
            redo: Vec::new(),
            owner: None,
            context: None,
            canvas: egui::Rect::ZERO,
            fit: false,
            editing_setting: false,
            path: Vec::new(),
            formula: None,
            hovered_pass: None,
            probe: Probe::default(),
            show_values: true,
            search: String::new(),
            dropped: None,
        }
    }
}

/// One level of the graph: the main graph or the inside of a group.
struct Level<'a> {
    nodes: &'a [Node],
    links: &'a [Link],
    layout: &'a [NodePosition],
}
impl Level<'_> {
    fn position(&self, id: NodeId) -> Option<[f32; 2]> {
        self.nodes.iter().any(|n| n.id == id).then(|| {
            self.layout
                .iter()
                .find(|p| p.id == id)
                .map_or([0.0, 0.0], |p| [p.x, p.y])
        })
    }
    fn wired(&self, id: NodeId, port: u8) -> bool {
        self.links.iter().any(|l| l.to == id && l.port == port)
    }
}

fn level<'a>(brush: &'a Brush, path: &[NodeId]) -> Option<Level<'a>> {
    let graph = brush.graph.as_ref()?;
    let mut level = Level {
        nodes: graph.nodes(),
        links: graph.links(),
        layout: &brush.graph_layout,
    };
    for id in path {
        let NodeKind::Group(group) = &level.nodes.iter().find(|n| n.id == *id)?.kind else {
            return None;
        };
        level = Level {
            nodes: &group.nodes,
            links: &group.links,
            layout: &group.layout,
        };
    }
    Some(level)
}

type LevelMut<'a> = (
    &'a mut Vec<Node>,
    &'a mut Vec<Link>,
    &'a mut Vec<NodePosition>,
);

fn level_mut<'a>(brush: &'a mut Brush, path: &[NodeId]) -> Option<LevelMut<'a>> {
    let Brush {
        graph,
        graph_layout,
        ..
    } = brush;
    let (mut nodes, mut links) = graph.as_mut()?.edit();
    let mut layout = graph_layout;
    for id in path {
        let current = nodes;
        let NodeKind::Group(group) = &mut current.iter_mut().find(|n| n.id == *id)?.kind else {
            return None;
        };
        let group = &mut **group;
        nodes = &mut group.nodes;
        links = &mut group.links;
        layout = &mut group.layout;
    }
    Some((nodes, links, layout))
}

impl GraphEditor {
    fn zoom(&self) -> f32 {
        if self.zoom > 0.0 { self.zoom } else { 1.0 }
    }
    fn to_screen(&self, p: [f32; 2]) -> egui::Pos2 {
        self.canvas.min + self.pan + egui::vec2(p[0], p[1]) * self.zoom()
    }
    fn to_graph(&self, p: egui::Pos2) -> [f32; 2] {
        let v = (p - self.canvas.min - self.pan) / self.zoom();
        [v.x, v.y]
    }
    fn checkpoint(&mut self, brush: &Brush) {
        self.undo
            .push((brush.graph.clone(), brush.graph_layout.clone()));
        if self.undo.len() > UNDO_LIMIT {
            self.undo.remove(0);
        }
        self.redo.clear();
    }
    /// The canvas as last drawn, in screen points.
    #[cfg(test)]
    pub(crate) fn canvas_rect(&self) -> egui::Rect {
        self.canvas
    }
    /// Screen position of a node's output dot or of input `port`, at the
    /// level being shown.
    #[cfg(test)]
    pub(crate) fn port_pos(
        &self,
        brush: &Brush,
        node: NodeId,
        input: Option<u8>,
    ) -> Option<egui::Pos2> {
        self.port_at(&level(brush, &self.path)?, node, input)
    }
    /// An input dot, or with `None` the first output dot.
    fn port_at(&self, level: &Level, node: NodeId, input: Option<u8>) -> Option<egui::Pos2> {
        match input {
            None => self.output_at(level, node, 0),
            Some(port) => {
                let top_left = self.to_screen(level.position(node)?);
                Some(
                    top_left
                        + egui::vec2(0.0, HEADER + (f32::from(port) + 0.5) * ROW) * self.zoom(),
                )
            }
        }
    }
    /// Output dot `index` (groups can have several).
    fn output_at(&self, level: &Level, node: NodeId, index: u8) -> Option<egui::Pos2> {
        let top_left = self.to_screen(level.position(node)?);
        Some(
            top_left
                + egui::vec2(NODE_WIDTH, HEADER + (f32::from(index) + 0.5) * ROW) * self.zoom(),
        )
    }
    fn node_rect(&self, level: &Level, node: &Node) -> egui::Rect {
        let at = level.position(node.id).unwrap_or_default();
        egui::Rect::from_min_size(self.to_screen(at), node_size(&node.kind) * self.zoom())
    }
    fn hit(&self, level: &Level, pointer: egui::Pos2) -> Hit {
        let reach = (PORT + 4.0) * self.zoom().max(1.0);
        // Topmost (last drawn) first.
        for node in level.nodes.iter().rev() {
            for index in 0..node.kind.outputs() {
                if self
                    .output_at(level, node.id, index)
                    .is_some_and(|p| p.distance(pointer) <= reach)
                {
                    return Hit::Output(node.id, index);
                }
            }
            for port in 0..node.kind.inputs() {
                if self
                    .port_at(level, node.id, Some(port))
                    .is_some_and(|p| p.distance(pointer) <= reach)
                {
                    return Hit::Input(node.id, port);
                }
            }
            if self.node_rect(level, node).contains(pointer) {
                return Hit::Node(node.id);
            }
        }
        Hit::Empty
    }
}

fn node_size(kind: &NodeKind) -> egui::Vec2 {
    // Port rows, then a footer for the node's setting or problem.
    let rows = kind.inputs().max(kind.outputs()).max(1) as f32;
    egui::vec2(NODE_WIDTH, HEADER + (rows + 1.0) * ROW)
}

fn set_position(layout: &mut Vec<NodePosition>, id: NodeId, [x, y]: [f32; 2]) {
    match layout.iter_mut().find(|p| p.id == id) {
        Some(p) => (p.x, p.y) = (x, y),
        None => layout.push(NodePosition { id, x, y }),
    }
}

fn next_id(nodes: &[Node]) -> NodeId {
    NodeId(nodes.iter().map(|n| n.id.0).max().unwrap_or(0) + 1)
}

/// Add a node at one level. Brush outputs belong to the main graph, once
/// each; group inputs and results come with their group.
fn level_add(
    nodes: &mut Vec<Node>,
    layout: &mut Vec<NodePosition>,
    kind: NodeKind,
    at: [f32; 2],
    inside_group: bool,
) -> Result<NodeId, String> {
    if let Some(output) = kind.output() {
        if inside_group {
            return Err(format!(
                "{} outputs belong in the main graph, not in a group",
                kind.name()
            ));
        }
        if nodes.iter().any(|n| n.kind.output() == Some(output)) {
            return Err(format!("The graph already has a {} output", kind.name()));
        }
    }
    if matches!(
        kind,
        NodeKind::GroupInput { .. } | NodeKind::GroupResult { .. }
    ) {
        return Err("Group inputs and results come with their group".into());
    }
    let id = next_id(nodes);
    nodes.push(Node { id, kind });
    set_position(layout, id, at);
    Ok(id)
}

/// Add a node of `kind` at `at` to the main graph.
pub(crate) fn add_node(
    graph: &mut BrushGraph,
    layout: &mut Vec<NodePosition>,
    kind: NodeKind,
    at: [f32; 2],
) -> Result<NodeId, String> {
    let id = level_add(graph.edit().0, layout, kind, at, false)?;
    graph.validate().map(|_| id)
}

/// Wire `from`'s output into input `port` of `to` in the main graph,
/// replacing what was wired there. Unchanged when the result is invalid.
pub(crate) fn connect(
    graph: &mut BrushGraph,
    from: NodeId,
    to: NodeId,
    port: u8,
) -> Result<(), String> {
    let mut candidate = graph.clone();
    level_connect(candidate.edit().1, (from, 0), to, port);
    candidate.validate().map_err(friendly)?;
    *graph = candidate;
    Ok(())
}

fn level_connect(links: &mut Vec<Link>, (from, from_port): (NodeId, u8), to: NodeId, port: u8) {
    links.retain(|l| !(l.to == to && l.port == port));
    links.push(Link {
        from,
        to,
        port,
        from_port,
    });
}

/// Remove the wire into input `port` of `to`, returning where it came from.
#[cfg(test)]
pub(crate) fn disconnect(graph: &mut BrushGraph, to: NodeId, port: u8) -> Option<NodeId> {
    level_disconnect(graph.edit().1, to, port).map(|(from, _)| from)
}

/// Remove the wire into input `port` of `to`; returns its source and output.
fn level_disconnect(links: &mut Vec<Link>, to: NodeId, port: u8) -> Option<(NodeId, u8)> {
    let link = links
        .iter()
        .find(|l| l.to == to && l.port == port)
        .copied()?;
    links.retain(|l| *l != link);
    Some((link.from, link.from_port))
}

/// Remove a node and every wire touching it from the main graph.
#[cfg(test)]
pub(crate) fn remove_node(graph: &mut BrushGraph, layout: &mut Vec<NodePosition>, id: NodeId) {
    let (nodes, links) = graph.edit();
    level_remove(nodes, links, layout, &[id]);
}

fn level_remove(
    nodes: &mut Vec<Node>,
    links: &mut Vec<Link>,
    layout: &mut Vec<NodePosition>,
    ids: &[NodeId],
) {
    nodes.retain(|n| !ids.contains(&n.id));
    links.retain(|l| !ids.contains(&l.from) && !ids.contains(&l.to));
    layout.retain(|p| !ids.contains(&p.id));
}

/// Errors in the words of the edit that caused them.
fn friendly(error: String) -> String {
    if error.contains("loop") {
        "That wire would make a loop".into()
    } else {
        error
    }
}

/// Pack `selection` into one group node: values wired in from outside become
/// its inputs and values read outside become its outputs (up to eight each).
pub(crate) fn group_nodes(
    nodes: &mut Vec<Node>,
    links: &mut Vec<Link>,
    layout: &mut Vec<NodePosition>,
    selection: &[NodeId],
) -> Result<NodeId, String> {
    let set: HashSet<NodeId> = selection
        .iter()
        .copied()
        .filter(|id| nodes.iter().any(|n| n.id == *id))
        .collect();
    if set.is_empty() {
        return Err("Select the nodes to group".into());
    }
    if nodes.iter().any(|n| {
        set.contains(&n.id)
            && (n.kind.output().is_some()
                || matches!(
                    n.kind,
                    NodeKind::GroupInput { .. } | NodeKind::GroupResult { .. }
                ))
    }) {
        return Err("Brush outputs, group inputs and results stay outside a new group".into());
    }
    // Inputs: each distinct value from outside that the nodes read.
    let mut sources: Vec<(NodeId, u8)> = Vec::new();
    for link in links.iter() {
        let source = (link.from, link.from_port);
        if set.contains(&link.to) && !set.contains(&link.from) && !sources.contains(&source) {
            sources.push(source);
        }
    }
    if sources.len() > usize::from(GROUP_INPUTS) {
        return Err(format!(
            "A group takes at most {GROUP_INPUTS} inputs; these nodes read {} values from outside",
            sources.len()
        ));
    }
    // Outputs: each distinct value of theirs that nodes outside read.
    let mut results: Vec<(NodeId, u8)> = Vec::new();
    for link in links.iter() {
        let result = (link.from, link.from_port);
        if set.contains(&link.from) && !set.contains(&link.to) && !results.contains(&result) {
            results.push(result);
        }
    }
    if results.is_empty() {
        // Nothing outside reads them yet: every value nothing inside reads.
        for node in nodes.iter().filter(|n| set.contains(&n.id)) {
            for port in 0..node.kind.outputs() {
                if !links
                    .iter()
                    .any(|l| l.from == node.id && l.from_port == port && set.contains(&l.to))
                {
                    results.push((node.id, port));
                }
            }
        }
    }
    if results.is_empty() {
        return Err("These nodes give no value for a group to pass on".into());
    }
    if results.len() > usize::from(GROUP_OUTPUTS) {
        return Err(format!(
            "A group gives at most {GROUP_OUTPUTS} values; these nodes give {}",
            results.len()
        ));
    }
    let position = |id: NodeId| {
        layout
            .iter()
            .find(|p| p.id == id)
            .map_or([0.0, 0.0], |p| [p.x, p.y])
    };
    // Inside: the chosen nodes keep their ids; inputs and results follow.
    let mut inner_nodes: Vec<Node> = nodes
        .iter()
        .filter(|n| set.contains(&n.id))
        .cloned()
        .collect();
    let mut inner_layout: Vec<NodePosition> = layout
        .iter()
        .filter(|p| set.contains(&p.id))
        .copied()
        .collect();
    let (mut min_x, mut max_x, mut min_y, mut sum_y) = (f32::MAX, f32::MIN, f32::MAX, 0.0);
    for id in &set {
        let [x, y] = position(*id);
        min_x = min_x.min(x);
        max_x = max_x.max(x);
        min_y = min_y.min(y);
        sum_y += y;
    }
    let mean_y = sum_y / set.len() as f32;
    let mut input_ids = Vec::new();
    for index in 0..sources.len() {
        let id = next_id(&inner_nodes);
        inner_nodes.push(Node {
            id,
            kind: NodeKind::GroupInput { index: index as u8 },
        });
        inner_layout.push(NodePosition {
            id,
            x: min_x - 170.0,
            y: min_y + index as f32 * 90.0,
        });
        input_ids.push(id);
    }
    let mut result_ids = Vec::new();
    for index in 0..results.len() {
        let id = next_id(&inner_nodes);
        inner_nodes.push(Node {
            id,
            kind: NodeKind::GroupResult { index: index as u8 },
        });
        inner_layout.push(NodePosition {
            id,
            x: max_x + 170.0,
            y: min_y + index as f32 * 90.0,
        });
        result_ids.push(id);
    }
    let mut inner_links: Vec<Link> = Vec::new();
    for link in links.iter().filter(|l| set.contains(&l.to)) {
        if set.contains(&link.from) {
            inner_links.push(*link);
        } else {
            let index = sources
                .iter()
                .position(|s| *s == (link.from, link.from_port))
                .unwrap();
            inner_links.push(Link {
                from: input_ids[index],
                to: link.to,
                port: link.port,
                from_port: 0,
            });
        }
    }
    for (index, (from, from_port)) in results.iter().enumerate() {
        inner_links.push(Link {
            from: *from,
            to: result_ids[index],
            port: 0,
            from_port: *from_port,
        });
    }
    // Outside: one group node where the chosen ones were.
    let group_id = next_id(nodes);
    let users: Vec<(u8, NodeId, u8)> = links
        .iter()
        .filter(|l| set.contains(&l.from) && !set.contains(&l.to))
        .map(|l| {
            let index = results
                .iter()
                .position(|r| *r == (l.from, l.from_port))
                .unwrap();
            (index as u8, l.to, l.port)
        })
        .collect();
    let ids: Vec<NodeId> = set.iter().copied().collect();
    level_remove(nodes, links, layout, &ids);
    nodes.push(Node {
        id: group_id,
        kind: NodeKind::Group(Box::new(Group {
            name: "Group".into(),
            inputs: sources.len() as u8,
            outputs: results.len() as u8,
            nodes: inner_nodes,
            links: inner_links,
            layout: inner_layout,
        })),
    });
    set_position(layout, group_id, [(min_x + max_x) / 2.0, mean_y]);
    for (index, (from, from_port)) in sources.iter().enumerate() {
        links.push(Link {
            from: *from,
            to: group_id,
            port: index as u8,
            from_port: *from_port,
        });
    }
    for (from_port, to, port) in users {
        links.push(Link {
            from: group_id,
            to,
            port,
            from_port,
        });
    }
    Ok(group_id)
}

/// Unpack a group node in place, rewiring its inputs and results. Returns
/// the nodes it held, under their new ids.
pub(crate) fn ungroup(
    nodes: &mut Vec<Node>,
    links: &mut Vec<Link>,
    layout: &mut Vec<NodePosition>,
    id: NodeId,
) -> Result<Vec<NodeId>, String> {
    let Some(NodeKind::Group(group)) = nodes.iter().find(|n| n.id == id).map(|n| n.kind.clone())
    else {
        return Err("Select a group to ungroup".into());
    };
    let at = layout
        .iter()
        .find(|p| p.id == id)
        .map_or([0.0, 0.0], |p| [p.x, p.y]);
    let plain = |n: &Node| {
        !matches!(
            n.kind,
            NodeKind::GroupInput { .. } | NodeKind::GroupResult { .. }
        )
    };
    let inner_at = |inner: NodeId| {
        group
            .layout
            .iter()
            .find(|p| p.id == inner)
            .map_or([0.0, 0.0], |p| [p.x, p.y])
    };
    let held: Vec<&Node> = group.nodes.iter().filter(|n| plain(n)).collect();
    let center = held.iter().fold([0.0, 0.0], |c, n| {
        let p = inner_at(n.id);
        [c[0] + p[0], c[1] + p[1]]
    });
    let center = if held.is_empty() {
        [0.0, 0.0]
    } else {
        [center[0] / held.len() as f32, center[1] / held.len() as f32]
    };
    // New ids above everything at this level.
    let mut fresh = std::collections::HashMap::new();
    let mut added = Vec::new();
    for (next, node) in (next_id(nodes).0..).zip(&held) {
        let new = NodeId(next);
        fresh.insert(node.id, new);
        added.push(new);
    }
    let input_index = |inner: NodeId| {
        group
            .nodes
            .iter()
            .find(|n| n.id == inner)
            .and_then(|n| match n.kind {
                NodeKind::GroupInput { index } => Some(index),
                _ => None,
            })
    };
    let outer_source = |index: u8| {
        links
            .iter()
            .find(|l| l.to == id && l.port == index)
            .map(|l| (l.from, l.from_port))
    };
    // What a wire inside reads from, as seen from outside.
    let resolve = |inner: NodeId, port: u8| -> Option<(NodeId, u8)> {
        fresh
            .get(&inner)
            .map(|new| (*new, port))
            .or_else(|| input_index(inner).and_then(outer_source))
    };
    let result_source = |index: u8| {
        let result = group
            .nodes
            .iter()
            .find(|n| n.kind == NodeKind::GroupResult { index })?
            .id;
        let link = group.links.iter().find(|l| l.to == result)?;
        resolve(link.from, link.from_port)
    };
    let mut new_links = Vec::new();
    for link in &group.links {
        if let Some(to) = fresh.get(&link.to)
            && let Some((from, from_port)) = resolve(link.from, link.from_port)
        {
            new_links.push(Link {
                from,
                to: *to,
                port: link.port,
                from_port,
            });
        }
    }
    for link in links.iter().filter(|l| l.from == id) {
        if let Some((from, from_port)) = result_source(link.from_port) {
            new_links.push(Link {
                from,
                to: link.to,
                port: link.port,
                from_port,
            });
        }
    }
    let new_nodes: Vec<Node> = held
        .iter()
        .map(|n| Node {
            id: fresh[&n.id],
            kind: n.kind.clone(),
        })
        .collect();
    let new_layout: Vec<NodePosition> = held
        .iter()
        .map(|n| {
            let p = inner_at(n.id);
            NodePosition {
                id: fresh[&n.id],
                x: at[0] + p[0] - center[0],
                y: at[1] + p[1] - center[1],
            }
        })
        .collect();
    level_remove(nodes, links, layout, &[id]);
    nodes.extend(new_nodes);
    links.extend(new_links);
    layout.extend(new_layout);
    Ok(added)
}

/// Every group in these brushes' graphs, once each, for Add node > Groups.
fn library_groups(brushes: &[&Brush]) -> Vec<Group> {
    let mut found: Vec<Group> = Vec::new();
    for brush in brushes {
        let Some(graph) = &brush.graph else { continue };
        for node in graph.nodes() {
            if let NodeKind::Group(group) = &node.kind {
                let same = |g: &Group| {
                    g.name == group.name && g.nodes == group.nodes && g.links == group.links
                };
                if !found.iter().any(same) {
                    found.push((**group).clone());
                }
            }
        }
    }
    found.sort_by_key(|g| g.name.to_lowercase());
    found
}

impl Dudel {
    #[cfg(test)]
    pub(crate) fn graph_editor_selected(&self) -> Option<NodeId> {
        self.graph_editor.selected.last().copied()
    }
    #[cfg(test)]
    pub(crate) fn graph_editor_path(&self) -> &[NodeId] {
        &self.graph_editor.path
    }
    fn graph_checkpoint(&mut self) {
        self.graph_editor.checkpoint(&self.brush);
    }
    fn graph_undo(&mut self) {
        if let Some((graph, layout)) = self.graph_editor.undo.pop() {
            let current = (
                std::mem::replace(&mut self.brush.graph, graph),
                std::mem::replace(&mut self.brush.graph_layout, layout),
            );
            self.graph_editor.redo.push(current);
            self.graph_after_history();
        }
    }
    fn graph_redo(&mut self) {
        if let Some((graph, layout)) = self.graph_editor.redo.pop() {
            let current = (
                std::mem::replace(&mut self.brush.graph, graph),
                std::mem::replace(&mut self.brush.graph_layout, layout),
            );
            self.graph_editor.undo.push(current);
            self.graph_after_history();
        }
    }
    /// After undo or redo: leave groups that no longer exist.
    fn graph_after_history(&mut self) {
        while !self.graph_editor.path.is_empty()
            && level(&self.brush, &self.graph_editor.path).is_none()
        {
            self.graph_editor.path.pop();
        }
        self.graph_editor.formula = None;
    }
    /// Apply `edit` to the level being shown, keep it only if the whole
    /// graph stays valid, and make it an undo step when `undo`.
    fn graph_edit<R>(
        &mut self,
        undo: bool,
        edit: impl FnOnce(&mut Vec<Node>, &mut Vec<Link>, &mut Vec<NodePosition>) -> Result<R, String>,
    ) -> Result<R, String> {
        let before = (self.brush.graph.clone(), self.brush.graph_layout.clone());
        let path = self.graph_editor.path.clone();
        let result = match level_mut(&mut self.brush, &path) {
            Some((nodes, links, layout)) => edit(nodes, links, layout),
            None => Err("The open group is gone".into()),
        }
        .and_then(
            |value| match self.brush.graph.as_ref().map(BrushGraph::validate) {
                Some(Err(error)) => Err(friendly(error)),
                _ => Ok(value),
            },
        );
        match result {
            Ok(value) => {
                if undo {
                    self.graph_editor.undo.push(before);
                    if self.graph_editor.undo.len() > UNDO_LIMIT {
                        self.graph_editor.undo.remove(0);
                    }
                    self.graph_editor.redo.clear();
                }
                Ok(value)
            }
            Err(error) => {
                (self.brush.graph, self.brush.graph_layout) = before;
                Err(error)
            }
        }
    }
    /// Run an edit, reporting a refusal in the status bar.
    fn graph_try<R>(
        &mut self,
        edit: impl FnOnce(&mut Vec<Node>, &mut Vec<Link>, &mut Vec<NodePosition>) -> Result<R, String>,
    ) -> Option<R> {
        match self.graph_edit(true, edit) {
            Ok(value) => Some(value),
            Err(error) => {
                self.status = error;
                None
            }
        }
    }
    /// Screenshot scene: Brush Studio's graph section with a made graph plus
    /// speed reducing flow through a curve, the curve selected.
    pub(crate) fn brush_graph_capture_fixture(&mut self) {
        self.brush_studio = true;
        self.brush_studio_section = crate::brush_studio::Section::Graph;
        self.graph_editor.owner = Some(self.selected_brush);
        self.make_brush_graph();
        let mut graph = self.brush.graph.clone().unwrap();
        let mut layout = Vec::new();
        let speed = add_node(&mut graph, &mut layout, NodeKind::Speed, [0.0, 0.0]).unwrap();
        let curve = add_node(
            &mut graph,
            &mut layout,
            NodeKind::Curve(crate::dynamics::Curve {
                points: vec![[0.0, 1.0], [0.6, 0.7], [1.0, 0.3]],
            }),
            [0.0, 0.0],
        )
        .unwrap();
        let flow = add_node(&mut graph, &mut layout, NodeKind::FlowOutput, [0.0, 0.0]).unwrap();
        connect(&mut graph, speed, curve, 0).unwrap();
        connect(&mut graph, curve, flow, 0).unwrap();
        self.brush.graph_layout = crate::brush_graph::auto_layout(&graph);
        self.brush.graph = Some(graph);
        self.graph_editor.selected = vec![curve];
        self.graph_editor.fit = true;
    }
    /// Screenshot scene: a color brush (random hue jitter, pressure blending
    /// toward the secondary color) with strokes on the Studio's test pad.
    pub(crate) fn color_brush_capture_fixture(&mut self) {
        self.brush_studio = true;
        self.brush_studio_section = crate::brush_studio::Section::Graph;
        self.graph_editor.owner = Some(self.selected_brush);
        self.brush = crate::brush::Brush {
            name: "Confetti marker".into(),
            size: 22.0,
            hardness: 0.8,
            spacing: 0.12,
            pressure_size: true,
            ..Default::default()
        };
        self.color = [40, 90, 200];
        self.secondary = [235, 120, 40];
        self.make_brush_graph();
        let mut graph = self.brush.graph.clone().unwrap();
        let mut layout = Vec::new();
        let random = add_node(
            &mut graph,
            &mut layout,
            NodeKind::Random { seed: 7 },
            [0.0; 2],
        )
        .unwrap();
        let hue = add_node(
            &mut graph,
            &mut layout,
            NodeKind::HueOutput { degrees: 25.0 },
            [0.0; 2],
        )
        .unwrap();
        let pressure = add_node(
            &mut graph,
            &mut layout,
            NodeKind::Pressure { shaped: false },
            [0.0; 2],
        )
        .unwrap();
        let mix = add_node(&mut graph, &mut layout, NodeKind::ColorMixOutput, [0.0; 2]).unwrap();
        connect(&mut graph, random, hue, 0).unwrap();
        connect(&mut graph, pressure, mix, 0).unwrap();
        self.brush.graph_layout = crate::brush_graph::auto_layout(&graph);
        self.brush.graph = Some(graph);
        self.graph_editor.selected = vec![hue];
        self.graph_editor.fit = true;
        let (color, secondary) = (self.color, self.secondary);
        self.scratchpad
            .demo(&self.brush, color, secondary, [900, 160]);
    }
    /// Screenshot scene: a graph with a group and a formula, the group open.
    pub(crate) fn graph_group_capture_fixture(&mut self) {
        self.brush_studio = true;
        self.brush_studio_section = crate::brush_studio::Section::Graph;
        self.graph_editor.owner = Some(self.selected_brush);
        let template = crate::brush_graph::templates()
            .into_iter()
            .find(|t| t.name == "Wobbly ink")
            .unwrap();
        self.apply_graph_template(template);
        let noise = level(&self.brush, &[])
            .unwrap()
            .nodes
            .iter()
            .filter(|n| matches!(n.kind, NodeKind::Noise { .. } | NodeKind::Remap { .. }))
            .map(|n| n.id)
            .collect::<Vec<_>>();
        let group = self
            .graph_try(|nodes, links, layout| group_nodes(nodes, links, layout, &noise))
            .unwrap();
        let _ = self.graph_edit(true, |nodes, _, _| {
            if let Some(NodeKind::Group(g)) = nodes
                .iter_mut()
                .find(|n| n.id == group)
                .map(|n| &mut n.kind)
            {
                g.name = "Hand wobble".into();
            }
            Ok(())
        });
        self.graph_editor.path = vec![group];
        let at = [0.0, 200.0];
        let formula = self.graph_try(|nodes, _, layout| {
            level_add(
                nodes,
                layout,
                NodeKind::Formula {
                    expression: "clamp(a * 1.2 - 0.1, 0, 1)".into(),
                },
                at,
                true,
            )
        });
        self.graph_editor.selected = formula.into_iter().collect();
        self.graph_editor.fit = true;
    }
    /// Replace the graph with a template's, as one undo step.
    pub(crate) fn apply_graph_template(&mut self, template: crate::brush_graph::Template) {
        self.graph_checkpoint();
        self.brush.graph_layout = crate::brush_graph::auto_layout(&template.graph);
        self.brush.graph = Some(template.graph);
        self.graph_editor.selected.clear();
        self.graph_editor.path.clear();
        self.graph_editor.fit = true;
        // A round tip cannot show turning; give it a flat one.
        self.status = if template.flat_tip && self.brush.aspect >= 0.99 {
            self.brush.aspect = 0.35;
            format!("{}: tip flattened to 35% so turning shows", template.name)
        } else {
            format!("{}: {}", template.name, template.description)
        };
    }
    /// The Templates menu's entries; returns the one chosen.
    fn templates_menu(ui: &mut egui::Ui) -> Option<crate::brush_graph::Template> {
        let mut chosen = None;
        for template in crate::brush_graph::templates() {
            if ui
                .button(template.name)
                .on_hover_text(template.description)
                .clicked()
            {
                chosen = Some(template);
                ui.close();
            }
        }
        chosen
    }
    /// Make a graph from the brush's pressure and dynamics settings.
    pub(crate) fn make_brush_graph(&mut self) {
        self.graph_checkpoint();
        let graph = BrushGraph::from_brush(&self.brush);
        self.brush.graph_layout = crate::brush_graph::auto_layout(&graph);
        self.brush.graph = Some(graph);
        self.graph_editor.path.clear();
        self.graph_editor.fit = true;
        self.status =
            "Brush graph made from the current settings; the brush paints the same".into();
    }
    fn add_graph_node(&mut self, kind: NodeKind, at: [f32; 2]) {
        self.add_graph_node_from(kind, at, None);
    }
    /// Add a node, wired from `from`'s output into its first input when
    /// given; one undo step either way.
    fn add_graph_node_from(&mut self, kind: NodeKind, at: [f32; 2], from: Option<(NodeId, u8)>) {
        if self.brush.graph.is_none() {
            self.brush.graph = Some(BrushGraph::new(vec![], vec![]));
        }
        let inside = !self.graph_editor.path.is_empty();
        if let Some(id) = self.graph_try(|nodes, links, layout| {
            let id = level_add(nodes, layout, kind, at, inside)?;
            if let Some(from) = from {
                level_connect(links, from, id, 0);
            }
            Ok(id)
        }) {
            self.graph_editor.selected = vec![id];
        }
    }
    fn remove_graph_nodes(&mut self, ids: Vec<NodeId>) {
        // Group inputs and results go only with their group.
        let removable: Vec<NodeId> = level(&self.brush, &self.graph_editor.path)
            .map(|level| {
                level
                    .nodes
                    .iter()
                    .filter(|n| {
                        ids.contains(&n.id)
                            && !matches!(
                                n.kind,
                                NodeKind::GroupInput { .. } | NodeKind::GroupResult { .. }
                            )
                    })
                    .map(|n| n.id)
                    .collect()
            })
            .unwrap_or_default();
        if removable.is_empty() {
            self.status = "Group inputs and results stay; ungroup to remove them".into();
            return;
        }
        let _ = self.graph_try(|nodes, links, layout| {
            level_remove(nodes, links, layout, &removable);
            Ok(())
        });
        self.graph_editor
            .selected
            .retain(|id| !removable.contains(id));
    }
    fn group_selection(&mut self) {
        let selection = self.graph_editor.selected.clone();
        if let Some(id) =
            self.graph_try(|nodes, links, layout| group_nodes(nodes, links, layout, &selection))
        {
            self.graph_editor.selected = vec![id];
            self.status = "Grouped · Open group to edit inside, or rename it".into();
        }
    }
    fn ungroup_selected(&mut self, id: NodeId) {
        if let Some(held) = self.graph_try(|nodes, links, layout| ungroup(nodes, links, layout, id))
        {
            self.graph_editor.selected = held;
        }
    }
    fn open_group(&mut self, id: NodeId) {
        let mut path = self.graph_editor.path.clone();
        path.push(id);
        if let Some(level) = level(&self.brush, &path)
            && level.nodes.len() < 129
        {
            self.graph_editor.path = path;
            self.graph_editor.selected.clear();
            self.graph_editor.formula = None;
            self.graph_editor.fit = true;
        }
    }

    /// Keys for the graph while the pointer is over its canvas: Delete,
    /// Ctrl+Z / Ctrl+Y / Ctrl+Shift+Z, Ctrl+A, Ctrl+G / Ctrl+Shift+G and Esc.
    /// They are taken before the app's shortcuts, so they never also act on
    /// the document. Returns whether one was used.
    pub(crate) fn graph_editor_keys(&mut self, ctx: &egui::Context) -> bool {
        use egui::{Key, Modifiers};
        let pass = ctx.cumulative_pass_nr();
        // Typing in a field (a node search, a formula) never edits the graph.
        if ctx.wants_keyboard_input()
            || self.brush.graph.is_none()
            || !self.brush_studio
            || self
                .graph_editor
                .hovered_pass
                .is_none_or(|hovered| hovered + 1 < pass)
        {
            return false;
        }
        let key = |modifiers: Modifiers, key: Key| ctx.input_mut(|i| i.consume_key(modifiers, key));
        let shift_command = Modifiers::COMMAND | Modifiers::SHIFT;
        let selection = self.graph_editor.selected.clone();
        if key(shift_command, Key::Z) || key(Modifiers::COMMAND, Key::Y) {
            self.graph_redo();
        } else if key(Modifiers::COMMAND, Key::Z) {
            self.graph_undo();
        } else if key(shift_command, Key::G) {
            match selection.as_slice() {
                [one] => self.ungroup_selected(*one),
                _ => self.status = "Select one group to ungroup".into(),
            }
        } else if key(Modifiers::COMMAND, Key::G) {
            self.group_selection();
        } else if key(Modifiers::COMMAND, Key::A) {
            self.graph_editor.selected = level(&self.brush, &self.graph_editor.path)
                .map(|l| l.nodes.iter().map(|n| n.id).collect())
                .unwrap_or_default();
        } else if key(Modifiers::NONE, Key::Delete) || key(Modifiers::NONE, Key::Backspace) {
            if !selection.is_empty() {
                self.remove_graph_nodes(selection);
            }
        } else if key(Modifiers::NONE, Key::Escape) {
            if !selection.is_empty() {
                self.graph_editor.selected.clear();
            } else if self.graph_editor.path.pop().is_some() {
                self.graph_editor.fit = true;
            }
        } else {
            return false;
        }
        true
    }

    /// The Brush graph section of Brush Studio.
    pub(crate) fn brush_graph_ui(&mut self, ui: &mut egui::Ui) {
        if self.graph_editor.owner != Some(self.selected_brush) {
            self.graph_editor = GraphEditor {
                owner: Some(self.selected_brush),
                fit: true,
                ..Default::default()
            };
        }
        if self.brush.graph.is_none() {
            self.graph_editor.path.clear();
            ui.label(
                "A brush graph computes each dab's size, opacity, flow, spacing, rotation, \
                 scatter and color from pen input, with nodes you wire together. Start from \
                 this brush's pressure and dynamics settings, a template, or an empty graph.",
            );
            ui.horizontal_wrapped(|ui| {
                if ui
                    .button("Make graph from current settings")
                    .on_hover_text("The brush paints exactly as before; edit from there")
                    .clicked()
                {
                    self.make_brush_graph();
                }
                let mut template = None;
                ui.menu_button("Start from a template", |ui| {
                    template = Self::templates_menu(ui);
                });
                if let Some(template) = template {
                    self.apply_graph_template(template);
                }
                if ui.button("Start an empty graph").clicked() {
                    self.graph_checkpoint();
                    self.brush.graph = Some(BrushGraph::new(vec![], vec![]));
                    self.brush.graph_layout.clear();
                }
                if ui
                    .add_enabled(
                        !self.graph_editor.undo.is_empty(),
                        egui::Button::new("Undo"),
                    )
                    .clicked()
                {
                    self.graph_undo();
                }
            });
            return;
        }
        if level(&self.brush, &self.graph_editor.path).is_none() {
            self.graph_editor.path.clear();
        }
        self.graph_toolbar(ui);
        self.graph_breadcrumb(ui);
        if let Some(Err(error)) = self.brush.graph.as_ref().map(BrushGraph::validate) {
            ui.colored_label(ui.visuals().error_fg_color, error);
        }
        // Node settings sit beside the canvas when there is room, else below.
        let side = ui.available_width() >= 540.0;
        // Fill what is visible below the toolbar, so toolbar and canvas show
        // together without scrolling.
        let visible = ui.clip_rect().bottom() - ui.cursor().top() - if side { 4.0 } else { 28.0 };
        let height = visible.clamp(180.0, 520.0);
        if side {
            ui.horizontal_top(|ui| {
                let width = ui.available_width() - 252.0;
                self.graph_canvas(ui, egui::vec2(width, height));
                ui.vertical(|ui| {
                    ui.set_width(240.0);
                    egui::ScrollArea::vertical()
                        .id_salt("graph node settings")
                        .max_height(height)
                        .max_width(240.0)
                        .auto_shrink([false, true])
                        .show(ui, |ui| {
                            // Keep labels, sliders and notes inside the strip.
                            ui.set_max_width(228.0);
                            ui.spacing_mut().slider_width = 150.0;
                            self.graph_node_settings(ui);
                            self.graph_probe_ui(ui);
                        });
                });
            });
        } else {
            self.graph_canvas(ui, egui::vec2(ui.available_width(), height));
            ui.add_space(4.0);
            self.graph_node_settings(ui);
            self.graph_probe_ui(ui);
        }
    }

    /// "Try inputs": the pen input the nodes' values are shown for. While
    /// the test pad is being drawn on, its latest dab is shown instead.
    fn graph_probe_ui(&mut self, ui: &mut egui::Ui) {
        ui.separator();
        let live = self.scratchpad.recent_dab().is_some();
        if live {
            // Return to the sliders when drawing stops.
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(500));
        }
        egui::CollapsingHeader::new("Try inputs")
            .id_salt("graph try inputs")
            .default_open(true)
            .show(ui, |ui| {
                ui.checkbox(&mut self.graph_editor.show_values, "Show values on nodes")
                    .on_hover_text("Each node shows what it computes for this input");
                if live {
                    note(ui, "Showing the latest dab on the test pad.");
                } else {
                    note(
                        ui,
                        "Set a pen input to see what every node computes, or draw on the test pad to see the real values.",
                    );
                }
                ui.add_enabled_ui(!live, |ui| {
                    let probe = &mut self.graph_editor.probe;
                    labeled(ui, "Pressure", egui::Slider::new(&mut probe.pressure, 0.0..=1.0));
                    labeled(ui, "Speed", egui::Slider::new(&mut probe.speed, 0.0..=1.0));
                    labeled(
                        ui,
                        "Tilt",
                        egui::Slider::new(&mut probe.tilt, 0.0..=90.0)
                            .suffix("°")
                            .integer(),
                    );
                    labeled(
                        ui,
                        "Direction",
                        egui::Slider::new(&mut probe.direction, -180.0..=180.0)
                            .suffix("°")
                            .integer(),
                    );
                    labeled(
                        ui,
                        "Distance along the stroke",
                        egui::Slider::new(&mut probe.distance, 0.0..=2000.0)
                            .suffix(" px")
                            .integer(),
                    );
                    labeled(ui, "Random", egui::Slider::new(&mut probe.random, 0.0..=1.0));
                    if ui.small_button("Reset").clicked() {
                        *probe = Probe::default();
                    }
                });
            });
    }

    /// Each node's value at the level shown, for the shown input: (node,
    /// port).
    fn graph_values(&self) -> std::collections::HashMap<(NodeId, u8), f32> {
        let editor = &self.graph_editor;
        if !editor.show_values {
            return Default::default();
        }
        let Some(compiled) = self.brush.graph.as_ref().and_then(|g| g.compiled().ok()) else {
            return Default::default();
        };
        let sample = self
            .scratchpad
            .recent_dab()
            .unwrap_or_else(|| editor.probe.sample());
        compiled
            .trace(&self.brush.graph_inputs(compiled, &sample), &editor.path)
            .into_iter()
            .map(|(id, port, value)| ((id, port), value))
            .collect()
    }

    /// Where the canvas is: the main graph, or a group inside it.
    fn graph_breadcrumb(&mut self, ui: &mut egui::Ui) {
        if self.graph_editor.path.is_empty() {
            return;
        }
        let mut names = Vec::new();
        for depth in 1..=self.graph_editor.path.len() {
            let name = level(&self.brush, &self.graph_editor.path[..depth - 1])
                .and_then(|level| {
                    level
                        .nodes
                        .iter()
                        .find(|n| n.id == self.graph_editor.path[depth - 1])
                        .map(|n| n.kind.title())
                })
                .unwrap_or_default();
            names.push(name);
        }
        let mut go = None;
        ui.horizontal_wrapped(|ui| {
            if ui
                .button("Back")
                .on_hover_text("Leave this group")
                .clicked()
            {
                go = Some(self.graph_editor.path.len() - 1);
            }
            if ui.link("Main graph").clicked() {
                go = Some(0);
            }
            for (depth, name) in names.iter().enumerate() {
                ui.label("›");
                if depth + 1 == names.len() {
                    ui.strong(name);
                } else if ui.link(name).clicked() {
                    go = Some(depth + 1);
                }
            }
        });
        if let Some(depth) = go {
            self.graph_editor.path.truncate(depth);
            self.graph_editor.selected.clear();
            self.graph_editor.formula = None;
            self.graph_editor.fit = true;
        }
    }

    fn graph_canvas(&mut self, ui: &mut egui::Ui, size: egui::Vec2) {
        let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click_and_drag());
        self.graph_editor.canvas = rect;
        if response.hovered() || response.dragged() {
            self.graph_editor.hovered_pass = Some(ui.ctx().cumulative_pass_nr());
        }
        if std::mem::take(&mut self.graph_editor.fit) {
            self.fit_graph_view();
        }
        self.graph_canvas_input(ui, &response);
        self.paint_graph(ui, rect, &response);
        self.graph_hover_help(&response);
        self.graph_context_menu(&response);
        self.dropped_wire_ui(ui.ctx());
    }

    /// What the node, input or output under the pointer does.
    fn graph_hover_help(&self, response: &egui::Response) {
        let editor = &self.graph_editor;
        if editor.drag.is_some() || editor.dropped.is_some() {
            return;
        }
        let (Some(pointer), Some(level)) = (response.hover_pos(), level(&self.brush, &editor.path))
        else {
            return;
        };
        let node = |id: NodeId| level.nodes.iter().find(|n| n.id == id);
        let help = match editor.hit(&level, pointer) {
            Hit::Node(id) => node(id).map(|n| n.kind.description().to_owned()),
            Hit::Input(id, port) => node(id).map(|n| {
                let name = n
                    .kind
                    .port_names()
                    .get(usize::from(port))
                    .copied()
                    .unwrap_or("Input");
                if level.wired(id, port) {
                    format!("{name} · drag to move this wire, or off to remove it")
                } else {
                    format!("{name} · drag a wire here from an output")
                }
            }),
            Hit::Output(..) => Some(
                "Drag to an input to connect, or onto empty canvas to add a node there".to_owned(),
            ),
            Hit::Empty => None,
        };
        if let Some(help) = help {
            response.clone().on_hover_text_at_pointer(help);
        }
    }

    /// After a wire is let go over empty canvas: a node search there. The
    /// node chosen is added where the wire ended, wired from its source.
    fn dropped_wire_ui(&mut self, ctx: &egui::Context) {
        let Some(mut dropped) = self.graph_editor.dropped else {
            return;
        };
        let inside = !self.graph_editor.path.is_empty();
        let nodes = level(&self.brush, &self.graph_editor.path).map(|l| l.nodes);
        let mut chosen = None;
        let mut close = ctx.input(|i| i.key_pressed(egui::Key::Escape));
        let area = egui::Area::new(egui::Id::new("graph wire search"))
            .order(egui::Order::Foreground)
            .fixed_pos(dropped.screen)
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.set_width(200.0);
                    ui.weak("Connect to a new node");
                    let search = ui.add(
                        egui::TextEdit::singleline(&mut self.graph_editor.search)
                            .hint_text("Search nodes")
                            .desired_width(f32::INFINITY),
                    );
                    if std::mem::take(&mut dropped.focus) {
                        search.request_focus();
                    }
                    let enter =
                        search.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    let found = matching_kinds(&self.graph_editor.search, nodes, inside, true);
                    egui::ScrollArea::vertical()
                        .max_height(240.0)
                        .show(ui, |ui| {
                            chosen = node_list_ui(ui, &found, enter);
                        });
                });
            });
        // A press elsewhere dismisses the search.
        if ctx.input(|i| i.pointer.any_pressed())
            && ctx
                .pointer_interact_pos()
                .is_some_and(|p| !area.response.rect.contains(p))
        {
            close = true;
        }
        if let Some(kind) = chosen {
            self.graph_editor.dropped = None;
            self.graph_editor.search.clear();
            self.add_graph_node_from(kind, dropped.at, Some((dropped.from, dropped.from_port)));
        } else if close {
            self.graph_editor.dropped = None;
            self.graph_editor.search.clear();
        } else {
            self.graph_editor.dropped = Some(dropped);
        }
    }

    /// The node list, with the groups found in the brush library.
    fn add_node_menu(&mut self, ui: &mut egui::Ui, at: [f32; 2]) {
        let inside = !self.graph_editor.path.is_empty();
        let nodes = level(&self.brush, &self.graph_editor.path).map(|l| l.nodes);
        let search = ui.add(
            egui::TextEdit::singleline(&mut self.graph_editor.search)
                .hint_text("Search nodes")
                .desired_width(180.0),
        );
        let enter = search.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        let chosen = if self.graph_editor.search.trim().is_empty() {
            node_catalog_ui(ui, nodes, inside)
        } else {
            let found = matching_kinds(&self.graph_editor.search, nodes, inside, false);
            node_list_ui(ui, &found, enter)
        };
        if chosen.is_some() {
            self.graph_editor.search.clear();
        }
        let mut brushes: Vec<&Brush> = vec![&self.brush];
        brushes.extend(self.brushes.iter());
        let groups = library_groups(&brushes);
        let mut group = None;
        ui.add_enabled_ui(!groups.is_empty(), |ui| {
            ui.menu_button("Groups", |ui| {
                for candidate in &groups {
                    if ui
                        .button(&candidate.name)
                        .on_hover_text(format!(
                            "{} nodes, {} inputs",
                            candidate.nodes.len(),
                            candidate.inputs
                        ))
                        .clicked()
                    {
                        group = Some(candidate.clone());
                        ui.close();
                    }
                }
            })
            .response
            .on_disabled_hover_text("Groups you make in any brush appear here");
        });
        if let Some(kind) = chosen.or(group.map(|g| NodeKind::Group(Box::new(g)))) {
            self.add_graph_node(kind, at);
            ui.close();
        }
    }

    fn graph_toolbar(&mut self, ui: &mut egui::Ui) {
        let top = self.graph_editor.path.is_empty();
        let selection = self.graph_editor.selected.clone();
        let single_group = match selection.as_slice() {
            [one] => level(&self.brush, &self.graph_editor.path)
                .and_then(|l| l.nodes.iter().find(|n| n.id == *one))
                .is_some_and(|n| matches!(n.kind, NodeKind::Group(_)))
                .then_some(*one),
            _ => None,
        };
        ui.horizontal_wrapped(|ui| {
            let center = self
                .graph_editor
                .to_graph(self.graph_editor.canvas.center());
            ui.menu_button("Add node", |ui| self.add_node_menu(ui, center));
            if top {
                let mut template = None;
                ui.menu_button("Templates", |ui| {
                    ui.weak("Replaces this graph (Undo brings it back)");
                    template = Self::templates_menu(ui);
                });
                if let Some(template) = template {
                    self.apply_graph_template(template);
                }
            }
            if ui
                .add_enabled(
                    !self.graph_editor.undo.is_empty(),
                    egui::Button::new("Undo"),
                )
                .on_hover_text("Undo the last graph change")
                .clicked()
            {
                self.graph_undo();
            }
            if ui
                .add_enabled(
                    !self.graph_editor.redo.is_empty(),
                    egui::Button::new("Redo"),
                )
                .clicked()
            {
                self.graph_redo();
            }
            if !selection.is_empty() {
                let label = if selection.len() == 1 {
                    "Delete node".to_owned()
                } else {
                    format!("Delete {} nodes", selection.len())
                };
                if ui.button(label).clicked() {
                    self.remove_graph_nodes(selection.clone());
                }
                if ui
                    .button("Group")
                    .on_hover_text("Pack the selected nodes into one node")
                    .clicked()
                {
                    self.group_selection();
                }
            }
            if let Some(group) = single_group {
                if ui.button("Open group").clicked() {
                    self.open_group(group);
                }
                if ui.button("Ungroup").clicked() {
                    self.ungroup_selected(group);
                }
            }
            if ui.button("Fit").on_hover_text("Show every node").clicked() {
                self.fit_graph_view();
            }
            if top
                && ui
                    .button("Remove graph")
                    .on_hover_text("Paint with the plain pressure and dynamics settings again")
                    .clicked()
            {
                self.graph_checkpoint();
                self.brush.graph = None;
                self.brush.graph_layout.clear();
                self.graph_editor.selected.clear();
            }
        });
    }

    fn fit_graph_view(&mut self) {
        let Some(level) = level(&self.brush, &self.graph_editor.path) else {
            return;
        };
        let mut bounds = egui::Rect::NOTHING;
        for node in level.nodes {
            let at = level.position(node.id).unwrap_or_default();
            bounds = bounds.union(egui::Rect::from_min_size(
                egui::pos2(at[0], at[1]),
                node_size(&node.kind),
            ));
        }
        let editor = &mut self.graph_editor;
        if !bounds.is_positive() {
            editor.zoom = 1.0;
            editor.pan = egui::vec2(16.0, 16.0);
            return;
        }
        let room = editor.canvas.size() - egui::vec2(32.0, 32.0);
        editor.zoom = (room.x / bounds.width())
            .min(room.y / bounds.height())
            .clamp(0.4, 1.0);
        editor.pan = editor.canvas.size() / 2.0 - bounds.center().to_vec2() * editor.zoom;
    }

    fn graph_canvas_input(&mut self, ui: &egui::Ui, response: &egui::Response) {
        // The wheel zooms around the pointer instead of scrolling the section.
        if let Some(pointer) = response.hover_pos() {
            let scroll = ui.input(|i| i.smooth_scroll_delta.y);
            if scroll != 0.0 {
                let editor = &mut self.graph_editor;
                let before = editor.to_graph(pointer);
                editor.zoom = (editor.zoom() * (scroll * 0.002).exp()).clamp(0.3, 2.5);
                let after = editor.to_screen(before);
                editor.pan += pointer - after;
                ui.ctx()
                    .input_mut(|i| i.smooth_scroll_delta = egui::Vec2::ZERO);
            }
        }
        let modifiers = ui.input(|i| i.modifiers);
        let adding = modifiers.shift || modifiers.command || modifiers.ctrl;
        let path = self.graph_editor.path.clone();
        let hit_at = |app: &Self, p: egui::Pos2| {
            level(&app.brush, &path).map_or(Hit::Empty, |l| app.graph_editor.hit(&l, p))
        };
        if response.drag_started_by(egui::PointerButton::Middle) {
            self.graph_editor.drag = Some(Drag::Pan);
        } else if response.drag_started_by(egui::PointerButton::Primary)
            && let Some(origin) = ui.input(|i| i.pointer.press_origin())
        {
            let drag = match hit_at(self, origin) {
                Hit::Output(node, from_port) => Drag::Wire {
                    from: node,
                    from_port,
                    moved: false,
                },
                Hit::Input(node, port) => {
                    // Pick up an existing wire to move it elsewhere.
                    match self.graph_edit(true, |_, links, _| {
                        level_disconnect(links, node, port).ok_or_else(String::new)
                    }) {
                        Ok((from, from_port)) => Drag::Wire {
                            from,
                            from_port,
                            moved: true,
                        },
                        Err(_) => Drag::Pan,
                    }
                }
                Hit::Node(node) => {
                    if !self.graph_editor.selected.contains(&node) {
                        if !adding {
                            self.graph_editor.selected.clear();
                        }
                        self.graph_editor.selected.push(node);
                    }
                    self.graph_checkpoint();
                    let level = level(&self.brush, &path).unwrap();
                    let start = self
                        .graph_editor
                        .selected
                        .iter()
                        .filter_map(|id| level.position(*id).map(|p| (*id, p)))
                        .collect();
                    Drag::Move {
                        from: self.graph_editor.to_graph(origin),
                        start,
                    }
                }
                Hit::Empty if modifiers.shift => Drag::Select { from: origin },
                Hit::Empty => Drag::Pan,
            };
            self.graph_editor.drag = Some(drag);
        }
        if response.dragged() {
            let delta = response.drag_delta();
            let pointer = response.interact_pointer_pos();
            match &self.graph_editor.drag {
                Some(Drag::Pan) => self.graph_editor.pan += delta,
                Some(Drag::Move { from, start }) => {
                    if let Some(pointer) = pointer {
                        let now = self.graph_editor.to_graph(pointer);
                        let offset = [now[0] - from[0], now[1] - from[1]];
                        let moves: Vec<(NodeId, [f32; 2])> = start
                            .iter()
                            .map(|(id, p)| (*id, [p[0] + offset[0], p[1] + offset[1]]))
                            .collect();
                        if let Some((_, _, layout)) = level_mut(&mut self.brush, &path) {
                            for (id, at) in moves {
                                set_position(layout, id, at);
                            }
                        }
                    }
                }
                Some(Drag::Wire { .. } | Drag::Select { .. }) | None => {}
            }
        }
        if response.drag_stopped() {
            let pointer = response
                .interact_pointer_pos()
                .or(ui.input(|i| i.pointer.latest_pos()));
            match self.graph_editor.drag.take() {
                Some(Drag::Wire {
                    from,
                    from_port,
                    moved,
                }) => match pointer.map(|p| (p, hit_at(self, p))) {
                    Some((_, Hit::Input(to, port))) => {
                        let _ = self.graph_try(|_, links, _| {
                            level_connect(links, (from, from_port), to, port);
                            Ok(())
                        });
                    }
                    // A new wire over empty canvas: offer a node to connect
                    // to. A wire moved off an input is just removed.
                    Some((pointer, Hit::Empty))
                        if !moved && self.graph_editor.canvas.contains(pointer) =>
                    {
                        self.graph_editor.search.clear();
                        self.graph_editor.dropped = Some(Dropped {
                            from,
                            from_port,
                            at: self.graph_editor.to_graph(pointer),
                            screen: pointer,
                            focus: true,
                        });
                    }
                    _ => {}
                },
                Some(Drag::Select { from }) => {
                    if let (Some(pointer), Some(level)) = (pointer, level(&self.brush, &path)) {
                        let area = egui::Rect::from_two_pos(from, pointer);
                        let boxed: Vec<NodeId> = level
                            .nodes
                            .iter()
                            .filter(|n| area.intersects(self.graph_editor.node_rect(&level, n)))
                            .map(|n| n.id)
                            .collect();
                        for id in boxed {
                            if !self.graph_editor.selected.contains(&id) {
                                self.graph_editor.selected.push(id);
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        if response.double_clicked()
            && let Some(pointer) = response.interact_pointer_pos()
            && let Hit::Node(node) = hit_at(self, pointer)
            && level(&self.brush, &path)
                .and_then(|l| l.nodes.iter().find(|n| n.id == node))
                .is_some_and(|n| matches!(n.kind, NodeKind::Group(_)))
        {
            self.open_group(node);
            return;
        }
        if response.clicked()
            && let Some(pointer) = response.interact_pointer_pos()
        {
            match hit_at(self, pointer) {
                Hit::Node(node) | Hit::Output(node, _) | Hit::Input(node, _) => {
                    let selected = &mut self.graph_editor.selected;
                    if adding {
                        if let Some(index) = selected.iter().position(|id| *id == node) {
                            selected.remove(index);
                        } else {
                            selected.push(node);
                        }
                    } else {
                        *selected = vec![node];
                    }
                }
                Hit::Empty => {
                    if !adding {
                        self.graph_editor.selected.clear();
                    }
                }
            }
        }
        if response.secondary_clicked()
            && let Some(pointer) = response.interact_pointer_pos()
        {
            let hit = hit_at(self, pointer);
            if let Hit::Node(node) = hit
                && !self.graph_editor.selected.contains(&node)
            {
                self.graph_editor.selected = vec![node];
            }
            self.graph_editor.context = Some((self.graph_editor.to_graph(pointer), hit));
        }
    }

    fn graph_context_menu(&mut self, response: &egui::Response) {
        response.context_menu(|ui| {
            let Some((at, hit)) = self.graph_editor.context else {
                return;
            };
            if let Hit::Node(_) = hit {
                let selection = self.graph_editor.selected.clone();
                let label = if selection.len() == 1 {
                    "Delete node".to_owned()
                } else {
                    format!("Delete {} nodes", selection.len())
                };
                if ui.button(label).clicked() {
                    self.remove_graph_nodes(selection);
                    ui.close();
                }
                if ui.button("Group").clicked() {
                    self.group_selection();
                    ui.close();
                }
                ui.separator();
            }
            ui.label("Add node");
            self.add_node_menu(ui, at);
        });
    }

    fn paint_graph(&self, ui: &egui::Ui, rect: egui::Rect, response: &egui::Response) {
        let editor = &self.graph_editor;
        let painter = ui.painter_at(rect);
        let visuals = ui.visuals();
        painter.rect_filled(rect, 4.0, visuals.extreme_bg_color);
        // Dot grid that pans and zooms with the nodes.
        let step = 24.0 * editor.zoom();
        if step >= 8.0 {
            let dot = visuals.weak_text_color().gamma_multiply(0.35);
            let offset = egui::vec2(editor.pan.x.rem_euclid(step), editor.pan.y.rem_euclid(step));
            let mut y = rect.top() + offset.y;
            while y < rect.bottom() {
                let mut x = rect.left() + offset.x;
                while x < rect.right() {
                    painter.circle_filled(egui::pos2(x, y), 1.0, dot);
                    x += step;
                }
                y += step;
            }
        }
        let Some(level) = level(&self.brush, &editor.path) else {
            return;
        };
        let zoom = editor.zoom();
        let wire = egui::Stroke::new(
            2.0 * zoom.max(0.6),
            visuals.text_color().gamma_multiply(0.8),
        );
        let curve = |a: egui::Pos2, b: egui::Pos2, stroke: egui::Stroke| {
            let bend = ((b.x - a.x).abs() * 0.5).max(30.0 * zoom);
            painter.add(egui::epaint::CubicBezierShape::from_points_stroke(
                [a, a + egui::vec2(bend, 0.0), b - egui::vec2(bend, 0.0), b],
                false,
                egui::Color32::TRANSPARENT,
                stroke,
            ));
        };
        for link in level.links {
            if let (Some(a), Some(b)) = (
                editor.output_at(&level, link.from, link.from_port),
                editor.port_at(&level, link.to, Some(link.port)),
            ) {
                curve(a, b, wire);
            }
        }
        match &editor.drag {
            Some(Drag::Wire {
                from, from_port, ..
            }) => {
                if let (Some(a), Some(b)) = (
                    editor.output_at(&level, *from, *from_port),
                    response.interact_pointer_pos(),
                ) {
                    curve(
                        a,
                        b,
                        egui::Stroke::new(wire.width, visuals.selection.stroke.color),
                    );
                }
            }
            Some(Drag::Select { from }) => {
                if let Some(pointer) = response.interact_pointer_pos() {
                    let area = egui::Rect::from_two_pos(*from, pointer);
                    painter.rect(
                        area,
                        2.0,
                        visuals.selection.bg_fill.gamma_multiply(0.25),
                        egui::Stroke::new(1.0_f32, visuals.selection.stroke.color),
                        egui::StrokeKind::Inside,
                    );
                }
            }
            _ => {}
        }
        let font = egui::FontId::proportional(12.0 * zoom);
        let small = egui::FontId::proportional(11.0 * zoom);
        let values = self.graph_values();
        let accent = visuals.selection.stroke.color;
        for node in level.nodes {
            let r = editor.node_rect(&level, node);
            let selected = editor.selected.contains(&node.id);
            let is_output = node.kind.output().is_some();
            let is_group = matches!(node.kind, NodeKind::Group(_));
            let is_boundary = matches!(
                node.kind,
                NodeKind::GroupInput { .. } | NodeKind::GroupResult { .. }
            );
            painter.rect(
                r,
                6.0 * zoom,
                visuals.window_fill,
                if selected {
                    egui::Stroke::new(2.0_f32, visuals.selection.stroke.color)
                } else if is_group {
                    egui::Stroke::new(1.5_f32, visuals.widgets.active.bg_fill)
                } else {
                    egui::Stroke::new(1.0_f32, visuals.widgets.noninteractive.bg_stroke.color)
                },
                egui::StrokeKind::Inside,
            );
            let header = egui::Rect::from_min_size(r.min, egui::vec2(r.width(), HEADER * zoom));
            painter.rect_filled(
                header.shrink(1.0),
                egui::CornerRadius {
                    nw: (6.0 * zoom) as u8,
                    ne: (6.0 * zoom) as u8,
                    sw: 0,
                    se: 0,
                },
                if is_output {
                    visuals.selection.bg_fill
                } else if is_group || is_boundary {
                    visuals.widgets.active.bg_fill
                } else {
                    visuals.widgets.inactive.bg_fill
                },
            );
            painter.text(
                header.left_center() + egui::vec2(8.0 * zoom, 0.0),
                egui::Align2::LEFT_CENTER,
                node.kind.title(),
                font.clone(),
                visuals.strong_text_color(),
            );
            for (port, name) in node.kind.port_names().iter().enumerate() {
                let port = port as u8;
                if let Some(p) = editor.port_at(&level, node.id, Some(port)) {
                    let linked = level.wired(node.id, port);
                    painter.circle(
                        p,
                        PORT * zoom,
                        if linked {
                            visuals.text_color()
                        } else {
                            visuals.extreme_bg_color
                        },
                        egui::Stroke::new(1.5_f32, visuals.text_color()),
                    );
                    painter.text(
                        p + egui::vec2(10.0 * zoom, 0.0),
                        egui::Align2::LEFT_CENTER,
                        name,
                        small.clone(),
                        if linked {
                            visuals.text_color()
                        } else {
                            visuals.weak_text_color()
                        },
                    );
                }
            }
            // What the node computes for the shown input: a bar under the
            // title, and the number at each output (or, for a brush
            // output, at the title's right).
            if let Some(value) = values.get(&(node.id, 0)).copied() {
                let fill = if value.is_nan() {
                    0.0
                } else {
                    value.clamp(0.0, 1.0)
                };
                let bar = egui::Rect::from_min_size(
                    header.left_bottom() + egui::vec2(1.0, -2.0 * zoom),
                    egui::vec2((r.width() - 2.0) * fill, 2.0 * zoom),
                );
                painter.rect_filled(bar, 0.0, accent);
                if is_output || matches!(node.kind, NodeKind::GroupResult { .. }) {
                    painter.text(
                        header.right_center() - egui::vec2(8.0 * zoom, 0.0),
                        egui::Align2::RIGHT_CENTER,
                        shown_value(value),
                        small.clone(),
                        visuals.strong_text_color(),
                    );
                }
            }
            let names = node.kind.output_names();
            for index in 0..node.kind.outputs() {
                if let Some(p) = editor.output_at(&level, node.id, index) {
                    painter.circle(p, PORT * zoom, visuals.text_color(), egui::Stroke::NONE);
                    if let Some(value) = values.get(&(node.id, index)) {
                        painter.text(
                            p + egui::vec2(7.0 * zoom, -7.0 * zoom),
                            egui::Align2::LEFT_BOTTOM,
                            shown_value(*value),
                            small.clone(),
                            accent,
                        );
                    }
                    if let Some(name) = names.get(usize::from(index)) {
                        painter.text(
                            p - egui::vec2(10.0 * zoom, 0.0),
                            egui::Align2::RIGHT_CENTER,
                            name,
                            small.clone(),
                            visuals.text_color(),
                        );
                    }
                }
            }
            // A plain-words problem or setting in the node's footer.
            let sink = !node.kind.has_output_port();
            let note = if sink && !level.wired(node.id, 0) {
                Some(("Not connected".to_owned(), visuals.warn_fg_color))
            } else if !sink
                && !is_group
                && !matches!(node.kind, NodeKind::Formula { .. })
                && node.kind.inputs() > 0
                && (0..node.kind.inputs()).all(|port| !level.wired(node.id, port))
            {
                Some(("Needs an input".to_owned(), visuals.warn_fg_color))
            } else {
                setting_label(&node.kind).map(|text| (text, visuals.weak_text_color()))
            };
            if let Some((text, color)) = note {
                painter.text(
                    r.left_bottom() + egui::vec2(8.0 * zoom, -ROW * zoom / 2.0),
                    egui::Align2::LEFT_CENTER,
                    text,
                    small.clone(),
                    color,
                );
            }
        }
    }

    fn graph_node_settings(&mut self, ui: &mut egui::Ui) {
        let selection = self.graph_editor.selected.clone();
        let Some(&selected) = selection.last() else {
            note(
                ui,
                "Select a node to change its settings. Right-click the canvas to add nodes; \
                 Ctrl-click or Shift-drag selects several to group.",
            );
            return;
        };
        if selection.len() > 1 {
            ui.strong(format!("{} nodes selected", selection.len()));
            note(
                ui,
                "Group packs them into one node: values wired in become its inputs, values \
                 read outside become its outputs (up to eight each).",
            );
            if ui.button("Group").clicked() {
                self.group_selection();
            }
            return;
        }
        let Some(mut kind) = level(&self.brush, &self.graph_editor.path).and_then(|level| {
            level
                .nodes
                .iter()
                .find(|n| n.id == selected)
                .map(|n| n.kind.clone())
        }) else {
            self.graph_editor.selected.clear();
            return;
        };
        let before = kind.clone();
        ui.strong(kind.title());
        note(ui, kind.description());
        let mut open = false;
        let mut unpack = false;
        match &mut kind {
            NodeKind::Pressure { shaped } => {
                ui.checkbox(shaped, "Shape with the brush's pressure curve")
                    .on_hover_text("Use the Pen pressure section's curve before this node");
            }
            NodeKind::Distance { length } => {
                labeled(
                    ui,
                    "Reaches 1 after",
                    egui::Slider::new(length, 1.0..=5000.0)
                        .logarithmic(true)
                        .suffix(" px"),
                );
            }
            NodeKind::Random { seed } => {
                ui.horizontal(|ui| {
                    ui.label("Seed");
                    ui.add(egui::DragValue::new(seed).range(0..=9999));
                });
                note(ui, "Seed 0 is the same random value brush dynamics use.");
            }
            NodeKind::Wave { period, shape } => {
                labeled(
                    ui,
                    "Repeats every",
                    egui::Slider::new(period, 2.0..=2000.0)
                        .logarithmic(true)
                        .suffix(" px"),
                );
                ui.horizontal_wrapped(|ui| {
                    for option in crate::brush_graph::WaveShape::ALL {
                        ui.selectable_value(shape, option, option.name());
                    }
                });
            }
            NodeKind::Noise { scale, seed } => {
                labeled(
                    ui,
                    "Bump length",
                    egui::Slider::new(scale, 2.0..=2000.0)
                        .logarithmic(true)
                        .suffix(" px"),
                );
                ui.horizontal(|ui| {
                    ui.label("Seed");
                    ui.add(egui::DragValue::new(seed).range(0..=9999));
                });
            }
            NodeKind::Steps { count } => {
                labeled(ui, "Levels", egui::Slider::new(count, 2..=32));
            }
            NodeKind::RotationOutput { degrees } => {
                labeled(
                    ui,
                    "Range",
                    egui::Slider::new(degrees, 0.0..=180.0)
                        .suffix("°")
                        .integer(),
                );
                note(
                    ui,
                    "0 turns the tip this far one way, 1 the other, 0.5 not at all. With 180°, Direction or Tilt direction steer it fully.",
                );
            }
            NodeKind::HueOutput { degrees } => {
                labeled(
                    ui,
                    "Range",
                    egui::Slider::new(degrees, 0.0..=180.0)
                        .suffix("°")
                        .integer(),
                );
                note(
                    ui,
                    "0 turns the hue this far one way, 1 the other, 0.5 not at all.",
                );
            }
            NodeKind::Constant { value } => {
                labeled(ui, "Value", egui::Slider::new(value, 0.0..=1.0));
            }
            NodeKind::Curve(curve) => {
                crate::dynamics::curve_ui(ui, curve);
                if ui.small_button("Reset curve").clicked() {
                    *curve = Default::default();
                }
            }
            NodeKind::Remap {
                in_low,
                in_high,
                out_low,
                out_high,
            } => {
                ui.label("From");
                labeled(ui, "Low", egui::Slider::new(in_low, 0.0..=1.0));
                labeled(ui, "High", egui::Slider::new(in_high, 0.0..=1.0));
                ui.label("To");
                labeled(ui, "Low ", egui::Slider::new(out_low, 0.0..=1.0));
                labeled(ui, "High ", egui::Slider::new(out_high, 0.0..=1.0));
                if (*in_high - *in_low).abs() < 0.01 {
                    *in_high = if *in_low <= 0.99 {
                        *in_low + 0.01
                    } else {
                        *in_low - 0.01
                    };
                }
            }
            NodeKind::Formula { expression } => {
                // The text being typed; the node keeps its last valid formula.
                let mut draft = match &self.graph_editor.formula {
                    Some((id, text)) if *id == selected => text.clone(),
                    _ => expression.clone(),
                };
                ui.add(
                    egui::TextEdit::singleline(&mut draft)
                        .font(egui::TextStyle::Monospace)
                        .char_limit(crate::brush_formula::MAX_CHARS)
                        .desired_width(f32::INFINITY),
                );
                match crate::brush_formula::Program::parse(&draft) {
                    Ok(program) => {
                        *expression = draft.clone();
                        let reads = program.reads();
                        let unused: Vec<&str> = ["a", "b", "c"]
                            .into_iter()
                            .zip(reads)
                            .filter(|(_, read)| !read)
                            .map(|(name, _)| name)
                            .collect();
                        if !unused.is_empty() && unused.len() < 3 {
                            note(ui, &format!("Not read: {}", unused.join(", ")));
                        }
                    }
                    Err(error) => {
                        ui.colored_label(ui.visuals().error_fg_color, error);
                    }
                }
                self.graph_editor.formula = Some((selected, draft));
                note(
                    ui,
                    &format!(
                        "Unwired inputs count as 0. A result that is not a number (such as \
                         dividing by 0) counts as missing. Names: {}.",
                        crate::brush_formula::NAMES
                    ),
                );
            }
            NodeKind::Group(group) => {
                ui.horizontal(|ui| {
                    ui.label("Name");
                    let mut name = group.name.clone();
                    if ui
                        .add(egui::TextEdit::singleline(&mut name).char_limit(40))
                        .changed()
                        && !name.trim().is_empty()
                    {
                        group.name = name;
                    }
                });
                note(
                    ui,
                    &format!(
                        "{} nodes inside, {} input{} and {} output{}. Groups travel with the \
                         brush when it is exported, and appear under Add node › Groups in your \
                         other brushes.",
                        group.nodes.len(),
                        group.inputs,
                        if group.inputs == 1 { "" } else { "s" },
                        group.outputs,
                        if group.outputs == 1 { "" } else { "s" }
                    ),
                );
                ui.horizontal(|ui| {
                    open = ui.button("Open group").clicked();
                    unpack = ui.button("Ungroup").clicked();
                });
            }
            NodeKind::GroupInput { index } => {
                note(
                    ui,
                    &format!(
                        "Whatever is wired into input {} of the group outside.",
                        *index + 1
                    ),
                );
            }
            _ => {}
        }
        if kind != before {
            // One undo step per gesture: checkpoint when the change starts.
            let held = ui.ctx().input(|i| i.pointer.any_down());
            let undo = !held || !self.graph_editor.editing_setting;
            self.graph_editor.editing_setting = held;
            let _ = self.graph_try_with(undo, |nodes, _, _| {
                if let Some(node) = nodes.iter_mut().find(|n| n.id == selected) {
                    node.kind = kind;
                }
                Ok(())
            });
        } else if !ui.ctx().input(|i| i.pointer.any_down()) {
            self.graph_editor.editing_setting = false;
        }
        if open {
            self.open_group(selected);
        } else if unpack {
            self.ungroup_selected(selected);
        }
    }

    fn graph_try_with<R>(
        &mut self,
        undo: bool,
        edit: impl FnOnce(&mut Vec<Node>, &mut Vec<Link>, &mut Vec<NodePosition>) -> Result<R, String>,
    ) -> Option<R> {
        match self.graph_edit(undo, edit) {
            Ok(value) => Some(value),
            Err(error) => {
                self.status = error;
                None
            }
        }
    }
}

/// A setting's name above its slider, so it fits the narrow strip.
fn labeled(ui: &mut egui::Ui, label: &str, slider: egui::Slider) -> egui::Response {
    ui.label(label);
    ui.add(slider)
}

/// Small wrapped help text.
fn note(ui: &mut egui::Ui, text: &str) {
    ui.add(egui::Label::new(egui::RichText::new(text).small()).wrap());
}

/// A short summary of a node's setting, shown on the node.
fn setting_label(kind: &NodeKind) -> Option<String> {
    match kind {
        NodeKind::Pressure { shaped: true } => Some("Brush curve".into()),
        NodeKind::Pressure { shaped: false } => Some("Raw".into()),
        NodeKind::Distance { length } => Some(format!("{length:.0} px")),
        NodeKind::Random { seed } => Some(format!("Seed {seed}")),
        NodeKind::Constant { value } => Some(format!("{value:.2}")),
        NodeKind::HueOutput { degrees } | NodeKind::RotationOutput { degrees } => {
            Some(format!("±{degrees:.0}°"))
        }
        NodeKind::Wave { period, shape } => Some(format!("{} · {period:.0} px", shape.name())),
        NodeKind::Noise { scale, .. } => Some(format!("{scale:.0} px bumps")),
        NodeKind::Steps { count } => Some(format!("{count} levels")),
        NodeKind::Formula { expression } => {
            let short: String = expression.chars().take(18).collect();
            Some(if expression.chars().count() > 18 {
                format!("{short}…")
            } else {
                short
            })
        }
        NodeKind::Group(group) => Some(format!(
            "{} nodes · double-click to open",
            group.nodes.len()
        )),
        _ => None,
    }
}

/// The grouped list of nodes. Outputs the main graph already has are
/// disabled; inside a group, brush outputs are not offered.
/// Nodes whose name or description contains every word of `query`. With
/// `wired`, only nodes that take an input. Outputs the level already has,
/// and brush outputs inside a group, are left out.
fn matching_kinds(
    query: &str,
    nodes: Option<&[Node]>,
    inside_group: bool,
    wired: bool,
) -> Vec<NodeKind> {
    let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    NodeKind::catalog()
        .into_iter()
        .filter(|(group, _)| !(inside_group && *group == "Outputs"))
        .flat_map(|(_, kinds)| kinds)
        .filter(|kind| !wired || kind.inputs() > 0)
        .filter(|kind| {
            !kind.output().is_some_and(|output| {
                nodes.is_some_and(|n| n.iter().any(|n| n.kind.output() == Some(output)))
            })
        })
        .filter(|kind| {
            let text = format!("{} {}", kind.name(), kind.description()).to_lowercase();
            words.iter().all(|word| text.contains(word.as_str()))
        })
        .collect()
}

/// A flat list of node kinds; Enter (`pick_first`) takes the first.
fn node_list_ui(ui: &mut egui::Ui, kinds: &[NodeKind], pick_first: bool) -> Option<NodeKind> {
    if kinds.is_empty() {
        ui.weak("No node matches");
        return None;
    }
    if pick_first {
        return kinds.first().cloned();
    }
    let mut chosen = None;
    for kind in kinds {
        if ui
            .button(kind.name())
            .on_hover_text(kind.description())
            .clicked()
        {
            chosen = Some(kind.clone());
        }
    }
    chosen
}

fn node_catalog_ui(
    ui: &mut egui::Ui,
    nodes: Option<&[Node]>,
    inside_group: bool,
) -> Option<NodeKind> {
    let mut chosen = None;
    for (group, kinds) in NodeKind::catalog() {
        if inside_group && group == "Outputs" {
            continue;
        }
        ui.menu_button(group, |ui| {
            for kind in kinds {
                let taken = kind.output().is_some_and(|output| {
                    nodes.is_some_and(|n| n.iter().any(|n| n.kind.output() == Some(output)))
                });
                if ui
                    .add_enabled(!taken, egui::Button::new(kind.name()))
                    .on_hover_text(kind.description())
                    .on_disabled_hover_text("The graph already has this output")
                    .clicked()
                {
                    chosen = Some(kind);
                }
            }
        });
    }
    chosen
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::brush_graph::Output;

    #[test]
    fn edits_keep_the_graph_valid() {
        let mut graph = BrushGraph::new(vec![], vec![]);
        let mut layout = vec![];
        let speed = add_node(&mut graph, &mut layout, NodeKind::Speed, [0.0, 0.0]).unwrap();
        let invert = add_node(&mut graph, &mut layout, NodeKind::Invert, [150.0, 0.0]).unwrap();
        let size = add_node(&mut graph, &mut layout, NodeKind::SizeOutput, [300.0, 0.0]).unwrap();
        assert!(add_node(&mut graph, &mut layout, NodeKind::SizeOutput, [0.0, 0.0]).is_err());
        connect(&mut graph, speed, invert, 0).unwrap();
        connect(&mut graph, invert, size, 0).unwrap();
        assert!(graph.drives(Output::Size));
        // A loop is refused and leaves the graph as it was.
        let second = add_node(&mut graph, &mut layout, NodeKind::Invert, [0.0, 100.0]).unwrap();
        connect(&mut graph, second, invert, 0).unwrap();
        let before = graph.clone();
        assert!(
            connect(&mut graph, invert, second, 0)
                .unwrap_err()
                .contains("loop")
        );
        assert!(graph == before);
        // Wiring into a used input replaces its wire.
        connect(&mut graph, speed, invert, 0).unwrap();
        assert_eq!(graph.links().iter().filter(|l| l.to == invert).count(), 1);
        // Picking a wire up and removing a node drop their links.
        assert_eq!(disconnect(&mut graph, size, 0), Some(invert));
        assert!(!graph.drives(Output::Size));
        remove_node(&mut graph, &mut layout, speed);
        assert!(graph.links().iter().all(|l| l.from != speed));
        assert!(layout.iter().all(|p| p.id != speed));
        graph.validate().unwrap();
    }

    #[test]
    fn auto_layout_places_wired_nodes_left_to_right() {
        let graph = BrushGraph::from_brush(&Brush::default());
        let layout = crate::brush_graph::auto_layout(&graph);
        let x = |id: NodeId| layout.iter().find(|p| p.id == id).unwrap().x;
        for link in graph.links() {
            assert!(x(link.from) < x(link.to));
        }
    }

    /// The default brush's graph: Pressure, Constant, Maximum → Size.
    fn pressure_graph() -> (BrushGraph, Vec<NodePosition>) {
        let graph = BrushGraph::from_brush(&Brush::default());
        let layout = crate::brush_graph::auto_layout(&graph);
        (graph, layout)
    }
    fn ids(graph: &BrushGraph, pick: fn(&NodeKind) -> bool) -> Vec<NodeId> {
        graph
            .nodes()
            .iter()
            .filter(|n| pick(&n.kind))
            .map(|n| n.id)
            .collect()
    }

    #[test]
    fn grouping_and_ungrouping_keep_what_the_brush_paints() {
        let (mut graph, mut layout) = pressure_graph();
        let evaluate = |g: &BrushGraph, pressure: f32| {
            g.compiled()
                .unwrap()
                .evaluate(&crate::brush_graph::Inputs {
                    pressure,
                    shaped_pressure: pressure,
                    speed: None,
                    tilt: None,
                    twist: None,
                    random: None,
                    distance: 0.0,
                    heading: None,
                })
                .size
        };
        let before: Vec<_> = [0.0, 0.02, 0.4, 1.0].map(|p| evaluate(&graph, p)).into();
        // Group the floor and the maximum: pressure comes in, size goes out.
        let picked = ids(&graph, |k| {
            matches!(k, NodeKind::Constant { .. } | NodeKind::Maximum)
        });
        let (nodes, links) = graph.edit();
        let group = group_nodes(nodes, links, &mut layout, &picked).unwrap();
        graph.validate().unwrap();
        let NodeKind::Group(inner) = &graph.nodes().iter().find(|n| n.id == group).unwrap().kind
        else {
            panic!("a group node");
        };
        assert_eq!(inner.inputs, 1);
        assert_eq!(graph.nodes().len(), 3, "pressure, group, size");
        let after: Vec<_> = [0.0, 0.02, 0.4, 1.0].map(|p| evaluate(&graph, p)).into();
        assert_eq!(before, after);
        // Ungrouping puts the same nodes and wiring back.
        let (nodes, links) = graph.edit();
        let held = ungroup(nodes, links, &mut layout, group).unwrap();
        assert_eq!(held.len(), 2);
        graph.validate().unwrap();
        let unpacked: Vec<_> = [0.0, 0.02, 0.4, 1.0].map(|p| evaluate(&graph, p)).into();
        assert_eq!(before, unpacked);
        assert_eq!(graph.nodes().len(), 4);
    }

    #[test]
    fn values_show_at_every_level_of_a_graph() {
        let (mut graph, mut layout) = pressure_graph();
        let picked = ids(&graph, |k| {
            matches!(k, NodeKind::Constant { .. } | NodeKind::Maximum)
        });
        let (nodes, links) = graph.edit();
        let group = group_nodes(nodes, links, &mut layout, &picked).unwrap();
        let inputs = crate::brush_graph::Inputs {
            pressure: 0.6,
            shaped_pressure: 0.6,
            speed: None,
            tilt: None,
            twist: None,
            random: None,
            distance: 0.0,
            heading: None,
        };
        let compiled = graph.compiled().unwrap();
        let value = |path: &[NodeId], node: NodeId| {
            compiled
                .trace(&inputs, path)
                .into_iter()
                .find(|(id, port, _)| *id == node && *port == 0)
                .map(|(_, _, v)| v)
        };
        // The main graph: the group gives the larger of pressure and floor.
        let size = ids(&graph, |k| *k == NodeKind::SizeOutput)[0];
        assert_eq!(value(&[], group), Some(0.6));
        assert_eq!(value(&[], size), Some(0.6));
        // Inside the group: what comes in, each node, and what goes out.
        let NodeKind::Group(inner) = &graph.nodes().iter().find(|n| n.id == group).unwrap().kind
        else {
            panic!("a group node");
        };
        let find =
            |pick: fn(&NodeKind) -> bool| inner.nodes.iter().find(|n| pick(&n.kind)).unwrap().id;
        let input = find(|k| matches!(k, NodeKind::GroupInput { .. }));
        let constant = find(|k| matches!(k, NodeKind::Constant { .. }));
        let maximum = find(|k| *k == NodeKind::Maximum);
        let result = find(|k| matches!(k, NodeKind::GroupResult { .. }));
        assert_eq!(value(&[group], input), Some(0.6));
        assert!(value(&[group], constant).is_some_and(|v| v < 0.6));
        assert_eq!(value(&[group], maximum), Some(0.6));
        assert_eq!(value(&[group], result), Some(0.6));
        // The levels do not mix.
        assert_eq!(value(&[], maximum), None);
    }

    #[test]
    fn groups_refuse_what_they_cannot_hold() {
        let (mut graph, mut layout) = pressure_graph();
        let size = ids(&graph, |k| *k == NodeKind::SizeOutput);
        let (nodes, links) = graph.edit();
        assert!(
            group_nodes(nodes, links, &mut layout, &size)
                .unwrap_err()
                .contains("outside")
        );
        // Nine separate values out of one group are too many.
        let mut graph = BrushGraph::new(vec![], vec![]);
        let mut layout = vec![];
        let picked: Vec<NodeId> = (0..9)
            .map(|_| add_node(&mut graph, &mut layout, NodeKind::Speed, [0.0; 2]).unwrap())
            .collect();
        let (nodes, links) = graph.edit();
        assert!(
            group_nodes(nodes, links, &mut layout, &picked)
                .unwrap_err()
                .contains("at most 8")
        );
    }

    #[test]
    fn groups_give_several_values_and_old_wires_still_load() {
        let mut graph = BrushGraph::new(vec![], vec![]);
        let mut layout = vec![];
        let a = add_node(
            &mut graph,
            &mut layout,
            NodeKind::Pressure { shaped: false },
            [0.0; 2],
        )
        .unwrap();
        let b = add_node(&mut graph, &mut layout, NodeKind::Invert, [0.0; 2]).unwrap();
        let size = add_node(&mut graph, &mut layout, NodeKind::SizeOutput, [0.0; 2]).unwrap();
        let flow = add_node(&mut graph, &mut layout, NodeKind::FlowOutput, [0.0; 2]).unwrap();
        connect(&mut graph, a, b, 0).unwrap();
        connect(&mut graph, a, size, 0).unwrap();
        connect(&mut graph, b, flow, 0).unwrap();
        let values = |g: &BrushGraph| {
            let v = g.compiled().unwrap().evaluate(&crate::brush_graph::Inputs {
                pressure: 0.3,
                shaped_pressure: 0.3,
                speed: None,
                tilt: None,
                twist: None,
                random: None,
                distance: 0.0,
                heading: None,
            });
            (v.size, v.flow)
        };
        let before = values(&graph);
        assert_eq!(before, (Some(0.3), Some(0.7)));
        // Both nodes into one group: two outputs, one wired to each setting.
        let (nodes, links) = graph.edit();
        let group = group_nodes(nodes, links, &mut layout, &[a, b]).unwrap();
        graph.validate().unwrap();
        let kind = &graph.nodes().iter().find(|n| n.id == group).unwrap().kind;
        assert_eq!((kind.inputs(), kind.outputs()), (0, 2));
        assert_eq!(kind.output_names(), ["Out 1", "Out 2"]);
        assert_eq!(values(&graph), before);
        assert!(
            graph
                .links()
                .iter()
                .any(|l| l.from == group && l.from_port == 1)
        );
        // Saved and loaded, then ungrouped: still the same.
        let json = serde_json::to_string(&graph).unwrap();
        let mut back: BrushGraph = serde_json::from_str(&json).unwrap();
        assert_eq!(values(&back), before);
        let (nodes, links) = back.edit();
        ungroup(nodes, links, &mut layout, group).unwrap();
        back.validate().unwrap();
        assert_eq!(values(&back), before);
        // Wires saved before outputs were numbered load as output 1.
        let old = r#"{"version":1,"nodes":[{"id":1,"kind":"Speed"},{"id":2,"kind":"SizeOutput"}],"links":[{"from":1,"to":2,"port":0}]}"#;
        let loaded: BrushGraph = serde_json::from_str(old).unwrap();
        loaded.validate().unwrap();
        assert_eq!(loaded.links()[0].from_port, 0);
        // A wire from an output a node does not have is refused.
        let bad = old.replace(r#""port":0}"#, r#""port":0,"from_port":1}"#);
        let bad: BrushGraph = serde_json::from_str(&bad).unwrap();
        assert!(bad.validate().unwrap_err().contains("no output 2"));
    }

    #[test]
    fn nested_groups_formulas_and_the_node_limit() {
        use NodeKind::*;
        // A formula inside a group inside a group, feeding Size.
        let formula = Formula {
            expression: "a * 0.5 + 0.25".into(),
        };
        let inner = crate::brush_graph::Group {
            name: "Half".into(),
            inputs: 1,
            outputs: 1,
            nodes: vec![
                Node {
                    id: NodeId(1),
                    kind: GroupInput { index: 0 },
                },
                Node {
                    id: NodeId(2),
                    kind: formula,
                },
                Node {
                    id: NodeId(3),
                    kind: GroupResult { index: 0 },
                },
            ],
            links: vec![
                Link {
                    from: NodeId(1),
                    to: NodeId(2),
                    port: 0,
                    from_port: 0,
                },
                Link {
                    from: NodeId(2),
                    to: NodeId(3),
                    port: 0,
                    from_port: 0,
                },
            ],
            layout: vec![],
        };
        let outer = crate::brush_graph::Group {
            name: "Wrapper".into(),
            inputs: 1,
            outputs: 1,
            nodes: vec![
                Node {
                    id: NodeId(1),
                    kind: GroupInput { index: 0 },
                },
                Node {
                    id: NodeId(2),
                    kind: Group(Box::new(inner)),
                },
                Node {
                    id: NodeId(3),
                    kind: GroupResult { index: 0 },
                },
            ],
            links: vec![
                Link {
                    from: NodeId(1),
                    to: NodeId(2),
                    port: 0,
                    from_port: 0,
                },
                Link {
                    from: NodeId(2),
                    to: NodeId(3),
                    port: 0,
                    from_port: 0,
                },
            ],
            layout: vec![],
        };
        let graph = BrushGraph::new(
            vec![
                Node {
                    id: NodeId(1),
                    kind: Pressure { shaped: false },
                },
                Node {
                    id: NodeId(2),
                    kind: Group(Box::new(outer)),
                },
                Node {
                    id: NodeId(3),
                    kind: SizeOutput,
                },
            ],
            vec![
                Link {
                    from: NodeId(1),
                    to: NodeId(2),
                    port: 0,
                    from_port: 0,
                },
                Link {
                    from: NodeId(2),
                    to: NodeId(3),
                    port: 0,
                    from_port: 0,
                },
            ],
        );
        graph.validate().unwrap();
        let size = graph
            .compiled()
            .unwrap()
            .evaluate(&crate::brush_graph::Inputs {
                pressure: 0.5,
                shaped_pressure: 0.5,
                speed: None,
                tilt: None,
                twist: None,
                random: None,
                distance: 0.0,
                heading: None,
            })
            .size;
        assert_eq!(size, Some(0.5));
        // Survives saving and loading.
        let json = serde_json::to_string(&graph).unwrap();
        let back: BrushGraph = serde_json::from_str(&json).unwrap();
        back.validate().unwrap();
        // Groups count toward the 128-node limit once opened: 120 nodes
        // plus a group of 9 formulas is 129.
        let chain = |count: u32| {
            let mut nodes = vec![Node {
                id: NodeId(1),
                kind: GroupInput { index: 0 },
            }];
            let mut links = Vec::new();
            for i in 0..count {
                nodes.push(Node {
                    id: NodeId(i + 2),
                    kind: Formula {
                        expression: "a * 0.9".into(),
                    },
                });
                links.push(Link {
                    from: NodeId(i + 1),
                    to: NodeId(i + 2),
                    port: 0,
                    from_port: 0,
                });
            }
            nodes.push(Node {
                id: NodeId(count + 2),
                kind: GroupResult { index: 0 },
            });
            links.push(Link {
                from: NodeId(count + 1),
                to: NodeId(count + 2),
                port: 0,
                from_port: 0,
            });
            Group(Box::new(crate::brush_graph::Group {
                name: "Chain".into(),
                inputs: 1,
                outputs: 1,
                nodes,
                links,
                layout: vec![],
            }))
        };
        let crowd = |extra: u32| {
            BrushGraph::new(
                (1..=120)
                    .map(|i| Node {
                        id: NodeId(i),
                        kind: Speed,
                    })
                    .chain(std::iter::once(Node {
                        id: NodeId(500),
                        kind: chain(extra),
                    }))
                    .collect(),
                vec![],
            )
        };
        crowd(8).validate().unwrap();
        assert!(crowd(9).validate().unwrap_err().contains("inside groups"));
        // A group may not hold a brush output, nor appear at the top.
        let bad = BrushGraph::new(
            vec![Node {
                id: NodeId(1),
                kind: GroupResult { index: 0 },
            }],
            vec![],
        );
        assert!(bad.validate().unwrap_err().contains("inside a group"));
    }

    #[test]
    fn the_library_lists_each_group_once() {
        let (mut graph, mut layout) = pressure_graph();
        let picked = ids(&graph, |k| {
            matches!(k, NodeKind::Constant { .. } | NodeKind::Maximum)
        });
        let (nodes, links) = graph.edit();
        group_nodes(nodes, links, &mut layout, &picked).unwrap();
        let brush = Brush {
            graph: Some(graph),
            ..Default::default()
        };
        let copy = brush.clone();
        let groups = library_groups(&[&brush, &copy, &Brush::default()]);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].name, "Group");
    }
}
