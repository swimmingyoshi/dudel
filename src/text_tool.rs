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
/// On-screen pixels around the box's resize corner that still grab it.
const HANDLE_REACH: f64 = 10.0;
/// Smallest box side a drag leaves, in canvas pixels.
const MIN_BOX: f64 = 8.0;

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
    /// Tests hold drawing back to stand in for a slow drawing.
    #[cfg(test)]
    held: AtomicBool,
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
            #[cfg(test)]
            while self.held.load(Ordering::SeqCst) && !stale() {
                std::thread::sleep(Duration::from_millis(1));
            }
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
    fn worker(&mut self) -> &Arc<DrawShared> {
        self.shared.get_or_insert_with(|| {
            let shared = Arc::new(DrawShared::default());
            let worker = shared.clone();
            let _ = std::thread::Builder::new()
                .name("text drawing".into())
                .spawn(move || worker.run());
            shared
        })
    }
    fn draw(&mut self, text: TextLayer, canvas: [usize; 2]) -> u64 {
        let shared = self.worker();
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
    /// Hold drawing back, or let it go on.
    #[cfg(test)]
    pub(crate) fn hold(&mut self, held: bool) {
        self.worker().held.store(held, Ordering::SeqCst);
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

/// What a press on an open text box takes hold of.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Grab {
    /// The box's side or corner: -1, 0 or 1 along the lines and across them.
    Resize([i8; 2]),
    /// The rest of the border.
    Move,
}

/// Text layers' measurements, kept while their records stay the same. Hovering
/// with the Pointer or Text tool asks for every layer's, every frame.
#[derive(Default)]
pub(crate) struct Extents(std::cell::RefCell<std::collections::HashMap<LayerId, Measured>>);

type Measured = (TextLayer, Option<[f64; 2]>);

impl Extents {
    /// How long and tall a layer's text is, or None without its font.
    pub(crate) fn get(&self, id: LayerId, text: &TextLayer) -> Option<[f64; 2]> {
        let mut known = self.0.borrow_mut();
        if let Some((record, extent)) = known.get(&id)
            && record == text
        {
            return *extent;
        }
        if known.len() > 4096 {
            known.clear();
        }
        let extent = text.extent().ok();
        known.insert(id, (text.clone(), extent));
        extent
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
    pub(crate) target: LayerId,
    /// This session made the layer, so leaving it empty removes it.
    created: bool,
    /// The document when the session began, which Escape returns to.
    before: Document,
    modified: bool,
    pub(crate) draft: TextLayer,
    /// The last draft drawing was tried for, successful or not.
    attempted: TextLayer,
    /// The drawing job still running for it.
    pending: Option<u64>,
    /// The foreground color when last seen: changing it recolors the text.
    color: [u8; 3],
    pub(crate) focus: bool,
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
    /// What a press on the box has hold of, where it began, and where the
    /// text then started.
    grab: Option<(Grab, [f64; 2], [f64; 2])>,
    /// That press has travelled far enough to count as a drag.
    grab_moved: bool,
    /// The text and its size when a resize began. Every step of the drag is
    /// measured from these, so that steps can't add up and walk the box away.
    resize_from: Option<(TextLayer, [f64; 2])>,
    /// When the session began: a second press soon after is a double-click.
    opened: std::time::Instant,
    /// The IME is composing, so Esc and Enter belong to it.
    pub(crate) composing: bool,
    /// Done was chosen while the last drawing was still running: the session
    /// takes no input and closes as soon as the drawing is in, since when.
    finishing: Option<std::time::Instant>,
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
    /// Forget any press on the box, keeping what it already did.
    pub(crate) fn release_press(&mut self) {
        self.grab = None;
        self.resize_from = None;
        self.selecting = None;
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
    #[cfg(test)]
    pub(crate) fn created(&self) -> bool {
        self.created
    }
    #[cfg(test)]
    pub(crate) fn clone_extent(&self) -> [f64; 2] {
        self.extent.unwrap()
    }
    /// The selected characters, first to last.
    #[cfg(test)]
    pub(crate) fn selected(&self) -> [usize; 2] {
        let [a, b] = self.cursor;
        [a.min(b), a.max(b)]
    }
    /// The box's handles, as the sides each one moves and where it sits. Lines
    /// have no box, so they have none.
    fn handles(&self) -> Vec<([i8; 2], [f64; 2])> {
        let Some([width, height]) = self.extent.filter(|_| self.draft.wrap.is_some()) else {
            return Vec::new();
        };
        let place = |side: i8, size: f64| (f64::from(side) + 1.0) / 2.0 * size;
        [
            [-1, -1],
            [0, -1],
            [1, -1],
            [1, 0],
            [1, 1],
            [0, 1],
            [-1, 1],
            [-1, 0],
        ]
        .map(|sides: [i8; 2]| {
            let at = self
                .draft
                .canvas_point(place(sides[0], width), place(sides[1], height));
            (sides, at)
        })
        .to_vec()
    }
    /// What a press here would take hold of.
    fn grab_at(&self, at: [f64; 2], reach: f64) -> Option<Grab> {
        let extent = self.extent?;
        let handle = self
            .handles()
            .into_iter()
            .find(|(_, [x, y])| (x - at[0]).hypot(y - at[1]) <= reach);
        match handle {
            Some((sides, _)) => Some(Grab::Resize(sides)),
            None => on_border(&self.draft, extent, at, reach).then_some(Grab::Move),
        }
    }
    /// The resize pointer that points the way a handle drags on screen, which
    /// turns with the text and the view.
    fn resize_icon(&self, sides: [i8; 2], view: &crate::view::View) -> egui::CursorIcon {
        use egui::CursorIcon;
        let [x, y] = self.draft.start;
        let point = |x: f64, y: f64| view.to_screen(egui::pos2(x as f32, y as f32));
        let [along, down] = [f64::from(sides[0]), f64::from(sides[1])];
        let way = point(
            x + self.draft.along[0] * along + self.draft.down[0] * down,
            y + self.draft.along[1] * along + self.draft.down[1] * down,
        ) - point(x, y);
        let angle = f64::from(way.y.atan2(way.x)).to_degrees().rem_euclid(180.0);
        match angle {
            a if (22.5..67.5).contains(&a) => CursorIcon::ResizeNwSe,
            a if (67.5..112.5).contains(&a) => CursorIcon::ResizeVertical,
            a if (112.5..157.5).contains(&a) => CursorIcon::ResizeNeSw,
            _ => CursorIcon::ResizeHorizontal,
        }
    }
    /// Move the sides `sides` names to a canvas point, as measured when the
    /// drag began. Moving the left or top side moves where the text starts,
    /// so the opposite side stays put. A side across the lines stops where
    /// the text needs it: past that the box can't shrink, and moving its start
    /// anyway would push the box along instead.
    fn resize_to(&mut self, at: [f64; 2], sides: [i8; 2]) {
        let Some((origin, extent)) = &self.resize_from else {
            return;
        };
        let point = origin.text_point(at);
        let mut shift = [0.0; 2];
        let mut sizes = [None; 2];
        for axis in 0..2 {
            let least = if axis == 0 {
                MIN_BOX
            } else {
                // The lines' own thickness at the width chosen just above.
                let mut lines = origin.clone();
                lines.wrap = sizes[0].or(origin.wrap);
                lines.box_height = None;
                lines.extent().map_or(MIN_BOX, |e| e[1].max(MIN_BOX))
            };
            let size = match sides[axis] {
                1 => point[axis].max(least),
                -1 => (extent[axis] - point[axis]).max(least),
                _ => continue,
            };
            if sides[axis] < 0 {
                shift[axis] = extent[axis] - size;
            }
            sizes[axis] = Some(size);
        }
        self.draft.start = origin.canvas_point(shift[0], shift[1]);
        self.draft.wrap = sizes[0].or(origin.wrap);
        self.draft.box_height = sizes[1].or(origin.box_height);
        self.measure();
    }
    /// Fit caret and outline to the draft, right away, while drawing catches up.
    fn measure(&mut self) {
        self.extent = self.draft.extent().ok();
        self.carets = self.draft.carets().unwrap_or_default();
    }
}

/// Whether a canvas point is on the border of a text's box.
fn on_border(text: &TextLayer, extent: [f64; 2], at: [f64; 2], reach: f64) -> bool {
    let [along, down] = text.text_point(at);
    let [width, height] = extent;
    let inset = reach * 0.4;
    let outer =
        (-reach..=width + reach).contains(&along) && (-reach..=height + reach).contains(&down);
    let inner =
        (inset..=width - inset).contains(&along) && (inset..=height - inset).contains(&down);
    outer && !inner
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

impl Dudel {
    /// Canvas input for the Text tool.
    pub(crate) fn text_packet(&mut self, at: [f32; 2], start: bool, end: bool, on_workspace: bool) {
        // Finishing text takes no input, and a press starts nothing new yet.
        if self.text_finishing() {
            return;
        }
        let point = at.map(f64::from);
        let scale = 1.0 / f64::from(self.zoom.max(1e-3));
        if start && on_workspace && self.double_click_opens_text(point) {
            return;
        }
        if start && on_workspace {
            let mut away = false;
            match &mut self.text_edit {
                Some(edit) => {
                    if let Some(grab) = edit.grab_at(point, HANDLE_REACH * scale) {
                        edit.grab = Some((grab, point, edit.draft.start));
                        edit.resize_from = edit.extent.map(|extent| (edit.draft.clone(), extent));
                        edit.grab_moved = false;
                    } else if edit.contains(point, 0.0) {
                        let caret = edit.caret_at(point);
                        edit.selecting = Some(caret);
                        edit.place_cursor = Some([caret, caret]);
                    } else {
                        away = true;
                    }
                    edit.focus = true;
                }
                None => self.text_drag = Some(Drag { from: at, to: at }),
            }
            // A press away from the text finishes it, and only the next press
            // starts something new.
            if away {
                self.request_text_finish();
            }
        }
        if let Some(edit) = &mut self.text_edit {
            if let Some((grab, from, origin)) = edit.grab {
                let offset = [point[0] - from[0], point[1] - from[1]];
                let travelled = offset[0].hypot(offset[1]);
                edit.grab_moved |= travelled >= f64::from(BOX_DRAG) * scale;
                if edit.grab_moved {
                    match grab {
                        Grab::Resize(sides) => edit.resize_to(point.map(f64::round), sides),
                        Grab::Move => {
                            let offset = offset.map(f64::round);
                            edit.draft.start = [origin[0] + offset[0], origin[1] + offset[1]];
                        }
                    }
                }
                if end {
                    // A click on the border is a click on the text.
                    if !edit.grab_moved {
                        let caret = edit.caret_at(point);
                        edit.place_cursor = Some([caret, caret]);
                    }
                    edit.grab = None;
                    edit.resize_from = None;
                }
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

    /// The first click of a double-click on another layer's text started new
    /// text there. The second gives that up and opens the text instead.
    fn double_click_opens_text(&mut self, at: [f64; 2]) -> bool {
        let Some(edit) = self.text_edit.as_ref().filter(|edit| {
            edit.created
                && edit.draft.string.is_empty()
                && edit.opened.elapsed().as_secs_f64() <= DOUBLE_CLICK
        }) else {
            return false;
        };
        let target = edit.target;
        let beneath = (0..self.doc.layers.len())
            .rev()
            .filter(|index| self.doc.layers[*index].id != target)
            .any(|index| self.text_hit(index, at));
        if !beneath {
            return false;
        }
        self.cancel_text_edit();
        if let Some(index) = self.text_at(at) {
            self.open_text(index, at);
        }
        true
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
        } else if let Some(index) = self.selected_text_at(from) {
            self.open_text(index, from);
        } else {
            let start = [from[0].clamp(0.0, width), from[1].clamp(0.0, height)];
            self.begin_new_text(start, None, None);
        }
    }

    /// A Text tool press on the selected text: the drag that may follow moves
    /// it, and a click edits it. Locked text is left to `place_text`, which
    /// says why.
    pub(crate) fn text_tool_press(&mut self, at: [f32; 2]) {
        if self.text_edit.is_some() {
            return;
        }
        let Some(index) = self.selected_text_at(at.map(f64::from)) else {
            return;
        };
        if !self.doc.layers[index].locked && self.select_paint_target(index, None) {
            self.text_moving = true;
            self.text_press = Some(at);
        }
    }

    /// At the end of dragging text: a press that hardly moved was a click, so
    /// pen jitter never nudges it. The Text tool then starts editing.
    pub(crate) fn text_press_was_a_click(&mut self, at: [f32; 2]) -> bool {
        let Some(from) = self.text_press.take() else {
            return false;
        };
        let travelled = (at[0] - from[0]).hypot(at[1] - from[1]);
        if travelled * self.zoom >= BOX_DRAG {
            return false;
        }
        self.cancel_move_drag();
        self.text_moving = false;
        self.status = "Text selected".into();
        if self.tool == Tool::Text
            && let Some(index) = self.selected_text_at(from.map(f64::from))
        {
            self.open_text(index, from.map(f64::from));
        }
        true
    }

    fn open_text(&mut self, index: usize, at: [f64; 2]) {
        self.begin_text_edit(index);
        if let Some(edit) = &mut self.text_edit {
            let caret = edit.caret_at(at);
            edit.place_cursor = Some([caret, caret]);
        }
    }

    /// The pointer to show with the Text tool at a screen position.
    pub(crate) fn text_cursor_icon(
        &self,
        view: &crate::view::View,
        pos: egui::Pos2,
    ) -> egui::CursorIcon {
        use egui::CursorIcon;
        let at = view.to_doc(pos);
        let at = [f64::from(at.x), f64::from(at.y)];
        let reach = HANDLE_REACH / f64::from(self.zoom.max(1e-3));
        let Some(edit) = &self.text_edit else {
            let found = self.selected_text_at(at).and_then(|index| {
                let text = self.doc.layers[index].text.as_ref()?;
                Some((text, text.extent().ok()?))
            });
            return match found {
                Some((text, extent)) if on_border(text, extent, at, reach) => CursorIcon::Move,
                _ => CursorIcon::Text,
            };
        };
        match edit.grab.map(|(grab, ..)| grab).or(edit.grab_at(at, reach)) {
            Some(Grab::Move) => CursorIcon::Move,
            Some(Grab::Resize(sides)) => edit.resize_icon(sides, view),
            None if edit.contains(at, 0.0) => CursorIcon::Text,
            None => CursorIcon::Default,
        }
    }

    /// The outline of the text under the pointer, showing that it can be taken.
    fn hovered_text_outline(
        &self,
        painter: &egui::Painter,
        view: &crate::view::View,
    ) -> Option<[[f64; 2]; 4]> {
        let hover = painter.ctx().input(|i| i.pointer.hover_pos())?;
        if !painter.clip_rect().contains(hover) {
            return None;
        }
        let at = view.to_doc(hover);
        let at = [f64::from(at.x), f64::from(at.y)];
        // The Text tool only takes hold of the selected text.
        let index = if self.tool == Tool::Text {
            self.selected_text_at(at)?
        } else {
            self.text_at(at)?
        };
        let text = self.doc.layers[index].text.as_ref()?;
        Some(corners(text, text.extent().ok()?))
    }

    /// The topmost visible text layer at a canvas point. Without its font a
    /// layer is found by its pixels instead of its layout.
    fn text_at(&self, at: [f64; 2]) -> Option<usize> {
        (0..self.doc.layers.len())
            .rev()
            .find(|index| self.text_hit(*index, at))
    }

    /// The selected text layer, if `at` is on it. The Text tool only takes
    /// hold of this one, so that new text can start on top of other text.
    fn selected_text_at(&self, at: [f64; 2]) -> Option<usize> {
        self.input_layer().filter(|index| self.text_hit(*index, at))
    }

    /// Whether the layer at `index` is shown text covering a canvas point.
    fn text_hit(&self, index: usize, at: [f64; 2]) -> bool {
        let Some(id) = self.doc.layers.get(index).map(|l| l.id) else {
            return false;
        };
        let margin = HIT_MARGIN / f64::from(self.zoom.max(1e-3));
        let Some(layer) = self.doc.layers.get(index) else {
            return false;
        };
        let Some(text) = &layer.text else {
            return false;
        };
        if !self.doc.shown(index) {
            return false;
        }
        match self.text_extents.get(id, text) {
            Some(extent) => within(text, extent, at, margin),
            None => {
                let reach = margin.ceil() as i64;
                let [x, y] = at.map(|v| v.floor() as i64);
                (y - reach..=y + reach).any(|y| {
                    (x - reach..=x + reach).any(|x| {
                        (0..self.doc.width as i64).contains(&x)
                            && (0..self.doc.height as i64).contains(&y)
                            && layer.pixels.rgba(y as usize * self.doc.width + x as usize)[3] != 0
                    })
                })
            }
        }
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
        self.text_press = Some(at);
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
            rightward: false,
            letters: Default::default(),
            format: Default::default(),
            wrap,
            box_height,
            line_spacing: 1.0,
            start,
            along: [1.0, 0.0],
            down: [0.0, 1.0],
            csp: None,
        };
        self.text_style.apply(&mut text);
        // A line starts where it was clicked, whichever way it runs.
        if wrap.is_none() {
            text.start = start;
        }
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
        self.text_style = crate::text_options::Style::of(&text);
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
            grab: None,
            grab_moved: false,
            resize_from: None,
            opened: std::time::Instant::now(),
            composing: false,
            finishing: None,
        });
        if let Some(edit) = &mut self.text_edit {
            edit.measure();
        }
        self.status = "Type your text · Ctrl+Enter finishes · Esc cancels".into();
    }

    /// Ask for the draft to be drawn if it has changed.
    pub(crate) fn draw_text_draft(&mut self) {
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
        // Typing that couldn't be drawn is lost, so say so.
        if let Some(error) = &edit.error
            && drawn.as_ref() != Some(&edit.draft)
        {
            let original = edit.before.layers.iter().find(|l| l.id == edit.target);
            if original.is_some_and(|l| l.text == drawn) {
                self.history.discard_pending_checkpoint(&self.doc);
                self.modified = edit.modified;
            } else {
                self.history.settle(&self.doc);
            }
            self.status = format!("Text kept as it was drawn: {error}");
            return;
        }
        let original = edit.before.layers.iter().find(|l| l.id == edit.target);
        let effects = &self.doc.layers[index].effects;
        if !edit.created && original.is_some_and(|l| l.text == drawn && l.effects == *effects) {
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

    /// Done, without waiting: when the last drawing is still running, the
    /// session stops taking input and closes once it is in, while the app
    /// stays responsive. Anything that needs the session closed right away
    /// still gets it through `finish_text_edit`, with the newest draft.
    pub(crate) fn request_text_finish(&mut self) {
        self.text_drag = None;
        self.draw_text_draft();
        self.take_drawn_text(false);
        if let Some(error) = self.undrawable_text() {
            self.status = format!("{error} · Choose an installed font to keep this, or Esc");
            return;
        }
        match &mut self.text_edit {
            Some(edit) if edit.pending.is_some() => {
                edit.release_press();
                edit.finishing.get_or_insert_with(std::time::Instant::now);
                self.status = "Finishing text… Esc keeps editing".into();
            }
            _ => self.finish_text_edit(),
        }
    }

    /// Why the open draft can't be drawn, when finishing now would lose it.
    fn undrawable_text(&self) -> Option<String> {
        let edit = self.text_edit.as_ref()?;
        let error = edit.error.clone()?;
        let layer = self.doc.layers.iter().find(|l| l.id == edit.target)?;
        let lost = edit.pending.is_none()
            && edit.attempted == edit.draft
            && layer.text.as_ref() != Some(&edit.draft)
            && !edit.draft.string.trim().is_empty();
        lost.then_some(error)
    }

    /// Done was chosen and the drawing isn't in yet.
    pub(crate) fn text_finishing(&self) -> bool {
        self.text_edit
            .as_ref()
            .is_some_and(|edit| edit.finishing.is_some())
    }

    /// Changed your mind while text was finishing: go on typing.
    pub(crate) fn keep_editing_text(&mut self) {
        if let Some(edit) = &mut self.text_edit
            && edit.finishing.take().is_some()
        {
            edit.focus = true;
            self.status = "Type your text · Ctrl+Enter finishes · Esc cancels".into();
        }
    }

    /// While finishing: close the session once the drawing is in.
    fn poll_text_finish(&mut self, ctx: &egui::Context) {
        self.take_drawn_text(false);
        let Some(edit) = &mut self.text_edit else {
            return;
        };
        if edit.pending.is_some()
            && edit
                .finishing
                .is_some_and(|since| since.elapsed() > DRAW_TIMEOUT)
        {
            self.text_drawer.cancel();
            edit.pending = None;
            edit.error = Some("The text took too long to draw".into());
        }
        if edit.pending.is_none() {
            self.finish_text_edit();
        } else {
            ctx.request_repaint_after(Duration::from_millis(16));
        }
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
        if self.text_finishing() {
            self.poll_text_finish(ctx);
            return;
        }
        if self.tool != Tool::Text {
            // Another tool can't wait for a font: close, saying what was lost.
            if self.undrawable_text().is_some() {
                self.finish_text_edit();
                return;
            }
            self.request_text_finish();
            if self.text_finishing() {
                self.poll_text_finish(ctx);
            }
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
        // Keys pressed during composition reach us as well as the IME.
        let mut composing = edit.composing;
        ctx.input(|i| {
            for event in &i.events {
                match event {
                    egui::Event::Ime(egui::ImeEvent::Preedit(text)) => {
                        edit.composing = !text.is_empty();
                        composing = true;
                    }
                    egui::Event::Ime(egui::ImeEvent::Enabled) => composing = true,
                    egui::Event::Ime(_) => {
                        edit.composing = false;
                        composing = true;
                    }
                    _ => {}
                }
            }
        });
        let (mut done, mut canceled) = (false, false);
        if !egui::Popup::is_any_open(ctx) && !composing {
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
        // Typing always lands here, unless another text field is in use. The
        // field takes focus before it runs, so that a press elsewhere never
        // leaves a frame without it: that would turn the IME off mid-word.
        let elsewhere = ctx
            .memory(|m| m.focused())
            .is_some_and(|f| f != id && egui::TextEdit::load_state(ctx, f).is_some());
        if std::mem::take(&mut edit.focus) || (!elsewhere && !egui::Popup::is_any_open(ctx)) {
            ctx.memory_mut(|m| m.request_focus(id));
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
            self.request_text_finish();
            if self.text_finishing() {
                ctx.request_repaint_after(Duration::from_millis(16));
            }
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
                .or_else(|| self.hovered_text_outline(painter, view))
        } else if self.tool == Tool::Text {
            self.hovered_text_outline(painter, view)
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
        if let Some(edit) = self.text_edit.as_ref().filter(|e| e.finishing.is_none()) {
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
        for (_, handle) in edit.handles() {
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

    /// Editor frames until `done`, as the app runs them, or fail after a while.
    fn frames_until(app: &mut Dudel, done: impl Fn(&Dudel) -> bool) {
        let ctx = egui::Context::default();
        for _ in 0..5000 {
            let _ = ctx.run(Default::default(), |ctx| app.text_editor(ctx));
            if done(app) {
                return;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        panic!("it never happened");
    }

    #[test]
    fn done_during_a_slow_drawing_waits_without_blocking_and_keeps_the_newest_draft() {
        let mut app = app();
        app.begin_new_text([20.0, 20.0], None, None);
        typed(&mut app, "Hem");
        app.text_drawer.hold(true);
        app.text_edit.as_mut().unwrap().draft.string = "Hemlo".into();
        let pressed = std::time::Instant::now();
        app.request_text_finish();
        assert!(
            pressed.elapsed() < Duration::from_secs(1),
            "Done doesn't wait"
        );
        assert!(app.text_finishing());
        // Meanwhile a press starts nothing.
        app.text_packet([250.0, 150.0], true, false, true);
        app.text_packet([250.0, 150.0], false, true, true);
        assert!(app.text_drag.is_none() && app.text_finishing());
        assert_eq!(app.doc.layers.len(), 2);
        // Esc goes back to typing, and Done finishes again.
        app.keep_editing_text();
        assert!(!app.text_finishing() && app.text_edit.is_some());
        app.request_text_finish();
        assert!(app.text_finishing());
        app.text_drawer.hold(false);
        frames_until(&mut app, |app| app.text_edit.is_none());
        let text = app.doc.layers[1].text.as_ref().unwrap();
        assert_eq!(text.string, "Hemlo");
        assert_eq!(app.status, "Text added");
        assert_eq!(undo_depth(&mut app), 1);
    }

    #[test]
    fn closing_text_that_is_finishing_still_keeps_the_newest_draft() {
        let mut app = app();
        app.begin_new_text([20.0, 20.0], None, None);
        typed(&mut app, "Hem");
        app.text_drawer.hold(true);
        app.text_edit.as_mut().unwrap().draft.string = "Hemlo".into();
        app.request_text_finish();
        assert!(app.text_finishing());
        app.text_drawer.hold(false);
        // Undo, saving or a tab change close every gesture at once.
        app.close_input_gestures();
        assert!(app.text_edit.is_none());
        assert_eq!(app.doc.layers[1].text.as_ref().unwrap().string, "Hemlo");
        assert_eq!(undo_depth(&mut app), 1);
    }

    #[test]
    fn done_with_the_drawing_already_in_closes_at_once() {
        let mut app = app();
        app.begin_new_text([20.0, 20.0], None, None);
        typed(&mut app, "Hemlo");
        app.request_text_finish();
        assert!(app.text_edit.is_none());
        assert_eq!(app.status, "Text added");
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
    fn done_without_the_font_keeps_the_text_and_says_why() {
        let mut app = app();
        app.begin_new_text([20.0, 20.0], None, None);
        typed(&mut app, "Hemlo");
        app.finish_text_edit();
        let index = app.doc.active;
        app.doc.layers[index].text.as_mut().unwrap().font = "No Such Font".into();
        let before = app.doc.clone();
        // Opened and left alone, it closes as usual.
        app.begin_text_edit(index);
        app.request_text_finish();
        assert!(app.text_edit.is_none());
        assert_eq!(app.status, "Text unchanged");

        // Typing that can't be drawn isn't thrown away by Done.
        app.begin_text_edit(index);
        typed(&mut app, "Changed");
        app.request_text_finish();
        assert!(app.text_edit.is_some(), "Done keeps the typing open");
        assert!(app.status.contains("No Such Font"), "{}", app.status);
        // Choosing an installed font keeps it.
        app.text_edit.as_mut().unwrap().draft.font = TEST_FONT.into();
        app.settle_text_draft();
        app.request_text_finish();
        assert!(app.text_edit.is_none());
        assert_eq!(
            app.doc.layers[index].text.as_ref().unwrap().string,
            "Changed"
        );

        // Anything that must close it anyway says what was lost.
        app.undo();
        assert_eq!(app.doc.layers[index].text, before.layers[index].text);
        app.begin_text_edit(index);
        typed(&mut app, "Lost");
        app.close_input_gestures();
        assert!(app.text_edit.is_none());
        assert_eq!(app.doc.layers[index].text, before.layers[index].text);
        assert!(app.doc.layers[index].pixels == before.layers[index].pixels);
        assert!(
            app.status.starts_with("Text kept as it was drawn"),
            "{}",
            app.status
        );
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
        assert!(app.text_edit.as_ref().unwrap().handles().is_empty());
        app.finish_text_edit();
        app.begin_new_text([20.0, 20.0], Some(100.0), Some(60.0));
        typed(&mut app, "Hi");
        app.text_packet([120.0, 80.0], true, false, true);
        app.text_packet([220.0, 150.0], false, false, true);
        app.text_packet([220.0, 150.0], false, true, true);
        let edit = app.text_edit.as_ref().unwrap();
        assert_eq!(edit.draft.wrap, Some(200.0));
        assert_eq!(edit.draft.box_height, Some(130.0));
        app.finish_text_edit();
        assert_eq!(undo_depth(&mut app), 1, "all of it is one step");
    }

    fn boxed(app: &mut Dudel) {
        app.zoom = 1.0;
        app.begin_new_text([60.0, 40.0], Some(120.0), Some(100.0));
        // Short enough for every box these tests resize to: one line.
        typed(app, "Hi");
    }

    fn drag(app: &mut Dudel, from: [f32; 2], to: [f32; 2]) {
        app.text_packet(from, true, false, true);
        app.text_packet(to, false, false, true);
        app.text_packet(to, false, true, true);
        app.settle_text_draft();
    }

    #[test]
    fn every_side_and_corner_resizes_and_the_far_side_stays() {
        for (from, to, start, size) in [
            ([60.0, 40.0], [40.0, 30.0], [40.0, 30.0], [140.0, 110.0]),
            ([120.0, 40.0], [120.0, 20.0], [60.0, 20.0], [120.0, 120.0]),
            ([180.0, 90.0], [230.0, 90.0], [60.0, 40.0], [170.0, 100.0]),
            ([120.0, 140.0], [120.0, 170.0], [60.0, 40.0], [120.0, 130.0]),
            ([60.0, 90.0], [100.0, 90.0], [100.0, 40.0], [80.0, 100.0]),
        ] {
            let mut app = app();
            boxed(&mut app);
            drag(&mut app, from, to);
            let edit = app.text_edit.as_ref().unwrap();
            assert_eq!(edit.draft.start, start, "{from:?}");
            assert_eq!(edit.extent.unwrap(), size, "{from:?}");
        }
    }

    /// A press, several moves and a release, as a real drag sends them.
    fn drag_through(app: &mut Dudel, points: &[[f32; 2]]) {
        app.text_packet(points[0], true, false, true);
        for at in &points[1..] {
            app.text_packet(*at, false, false, true);
        }
        app.text_packet(*points.last().unwrap(), false, true, true);
        app.settle_text_draft();
    }

    #[test]
    fn dragging_a_side_past_the_other_never_walks_the_box_away() {
        let mut app = app();
        boxed(&mut app);
        let before = app.text_edit.as_ref().unwrap().extent.unwrap();
        let right = 60.0 + before[0];
        // The left side, dragged in steps far beyond the right side.
        let steps: Vec<[f32; 2]> = (0..12).map(|i| [60.0 + 40.0 * i as f32, 90.0]).collect();
        drag_through(&mut app, &steps);
        let edit = app.text_edit.as_ref().unwrap();
        let extent = edit.extent.unwrap();
        let far = edit.draft.start[0] + extent[0];
        assert!(
            (far - right).abs() < 1e-6,
            "the right side stays put: {far} {right}"
        );
        assert!(
            edit.draft.start[0] >= 60.0,
            "and the box stays where it was"
        );
    }

    #[test]
    fn a_side_across_the_lines_stops_where_the_text_needs_it() {
        let mut app = app();
        boxed(&mut app);
        let before = app.text_edit.as_ref().unwrap().clone_extent();
        let bottom = 40.0 + before[1];
        // The top side, dragged far below the bottom in steps.
        let steps: Vec<[f32; 2]> = (0..12).map(|i| [120.0, 40.0 + 30.0 * i as f32]).collect();
        drag_through(&mut app, &steps);
        let edit = app.text_edit.as_ref().unwrap();
        let extent = edit.extent.unwrap();
        let far = edit.draft.start[1] + extent[1];
        assert!(
            (far - bottom).abs() < 1e-6,
            "the bottom stays put: {far} {bottom}"
        );
        let lines = edit.draft.carets().unwrap()[0].height;
        assert!(
            extent[1] >= lines - 1e-6,
            "and the box still holds the line"
        );
        // Dragging back out works from where this drag begins.
        let top = edit.draft.start[1] as f32;
        drag_through(&mut app, &[[120.0, top], [120.0, 20.0]]);
        let edit = app.text_edit.as_ref().unwrap();
        assert!((edit.draft.start[1] + edit.extent.unwrap()[1] - bottom).abs() < 1e-6);
    }

    #[test]
    fn the_border_moves_the_box_in_the_same_undo_step() {
        let mut app = app();
        boxed(&mut app);
        drag(&mut app, [100.0, 40.0], [130.0, 70.0]);
        assert_eq!(app.text_edit.as_ref().unwrap().draft.start, [90.0, 70.0]);
        app.finish_text_edit();
        let text = app.doc.layers[app.doc.active].text.as_ref().unwrap();
        assert_eq!(text.start, [90.0, 70.0]);
        assert_eq!(undo_depth(&mut app), 1);
    }

    #[test]
    fn a_click_on_the_border_places_the_caret_without_moving_anything() {
        let mut app = app();
        boxed(&mut app);
        drag(&mut app, [60.0, 70.0], [62.0, 71.0]);
        let edit = app.text_edit.as_ref().unwrap();
        assert_eq!(edit.draft.start, [60.0, 40.0]);
        assert_eq!(edit.place_cursor, Some([0, 0]));
    }

    #[test]
    fn lines_have_a_border_to_move_but_nothing_to_resize() {
        let mut app = app();
        app.zoom = 1.0;
        app.begin_new_text([60.0, 40.0], None, None);
        typed(&mut app, "Hemlo");
        let edit = app.text_edit.as_ref().unwrap();
        assert_eq!(edit.grab_at([60.0, 40.0], 10.0), Some(Grab::Move));
    }

    #[test]
    fn vertical_text_starts_at_the_click_and_a_box_stays_as_drawn() {
        let mut app = app();
        app.zoom = 1.0;
        app.text_style.vertical = true;
        // Right to left, as Japanese reads.
        app.text_style.rightward = false;
        app.begin_new_text([200.0, 30.0], None, None);
        typed(&mut app, "あい");
        let edit = app.text_edit.as_ref().unwrap();
        assert_eq!(edit.draft.start, [200.0, 30.0]);
        assert_eq!(edit.draft.along, [0.0, 1.0]);
        app.finish_text_edit();
        app.begin_new_text([40.0, 20.0], Some(120.0), Some(150.0));
        typed(&mut app, "あい");
        let edit = app.text_edit.as_ref().unwrap();
        assert_eq!(
            edit.draft.start,
            [160.0, 20.0],
            "the box's top right corner"
        );
        assert_eq!(edit.extent.unwrap(), [150.0, 120.0]);
        let corners = corners(&edit.draft, edit.extent.unwrap());
        assert_eq!(
            corners[2],
            [40.0, 170.0],
            "the box drawn, turned around nothing"
        );
    }

    #[test]
    fn left_to_right_lines_start_at_the_click_and_fill_the_box_from_its_left() {
        let mut app = app();
        app.zoom = 1.0;
        app.text_style.vertical = true;
        app.text_style.rightward = true;
        app.begin_new_text([40.0, 30.0], None, None);
        typed(&mut app, "Hemlo");
        let edit = app.text_edit.as_ref().unwrap();
        assert_eq!(edit.draft.start, [40.0, 30.0]);
        assert_eq!(edit.draft.down, [1.0, 0.0]);
        app.finish_text_edit();
        app.begin_new_text([40.0, 20.0], Some(120.0), Some(150.0));
        let edit = app.text_edit.as_ref().unwrap();
        assert_eq!(edit.draft.start, [40.0, 20.0], "the box's top left corner");
        assert_eq!(edit.extent.unwrap(), [150.0, 120.0]);
    }

    #[test]
    fn the_options_bar_faces_open_text_another_way_in_the_same_undo_step() {
        let mut app = app();
        app.text_style.vertical = true;
        app.begin_new_text([40.0, 20.0], Some(120.0), Some(150.0));
        typed(&mut app, "Meow");
        let mut style = app.shown_text_style();
        style.letters = crate::text_layer::Letters::TurnedLeft;
        app.set_text_style(style);
        app.settle_text_draft();
        app.finish_text_edit();
        let text = app.doc.layers[app.doc.active].text.as_ref().unwrap();
        assert_eq!(text.letters, crate::text_layer::Letters::TurnedLeft);
        assert_eq!(undo_depth(&mut app), 1);
    }

    #[test]
    fn turning_vertical_off_clears_left_to_right() {
        let mut app = app();
        app.text_style.vertical = true;
        app.text_style.rightward = true;
        app.begin_new_text([40.0, 20.0], Some(120.0), Some(150.0));
        typed(&mut app, "Hemlo");
        let mut style = app.shown_text_style();
        assert!(style.rightward);
        style.vertical = false;
        app.set_text_style(style);
        app.settle_text_draft();
        let edit = app.text_edit.as_ref().unwrap();
        assert!(!edit.draft.vertical && !edit.draft.rightward);
        edit.draft.validate().unwrap();
    }

    #[test]
    fn the_options_bar_turns_open_text_vertical_in_the_same_undo_step() {
        let mut app = app();
        app.begin_new_text([40.0, 20.0], Some(120.0), Some(150.0));
        typed(&mut app, "Hemlo");
        let mut style = app.shown_text_style();
        style.vertical = true;
        app.set_text_style(style);
        app.settle_text_draft();
        app.finish_text_edit();
        let text = app.doc.layers[app.doc.active].text.as_ref().unwrap();
        assert!(text.vertical);
        assert_eq!(undo_depth(&mut app), 1);
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
                rightward: false,
                letters: Default::default(),
                format: Default::default(),
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
        let mut style = crate::text_options::Style {
            font: "x".repeat(300),
            size: f64::NAN,
            align: Align::Right,
            line_spacing: 90.0,
            vertical: false,
            rightward: false,
            letters: Default::default(),
            format: Default::default(),
        };
        style.sanitize();
        assert!(style.font.is_empty());
        assert_eq!(style.size, 48.0);
        assert_eq!(style.line_spacing, 1.0);
        assert_eq!(style.align, Align::Right);
    }

    #[test]
    fn previews_show_a_font_in_itself() {
        let image = crate::text_options::preview_image(TEST_FONT, TEST_FONT).unwrap();
        assert_eq!(image.size, [300, 28]);
        assert!(image.pixels.iter().any(|p| p.a() != 0));
        assert!(crate::text_options::preview_image("No Such Font", "No Such Font").is_none());
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
            rightward: false,
            letters: Default::default(),
            format: Default::default(),
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
