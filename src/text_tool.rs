//! The Text tool. A click starts a line and a drag starts a box of that size,
//! which lines wrap in; a click on existing text edits it. Typing lands on the
//! canvas. An invisible text field at the caret does the typing, so IME,
//! selection, clipboard and undo while typing all come from egui, and this
//! file draws the caret and selection from its cursor.
//!
//! An editing session changes the document directly under one history
//! checkpoint, so it undoes in one step however much was typed. Anything else
//! that edits the document closes the session first, through
//! `close_input_gestures`.
use crate::{
    Dudel,
    document::{Document, LayerId},
    document_edits::Edit,
    egui,
    text_layer::{MAX_TEXT, TextLayer},
    text_render::{self, Align},
    tools::Tool,
};
use std::{
    collections::HashMap,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Duration,
};

/// On-screen pixels a press must travel to draw a box rather than click.
const BOX_DRAG: f32 = 8.0;
/// Seconds between two presses of a double-click.
const DOUBLE_CLICK: f64 = 0.4;
/// On-screen pixels around text that still count as clicking it.
const HIT_MARGIN: f64 = 6.0;
const PREVIEW_SIZE: [usize; 2] = [300, 28];
/// Font previews kept before they are all dropped and redrawn on demand.
const PREVIEW_LIMIT: usize = 256;
/// Font previews drawn per frame, so that scrolling the list stays smooth.
const PREVIEWS_PER_FRAME: usize = 2;
/// On-screen pixels around the box's resize corner that still grab it.
const HANDLE_REACH: f64 = 10.0;
/// Smallest box side a drag leaves, in canvas pixels.
const MIN_BOX: f64 = 8.0;
/// Wheel travel that moves one font along.
const FONT_WHEEL_STEP: f32 = 40.0;

/// Longest a finished session waits for its last drawing.
const DRAW_TIMEOUT: Duration = Duration::from_secs(120);

struct DrawJob {
    id: u64,
    text: TextLayer,
    canvas: [usize; 2],
}

struct Drawn {
    id: u64,
    text: TextLayer,
    result: Result<crate::pixels::Pixels, String>,
}

#[derive(Default)]
struct DrawShared {
    job: Mutex<Option<DrawJob>>,
    wake: Condvar,
    finished: Mutex<Option<Drawn>>,
    done: Condvar,
    /// The newest job's id. Anything older stops at its next glyph.
    latest: AtomicU64,
    closed: AtomicBool,
}

impl DrawShared {
    fn run(&self) {
        loop {
            let mut slot = self.job.lock().unwrap_or_else(|e| e.into_inner());
            while slot.is_none() && !self.closed.load(Ordering::SeqCst) {
                slot = self.wake.wait(slot).unwrap_or_else(|e| e.into_inner());
            }
            let job = slot.take();
            drop(slot);
            let Some(job) = job else {
                return;
            };
            let stale = || self.latest.load(Ordering::SeqCst) != job.id;
            let result = job.text.render(job.canvas, &stale);
            if !stale() {
                *self.finished.lock().unwrap_or_else(|e| e.into_inner()) = Some(Drawn {
                    id: job.id,
                    text: job.text,
                    result,
                });
                self.done.notify_all();
            }
        }
    }
}

/// Draws text away from the UI thread, because a large text on a large canvas
/// takes seconds. Typing replaces the job and stops the one running, so only
/// the newest draft is ever finished. One thread for the life of the app:
/// threads that come and go leak in the graphics driver.
#[derive(Default)]
pub(crate) struct Drawer {
    shared: Option<Arc<DrawShared>>,
}

impl Drawer {
    fn draw(&mut self, text: TextLayer, canvas: [usize; 2]) -> u64 {
        let shared = self.shared.get_or_insert_with(|| {
            let shared = Arc::new(DrawShared::default());
            let worker = shared.clone();
            let _ = std::thread::Builder::new()
                .name("text drawing".into())
                .spawn(move || worker.run());
            shared
        });
        let id = shared.latest.fetch_add(1, Ordering::SeqCst) + 1;
        *shared.job.lock().unwrap_or_else(|e| e.into_inner()) = Some(DrawJob { id, text, canvas });
        shared.wake.notify_one();
        id
    }
    /// Stop whatever is being drawn.
    fn cancel(&self) {
        if let Some(shared) = &self.shared {
            shared.latest.fetch_add(1, Ordering::SeqCst);
        }
    }
    /// Job `id`'s result, once it is finished.
    fn take(&self, id: u64, wait: bool) -> Option<Drawn> {
        let shared = self.shared.as_ref()?;
        let mut finished = shared.finished.lock().unwrap_or_else(|e| e.into_inner());
        if wait {
            finished = shared
                .done
                .wait_timeout_while(finished, DRAW_TIMEOUT, |slot| {
                    slot.as_ref().is_none_or(|drawn| drawn.id != id)
                })
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
        finished.take_if(|drawn| drawn.id == id)
    }
}

impl Drop for Drawer {
    fn drop(&mut self) {
        if let Some(shared) = &self.shared {
            shared.closed.store(true, Ordering::SeqCst);
            shared.latest.fetch_add(1, Ordering::SeqCst);
            shared.wake.notify_one();
        }
    }
}

/// What new text looks like; remembered between launches.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub(crate) struct Style {
    pub(crate) font: String,
    pub(crate) size: f64,
    pub(crate) align: Align,
    pub(crate) line_spacing: f64,
}

impl Default for Style {
    fn default() -> Self {
        Self {
            font: String::new(),
            size: 48.0,
            align: Align::Left,
            line_spacing: 1.0,
        }
    }
}

impl Style {
    fn of(text: &TextLayer) -> Self {
        Self {
            font: text.font.clone(),
            size: text.size,
            align: text.align,
            line_spacing: text.line_spacing,
        }
    }
    fn apply(&self, text: &mut TextLayer) {
        text.font.clone_from(&self.font);
        text.size = self.size;
        text.align = self.align;
        text.line_spacing = self.line_spacing;
    }
    /// Make a saved style safe to use.
    pub(crate) fn sanitize(&mut self) {
        let fallback = Self::default();
        if self.font.len() > 256 {
            self.font.clear();
        }
        if !(1.0..=8192.0).contains(&self.size) {
            self.size = fallback.size;
        }
        if !(0.5..=5.0).contains(&self.line_spacing) {
            self.line_spacing = fallback.line_spacing;
        }
    }
}

/// A press with the Text tool, in canvas pixels.
#[derive(Clone, Copy)]
pub(crate) struct Drag {
    from: [f32; 2],
    to: [f32; 2],
}

/// An open editing session.
pub(crate) struct Editing {
    target: LayerId,
    /// This session made the layer, so leaving it empty removes it.
    created: bool,
    /// The document when the session began, which Escape returns to.
    before: Document,
    modified: bool,
    draft: TextLayer,
    /// The last draft drawing was tried for, successful or not.
    attempted: TextLayer,
    /// The drawing job still running for it.
    pending: Option<u64>,
    /// The foreground color when last seen: changing it recolors the text.
    color: [u8; 3],
    focus: bool,
    error: Option<String>,
    missing: Vec<char>,
    /// The draft's length and height, when its font is installed.
    extent: Option<[f64; 2]>,
    /// Where the caret can stand in the draft.
    carets: Vec<text_render::Caret>,
    /// The field's cursor as last seen: where it is, and the selection's other end.
    cursor: [usize; 2],
    /// A cursor for the field to take, set by a press on the canvas.
    place_cursor: Option<[usize; 2]>,
    /// The field still holds an earlier session's state.
    fresh: bool,
    /// A press on the text is dragging out a selection from this character.
    selecting: Option<usize>,
    /// A press on the corner is resizing the box.
    resizing: bool,
}

impl Editing {
    pub(crate) fn source(&self) -> &Document {
        &self.before
    }
    /// Whether a canvas point is on the text.
    fn contains(&self, at: [f64; 2], margin: f64) -> bool {
        self.extent
            .is_some_and(|extent| within(&self.draft, extent, at, margin))
    }
    /// The character a canvas point is nearest to.
    fn caret_at(&self, at: [f64; 2]) -> usize {
        let [along, down] = self.draft.text_point(at);
        let score = |caret: &text_render::Caret| {
            let line = (down - down.clamp(caret.top, caret.top + caret.height)).abs();
            (line, (along - caret.along).abs())
        };
        let closest = self.carets.iter().enumerate().min_by(|a, b| {
            let (a, b) = (score(a.1), score(b.1));
            a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1))
        });
        closest.map_or(0, |(index, _)| index)
    }
    /// The canvas point in the middle of the caret before character `index`.
    #[cfg(test)]
    pub(crate) fn caret_position(&self, index: usize) -> [f32; 2] {
        let caret = self.carets[index];
        let at = self
            .draft
            .canvas_point(caret.along, caret.top + caret.height / 2.0);
        at.map(|v| v as f32)
    }
    /// The selected characters, first to last.
    #[cfg(test)]
    pub(crate) fn selected(&self) -> [usize; 2] {
        let [a, b] = self.cursor;
        [a.min(b), a.max(b)]
    }
    /// The box's resize corner, on the canvas. Lines have no box to resize.
    fn handle(&self) -> Option<[f64; 2]> {
        let extent = self.extent?;
        self.draft.wrap?;
        Some(self.draft.canvas_point(extent[0], extent[1]))
    }
    fn resize_to(&mut self, at: [f64; 2]) {
        let [width, height] = self.draft.text_point(at);
        self.draft.wrap = Some(width.max(MIN_BOX));
        self.draft.box_height = Some(height.max(MIN_BOX));
    }
    /// Fit caret and outline to the draft, right away, while drawing catches up.
    fn measure(&mut self) {
        self.extent = self.draft.extent().ok();
        self.carets = self.draft.carets().unwrap_or_default();
    }
}

fn within(text: &TextLayer, extent: [f64; 2], at: [f64; 2], margin: f64) -> bool {
    let offset = [at[0] - text.start[0], at[1] - text.start[1]];
    let along = offset[0] * text.along[0] + offset[1] * text.along[1];
    let down = offset[0] * text.down[0] + offset[1] * text.down[1];
    (-margin..=extent[0] + margin).contains(&along)
        && (-margin..=extent[1] + margin).contains(&down)
}

/// The rectangle a text fills, as canvas corners in drawing order.
fn corners(text: &TextLayer, extent: [f64; 2]) -> [[f64; 2]; 4] {
    let point = |along: f64, down: f64| {
        [
            text.start[0] + text.along[0] * along + text.down[0] * down,
            text.start[1] + text.along[1] * along + text.down[1] * down,
        ]
    };
    // An empty line still shows where it starts.
    let length = extent[0].max(text.size * 0.05);
    [
        point(0.0, 0.0),
        point(length, 0.0),
        point(length, extent[1]),
        point(0.0, extent[1]),
    ]
}

/// The canvas pixels a text can reach. Glyphs may overhang their advance, so
/// this allows one em beyond the layout on every side.
fn reach(text: &TextLayer, extent: [f64; 2], canvas: [usize; 2]) -> [usize; 4] {
    let points = corners(text, extent);
    let side = |axis: usize, pick: fn(f64, f64) -> f64| {
        points.iter().map(|p| p[axis]).reduce(pick).unwrap_or(0.0)
    };
    let clamp = |value: f64, size: usize| value.clamp(0.0, size as f64) as usize;
    [
        clamp(side(0, f64::min) - text.size, canvas[0]),
        clamp(side(1, f64::min) - text.size, canvas[1]),
        clamp(side(0, f64::max).ceil() + text.size, canvas[0]),
        clamp(side(1, f64::max).ceil() + text.size, canvas[1]),
    ]
}

/// A font's name drawn in that font, or None when it can't draw its own name.
fn preview_image(name: &str) -> Option<egui::ColorImage> {
    let font = text_render::find_font(name)?;
    if !text_render::missing_characters(name, &font).is_empty() {
        return None;
    }
    let text = TextLayer {
        string: name.into(),
        font: name.into(),
        size: 18.0,
        color: [255, 255, 255],
        align: Align::Left,
        vertical: false,
        wrap: None,
        box_height: None,
        line_spacing: 1.0,
        start: [2.0, 2.0],
        along: [1.0, 0.0],
        down: [0.0, 1.0],
        csp: None,
    };
    let pixels = text.render(PREVIEW_SIZE, &|| false).ok()?;
    let bytes: Vec<u8> = (0..PREVIEW_SIZE[0] * PREVIEW_SIZE[1])
        .flat_map(|index| pixels.rgba(index))
        .collect();
    Some(egui::ColorImage::from_rgba_unmultiplied(
        PREVIEW_SIZE,
        &bytes,
    ))
}

/// Previews of the fonts shown so far.
#[derive(Default)]
pub(crate) struct FontList {
    search: String,
    previews: HashMap<String, Option<egui::TextureHandle>>,
    /// Wheel travel over the font button not yet spent on a step.
    wheel: f32,
    /// The first row the open list showed last.
    #[cfg(test)]
    pub(crate) first_row: usize,
}

impl Dudel {
    /// Canvas input for the Text tool.
    pub(crate) fn text_packet(&mut self, at: [f32; 2], start: bool, end: bool, on_workspace: bool) {
        let point = at.map(f64::from);
        let scale = 1.0 / f64::from(self.zoom.max(1e-3));
        if start && on_workspace {
            match &mut self.text_edit {
                Some(edit)
                    if edit.handle().is_some_and(|[x, y]| {
                        (x - point[0]).hypot(y - point[1]) <= HANDLE_REACH * scale
                    }) =>
                {
                    edit.resizing = true;
                }
                // A press on the text places the caret; anywhere else finishes
                // it, and only the next press starts something new.
                Some(edit) if edit.contains(point, HIT_MARGIN * scale) => {
                    let caret = edit.caret_at(point);
                    edit.selecting = Some(caret);
                    edit.place_cursor = Some([caret, caret]);
                    edit.focus = true;
                }
                Some(_) => self.finish_text_edit(),
                None => self.text_drag = Some(Drag { from: at, to: at }),
            }
        }
        if let Some(edit) = &mut self.text_edit {
            if edit.resizing {
                edit.resize_to(point);
                edit.resizing = !end;
                edit.focus = true;
            } else if let Some(anchor) = edit.selecting {
                edit.place_cursor = Some([edit.caret_at(point), anchor]);
                edit.selecting = edit.selecting.filter(|_| !end);
            }
            return;
        }
        let Some(drag) = &mut self.text_drag else {
            return;
        };
        drag.to = at;
        if end {
            let drag = *drag;
            self.text_drag = None;
            self.place_text(drag);
        }
    }

    fn place_text(&mut self, drag: Drag) {
        // Whole pixels, so that text lands the same way each time it is drawn.
        let [from, to] = [drag.from, drag.to].map(|p| p.map(|v| f64::from(v).round()));
        let [width, height] = [self.doc.width as f64, self.doc.height as f64];
        let left = from[0].min(to[0]);
        let top = from[1].min(to[1]);
        let span = (to[0] - from[0]).abs();
        let travelled = (to[0] - from[0]).hypot(to[1] - from[1]) as f32;
        if travelled * self.zoom >= BOX_DRAG {
            let tall = (to[1] - from[1]).abs();
            self.begin_new_text(
                [left, top],
                Some(span.max(MIN_BOX)),
                Some(tall.max(MIN_BOX)),
            );
        } else if let Some(index) = self.text_at(from) {
            self.begin_text_edit(index);
            if let Some(edit) = &mut self.text_edit {
                let caret = edit.caret_at(from);
                edit.place_cursor = Some([caret, caret]);
            }
        } else {
            let start = [from[0].clamp(0.0, width), from[1].clamp(0.0, height)];
            self.begin_new_text(start, None, None);
        }
    }

    /// The topmost visible text layer at a canvas point. Without its font a
    /// layer is found by its pixels instead of its layout.
    fn text_at(&self, at: [f64; 2]) -> Option<usize> {
        let margin = HIT_MARGIN / f64::from(self.zoom.max(1e-3));
        let context = crate::pending_layers::PendingLayers::from_document(&self.doc);
        (0..self.doc.layers.len()).rev().find(|index| {
            let layer = &self.doc.layers[*index];
            let Some(text) = &layer.text else {
                return false;
            };
            if !context.hidden_ancestors(layer.id).is_empty() {
                return false;
            }
            match text.extent() {
                Ok(extent) => within(text, extent, at, margin),
                Err(_) => {
                    let reach = margin.ceil() as i64;
                    let [x, y] = at.map(|v| v.floor() as i64);
                    (y - reach..=y + reach).any(|y| {
                        (x - reach..=x + reach).any(|x| {
                            (0..self.doc.width as i64).contains(&x)
                                && (0..self.doc.height as i64).contains(&y)
                                && layer.pixels.rgba(y as usize * self.doc.width + x as usize)[3]
                                    != 0
                        })
                    })
                }
            }
        })
    }

    /// Whether a Pointer press here would take hold of text, rather than pan.
    pub(crate) fn pointer_grabs_text(&self, at: egui::Pos2) -> bool {
        self.picked_lines().is_empty() && self.text_at([f64::from(at.x), f64::from(at.y)]).is_some()
    }

    /// A Pointer press: text under it is selected, ready to move, and a second
    /// press opens the editor. True when the press is used up.
    pub(crate) fn pointer_text_press(&mut self, at: [f32; 2], now: f64) -> bool {
        let found = self.text_at(at.map(f64::from));
        let Some(index) = found.filter(|_| self.picked_lines().is_empty()) else {
            self.text_click = None;
            return false;
        };
        let id = self.doc.layers[index].id;
        let last = self.text_click.replace((id, now));
        if last.is_some_and(|(layer, time)| layer == id && now - time <= DOUBLE_CLICK) {
            self.text_click = None;
            self.select_tool(Tool::Text);
            self.begin_text_edit(index);
            return true;
        }
        if self.doc.layers[index].locked {
            self.status = "Layer is locked — unlock it to move its text".into();
            return true;
        }
        if !self.select_paint_target(index, None) {
            return true;
        }
        self.text_moving = true;
        false
    }

    /// The installed font new text uses, choosing one if the remembered font
    /// is unset or not on this computer.
    fn ready_text_font(&mut self) -> bool {
        if !text_render::ready() {
            text_render::warm_up();
            self.status = "Loading fonts…".into();
            return false;
        }
        if !self.text_style.font.is_empty()
            && text_render::find_font(&self.text_style.font).is_some()
        {
            return true;
        }
        match text_render::default_font() {
            Some(font) => {
                self.text_style.font = font;
                true
            }
            None => {
                self.status = "No installed fonts were found".into();
                false
            }
        }
    }

    /// Start a session on a new, empty text layer above the selected one.
    pub(crate) fn begin_new_text(
        &mut self,
        start: [f64; 2],
        wrap: Option<f64>,
        box_height: Option<f64>,
    ) {
        if !self.doc.has_room(1, 1) {
            self.status = "Layer limit reached".into();
            return;
        }
        if !self.ready_text_font() {
            return;
        }
        self.finish_layer_rename(true);
        self.cancel_gestures();
        let mut text = TextLayer {
            string: String::new(),
            font: String::new(),
            size: 0.0,
            color: self.color,
            align: Align::Left,
            vertical: false,
            wrap,
            box_height,
            line_spacing: 1.0,
            start,
            along: [1.0, 0.0],
            down: [0.0, 1.0],
            csp: None,
        };
        self.text_style.apply(&mut text);
        let id = LayerId::new();
        let before = self.doc.clone();
        let modified = self.modified;
        let created = Edit::NewText {
            id,
            text: Box::new(text.clone()),
        }
        .prepare(&self.doc);
        match created {
            Ok((doc, _)) => {
                self.history.begin_gesture_checkpoint(&self.doc);
                self.doc = doc;
                self.mask_focus = None;
                self.changed();
                self.open_text_session(id, true, before, modified, text);
            }
            Err(error) => self.status = error,
        }
    }

    /// Start a session on the text layer at `index`.
    pub(crate) fn begin_text_edit(&mut self, index: usize) {
        let Some(id) = self
            .doc
            .layers
            .get(index)
            .filter(|l| l.text.is_some())
            .map(|l| l.id)
        else {
            return;
        };
        self.finish_layer_rename(true);
        self.cancel_gestures();
        // Finishing what was open can move or remove the layer.
        let Some(index) = self.doc.layers.iter().position(|l| l.id == id) else {
            return;
        };
        if self.doc.layers[index].locked {
            self.status = "Layer is locked — unlock it to change its text".into();
            return;
        }
        if !self.select_paint_target(index, None) {
            return;
        }
        let Some(text) = self.doc.layers[self.doc.active].text.clone() else {
            return;
        };
        // Nothing redraws until something changes, so opening imported text
        // and closing it again leaves it exactly as CSP laid it out.
        let text = text.released();
        let before = self.doc.clone();
        let modified = self.modified;
        self.history.begin_gesture_checkpoint(&self.doc);
        self.text_style = Style::of(&text);
        self.open_text_session(id, false, before, modified, text);
    }

    fn open_text_session(
        &mut self,
        target: LayerId,
        created: bool,
        before: Document,
        modified: bool,
        draft: TextLayer,
    ) {
        let extent = draft.extent();
        let error = extent.as_ref().err().map(|_| {
            format!(
                "The font {} is not installed. Choose another to change this text.",
                draft.font
            )
        });
        let end = draft.string.chars().count();
        self.text_edit = Some(Editing {
            target,
            created,
            before,
            modified,
            missing: draft.missing_characters(),
            attempted: draft.clone(),
            pending: None,
            draft,
            color: self.color,
            focus: true,
            error,
            extent: extent.ok(),
            carets: Vec::new(),
            cursor: [end, end],
            place_cursor: Some([end, end]),
            fresh: true,
            selecting: None,
            resizing: false,
        });
        if let Some(edit) = &mut self.text_edit {
            edit.measure();
        }
        self.status = "Type your text · Ctrl+Enter finishes · Esc cancels".into();
    }

    /// Ask for the draft to be drawn if it has changed.
    fn draw_text_draft(&mut self) {
        let canvas = [self.doc.width, self.doc.height];
        let Some(edit) = &mut self.text_edit else {
            return;
        };
        if edit.draft == edit.attempted {
            return;
        }
        edit.attempted = edit.draft.clone();
        edit.measure();
        match edit.draft.validate() {
            Ok(()) => edit.pending = Some(self.text_drawer.draw(edit.draft.clone(), canvas)),
            Err(error) => {
                edit.pending = None;
                edit.error = Some(error);
            }
        }
    }

    /// Put finished drawing on the layer, optionally waiting for it.
    fn take_drawn_text(&mut self, wait: bool) {
        let canvas = [self.doc.width, self.doc.height];
        let Some(edit) = &mut self.text_edit else {
            return;
        };
        let Some(id) = edit.pending else {
            return;
        };
        let Some(finished) = self.text_drawer.take(id, wait) else {
            if wait {
                edit.pending = None;
                edit.error = Some("The text took too long to draw".into());
            }
            return;
        };
        edit.pending = None;
        let Some(index) = self.doc.layers.iter().position(|l| l.id == edit.target) else {
            return;
        };
        let pixels = match finished.result {
            Ok(pixels) => pixels,
            Err(error) => {
                edit.error = Some(error);
                return;
            }
        };
        let text = finished.text;
        let extent = text.extent().ok();
        let old = self.doc.layers[index].text.as_ref().and_then(|text| {
            let extent = text.extent().ok()?;
            Some(reach(text, extent, canvas))
        });
        edit.error = None;
        edit.missing = text.missing_characters();
        if self.doc.layers[index].text.as_ref() == Some(&text) {
            return;
        }
        let new = extent.map(|extent| reach(&text, extent, canvas));
        let layer = &mut self.doc.layers[index];
        layer.pixels = pixels;
        layer.text = Some(text);
        // Effects reach past the text, so they redraw everything.
        let local = !self.doc.layers.iter().any(|l| !l.effects.is_empty());
        match old.zip(new).filter(|_| local) {
            Some((old, new)) => {
                self.dirty_region.include(old);
                self.dirty_region.include(new);
                self.metadata_changed();
            }
            None => self.changed(),
        }
    }

    /// Wait until the layer shows the latest draft.
    pub(crate) fn settle_text_draft(&mut self) {
        self.draw_text_draft();
        self.take_drawn_text(true);
    }

    fn return_to_text_start(&mut self, edit: Editing) {
        self.doc = edit.before;
        self.history.discard_pending_checkpoint(&self.doc);
        self.changed();
        self.modified = edit.modified;
    }

    /// Close the session and keep what it drew, as one undo step. Text left
    /// empty is removed, so that no invisible layer stays behind.
    pub(crate) fn finish_text_edit(&mut self) {
        self.text_drag = None;
        self.settle_text_draft();
        let Some(edit) = self.text_edit.take() else {
            return;
        };
        let Some(index) = self.doc.layers.iter().position(|l| l.id == edit.target) else {
            self.history.settle(&self.doc);
            return;
        };
        let drawn = self.doc.layers[index].text.clone();
        let blank = drawn
            .as_ref()
            .is_none_or(|text| text.string.trim().is_empty());
        if blank && edit.created {
            self.return_to_text_start(edit);
            self.status = "Empty text removed".into();
            return;
        }
        if blank {
            self.doc.active = index;
            if self.doc.delete_active().is_ok() {
                self.history.settle(&self.doc);
                self.changed();
                self.status = "Empty text layer removed".into();
                return;
            }
        }
        let original = edit.before.layers.iter().find(|l| l.id == edit.target);
        if !edit.created && original.is_some_and(|l| l.text == drawn) {
            self.history.discard_pending_checkpoint(&self.doc);
            self.modified = edit.modified;
            self.status = "Text unchanged".into();
            return;
        }
        self.history.settle(&self.doc);
        self.note_recent_color();
        self.status = if edit.created {
            "Text added"
        } else {
            "Text changed"
        }
        .into();
    }

    /// Close the session and put everything back.
    pub(crate) fn cancel_text_edit(&mut self) {
        self.text_drag = None;
        if let Some(edit) = self.text_edit.take() {
            self.text_drawer.cancel();
            self.return_to_text_start(edit);
            self.status = "Text canceled".into();
        }
    }

    /// While a session is open: the invisible field that takes the typing.
    pub(crate) fn text_editor(&mut self, ctx: &egui::Context) {
        if self.text_edit.is_none() {
            return;
        }
        if self.tool != Tool::Text {
            self.finish_text_edit();
            return;
        }
        self.take_drawn_text(false);
        let view = self.view(self.canvas_area);
        let color = self.color;
        let Some(edit) = &mut self.text_edit else {
            return;
        };
        if edit.color != color {
            edit.color = color;
            edit.draft.color = color;
        }
        let (mut done, mut canceled) = (false, false);
        if !egui::Popup::is_any_open(ctx) {
            ctx.input_mut(|i| {
                canceled = i.consume_key(egui::Modifiers::NONE, egui::Key::Escape);
                done = i.consume_key(egui::Modifiers::COMMAND, egui::Key::Enter);
            });
        }
        let id = egui::Id::new("text tool field");
        if std::mem::take(&mut edit.fresh) {
            egui::TextEdit::store_state(ctx, id, Default::default());
        }
        if let Some([primary, secondary]) = edit.place_cursor.take() {
            let mut state = egui::TextEdit::load_state(ctx, id).unwrap_or_default();
            let cursor = |index| egui::text::CCursor::new(index);
            state
                .cursor
                .set_char_range(Some(egui::text_selection::CCursorRange::two(
                    cursor(secondary),
                    cursor(primary),
                )));
            egui::TextEdit::store_state(ctx, id, state);
            edit.cursor = [primary, secondary];
        }
        // The field sits at the caret, which is where the IME puts its window.
        let caret = edit
            .carets
            .get(edit.cursor[0])
            .map_or(edit.draft.start, |c| {
                edit.draft.canvas_point(c.along, c.top + c.height)
            });
        let at = view.to_screen(egui::pos2(caret[0] as f32, caret[1] as f32));
        egui::Area::new(egui::Id::new("text tool input"))
            .fixed_pos(at)
            .interactable(false)
            .constrain(false)
            .show(ctx, |ui| {
                let visuals = ui.visuals_mut();
                visuals.text_cursor.stroke = egui::Stroke::NONE;
                visuals.selection.bg_fill = egui::Color32::TRANSPARENT;
                visuals.selection.stroke = egui::Stroke::NONE;
                let output = egui::TextEdit::multiline(&mut edit.draft.string)
                    .id(id)
                    .frame(false)
                    .text_color(egui::Color32::TRANSPARENT)
                    .desired_rows(1)
                    .desired_width(4096.0)
                    .show(ui);
                let idle = ctx.memory(|m| m.focused().is_none()) && !egui::Popup::is_any_open(ctx);
                if std::mem::take(&mut edit.focus) || idle {
                    output.response.request_focus();
                }
                if let Some(range) = output.cursor_range {
                    edit.cursor = [range.primary.index, range.secondary.index];
                }
                while edit.draft.string.len() > MAX_TEXT {
                    edit.draft.string.pop();
                }
            });
        self.draw_text_draft();
        if canceled {
            self.cancel_text_edit();
        } else if done {
            self.finish_text_edit();
        } else if self.text_edit.as_ref().is_some_and(|e| e.pending.is_some()) {
            ctx.request_repaint_after(Duration::from_millis(16));
        } else {
            // The caret blinks.
            ctx.request_repaint_after(Duration::from_millis(500));
        }
    }

    /// The box and caret of the text being edited, or the box being dragged.
    pub(crate) fn paint_text_overlay(&self, painter: &egui::Painter, view: &crate::view::View) {
        let screen = |p: [f64; 2]| view.to_screen(egui::pos2(p[0] as f32, p[1] as f32));
        let outline = if let Some(drag) = &self.text_drag {
            let [a, b] = [drag.from, drag.to].map(|p| p.map(f64::from));
            Some([a, [b[0], a[1]], b, [a[0], b[1]]])
        } else if let Some(edit) = &self.text_edit {
            let extent = edit.extent.unwrap_or([0.0, edit.draft.size]);
            Some(corners(&edit.draft, extent))
        } else if self.tool == Tool::Pointer && !self.text_moving {
            // The Pointer shows which object it has hold of.
            self.selected_text()
                .and_then(|(_, text)| Some(corners(text, text.extent().ok()?)))
        } else {
            None
        };
        let Some(outline) = outline else {
            return;
        };
        let mut points: Vec<_> = outline.iter().map(|p| screen(*p)).collect();
        points.push(points[0]);
        let line = |color| egui::Stroke::new(1.0_f32, color);
        painter.add(egui::Shape::line(
            points.clone(),
            line(egui::Color32::WHITE),
        ));
        painter.extend(egui::Shape::dashed_line(
            &points,
            line(egui::Color32::BLACK),
            5.0,
            5.0,
        ));
        if let Some(edit) = &self.text_edit {
            self.paint_text_caret(painter, edit, &screen, points[3]);
        }
    }

    fn paint_text_caret(
        &self,
        painter: &egui::Painter,
        edit: &Editing,
        screen: &dyn Fn([f64; 2]) -> egui::Pos2,
        below: egui::Pos2,
    ) {
        use egui::{Color32, Stroke};
        let text = &edit.draft;
        let [primary, secondary] = edit.cursor;
        let (first, last) = (primary.min(secondary), primary.max(secondary));
        for pair in edit
            .carets
            .get(first..=last.min(edit.carets.len().saturating_sub(1)))
            .unwrap_or(&[])
            .windows(2)
        {
            if pair[0].top == pair[1].top {
                let (a, b) = (pair[0], pair[1]);
                let quad = [
                    text.canvas_point(a.along, a.top),
                    text.canvas_point(b.along, a.top),
                    text.canvas_point(b.along, a.top + a.height),
                    text.canvas_point(a.along, a.top + a.height),
                ]
                .map(screen)
                .to_vec();
                painter.add(egui::Shape::convex_polygon(
                    quad,
                    Color32::from_rgba_unmultiplied(75, 150, 255, 90),
                    Stroke::NONE,
                ));
            }
        }
        let time = painter.ctx().input(|i| i.time);
        if let Some(caret) = edit.carets.get(primary)
            && first == last
            && time.rem_euclid(1.0) < 0.5
        {
            let top = screen(text.canvas_point(caret.along, caret.top));
            let foot = screen(text.canvas_point(caret.along, caret.top + caret.height));
            painter.line_segment([top, foot], Stroke::new(3.0_f32, Color32::WHITE));
            painter.line_segment([top, foot], Stroke::new(1.5_f32, Color32::BLACK));
        }
        if let Some(handle) = edit.handle() {
            let rect = egui::Rect::from_center_size(screen(handle), egui::vec2(9.0, 9.0));
            painter.rect_filled(rect, 1.0, Color32::WHITE);
            painter.rect_stroke(
                rect,
                1.0,
                Stroke::new(1.0_f32, Color32::BLACK),
                egui::StrokeKind::Middle,
            );
        }
        let warning = edit.error.clone().or_else(|| {
            let missing: String = edit.missing.iter().take(12).collect();
            (!missing.is_empty()).then(|| format!("This font can't draw: {missing}"))
        });
        if let Some(warning) = warning {
            let font = egui::FontId::proportional(13.0);
            let at = below + egui::vec2(0.0, 8.0);
            let galley = painter.layout_no_wrap(warning, font, Color32::WHITE);
            let back = egui::Rect::from_min_size(at, galley.size()).expand(4.0);
            painter.rect_filled(back, 3.0, Color32::from_black_alpha(190));
            painter.galley(at, galley, Color32::WHITE);
        }
    }

    /// The text the options bar shows and changes: the open session, else a
    /// selected text layer, else what new text will look like.
    fn shown_text_style(&self) -> Style {
        if let Some(edit) = &self.text_edit {
            return Style::of(&edit.draft);
        }
        self.selected_text()
            .map_or_else(|| self.text_style.clone(), |(_, text)| Style::of(text))
    }

    fn selected_text(&self) -> Option<(usize, &TextLayer)> {
        let index = self.input_layer()?;
        Some((index, self.doc.layers.get(index)?.text.as_ref()?))
    }

    fn set_text_style(&mut self, style: Style) {
        self.text_style = style.clone();
        // Changing a selected text layer's look edits that layer.
        if self.text_edit.is_none()
            && let Some((index, _)) = self.selected_text()
        {
            self.begin_text_edit(index);
            self.text_style = style.clone();
        }
        if let Some(edit) = &mut self.text_edit {
            style.apply(&mut edit.draft);
            edit.focus = true;
        }
        self.draw_text_draft();
    }

    /// Font, size, color, alignment and spacing, in the options bar.
    pub(crate) fn text_options_ui(&mut self, ui: &mut egui::Ui, narrow: bool) {
        text_render::warm_up();
        let original = self.shown_text_style();
        let mut style = original.clone();
        let label = if style.font.is_empty() {
            "Font"
        } else {
            style.font.as_str()
        };
        let label: String = label.chars().take(22).collect();
        let button = ui
            .menu_button(label, |ui| self.font_menu_ui(ui, &mut style))
            .response
            .on_hover_text("Font · scroll to step through fonts");
        if button.hovered() {
            self.step_font_with_wheel(ui, &mut style);
        }
        ui.add(
            egui::DragValue::new(&mut style.size)
                .range(1.0..=8192.0)
                .speed(0.5)
                .max_decimals(1)
                .suffix(" px"),
        )
        .on_hover_text("Text size in canvas pixels");
        if !narrow {
            let dpi = self.doc.dpi;
            let mut points = style.size * 72.0 / dpi;
            if ui
                .add(
                    egui::DragValue::new(&mut points)
                        .range(72.0 / dpi..=8192.0 * 72.0 / dpi)
                        .speed(0.25)
                        .max_decimals(1)
                        .suffix(" pt"),
                )
                .on_hover_text("Text size in points at this document's DPI")
                .changed()
            {
                style.size = (points * dpi / 72.0).clamp(1.0, 8192.0);
            }
        }
        self.quick_color_ui(ui);
        for (align, name) in [
            (Align::Left, "Left"),
            (Align::Center, "Center"),
            (Align::Right, "Right"),
        ] {
            ui.selectable_value(&mut style.align, align, name);
        }
        if !narrow {
            ui.add(
                egui::DragValue::new(&mut style.line_spacing)
                    .range(0.5..=5.0)
                    .speed(0.01)
                    .max_decimals(2)
                    .prefix("Spacing "),
            )
            .on_hover_text("Line spacing, as a multiple of the font's line height");
        }
        if style != original {
            self.set_text_style(style);
        }
        // A pen display may have no keyboard to finish with.
        if self.text_edit.is_some() {
            if ui.button("Done").on_hover_text("Ctrl+Enter").clicked() {
                self.finish_text_edit();
            } else if ui.button("Cancel").on_hover_text("Esc").clicked() {
                self.cancel_text_edit();
            }
        }
    }

    /// The wheel over the font button moves through the fonts, as it does over
    /// the blend box. Wheel up moves up the list.
    fn step_font_with_wheel(&mut self, ui: &mut egui::Ui, style: &mut Style) {
        let delta = ui.input_mut(|i| {
            let delta = i.raw_scroll_delta.y;
            i.raw_scroll_delta = egui::Vec2::ZERO;
            i.smooth_scroll_delta = egui::Vec2::ZERO;
            delta
        });
        self.font_list.wheel += delta;
        let steps = (self.font_list.wheel / FONT_WHEEL_STEP).trunc();
        self.font_list.wheel -= steps * FONT_WHEEL_STEP;
        let names = text_render::font_names();
        if steps == 0.0 || names.is_empty() {
            return;
        }
        let current = names
            .iter()
            .position(|name| *name == style.font)
            .unwrap_or(0);
        let next = (current as i64 - steps as i64).clamp(0, names.len() as i64 - 1);
        style.font.clone_from(&names[next as usize]);
    }

    fn font_menu_ui(&mut self, ui: &mut egui::Ui, style: &mut Style) {
        let names = text_render::font_names();
        if names.is_empty() {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Loading fonts…");
            });
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(100));
            return;
        }
        ui.add(
            egui::TextEdit::singleline(&mut self.font_list.search)
                .hint_text("Search fonts…")
                .desired_width(PREVIEW_SIZE[0] as f32),
        );
        let search = self.font_list.search.to_lowercase();
        let found: Vec<&String> = names
            .iter()
            .filter(|name| {
                let name = name.to_lowercase();
                search.split_whitespace().all(|word| name.contains(word))
            })
            .collect();
        if self.font_list.previews.len() > PREVIEW_LIMIT {
            self.font_list.previews.clear();
        }
        let mut budget = PREVIEWS_PER_FRAME;
        let height = PREVIEW_SIZE[1] as f32 + 4.0;
        egui::ScrollArea::vertical()
            .max_height(320.0)
            .auto_shrink([false, false])
            .show_rows(ui, height, found.len(), |ui, rows| {
                #[cfg(test)]
                {
                    self.font_list.first_row = rows.start;
                }
                for name in &found[rows] {
                    let size = egui::vec2(PREVIEW_SIZE[0] as f32, height);
                    let (rect, row) = ui.allocate_exact_size(size, egui::Sense::click());
                    if !self.font_list.previews.contains_key(*name) {
                        if budget == 0 {
                            ui.ctx().request_repaint();
                        } else {
                            budget -= 1;
                            let preview = preview_image(name).map(|image| {
                                ui.ctx().load_texture(
                                    format!("font preview {name}"),
                                    image,
                                    egui::TextureOptions::LINEAR,
                                )
                            });
                            self.font_list.previews.insert((*name).clone(), preview);
                        }
                    }
                    let selected = style.font == **name;
                    if selected || row.hovered() {
                        ui.painter().rect_filled(
                            rect,
                            2.0,
                            ui.visuals().widgets.hovered.weak_bg_fill,
                        );
                    }
                    let ink = ui.visuals().text_color();
                    match self.font_list.previews.get(*name) {
                        Some(Some(texture)) => {
                            let image = egui::Rect::from_min_size(
                                rect.min + egui::vec2(0.0, 2.0),
                                egui::vec2(PREVIEW_SIZE[0] as f32, PREVIEW_SIZE[1] as f32),
                            );
                            let uv =
                                egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0));
                            ui.painter().image(texture.id(), image, uv, ink);
                        }
                        _ => {
                            ui.painter().text(
                                rect.left_center() + egui::vec2(2.0, 0.0),
                                egui::Align2::LEFT_CENTER,
                                name.as_str(),
                                egui::FontId::proportional(14.0),
                                ink,
                            );
                        }
                    }
                    if row.on_hover_text(name.as_str()).clicked() {
                        style.font.clone_from(name);
                        ui.close();
                    }
                }
            });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text_render::TEST_FONT;

    fn app() -> Dudel {
        let mut app = Dudel {
            doc: Document::new(300, 200),
            ..Default::default()
        };
        app.text_style.font = TEST_FONT.into();
        app.tool = Tool::Text;
        app
    }

    fn typed(app: &mut Dudel, words: &str) {
        app.text_edit.as_mut().unwrap().draft.string = words.into();
        app.settle_text_draft();
    }

    fn ink(app: &Dudel) -> usize {
        let pixels = &app.doc.layers[app.doc.active].pixels;
        (0..app.doc.width * app.doc.height)
            .filter(|p| pixels.rgba(*p)[3] != 0)
            .count()
    }

    fn undo_depth(app: &mut Dudel) -> usize {
        let mut steps = 0;
        while app.history.can_undo() {
            app.undo();
            steps += 1;
        }
        steps
    }

    #[test]
    fn a_whole_session_is_one_undo_step() {
        let mut app = app();
        app.begin_new_text([20.0, 20.0], None, None);
        for words in ["H", "He", "Hem", "Heml", "Hemlo"] {
            typed(&mut app, words);
            assert!(ink(&app) > 0, "drawn live");
        }
        app.finish_text_edit();
        assert_eq!(app.doc.layers.len(), 2);
        assert!(app.modified);
        assert_eq!(undo_depth(&mut app), 1);
        assert_eq!(app.doc.layers.len(), 1);
        app.redo();
        assert_eq!(app.doc.layers.len(), 2);
        let text = app.doc.layers[1].text.as_ref().unwrap();
        assert_eq!(text.string, "Hemlo");
    }

    #[test]
    fn escape_changes_nothing() {
        let mut app = app();
        app.begin_new_text([20.0, 20.0], None, None);
        typed(&mut app, "Hemlo");
        app.finish_text_edit();
        let saved = app.doc.clone();
        app.modified = false;
        let index = app.doc.active;
        app.begin_text_edit(index);
        typed(&mut app, "Something else");
        assert_ne!(app.doc.layers[index].text.as_ref().unwrap().string, "Hemlo");
        app.cancel_text_edit();
        assert!(app.doc.layers[index].text == saved.layers[index].text);
        assert!(app.doc.layers[index].pixels == saved.layers[index].pixels);
        assert!(!app.modified);
        assert_eq!(undo_depth(&mut app), 1);
    }

    #[test]
    fn new_text_left_empty_disappears_without_history() {
        for words in ["", "  \n "] {
            let mut app = app();
            app.begin_new_text([20.0, 20.0], None, None);
            assert_eq!(app.doc.layers.len(), 2);
            typed(&mut app, words);
            app.finish_text_edit();
            assert_eq!(app.doc.layers.len(), 1);
            assert!(!app.history.can_undo() && !app.modified);
        }
    }

    #[test]
    fn emptying_existing_text_removes_the_layer_in_one_step() {
        let mut app = app();
        app.begin_new_text([20.0, 20.0], None, None);
        typed(&mut app, "Hemlo");
        app.finish_text_edit();
        let index = app.doc.active;
        app.begin_text_edit(index);
        typed(&mut app, "");
        app.finish_text_edit();
        assert_eq!(app.doc.layers.len(), 1);
        app.undo();
        assert_eq!(app.doc.layers.len(), 2);
        let text = app.doc.layers[1].text.as_ref().unwrap();
        assert_eq!(text.string, "Hemlo");
    }

    #[test]
    fn reopening_without_changes_adds_no_history() {
        let mut app = app();
        app.begin_new_text([20.0, 20.0], None, None);
        typed(&mut app, "Hemlo");
        app.finish_text_edit();
        app.modified = false;
        let index = app.doc.active;
        app.begin_text_edit(index);
        app.finish_text_edit();
        assert!(!app.modified);
        assert_eq!(undo_depth(&mut app), 1);
    }

    #[test]
    fn other_edits_close_the_session_first() {
        let mut app = app();
        app.begin_new_text([20.0, 20.0], None, None);
        typed(&mut app, "Hemlo");
        app.undo();
        assert!(app.text_edit.is_none());
        assert_eq!(app.doc.layers.len(), 1, "undo removed the finished text");
    }

    #[test]
    fn locked_text_stays_as_it_is() {
        let mut app = app();
        app.begin_new_text([20.0, 20.0], None, None);
        typed(&mut app, "Hemlo");
        app.finish_text_edit();
        let index = app.doc.active;
        app.doc.layers[index].locked = true;
        app.begin_text_edit(index);
        assert!(app.text_edit.is_none());
        assert!(app.status.contains("locked"));
    }

    fn imported(app: &mut Dudel) -> usize {
        app.begin_new_text([40.0, 30.0], Some(220.0), None);
        typed(app, "Hemlo");
        app.finish_text_edit();
        let index = app.doc.active;
        app.doc.layers[index].text.as_mut().unwrap().csp = Some([110.0, 60.0]);
        app.history = crate::document::History::new();
        app.modified = false;
        index
    }

    #[test]
    fn opening_imported_text_changes_nothing_until_it_is_edited() {
        let mut app = app();
        let index = imported(&mut app);
        let pixels = app.doc.layers[index].pixels.clone();
        app.begin_text_edit(index);
        app.finish_text_edit();
        let layer = &app.doc.layers[index];
        assert!(layer.text.as_ref().unwrap().csp.is_some());
        assert!(layer.pixels == pixels && !app.modified && !app.history.can_undo());
        app.begin_text_edit(index);
        typed(&mut app, "Hemlo!");
        app.finish_text_edit();
        assert!(app.doc.layers[index].text.as_ref().unwrap().csp.is_none());
        app.undo();
        assert!(app.doc.layers[index].text.as_ref().unwrap().csp.is_some());
    }

    #[test]
    fn a_missing_font_keeps_the_pixels_and_says_so() {
        let mut app = app();
        app.begin_new_text([20.0, 20.0], None, None);
        typed(&mut app, "Hemlo");
        app.finish_text_edit();
        let index = app.doc.active;
        let pixels = app.doc.layers[index].pixels.clone();
        app.doc.layers[index].text.as_mut().unwrap().font = "No Such Font".into();
        app.begin_text_edit(index);
        let edit = app.text_edit.as_ref().unwrap();
        assert!(edit.error.as_ref().unwrap().contains("No Such Font"));
        typed(&mut app, "Changed");
        assert!(app.text_edit.as_ref().unwrap().error.is_some());
        assert!(app.doc.layers[index].pixels == pixels);
        // Choosing a font that exists redraws it.
        app.text_edit.as_mut().unwrap().draft.font = TEST_FONT.into();
        app.settle_text_draft();
        assert!(app.text_edit.as_ref().unwrap().error.is_none());
        assert!(app.doc.layers[index].pixels != pixels);
    }

    #[test]
    fn clicks_find_the_topmost_visible_text_and_nothing_else() {
        let mut app = app();
        app.zoom = 1.0;
        app.begin_new_text([20.0, 20.0], None, None);
        typed(&mut app, "Lower");
        app.finish_text_edit();
        app.begin_new_text([20.0, 20.0], None, None);
        typed(&mut app, "Upper");
        app.finish_text_edit();
        assert_eq!(app.text_at([30.0, 40.0]), Some(2));
        assert_eq!(app.text_at([250.0, 180.0]), None);
        app.doc.layers[2].visible = false;
        assert_eq!(app.text_at([30.0, 40.0]), Some(1));
    }

    #[test]
    fn a_click_makes_a_line_and_a_drag_makes_a_box() {
        let mut app = app();
        app.zoom = 1.0;
        app.text_packet([40.0, 50.0], true, false, true);
        app.text_packet([41.0, 50.0], false, true, true);
        let edit = app.text_edit.as_ref().unwrap();
        assert_eq!(edit.draft.wrap, None);
        assert_eq!(edit.draft.start, [40.0, 50.0]);
        // The next press finishes it and starts nothing.
        typed(&mut app, "Hemlo");
        app.text_packet([280.0, 190.0], true, false, true);
        app.text_packet([280.0, 190.0], false, true, true);
        assert!(app.text_edit.is_none());
        assert_eq!(app.doc.layers.len(), 2);
        app.text_packet([200.0, 190.0], true, false, true);
        app.text_packet([120.0, 100.0], false, false, true);
        app.text_packet([120.0, 100.0], false, true, true);
        let edit = app.text_edit.as_ref().unwrap();
        assert_eq!(edit.draft.wrap, Some(80.0));
        assert_eq!(edit.draft.box_height, Some(90.0));
        assert_eq!(edit.draft.start, [120.0, 100.0]);
        assert_eq!(
            edit.extent.unwrap(),
            [80.0, 90.0],
            "the box is what was drawn"
        );
    }

    #[test]
    fn the_corner_resizes_a_box_and_lines_have_no_corner() {
        let mut app = app();
        app.zoom = 1.0;
        app.begin_new_text([20.0, 20.0], None, None);
        assert!(app.text_edit.as_ref().unwrap().handle().is_none());
        app.finish_text_edit();
        app.begin_new_text([20.0, 20.0], Some(100.0), Some(60.0));
        typed(&mut app, "Hemlo");
        app.text_packet([120.0, 80.0], true, false, true);
        app.text_packet([220.0, 150.0], false, false, true);
        app.text_packet([220.0, 150.0], false, true, true);
        let edit = app.text_edit.as_ref().unwrap();
        assert_eq!(edit.draft.wrap, Some(200.0));
        assert_eq!(edit.draft.box_height, Some(130.0));
        app.finish_text_edit();
        assert_eq!(undo_depth(&mut app), 1, "all of it is one step");
    }

    #[test]
    fn presses_on_the_text_place_the_caret_and_drags_select() {
        let mut app = app();
        app.zoom = 1.0;
        app.begin_new_text([20.0, 20.0], None, None);
        typed(&mut app, "Hemlo");
        let edit = app.text_edit.as_ref().unwrap();
        let carets = edit.carets.clone();
        assert_eq!(carets.len(), 6);
        let at = |index: usize| [(20.0 + carets[index].along) as f32, 40.0];
        app.text_packet(at(1), true, false, true);
        app.text_packet(at(1), false, true, true);
        assert_eq!(app.text_edit.as_ref().unwrap().place_cursor, Some([1, 1]));
        app.text_packet(at(1), true, false, true);
        app.text_packet(at(4), false, false, true);
        app.text_packet(at(4), false, true, true);
        let edit = app.text_edit.as_ref().unwrap();
        assert_eq!(edit.place_cursor, Some([4, 1]));
        assert!(edit.selecting.is_none());
        assert_eq!(app.doc.layers.len(), 2, "no new layer");
    }

    #[test]
    fn a_press_outside_the_workspace_starts_nothing() {
        let mut app = app();
        app.text_packet([40.0, 50.0], true, false, false);
        app.text_packet([40.0, 50.0], false, true, false);
        assert!(app.text_edit.is_none() && app.doc.layers.len() == 1);
    }

    #[test]
    fn the_foreground_color_recolors_open_text_only() {
        let mut app = app();
        app.color = [10, 20, 30];
        app.begin_new_text([20.0, 20.0], None, None);
        typed(&mut app, "Hemlo");
        app.finish_text_edit();
        let index = app.doc.active;
        app.color = [200, 0, 0];
        assert_eq!(
            app.doc.layers[index].text.as_ref().unwrap().color,
            [10, 20, 30]
        );
        app.begin_text_edit(index);
        app.canvas_area = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(300.0, 200.0));
        let ctx = egui::Context::default();
        let _ = ctx.run(Default::default(), |ctx| {
            app.color = [0, 200, 0];
            app.text_editor(ctx);
        });
        app.finish_text_edit();
        assert_eq!(
            app.doc.layers[index].text.as_ref().unwrap().color,
            [0, 200, 0]
        );
    }

    #[test]
    fn typing_never_waits_and_finishing_keeps_the_newest_draft() {
        let mut app = app();
        app.begin_new_text([20.0, 20.0], None, None);
        for words in ["H", "He", "Hem"] {
            app.text_edit.as_mut().unwrap().draft.string = words.into();
            app.draw_text_draft();
        }
        app.finish_text_edit();
        let text = app.doc.layers[app.doc.active].text.as_ref().unwrap();
        assert_eq!(text.string, "Hem", "finishing waits for the last draft");
        assert!(ink(&app) > 0);
    }

    #[test]
    fn escape_ignores_a_drawing_still_running() {
        let mut app = app();
        app.begin_new_text([20.0, 20.0], None, None);
        app.text_edit.as_mut().unwrap().draft.string = "Hemlo".into();
        app.draw_text_draft();
        app.cancel_text_edit();
        assert!(app.text_edit.is_none());
        assert_eq!(app.doc.layers.len(), 1);
        assert!(!app.history.can_undo());
    }

    #[test]
    fn switching_tabs_finishes_the_text_and_keeps_tabs_apart() {
        let mut app = app();
        app.begin_new_text([20.0, 20.0], None, None);
        typed(&mut app, "Hemlo");
        assert!(app.prepare_tab_for_open());
        assert!(app.text_edit.is_none());
        assert_eq!(app.doc.layers.len(), 1, "the new tab is its own project");
        app.request_tab(0);
        app.process_tab_request();
        assert_eq!(app.doc.layers.len(), 2);
        assert_eq!(app.doc.layers[1].text.as_ref().unwrap().string, "Hemlo");
    }

    /// Not a pass or fail: `cargo test --release text_redraw_cost -- --ignored
    /// --nocapture` shows what one keystroke costs at the extremes.
    #[test]
    #[ignore]
    fn text_redraw_cost() {
        let paragraph = "The quick brown fox jumps over the lazy dog. ".repeat(10);
        for (name, canvas, size, turned) in [
            ("line 2k canvas", [2000, 2000], 48.0, false),
            ("paragraph 100 MP", [10_000, 10_000], 200.0, false),
            ("huge 100 MP", [10_000, 10_000], 2000.0, false),
            ("turned 100 MP", [10_000, 10_000], 200.0, true),
        ] {
            let angle: f64 = if turned { 0.5 } else { 0.0 };
            let text = TextLayer {
                string: paragraph.clone(),
                font: TEST_FONT.into(),
                size,
                color: [0; 3],
                align: Align::Left,
                vertical: false,
                wrap: Some(canvas[0] as f64 * 0.8),
                box_height: None,
                line_spacing: 1.0,
                start: [100.0, 100.0],
                along: [angle.cos(), angle.sin()],
                down: [-angle.sin(), angle.cos()],
                csp: None,
            };
            let start = std::time::Instant::now();
            let runs = 5;
            for _ in 0..runs {
                text.render(canvas, &|| false).unwrap();
            }
            println!("{name}: {:?}", start.elapsed() / runs);
        }
    }

    #[test]
    fn saved_styles_are_made_safe() {
        let mut style = Style {
            font: "x".repeat(300),
            size: f64::NAN,
            align: Align::Right,
            line_spacing: 90.0,
        };
        style.sanitize();
        assert!(style.font.is_empty());
        assert_eq!(style.size, 48.0);
        assert_eq!(style.line_spacing, 1.0);
        assert_eq!(style.align, Align::Right);
    }

    #[test]
    fn previews_show_a_font_in_itself() {
        let image = preview_image(TEST_FONT).unwrap();
        assert_eq!(image.size, PREVIEW_SIZE);
        assert!(image.pixels.iter().any(|p| p.a() != 0));
        assert!(preview_image("No Such Font").is_none());
    }

    #[test]
    fn redraw_reach_covers_the_text_and_stays_on_the_canvas() {
        let text = TextLayer {
            string: "Hemlo".into(),
            font: TEST_FONT.into(),
            size: 40.0,
            color: [0; 3],
            align: Align::Left,
            vertical: false,
            wrap: None,
            box_height: None,
            line_spacing: 1.0,
            start: [-10.0, 180.0],
            along: [1.0, 0.0],
            down: [0.0, 1.0],
            csp: None,
        };
        let extent = text.extent().unwrap();
        let [left, top, right, bottom] = reach(&text, extent, [300, 200]);
        assert_eq!((left, bottom), (0, 200));
        assert!(top < 180 && right > 80 && right <= 300);
    }
}
