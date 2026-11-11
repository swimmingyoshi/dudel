//! The brush graph editor in Brush Studio: a node canvas drawn with egui's
//! painter. Drag a node to move it, drag from an output dot to an input dot
//! to wire them, drag empty space to pan and use the wheel to zoom; right-click
//! adds or removes nodes. Every edit is checked before it is made, so the
//! graph the brush paints with is always valid. The editor keeps its own undo
//! steps, separate from the document's.
use crate::{
    Dudel,
    brush::Brush,
    brush_graph::{BrushGraph, Link, Node, NodeId, NodeKind, NodePosition, Output},
    egui,
};

const NODE_WIDTH: f32 = 132.0;
const HEADER: f32 = 22.0;
const ROW: f32 = 20.0;
const PORT: f32 = 5.0;
const UNDO_LIMIT: usize = 100;

type Snapshot = (Option<BrushGraph>, Vec<NodePosition>);

enum Drag {
    Move { node: NodeId, grab: egui::Vec2 },
    Wire { from: NodeId },
    Pan,
}

/// What a point on the canvas is over.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Hit {
    Output(NodeId),
    Input(NodeId, u8),
    Node(NodeId),
    Empty,
}

pub(crate) struct GraphEditor {
    pan: egui::Vec2,
    zoom: f32,
    selected: Option<NodeId>,
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
}

impl Default for GraphEditor {
    fn default() -> Self {
        Self {
            pan: egui::Vec2::ZERO,
            zoom: 1.0,
            selected: None,
            drag: None,
            undo: Vec::new(),
            redo: Vec::new(),
            owner: None,
            context: None,
            canvas: egui::Rect::ZERO,
            fit: false,
            editing_setting: false,
        }
    }
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
    /// Screen position of a node's output dot or of input `port`.
    pub(crate) fn port_pos(
        &self,
        brush: &Brush,
        node: NodeId,
        input: Option<u8>,
    ) -> Option<egui::Pos2> {
        let at = position(brush, node)?;
        let zoom = self.zoom();
        let top_left = self.to_screen(at);
        Some(match input {
            None => top_left + egui::vec2(NODE_WIDTH, HEADER + ROW / 2.0) * zoom,
            Some(port) => top_left + egui::vec2(0.0, HEADER + (f32::from(port) + 0.5) * ROW) * zoom,
        })
    }
    fn node_rect(&self, brush: &Brush, node: &Node) -> egui::Rect {
        let at = position(brush, node.id).unwrap_or_default();
        egui::Rect::from_min_size(self.to_screen(at), node_size(&node.kind) * self.zoom())
    }
    fn hit(&self, brush: &Brush, pointer: egui::Pos2) -> Hit {
        let Some(graph) = &brush.graph else {
            return Hit::Empty;
        };
        let reach = (PORT + 4.0) * self.zoom().max(1.0);
        // Topmost (last drawn) first.
        for node in graph.nodes().iter().rev() {
            if node.kind.output().is_none()
                && self
                    .port_pos(brush, node.id, None)
                    .is_some_and(|p| p.distance(pointer) <= reach)
            {
                return Hit::Output(node.id);
            }
            for port in 0..node.kind.inputs() {
                if self
                    .port_pos(brush, node.id, Some(port))
                    .is_some_and(|p| p.distance(pointer) <= reach)
                {
                    return Hit::Input(node.id, port);
                }
            }
            if self.node_rect(brush, node).contains(pointer) {
                return Hit::Node(node.id);
            }
        }
        Hit::Empty
    }
}

fn node_size(kind: &NodeKind) -> egui::Vec2 {
    // Port rows, then a footer for the node's setting or problem.
    let rows = kind.inputs().max(1) as f32;
    egui::vec2(NODE_WIDTH, HEADER + (rows + 1.0) * ROW)
}

fn position(brush: &Brush, node: NodeId) -> Option<[f32; 2]> {
    brush
        .graph
        .as_ref()?
        .nodes()
        .iter()
        .any(|n| n.id == node)
        .then(|| {
            brush
                .graph_layout
                .iter()
                .find(|p| p.id == node)
                .map_or([0.0, 0.0], |p| [p.x, p.y])
        })
}

fn set_position(layout: &mut Vec<NodePosition>, id: NodeId, [x, y]: [f32; 2]) {
    match layout.iter_mut().find(|p| p.id == id) {
        Some(p) => (p.x, p.y) = (x, y),
        None => layout.push(NodePosition { id, x, y }),
    }
}

/// Add a node of `kind` at `at`. Refused when it would be a second output of
/// the same kind or the graph is full.
pub(crate) fn add_node(
    graph: &mut BrushGraph,
    layout: &mut Vec<NodePosition>,
    kind: NodeKind,
    at: [f32; 2],
) -> Result<NodeId, String> {
    if let Some(output) = kind.output()
        && graph
            .nodes()
            .iter()
            .any(|n| n.kind.output() == Some(output))
    {
        return Err(format!("The graph already has a {} output", kind.name()));
    }
    if graph.nodes().len() >= 64 {
        return Err("Brush graphs hold at most 64 nodes".into());
    }
    let id = NodeId(graph.nodes().iter().map(|n| n.id.0).max().unwrap_or(0) + 1);
    graph.edit().0.push(Node { id, kind });
    set_position(layout, id, at);
    Ok(id)
}

/// Wire `from`'s output into input `port` of `to`, replacing what was wired
/// there. The graph is unchanged when the result would be invalid.
pub(crate) fn connect(
    graph: &mut BrushGraph,
    from: NodeId,
    to: NodeId,
    port: u8,
) -> Result<(), String> {
    let mut candidate = graph.clone();
    let (_, links) = candidate.edit();
    links.retain(|l| !(l.to == to && l.port == port));
    links.push(Link { from, to, port });
    candidate.validate().map_err(|error| {
        if error.contains("loop") {
            "That wire would make a loop".to_owned()
        } else {
            error
        }
    })?;
    *graph = candidate;
    Ok(())
}

/// Remove the wire into input `port` of `to`, returning where it came from.
pub(crate) fn disconnect(graph: &mut BrushGraph, to: NodeId, port: u8) -> Option<NodeId> {
    let link = graph
        .links()
        .iter()
        .find(|l| l.to == to && l.port == port)
        .copied()?;
    graph.edit().1.retain(|l| *l != link);
    Some(link.from)
}

/// Remove a node and every wire touching it.
pub(crate) fn remove_node(graph: &mut BrushGraph, layout: &mut Vec<NodePosition>, id: NodeId) {
    let (nodes, links) = graph.edit();
    nodes.retain(|n| n.id != id);
    links.retain(|l| l.from != id && l.to != id);
    layout.retain(|p| p.id != id);
}

impl Dudel {
    #[cfg(test)]
    pub(crate) fn graph_editor_selected(&self) -> Option<NodeId> {
        self.graph_editor.selected
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
        }
    }
    fn graph_redo(&mut self) {
        if let Some((graph, layout)) = self.graph_editor.redo.pop() {
            let current = (
                std::mem::replace(&mut self.brush.graph, graph),
                std::mem::replace(&mut self.brush.graph_layout, layout),
            );
            self.graph_editor.undo.push(current);
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
        self.graph_editor.selected = Some(curve);
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
        self.graph_editor.selected = Some(hue);
        self.graph_editor.fit = true;
        let (color, secondary) = (self.color, self.secondary);
        self.scratchpad
            .demo(&self.brush, color, secondary, [900, 160]);
    }
    /// Make a graph from the brush's pressure and dynamics settings.
    pub(crate) fn make_brush_graph(&mut self) {
        self.graph_checkpoint();
        let graph = BrushGraph::from_brush(&self.brush);
        self.brush.graph_layout = crate::brush_graph::auto_layout(&graph);
        self.brush.graph = Some(graph);
        self.graph_editor.fit = true;
        self.status =
            "Brush graph made from the current settings; the brush paints the same".into();
    }
    fn add_graph_node(&mut self, kind: NodeKind, at: [f32; 2]) {
        let mut graph = self
            .brush
            .graph
            .clone()
            .unwrap_or_else(|| BrushGraph::new(vec![], vec![]));
        let mut layout = self.brush.graph_layout.clone();
        match add_node(&mut graph, &mut layout, kind, at) {
            Ok(id) => {
                self.graph_checkpoint();
                self.brush.graph = Some(graph);
                self.brush.graph_layout = layout;
                self.graph_editor.selected = Some(id);
            }
            Err(error) => self.status = error,
        }
    }
    fn remove_graph_node(&mut self, id: NodeId) {
        self.graph_checkpoint();
        if let Some(graph) = &mut self.brush.graph {
            remove_node(graph, &mut self.brush.graph_layout, id);
        }
        if self.graph_editor.selected == Some(id) {
            self.graph_editor.selected = None;
        }
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
            ui.label(
                "A brush graph computes size, opacity, flow and spacing for every dab \
                 from pen input, with nodes you wire together. Start from this brush's \
                 pressure and dynamics settings, or from an empty graph.",
            );
            ui.horizontal_wrapped(|ui| {
                if ui
                    .button("Make graph from current settings")
                    .on_hover_text("The brush paints exactly as before; edit from there")
                    .clicked()
                {
                    self.make_brush_graph();
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
        self.graph_toolbar(ui);
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
                        });
                });
            });
        } else {
            self.graph_canvas(ui, egui::vec2(ui.available_width(), height));
            ui.add_space(4.0);
            self.graph_node_settings(ui);
        }
    }

    fn graph_canvas(&mut self, ui: &mut egui::Ui, size: egui::Vec2) {
        let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click_and_drag());
        self.graph_editor.canvas = rect;
        if std::mem::take(&mut self.graph_editor.fit) {
            self.fit_graph_view();
        }
        self.graph_canvas_input(ui, &response);
        self.paint_graph(ui, rect, &response);
        self.graph_context_menu(&response);
    }

    fn graph_toolbar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            let center = self
                .graph_editor
                .to_graph(self.graph_editor.canvas.center());
            ui.menu_button("Add node", |ui| {
                if let Some(kind) = node_catalog_ui(ui, self.brush.graph.as_ref()) {
                    self.add_graph_node(kind, center);
                    ui.close();
                }
            });
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
            if let Some(selected) = self.graph_editor.selected
                && ui.button("Delete node").clicked()
            {
                self.remove_graph_node(selected);
            }
            if ui.button("Fit").on_hover_text("Show every node").clicked() {
                self.fit_graph_view();
            }
            if ui
                .button("Remove graph")
                .on_hover_text("Paint with the plain pressure and dynamics settings again")
                .clicked()
            {
                self.graph_checkpoint();
                self.brush.graph = None;
                self.brush.graph_layout.clear();
                self.graph_editor.selected = None;
            }
        });
    }

    fn fit_graph_view(&mut self) {
        let editor = &mut self.graph_editor;
        let Some(graph) = &self.brush.graph else {
            return;
        };
        let mut bounds = egui::Rect::NOTHING;
        for node in graph.nodes() {
            let at = position(&self.brush, node.id).unwrap_or_default();
            bounds = bounds.union(egui::Rect::from_min_size(
                egui::pos2(at[0], at[1]),
                node_size(&node.kind),
            ));
        }
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
        if response.drag_started_by(egui::PointerButton::Middle) {
            self.graph_editor.drag = Some(Drag::Pan);
        } else if response.drag_started_by(egui::PointerButton::Primary)
            && let Some(origin) = ui.input(|i| i.pointer.press_origin())
        {
            self.graph_editor.drag = Some(match self.graph_editor.hit(&self.brush, origin) {
                Hit::Output(node) => Drag::Wire { from: node },
                Hit::Input(node, port) => {
                    // Pick up an existing wire to move it elsewhere.
                    let mut graph = self.brush.graph.clone().unwrap();
                    if let Some(from) = disconnect(&mut graph, node, port) {
                        self.graph_checkpoint();
                        self.brush.graph = Some(graph);
                        Drag::Wire { from }
                    } else {
                        Drag::Pan
                    }
                }
                Hit::Node(node) => {
                    self.graph_checkpoint();
                    self.graph_editor.selected = Some(node);
                    let at = self
                        .graph_editor
                        .to_screen(position(&self.brush, node).unwrap());
                    Drag::Move {
                        node,
                        grab: origin - at,
                    }
                }
                Hit::Empty => Drag::Pan,
            });
        }
        if response.dragged() {
            let delta = response.drag_delta();
            let pointer = response.interact_pointer_pos();
            match &self.graph_editor.drag {
                Some(Drag::Pan) => self.graph_editor.pan += delta,
                Some(Drag::Move { node, grab }) => {
                    if let Some(pointer) = pointer {
                        let at = self.graph_editor.to_graph(pointer - *grab);
                        set_position(&mut self.brush.graph_layout, *node, at);
                    }
                }
                Some(Drag::Wire { .. }) | None => {}
            }
        }
        if response.drag_stopped() {
            if let Some(Drag::Wire { from }) = self.graph_editor.drag.take()
                && let Some(pointer) = response
                    .interact_pointer_pos()
                    .or(ui.input(|i| i.pointer.latest_pos()))
                && let Hit::Input(to, port) = self.graph_editor.hit(&self.brush, pointer)
            {
                let mut graph = self.brush.graph.clone().unwrap();
                match connect(&mut graph, from, to, port) {
                    Ok(()) => {
                        self.graph_checkpoint();
                        self.brush.graph = Some(graph);
                    }
                    Err(error) => self.status = error,
                }
            }
            self.graph_editor.drag = None;
        }
        if response.clicked()
            && let Some(pointer) = response.interact_pointer_pos()
        {
            self.graph_editor.selected = match self.graph_editor.hit(&self.brush, pointer) {
                Hit::Node(node) | Hit::Output(node) | Hit::Input(node, _) => Some(node),
                Hit::Empty => None,
            };
        }
        if response.secondary_clicked()
            && let Some(pointer) = response.interact_pointer_pos()
        {
            let hit = self.graph_editor.hit(&self.brush, pointer);
            if let Hit::Node(node) = hit {
                self.graph_editor.selected = Some(node);
            }
            self.graph_editor.context = Some((self.graph_editor.to_graph(pointer), hit));
        }
    }

    fn graph_context_menu(&mut self, response: &egui::Response) {
        response.context_menu(|ui| {
            let Some((at, hit)) = self.graph_editor.context else {
                return;
            };
            if let Hit::Node(node) = hit {
                if ui.button("Delete node").clicked() {
                    self.remove_graph_node(node);
                    ui.close();
                }
                ui.separator();
            }
            ui.label("Add node");
            if let Some(kind) = node_catalog_ui(ui, self.brush.graph.as_ref()) {
                self.add_graph_node(kind, at);
                ui.close();
            }
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
        let Some(graph) = &self.brush.graph else {
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
        for link in graph.links() {
            if let (Some(a), Some(b)) = (
                editor.port_pos(&self.brush, link.from, None),
                editor.port_pos(&self.brush, link.to, Some(link.port)),
            ) {
                curve(a, b, wire);
            }
        }
        if let Some(Drag::Wire { from }) = &editor.drag
            && let (Some(a), Some(b)) = (
                editor.port_pos(&self.brush, *from, None),
                response.interact_pointer_pos(),
            )
        {
            curve(
                a,
                b,
                egui::Stroke::new(wire.width, visuals.selection.stroke.color),
            );
        }
        let font = egui::FontId::proportional(12.0 * zoom);
        let small = egui::FontId::proportional(11.0 * zoom);
        let connected =
            |id: NodeId, port: u8| graph.links().iter().any(|l| l.to == id && l.port == port);
        for node in graph.nodes() {
            let r = editor.node_rect(&self.brush, node);
            let selected = editor.selected == Some(node.id);
            let is_output = node.kind.output().is_some();
            painter.rect(
                r,
                6.0 * zoom,
                visuals.window_fill,
                if selected {
                    egui::Stroke::new(2.0_f32, visuals.selection.stroke.color)
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
                } else {
                    visuals.widgets.inactive.bg_fill
                },
            );
            painter.text(
                header.left_center() + egui::vec2(8.0 * zoom, 0.0),
                egui::Align2::LEFT_CENTER,
                node.kind.name(),
                font.clone(),
                visuals.strong_text_color(),
            );
            for (port, name) in node.kind.port_names().iter().enumerate() {
                let port = port as u8;
                if let Some(p) = editor.port_pos(&self.brush, node.id, Some(port)) {
                    let linked = connected(node.id, port);
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
            if !is_output && let Some(p) = editor.port_pos(&self.brush, node.id, None) {
                painter.circle(p, PORT * zoom, visuals.text_color(), egui::Stroke::NONE);
            }
            // A plain-words problem or setting in the node's footer.
            let note = if is_output && !connected(node.id, 0) {
                Some(("Not connected".to_owned(), visuals.warn_fg_color))
            } else if !is_output
                && node.kind.inputs() > 0
                && (0..node.kind.inputs()).all(|port| !connected(node.id, port))
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
        let Some(selected) = self.graph_editor.selected else {
            note(
                ui,
                "Select a node to change its settings. Right-click the canvas to add nodes.",
            );
            return;
        };
        let Some(graph) = &self.brush.graph else {
            return;
        };
        let Some(mut kind) = graph
            .nodes()
            .iter()
            .find(|n| n.id == selected)
            .map(|n| n.kind.clone())
        else {
            self.graph_editor.selected = None;
            return;
        };
        let before = kind.clone();
        ui.strong(kind.name());
        note(ui, kind.description());
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
            _ => {}
        }
        if kind != before {
            // One undo step per gesture: checkpoint when the change starts.
            if !ui.ctx().input(|i| i.pointer.any_down()) || !self.graph_editor.editing_setting {
                self.graph_checkpoint();
            }
            self.graph_editor.editing_setting = ui.ctx().input(|i| i.pointer.any_down());
            if let Some(graph) = &mut self.brush.graph
                && let Some(node) = graph.edit().0.iter_mut().find(|n| n.id == selected)
            {
                node.kind = kind;
            }
        } else if !ui.ctx().input(|i| i.pointer.any_down()) {
            self.graph_editor.editing_setting = false;
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
        NodeKind::HueOutput { degrees } => Some(format!("±{degrees:.0}°")),
        _ => None,
    }
}

/// The grouped list of nodes; outputs the graph already has are disabled.
fn node_catalog_ui(ui: &mut egui::Ui, graph: Option<&BrushGraph>) -> Option<NodeKind> {
    let mut chosen = None;
    for (group, kinds) in NodeKind::catalog() {
        ui.menu_button(group, |ui| {
            for kind in kinds {
                let taken = kind.output().is_some_and(|output: Output| {
                    graph.is_some_and(|g| g.nodes().iter().any(|n| n.kind.output() == Some(output)))
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
}
