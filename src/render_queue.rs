//! FIFO raster strokes and explicit document-command fences. The controller
//! exclusively owns document/history mutations while this queue is nonempty.
use crate::{
    document::{Document, History},
    stroke::Sample,
    stroke_buffer::{DabPaint, Restore, Settings, StrokeBuffer, TailReplay},
};
use std::{collections::VecDeque, sync::Arc};

const MAX_COMMANDS: usize = 4096;
const MAX_INPUT_BYTES: usize = 4 * 1024 * 1024;
pub type StrokeId = u64;
enum Command {
    Begin {
        id: StrokeId,
        settings: Arc<Settings>,
        bytes: usize,
        current_target: bool,
        identity: Option<crate::document::LayerId>,
    },
    Dab {
        id: StrokeId,
        sample: Sample,
    },
    /// A stroke's end, with its recording when it was drawn on a vector layer.
    End(StrokeId, Option<Arc<crate::vector_layer::Stroke>>),
    Skipped(StrokeId),
    Undo,
    Redo,
    Snapshot(u64),
    Barrier(u64),
    Edit {
        ticket: u64,
        edit: crate::document_edits::Edit,
    },
    Opacity {
        ticket: u64,
        target: crate::document::LayerId,
        value: f32,
        start: bool,
    },
    OpacityEnd(u64),
    CancelledEdit(u64, &'static str),
}
impl Command {
    fn id(&self) -> Option<StrokeId> {
        match self {
            Self::Begin { id, .. }
            | Self::Dab { id, .. }
            | Self::End(id, _)
            | Self::Skipped(id) => Some(*id),
            _ => None,
        }
    }
    fn bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + match self {
                Self::Begin { bytes, .. } => *bytes,
                Self::Edit { edit, .. } => edit.payload_bytes(),
                _ => 0,
            }
    }
}
enum Operation {
    Idle,
    Dab(Box<DabPaint>),
    Finish(Option<Box<TailReplay>>),
    Restore(Box<Restore>),
    Compact(Box<crate::document::Settlement>),
    Settle,
}
struct Active {
    id: StrokeId,
    settings: Arc<Settings>,
    buffer: StrokeBuffer,
    operation: Operation,
    cancel: bool,
    retired_prefix: usize,
    failure: Option<String>,
    /// Added to the vector layer's strokes when the stroke settles.
    record: Option<Arc<crate::vector_layer::Stroke>>,
}
pub struct Completed {
    pub id: StrokeId,
    pub changed: bool,
    pub cancelled: bool,
}
#[derive(Clone, Copy, Default, serde::Serialize)]
pub(crate) struct Timing {
    units: usize,
    total_ms: f64,
    pub(crate) max_ms: f64,
}
pub(crate) const PHASES: [&str; 8] = [
    "live_row",
    "tail_unit",
    "settle",
    "cancel",
    "begin",
    "undo",
    "redo",
    "snapshot",
];
fn record_timing(progress: &mut Progress, phase: usize, started: Option<std::time::Instant>) {
    if let Some(started) = started {
        let elapsed = started.elapsed().as_secs_f64() * 1000.0;
        let timing = &mut progress.timings[phase];
        timing.units += 1;
        timing.total_ms += elapsed;
        timing.max_ms = timing.max_ms.max(elapsed);
    }
}
#[derive(Default)]
pub struct Progress {
    pub work_units: usize,
    pub dabs: usize,
    pub damage: crate::render_region::Damage,
    pub completed: Vec<Completed>,
    pub started: Option<StrokeId>,
    pub snapshot: Option<(u64, Document)>,
    pub barrier: Option<u64>,
    pub history_error: Option<String>,
    pub history_actions: Vec<(bool, bool)>,
    pub edits: Vec<(u64, Result<crate::document_edits::Outcome, String>)>,
    pub render_error: Option<String>,
    pub opacity: Option<OpacityProgress>,
    pub move_done: Option<MoveProgress>,
    pub(crate) timings: [Timing; 8],
    pub(crate) settlement_payload_peak_bytes: usize,
    pub(crate) finished_scratch_released_bytes: usize,
}
pub struct OpacityProgress {
    pub ticket: u64,
    pub started: bool,
    pub ended: bool,
    pub changed: bool,
    pub error: Option<String>,
}
pub struct MoveProgress {
    pub affine: bool,
    pub offset: [i64; 2],
    pub changed: bool,
    pub canceled: bool,
    pub modified_before: bool,
    pub error: Option<String>,
}
struct FinishingMove {
    engine: crate::move_preview::Engine,
    checkpointed: bool,
    modified_before: bool,
    canceled: bool,
    cursor: Option<crate::document::Settlement>,
}
struct ActiveOpacity {
    ticket: u64,
    target: crate::document::LayerId,
    original: Option<f32>,
    checkpoint: bool,
    failed: bool,
}
type EditResult = Result<(Document, crate::document_edits::Outcome), String>;
struct EditSettlement {
    ticket: u64,
    outcome: crate::document_edits::Outcome,
    cursor: crate::document::Settlement,
}
struct EditJob {
    ticket: u64,
    label: &'static str,
    before: Document,
    cancel: Arc<std::sync::atomic::AtomicBool>,
    worker: Option<std::thread::JoinHandle<EditResult>>,
    result: Option<EditResult>,
}
impl EditJob {
    fn ready(&self) -> bool {
        self.result.is_some()
            || self
                .worker
                .as_ref()
                .is_some_and(|worker| worker.is_finished())
    }
    fn join(&mut self) {
        if let Some(worker) = self.worker.take() {
            self.result = Some(
                worker
                    .join()
                    .unwrap_or_else(|_| Err(format!("{} worker failed", self.label))),
            );
        }
    }
}
pub struct Queue {
    commands: VecDeque<Command>,
    active: Option<Active>,
    input_open: Option<StrokeId>,
    input_opacity: Option<(u64, crate::document::LayerId)>,
    opacity: Option<ActiveOpacity>,
    background_edit: Option<EditJob>,
    edit_settlement: Option<EditSettlement>,
    prepared_done: Option<(u64, crate::document_edits::Outcome)>,
    finishing_move: Option<FinishingMove>,
    awaiting_barrier: Option<u64>,
    next_id: u64,
    next_ticket: u64,
    bytes: usize,
    command_limit: usize,
    byte_limit: usize,
    profile: bool,
}
impl Default for Queue {
    fn default() -> Self {
        Self {
            commands: VecDeque::new(),
            active: None,
            input_open: None,
            input_opacity: None,
            opacity: None,
            background_edit: None,
            edit_settlement: None,
            prepared_done: None,
            finishing_move: None,
            awaiting_barrier: None,
            next_id: 0,
            next_ticket: 0,
            bytes: 0,
            command_limit: MAX_COMMANDS,
            byte_limit: MAX_INPUT_BYTES,
            profile: false,
        }
    }
}
impl Queue {
    /// Publish a verified preview without recomputing its pixels on Apply. The
    /// original supplies the undo checkpoint; history compaction stays sliced.
    pub(crate) fn finish_prepared_edit(
        &mut self,
        doc: &mut Document,
        history: &mut History,
        before: &Document,
        updated: Document,
    ) -> Result<Option<u64>, String> {
        if self.busy() || !crate::document::same_snapshot(doc, before) {
            return Err("Preview source changed before Apply".into());
        }
        updated.validate()?;
        if crate::document::same_snapshot(doc, &updated) {
            return Ok(None);
        }
        let ticket = self
            .next_ticket
            .checked_add(1)
            .ok_or("Rendering ticket exhausted")?;
        let outcome = crate::document_edits::Outcome {
            changed: true,
            undoable: true,
            redraw: true,
        };
        history.begin_gesture_checkpoint(doc);
        *doc = updated;
        self.next_ticket = ticket;
        if let Some(mut cursor) = history.begin_settle(doc) {
            cursor.prefer_smaller_full_snapshot();
            self.edit_settlement = Some(EditSettlement {
                ticket,
                outcome,
                cursor,
            });
        } else {
            history.settle(doc);
            self.prepared_done = Some((ticket, outcome));
        }
        Ok(Some(ticket))
    }
    pub(crate) fn finish_move(
        &mut self,
        engine: crate::move_preview::Engine,
        checkpointed: bool,
        modified_before: bool,
    ) -> Result<(), (Box<crate::move_preview::Engine>, String)> {
        if self.active.is_some()
            || self.background_edit.is_some()
            || self.edit_settlement.is_some()
            || self.finishing_move.is_some()
            || self.opacity.is_some()
            || self.awaiting_barrier.is_some()
        {
            return Err((
                Box::new(engine),
                "Move finish encountered another active document owner".into(),
            ));
        }
        self.finishing_move = Some(FinishingMove {
            engine,
            checkpointed,
            modified_before,
            canceled: false,
            cursor: None,
        });
        Ok(())
    }
    /// Exclusive stroke buffers/cursors, excluding shared captured Settings,
    /// map control bytes, allocator overhead and separately tracked settlement.
    pub(crate) fn stroke_working_bytes(&self) -> usize {
        self.active.as_ref().map_or(0, |active| {
            active.buffer.payload_capacity_bytes()
                + match &active.operation {
                    Operation::Dab(cursor) => {
                        std::mem::size_of::<DabPaint>() + cursor.payload_capacity_bytes()
                    }
                    Operation::Finish(Some(cursor)) => {
                        std::mem::size_of::<TailReplay>() + cursor.payload_capacity_bytes()
                    }
                    Operation::Restore(cursor) => {
                        std::mem::size_of::<Restore>() + cursor.payload_capacity_bytes()
                    }
                    _ => 0,
                }
        })
    }
    pub(crate) fn register_settlement_allocations(
        &self,
        registry: &mut crate::snapshot_memory::Registry,
    ) {
        if let Some(active) = &self.active
            && let Operation::Compact(cursor) = &active.operation
        {
            cursor.register_allocations(registry);
        }
        if let Some(settlement) = &self.edit_settlement {
            settlement.cursor.register_allocations(registry);
        }
        if let Some(movement) = &self.finishing_move {
            movement.engine.register_allocations(registry);
            if let Some(cursor) = &movement.cursor {
                cursor.register_allocations(registry);
            }
        }
    }
    pub fn waiting_edit(&self) -> bool {
        self.finishing_move.as_ref().is_some_and(|movement| {
            movement.cursor.is_none() && !movement.canceled && movement.engine.waiting_worker()
        }) || self
            .background_edit
            .as_ref()
            .is_some_and(|job| !job.ready())
    }
    pub fn wait_edit(&mut self) {
        if let Some(movement) = &mut self.finishing_move {
            movement.engine.wait_worker();
        }
        if let Some(job) = &mut self.background_edit {
            job.join();
        }
    }
    pub fn has_background_edit(&self) -> bool {
        self.finishing_move
            .as_ref()
            .is_some_and(|movement| movement.cursor.is_none())
            || self.background_edit.is_some()
            || self.commands.iter().any(|command| {
                matches!(
                    command,
                    Command::Edit { edit, .. } if edit.background_label(true).is_some()
                )
            })
    }
    pub fn cancel_background_edit(&mut self) -> bool {
        if let Some(movement) = &mut self.finishing_move
            && movement.cursor.is_none()
        {
            movement.canceled = true;
            return true;
        }
        if let Some(job) = &self.background_edit {
            job.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
            return true;
        }
        if let Some(command) = self.commands.iter_mut().find(|command| {
            matches!(
                command,
                Command::Edit { edit, .. } if edit.background_label(true).is_some()
            )
        }) && let Command::Edit { ticket, edit } = command
        {
            let ticket = *ticket;
            let label = edit.background_label(true).unwrap();
            self.bytes -= command.bytes();
            *command = Command::CancelledEdit(ticket, label);
            self.bytes += command.bytes();
            return true;
        }
        false
    }
    /// Cancel this unpublished raster edit, preserving earlier document owners.
    pub(crate) fn cancel_edit(&mut self, ticket: u64) -> bool {
        if let Some(job) = &self.background_edit
            && job.ticket == ticket
        {
            job.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
            return true;
        }
        if let Some(command) = self.commands.iter_mut().find(|command| matches!(command, Command::Edit { ticket: pending, .. } if *pending == ticket))
            && let Command::Edit { edit, .. } = command
            && let Some(label) = edit.background_label(true) {
            self.bytes -= command.bytes();
            *command = Command::CancelledEdit(ticket, label);
            self.bytes += command.bytes();
            return true;
        }
        false
    }
    fn room(&self, command: &Command, extra_slots: usize) -> bool {
        let reserved = usize::from(self.input_open.is_some() || self.input_opacity.is_some());
        self.commands.len() + 1 + reserved + extra_slots <= self.command_limit
            && self
                .bytes
                .saturating_add(command.bytes())
                .saturating_add((reserved + extra_slots) * std::mem::size_of::<Command>())
                <= self.byte_limit
    }
    fn push(&mut self, command: Command) {
        self.bytes += command.bytes();
        self.commands.push_back(command);
    }
    fn pop(&mut self) -> Option<Command> {
        let command = self.commands.pop_front()?;
        self.bytes -= command.bytes();
        Some(command)
    }
    pub fn queued_input_bytes(&self) -> usize {
        self.bytes
    }
    pub fn busy(&self) -> bool {
        self.prepared_done.is_some()
            || self.active.is_some()
            || self.background_edit.is_some()
            || self.edit_settlement.is_some()
            || self.finishing_move.is_some()
            || self.opacity.is_some()
            || !self.commands.is_empty()
            || self.awaiting_barrier.is_some()
    }
    /// Like `busy`, but queued snapshots only read the document, so a save
    /// that finishes ahead of a later export's snapshot is still current.
    pub fn changes_pending(&self) -> bool {
        self.prepared_done.is_some()
            || self.active.is_some()
            || self.background_edit.is_some()
            || self.edit_settlement.is_some()
            || self.finishing_move.is_some()
            || self.opacity.is_some()
            || self.awaiting_barrier.is_some()
            || self
                .commands
                .iter()
                .any(|command| !matches!(command, Command::Snapshot(_)))
    }
    /// Queued dabs not yet painted, counted up to `limit`: how far
    /// painting trails the pen.
    pub fn queued_dabs(&self, limit: usize) -> usize {
        self.commands
            .iter()
            .filter(|c| matches!(c, Command::Dab { .. }))
            .take(limit)
            .count()
    }
    pub fn needs_work(&self) -> bool {
        if self.prepared_done.is_some() {
            return true;
        }
        if let Some(movement) = &self.finishing_move {
            return movement.cursor.is_some()
                || movement.canceled
                || !movement.engine.waiting_worker();
        }
        if self.edit_settlement.is_some() {
            return true;
        }
        if let Some(fill) = &self.background_edit {
            return fill.ready();
        }
        self.awaiting_barrier.is_none()
            && (!self.commands.is_empty()
                || self.active.as_ref().is_some_and(|active| {
                    active.cancel || !matches!(active.operation, Operation::Idle)
                }))
    }
    pub fn begin(&mut self, settings: Arc<Settings>) -> Result<StrokeId, String> {
        self.begin_target(settings, false, None)
    }
    /// Inherit the active layer after preceding queued document commands, while
    /// keeping brush/color/erase/symmetry captured at input delivery.
    pub fn begin_current(&mut self, settings: Arc<Settings>) -> Result<StrokeId, String> {
        self.begin_target(settings, true, None)
    }
    /// Keep the captured layer identity. A deleted/replaced or inactive target
    /// cancels this stroke rather than painting the layer that reused its index.
    pub fn begin_layer(
        &mut self,
        settings: Arc<Settings>,
        identity: crate::document::LayerId,
    ) -> Result<StrokeId, String> {
        self.begin_target(settings, false, Some(identity))
    }
    fn begin_target(
        &mut self,
        settings: Arc<Settings>,
        current_target: bool,
        identity: Option<crate::document::LayerId>,
    ) -> Result<StrokeId, String> {
        if self.input_open.is_some() || self.input_opacity.is_some() {
            return Err("End or cancel the current input stroke first".into());
        }
        settings.brush.validate()?;
        // Conservative metadata estimate plus retained asset/name capacities.
        // Queue entries are additional; active buffers/history/snapshots are not input.
        let bytes = serde_json::to_vec(&*settings)
            .map_err(|e| e.to_string())?
            .len()
            + settings
                .brush
                .image
                .as_ref()
                .map_or(0, |i| i.alpha.capacity())
            + settings
                .brush
                .grain
                .as_ref()
                .map_or(0, |i| i.alpha.capacity())
            + settings.brush.name.capacity()
            + settings.brush.tags.capacity()
            + settings.brush.graph.as_ref().map_or(0, |g| g.heap_bytes());
        let bytes = bytes
            + [
                &settings.brush.dynamics.size,
                &settings.brush.dynamics.opacity,
                &settings.brush.dynamics.flow,
                &settings.brush.dynamics.spacing,
            ]
            .into_iter()
            .flatten()
            .map(|d| {
                d.inputs.capacity() * std::mem::size_of::<crate::dynamics::Effector>()
                    + d.inputs
                        .iter()
                        .map(|i| i.curve.points.capacity() * std::mem::size_of::<[f32; 2]>())
                        .sum::<usize>()
            })
            .sum::<usize>();
        let id = self
            .next_id
            .checked_add(1)
            .ok_or("Stroke identifier exhausted")?;
        let command = Command::Begin {
            id,
            settings,
            bytes,
            current_target,
            identity,
        };
        if !self.room(&command, 1) {
            return Err("Rendering input queue is full".into());
        }
        self.next_id = id;
        self.push(command);
        self.input_open = Some(id);
        Ok(id)
    }
    pub fn dab(&mut self, id: StrokeId, sample: Sample) -> Result<(), String> {
        if self.input_open != Some(id) {
            return Err("Input stroke is no longer open".into());
        }
        if ![sample.x, sample.y, sample.pressure]
            .iter()
            .all(|v| v.is_finite())
            || !(0.0..=1.0).contains(&sample.pressure)
            || !sample.distance.is_finite()
            || sample.distance < 0.0
            || sample.time.is_some_and(|v| !v.is_finite() || v < 0.0)
            || [sample.speed, sample.random, sample.taper]
                .iter()
                .any(|v| v.is_some_and(|v| !v.is_finite() || !(0.0..=1.0).contains(&v)))
            || sample.spacing.is_some_and(|v| !v.is_finite() || v < 0.0)
            || sample
                .tilt
                .is_some_and(|v| v.iter().any(|v| !v.is_finite()))
            || sample.twist.is_some_and(|v| !v.is_finite())
        {
            self.cancel_open();
            return Err("Invalid queued dab canceled the affected stroke".into());
        }
        let command = Command::Dab { id, sample };
        if !self.room(&command, 0) {
            self.cancel_open();
            return Err("Rendering input overflow canceled the affected stroke".into());
        }
        self.push(command);
        Ok(())
    }
    pub fn end(&mut self, id: StrokeId) -> Result<(), String> {
        self.end_recorded(id, None)
    }
    /// End a stroke drawn on a vector layer: `record` joins the layer's
    /// strokes in the same history step as its pixels.
    pub fn end_recorded(
        &mut self,
        id: StrokeId,
        record: Option<Arc<crate::vector_layer::Stroke>>,
    ) -> Result<(), String> {
        if self.input_open != Some(id) {
            return Err("Input stroke is no longer open".into());
        }
        // Begin/dab reserve this control slot so a stroke cannot lose its end.
        self.push(Command::End(id, record));
        self.input_open = None;
        Ok(())
    }
    pub fn cancel_open(&mut self) -> bool {
        let Some(id) = self.input_open.take() else {
            return false;
        };
        self.commands.retain(|command| command.id() != Some(id));
        self.bytes = self.commands.iter().map(Command::bytes).sum();
        if let Some(active) = &mut self.active
            && active.id == id
        {
            active.cancel = true;
        } else {
            self.push(Command::Skipped(id));
        }
        true
    }
    fn control(&mut self, command: Command) -> Result<(), String> {
        if self.input_open.is_some() || self.input_opacity.is_some() {
            return Err("Close the input gesture before a document command".into());
        }
        if !self.room(&command, 0) {
            return Err("Rendering command queue is full".into());
        }
        self.push(command);
        Ok(())
    }
    pub fn undo(&mut self) -> Result<(), String> {
        self.control(Command::Undo)
    }
    pub fn redo(&mut self) -> Result<(), String> {
        self.control(Command::Redo)
    }
    fn ticket(&mut self, barrier: bool) -> Result<u64, String> {
        let ticket = self
            .next_ticket
            .checked_add(1)
            .ok_or("Rendering ticket exhausted")?;
        self.control(if barrier {
            Command::Barrier(ticket)
        } else {
            Command::Snapshot(ticket)
        })?;
        self.next_ticket = ticket;
        Ok(ticket)
    }
    pub fn snapshot(&mut self) -> Result<u64, String> {
        self.ticket(false)
    }
    pub fn barrier(&mut self) -> Result<u64, String> {
        self.ticket(true)
    }
    pub fn edit(&mut self, edit: crate::document_edits::Edit) -> Result<u64, String> {
        edit.validate_arguments()?;
        let ticket = self
            .next_ticket
            .checked_add(1)
            .ok_or("Rendering ticket exhausted")?;
        self.control(Command::Edit { ticket, edit })?;
        self.next_ticket = ticket;
        Ok(ticket)
    }
    pub fn start_opacity(
        &mut self,
        target: crate::document::LayerId,
        value: f32,
    ) -> Result<u64, String> {
        if self.input_open.is_some() || self.input_opacity.is_some() {
            return Err("Close the current gesture before an opacity drag".into());
        }
        if !value.is_finite() || !(0.0..=1.0).contains(&value) {
            return Err("Invalid opacity".into());
        }
        let ticket = self
            .next_ticket
            .checked_add(1)
            .ok_or("Rendering ticket exhausted")?;
        let command = Command::Opacity {
            ticket,
            target,
            value,
            start: true,
        };
        if !self.room(&command, 1) {
            return Err("Rendering command queue is full".into());
        }
        self.push(command);
        self.next_ticket = ticket;
        self.input_opacity = Some((ticket, target));
        Ok(ticket)
    }
    pub fn update_opacity(
        &mut self,
        ticket: u64,
        target: crate::document::LayerId,
        value: f32,
    ) -> Result<(), String> {
        if self.input_opacity != Some((ticket, target)) {
            return Err("Opacity gesture does not match".into());
        }
        if !value.is_finite() || !(0.0..=1.0).contains(&value) {
            return Err("Invalid opacity".into());
        }
        if let Some(Command::Opacity {
            ticket: pending,
            target: pending_target,
            value: pending_value,
            ..
        }) = self.commands.back_mut()
            && *pending == ticket
            && *pending_target == target
        {
            *pending_value = value;
            return Ok(());
        }
        let command = Command::Opacity {
            ticket,
            target,
            value,
            start: false,
        };
        if !self.room(&command, 0) {
            return Err("Rendering command queue is full".into());
        }
        self.push(command);
        Ok(())
    }
    pub fn end_opacity(&mut self, ticket: u64) -> Result<(), String> {
        if self.input_opacity.map(|(id, _)| id) != Some(ticket) {
            return Err("Opacity gesture does not match".into());
        }
        self.input_opacity = None;
        self.push(Command::OpacityEnd(ticket)); // Space was reserved at admission.
        Ok(())
    }
    /// Reserve a target before execution. Following commands may refer to it;
    /// if creation fails, those targets remain missing and cannot hit old artwork.
    pub fn new_layer(&mut self) -> Result<(u64, crate::document::LayerId), String> {
        let id = crate::document::LayerId::new();
        let ticket = self.edit(crate::document_edits::Edit::NewLayerWithId(id))?;
        Ok((ticket, id))
    }
    pub fn release_barrier(&mut self, ticket: u64) -> Result<(), String> {
        if self.awaiting_barrier != Some(ticket) {
            return Err("Rendering barrier ticket does not match".into());
        }
        self.awaiting_barrier = None;
        Ok(())
    }
    /// Raster work yields per row. Checkpoint/settlement, restoration/reclamation,
    /// snapshots and undo/redo are still synchronous control units, not time bounds.
    pub fn advance(
        &mut self,
        doc: &mut Document,
        history: &mut History,
        max_work: usize,
    ) -> Result<Progress, String> {
        self.advance_while(doc, history, max_work, || true)
    }
    pub(crate) fn set_profiling(&mut self, enabled: bool) {
        self.profile = enabled;
    }
    /// Check the UI deadline between units. A synchronous control unit or one
    /// raster row can overrun it; this is a yield deadline, not a hard time bound.
    pub fn advance_until(
        &mut self,
        doc: &mut Document,
        history: &mut History,
        max_work: usize,
        deadline: std::time::Instant,
    ) -> Result<Progress, String> {
        self.advance_while(doc, history, max_work, || {
            std::time::Instant::now() < deadline
        })
    }
    fn advance_while(
        &mut self,
        doc: &mut Document,
        history: &mut History,
        max_work: usize,
        mut can_continue: impl FnMut() -> bool,
    ) -> Result<Progress, String> {
        let mut progress = Progress::default();
        if let Some(ticket) = self.awaiting_barrier {
            progress.barrier = Some(ticket);
            return Ok(progress);
        }
        while progress.work_units < max_work && can_continue() {
            if let Some(done) = self.prepared_done.take() {
                progress.edits.push((done.0, Ok(done.1)));
                progress.work_units += 1;
                continue;
            }
            let started = self.profile.then(std::time::Instant::now);
            if let Some(movement) = &mut self.finishing_move {
                if movement.cursor.is_none()
                    && !movement.canceled
                    && movement.engine.waiting_worker()
                {
                    break;
                }
                if let Some(cursor) = &mut movement.cursor {
                    let step = cursor.step(history, doc, 1);
                    progress.work_units += 1;
                    progress.settlement_payload_peak_bytes = progress
                        .settlement_payload_peak_bytes
                        .max(cursor.peak_payload_capacity_bytes());
                    match step {
                        Ok(step) if !step.complete => {
                            record_timing(&mut progress, 2, started);
                            continue;
                        }
                        Ok(_) => {}
                        Err(error) => {
                            cursor.retain_full(history, doc);
                            let movement = self.finishing_move.take().unwrap();
                            progress.move_done = Some(MoveProgress {
                                affine: movement.engine.is_affine(),
                                offset: movement.engine.applied,
                                changed: movement.engine.net_changed,
                                canceled: false,
                                modified_before: movement.modified_before,
                                error: Some(format!(
                                    "Move published; history settlement failed: {error}"
                                )),
                            });
                            break;
                        }
                    }
                    let movement = self.finishing_move.take().unwrap();
                    progress.move_done = Some(MoveProgress {
                        affine: movement.engine.is_affine(),
                        offset: movement.engine.applied,
                        changed: movement.engine.net_changed,
                        canceled: false,
                        modified_before: movement.modified_before,
                        error: None,
                    });
                    break;
                }
                let result = if movement.canceled {
                    movement.engine.guard(doc).map(|()| None)
                } else {
                    movement.engine.poll(doc)
                };
                match result {
                    Err(error) => {
                        if movement.checkpointed {
                            history.retain_gesture_snapshot(movement.engine.original(), doc);
                        }
                        let movement = self.finishing_move.take().unwrap();
                        progress.work_units += 1;
                        progress.move_done = Some(MoveProgress {
                            affine: movement.engine.is_affine(),
                            offset: movement.engine.applied,
                            changed: movement.engine.net_changed,
                            canceled: false,
                            modified_before: movement.modified_before,
                            error: Some(error),
                        });
                        break;
                    }
                    Ok(Some(candidate)) => {
                        let changed = candidate.changed;
                        if changed && !movement.checkpointed {
                            history.begin_gesture_checkpoint(doc);
                            movement.checkpointed = true;
                        }
                        movement.engine.publish(doc, candidate)?;
                        progress.work_units += 1;
                        if changed {
                            progress.damage.include([0, 0, doc.width, doc.height]);
                        }
                    }
                    Ok(None) => {
                        progress.work_units += 1;
                        if !movement.canceled && !movement.engine.finished() {
                            break;
                        }
                    }
                }
                if movement.canceled {
                    if movement.checkpointed {
                        let source = movement.engine.original();
                        doc.layers[doc.active].pixels = source.layers[source.active].pixels.clone();
                        doc.selection = source.selection;
                        doc.layers[doc.active].mask = source.layers[source.active].mask.clone();
                        doc.selection_mask = source.selection_mask.clone();
                        history.discard_pending_checkpoint(doc);
                        progress.damage.include([0, 0, doc.width, doc.height]);
                    }
                } else if movement.engine.net_changed && movement.checkpointed {
                    // Vector strokes follow before history keeps the result.
                    movement.engine.follow_vector(doc);
                    movement.engine.follow_text(doc);
                    progress.damage.include([0, 0, doc.width, doc.height]);
                    if let Some(mut cursor) = history.begin_settle(doc) {
                        cursor.prefer_smaller_full_snapshot();
                        movement.cursor = Some(cursor);
                        continue;
                    }
                    history.retain_gesture_snapshot(movement.engine.original(), doc);
                } else if movement.checkpointed {
                    history.discard_pending_checkpoint(doc);
                }
                let movement = self.finishing_move.take().unwrap();
                progress.move_done = Some(MoveProgress {
                    affine: movement.engine.is_affine(),
                    offset: movement.engine.applied,
                    changed: !movement.canceled && movement.engine.net_changed,
                    canceled: movement.canceled,
                    modified_before: movement.modified_before,
                    error: None,
                });
                // Restore net-zero/canceled unsaved state before later commands run.
                break;
            }
            if let Some(settlement) = &mut self.edit_settlement {
                match settlement.cursor.step(history, doc, 1) {
                    Ok(step) => {
                        progress.work_units += step.work_units;
                        progress.settlement_payload_peak_bytes = progress
                            .settlement_payload_peak_bytes
                            .max(settlement.cursor.peak_payload_capacity_bytes());
                        if step.complete {
                            let settlement = self.edit_settlement.take().unwrap();
                            progress
                                .edits
                                .push((settlement.ticket, Ok(settlement.outcome)));
                        }
                    }
                    Err(error) => {
                        let settlement = self.edit_settlement.take().unwrap();
                        settlement.cursor.retain_full(history, doc);
                        progress.work_units += 1;
                        progress.edits.push((
                            settlement.ticket,
                            Err(format!(
                                "Edit published; history compaction failed: {error}"
                            )),
                        ));
                    }
                }
                record_timing(&mut progress, 2, started);
                continue;
            }
            if let Some(job) = &self.background_edit {
                if !job.ready() {
                    break;
                }
                let mut job = self.background_edit.take().unwrap();
                job.join();
                progress.work_units += 1;
                let result = if job.cancel.load(std::sync::atomic::Ordering::Relaxed) {
                    Err(format!("{} canceled", job.label))
                } else if !crate::document::same_snapshot(doc, &job.before) {
                    Err(format!("{} source changed before publication", job.label))
                } else {
                    job.result.take().unwrap()
                };
                match result {
                    Ok((updated, outcome)) => {
                        if outcome.changed {
                            history.begin_gesture_checkpoint(doc);
                            *doc = updated;
                            if outcome.redraw {
                                progress.damage.include([0, 0, doc.width, doc.height]);
                            }
                            if let Some(mut cursor) = history.begin_settle(doc) {
                                cursor.prefer_smaller_full_snapshot();
                                self.edit_settlement = Some(EditSettlement {
                                    ticket: job.ticket,
                                    outcome,
                                    cursor,
                                });
                            } else {
                                history.settle(doc);
                                progress.edits.push((job.ticket, Ok(outcome)));
                            }
                        } else {
                            progress.edits.push((job.ticket, Ok(outcome)));
                        }
                    }
                    Err(error) => progress.edits.push((job.ticket, Err(error))),
                }
                continue;
            }
            if let Some(active) = &mut self.active {
                if active.cancel {
                    progress.work_units += 1;
                    active.operation = Operation::Restore(Box::new(
                        std::mem::take(&mut active.buffer).begin_restore(doc),
                    ));
                    active.cancel = false;
                    record_timing(&mut progress, 3, started);
                    continue;
                }
                match &mut active.operation {
                    Operation::Compact(cursor) => {
                        progress.finished_scratch_released_bytes +=
                            active.buffer.retire_finished_scratch(active.retired_prefix);
                        active.retired_prefix += 1;
                        let step = match cursor.step(history, doc, 1) {
                            Ok(step) => step,
                            Err(error) => {
                                progress.history_error = Some(error);
                                return Ok(progress);
                            }
                        };
                        progress.work_units += step.work_units;
                        progress.settlement_payload_peak_bytes = progress
                            .settlement_payload_peak_bytes
                            .max(cursor.peak_payload_capacity_bytes());
                        if step.complete {
                            let active = self.active.take().unwrap();
                            progress.completed.push(Completed {
                                id: active.id,
                                changed: true,
                                cancelled: false,
                            });
                            drop(active);
                            record_timing(&mut progress, 2, started);
                            break;
                        }
                        record_timing(&mut progress, 2, started);
                        continue;
                    }
                    Operation::Restore(cursor) => {
                        let step = match cursor.step(doc, 1) {
                            Ok(step) => step,
                            Err(error) => {
                                progress.render_error = Some(error);
                                return Ok(progress);
                            }
                        };
                        progress.work_units += step.work_units;
                        progress.damage.merge(step.changed_region);
                        if step.complete {
                            let active = self.active.take().unwrap();
                            history.discard_pending_checkpoint(doc);
                            if let Some(error) = &active.failure {
                                progress.render_error = Some(error.clone());
                            }
                            progress.completed.push(Completed {
                                id: active.id,
                                changed: false,
                                cancelled: true,
                            });
                            drop(active);
                            record_timing(&mut progress, 3, started);
                            break;
                        }
                        record_timing(&mut progress, 3, started);
                        continue;
                    }
                    Operation::Dab(cursor) => {
                        let step = match cursor.step(&mut active.buffer, doc, 1) {
                            Ok(step) => step,
                            Err(error) => {
                                progress.render_error = Some(error);
                                return Ok(progress);
                            }
                        };
                        progress.work_units += step.work_units;
                        progress.dabs += step.painted.dabs;
                        progress.damage.merge(step.changed_region);
                        if step.complete {
                            active.operation = Operation::Idle;
                        }
                        record_timing(&mut progress, 0, started);
                        continue;
                    }
                    Operation::Finish(cursor) => {
                        progress.work_units += 1;
                        if let Some(cursor) = cursor {
                            let step = match cursor.step(&mut active.buffer, doc, 1) {
                                Ok(step) => step,
                                Err(error) => {
                                    progress.render_error = Some(error);
                                    return Ok(progress);
                                }
                            };
                            progress.dabs += step.painted.dabs;
                            progress.damage.merge(step.changed_region);
                            if step.complete {
                                active.operation = Operation::Settle;
                            }
                        } else {
                            active.operation = Operation::Settle;
                        }
                        record_timing(&mut progress, 1, started);
                        continue;
                    }
                    Operation::Settle => {
                        progress.work_units += 1;
                        active.buffer.reclaim_empty_tiles(doc);
                        if doc.version == 2
                            && let Err(error) = doc.validate()
                        {
                            active.failure = Some(format!("Stroke rolled back: {error}"));
                            active.cancel = true;
                            active.operation = Operation::Idle;
                            continue;
                        }
                        let changed = active.buffer.has_visible_changes(doc);
                        // The recording joins the layer before history keeps it.
                        if changed
                            && let Some(record) = active.record.take()
                            && let Some(rect) =
                                crate::vector_layer::land_stroke(doc, active.settings.layer, record)
                        {
                            progress.damage.include(rect);
                        }
                        if changed && let Some(cursor) = history.begin_settle(doc) {
                            active.operation = Operation::Compact(Box::new(cursor));
                            record_timing(&mut progress, 2, started);
                            continue;
                        }
                        let active = self.active.take().unwrap();
                        if changed {
                            history.settle(doc);
                        } else {
                            history.discard_pending_checkpoint(doc);
                        }
                        progress.completed.push(Completed {
                            id: active.id,
                            changed,
                            cancelled: false,
                        });
                        drop(active);
                        record_timing(&mut progress, 2, started);
                        break;
                    }
                    Operation::Idle => {}
                }
            }
            // Capture gesture state after prior commands have reached the app.
            if progress.work_units != 0
                && matches!(
                    self.commands.front(),
                    Some(Command::Opacity { .. } | Command::OpacityEnd(_) | Command::Begin { .. })
                )
            {
                break;
            }
            let Some(command) = self.pop() else {
                break;
            };
            progress.work_units += 1;
            match command {
                Command::CancelledEdit(ticket, label) => progress
                    .edits
                    .push((ticket, Err(format!("{label} canceled")))),
                Command::Opacity {
                    ticket,
                    target,
                    value,
                    start,
                } => {
                    if self.active.is_some() {
                        return Err("Opacity encountered an unfinished stroke".into());
                    }
                    if start {
                        if self.opacity.is_some() {
                            return Err("Opacity encountered an unfinished drag".into());
                        }
                        self.opacity = Some(ActiveOpacity {
                            ticket,
                            target,
                            original: doc
                                .layers
                                .iter()
                                .find(|layer| layer.id == target)
                                .map(|layer| layer.opacity),
                            checkpoint: false,
                            failed: false,
                        });
                    }
                    let active = self
                        .opacity
                        .as_mut()
                        .ok_or("Opacity update has no gesture")?;
                    if active.ticket != ticket || active.target != target {
                        return Err("Opacity update target does not match".into());
                    }
                    let mut event = OpacityProgress {
                        ticket,
                        started: start,
                        ended: false,
                        changed: false,
                        error: None,
                    };
                    if !active.failed {
                        let edit = crate::document_edits::Edit::Property {
                            target: crate::document_edits::Target::Identity(target),
                            property: crate::document_edits::Property::Opacity(value),
                        };
                        match edit.prepare(doc) {
                            Ok((updated, outcome)) => {
                                if outcome.changed {
                                    if !active.checkpoint {
                                        history.begin_gesture_checkpoint(doc);
                                        active.checkpoint = true;
                                    }
                                    *doc = updated;
                                    event.changed = true;
                                }
                            }
                            Err(error) => {
                                active.failed = true;
                                event.error = Some(error);
                            }
                        }
                    }
                    progress.opacity = Some(event);
                    break;
                }
                Command::OpacityEnd(ticket) => {
                    let active = self.opacity.take().ok_or("Opacity end has no gesture")?;
                    if active.ticket != ticket {
                        self.opacity = Some(active);
                        return Err("Opacity end does not match".into());
                    }
                    let changed = active
                        .original
                        .zip(
                            doc.layers
                                .iter()
                                .find(|layer| layer.id == active.target)
                                .map(|layer| layer.opacity),
                        )
                        .is_some_and(|(original, current)| original != current);
                    if active.checkpoint {
                        if changed {
                            history.settle(doc);
                        } else {
                            history.discard_pending_checkpoint(doc);
                        }
                    }
                    progress.opacity = Some(OpacityProgress {
                        ticket,
                        started: false,
                        ended: true,
                        changed,
                        error: None,
                    });
                    break;
                }
                Command::Begin {
                    id,
                    mut settings,
                    current_target,
                    identity,
                    ..
                } => {
                    if current_target && settings.layer != doc.active {
                        Arc::make_mut(&mut settings).layer = doc.active;
                    }
                    let missing_identity = if let Some(identity) = identity {
                        if let Some(index) =
                            doc.layers.iter().position(|layer| layer.id == identity)
                        {
                            if settings.layer != index {
                                Arc::make_mut(&mut settings).layer = index;
                            }
                            false
                        } else {
                            true
                        }
                    } else {
                        false
                    };
                    if self.active.is_some()
                        || missing_identity
                        || settings.layer != doc.active
                        || settings.layer >= doc.layers.len()
                        || settings
                            .target
                            .blocker(doc, doc.active, settings.erase)
                            .is_some()
                    {
                        self.commands.retain(|command| command.id() != Some(id));
                        self.bytes = self.commands.iter().map(Command::bytes).sum();
                        if self.input_open == Some(id) {
                            self.input_open = None;
                        }
                        progress.completed.push(Completed {
                            id,
                            changed: false,
                            cancelled: true,
                        });
                        progress.render_error =
                            Some("Queued stroke target changed before rendering".into());
                        return Ok(progress);
                    }
                    history.begin_stroke_checkpoint(doc);
                    self.active = Some(Active {
                        id,
                        settings,
                        buffer: Default::default(),
                        operation: Operation::Idle,
                        cancel: false,
                        retired_prefix: 0,
                        failure: None,
                        record: None,
                    });
                    progress.started = Some(id);
                    record_timing(&mut progress, 4, started);
                    break;
                }
                Command::Dab { id, sample } => {
                    let active = self.active.as_mut().ok_or("Queued dab has no stroke")?;
                    if active.id != id {
                        return Err("Queued dab identifier does not match".into());
                    }
                    active.operation = Operation::Dab(Box::new(
                        active
                            .buffer
                            .begin_dab(doc, sample, active.settings.clone())
                            .ok_or("Queued dab target changed")?,
                    ));
                }
                Command::End(id, record) => {
                    let active = self.active.as_mut().ok_or("Queued end has no stroke")?;
                    if active.id != id {
                        return Err("Queued end identifier does not match".into());
                    }
                    active.record = record;
                    active.operation = Operation::Finish(
                        active
                            .buffer
                            .begin_tail(doc, &active.settings)
                            .map(Box::new),
                    );
                }
                Command::Skipped(id) => progress.completed.push(Completed {
                    id,
                    changed: false,
                    cancelled: true,
                }),
                Command::Undo | Command::Redo => {
                    if self.active.is_some() {
                        return Err("Document command encountered an unfinished stroke".into());
                    }
                    let undo = matches!(command, Command::Undo);
                    let changed = if undo {
                        history.undo(doc)
                    } else {
                        history.redo(doc)
                    };
                    progress.history_actions.push((undo, changed));
                    if changed {
                        progress.damage.include([0, 0, doc.width, doc.height]);
                    }
                    let error = history.take_error();
                    if progress.history_error.is_none() {
                        progress.history_error = error;
                    }
                    record_timing(
                        &mut progress,
                        if matches!(command, Command::Undo) {
                            5
                        } else {
                            6
                        },
                        started,
                    );
                }
                Command::Snapshot(ticket) => {
                    if self.active.is_some() {
                        return Err("Snapshot encountered an unfinished stroke".into());
                    }
                    progress.snapshot = Some((ticket, doc.clone()));
                    record_timing(&mut progress, 7, started);
                    break;
                }
                Command::Edit { ticket, edit } => {
                    if self.active.is_some() {
                        return Err("Edit encountered an unfinished stroke".into());
                    }
                    if let Some(label) = edit.background_label(
                        doc.selection_mask.is_some() || doc.layers.iter().any(|l| l.mask.is_some()),
                    ) {
                        let before = doc.clone();
                        let source = before.clone();
                        let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
                        let control = cancel.clone();
                        match std::thread::Builder::new()
                            .name(format!("dudel-{}", label.to_lowercase()))
                            .spawn(move || edit.prepare_control(&source, Some(&control)))
                        {
                            Ok(worker) => {
                                self.background_edit = Some(EditJob {
                                    ticket,
                                    label,
                                    before,
                                    cancel,
                                    worker: Some(worker),
                                    result: None,
                                })
                            }
                            Err(error) => progress
                                .edits
                                .push((ticket, Err(format!("Could not start {label}: {error}")))),
                        }
                        break;
                    }
                    match edit.prepare(doc) {
                        Ok((updated, outcome)) => {
                            if outcome.changed {
                                if outcome.undoable {
                                    history.checkpoint(doc);
                                }
                                *doc = updated;
                                if outcome.undoable {
                                    history.settle(doc);
                                }
                                if outcome.redraw {
                                    progress.damage.include([0, 0, doc.width, doc.height]);
                                }
                            }
                            progress.edits.push((ticket, Ok(outcome)));
                        }
                        Err(error) => progress.edits.push((ticket, Err(error))),
                    }
                }
                Command::Barrier(ticket) => {
                    if self.active.is_some() {
                        return Err("Barrier encountered an unfinished stroke".into());
                    }
                    self.awaiting_barrier = Some(ticket);
                    progress.barrier = Some(ticket);
                    break;
                }
            }
        }
        Ok(progress)
    }
}

impl Drop for EditJob {
    fn drop(&mut self) {
        self.cancel
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

/// Headless owner scheduling probe. Worker joins are outside owner-call timing;
/// wall time includes them. Does not measure display, OS IO or GUI/input latency.
pub(crate) fn measure_edit(
    source: &Document,
    edit: crate::document_edits::Edit,
) -> Result<(Document, History, serde_json::Value), String> {
    let mut doc = source.clone();
    let mut history = History::new();
    let mut queue = Queue::default();
    let wall = std::time::Instant::now();
    let started = std::time::Instant::now();
    let ticket = queue.edit(edit)?;
    let admission_ms = started.elapsed().as_secs_f64() * 1000.0;
    let snapshot_ticket = queue.snapshot()?;
    let mut snapshot = None;
    let mut completed = false;
    let mut calls = 0;
    let mut max_owner_ms = 0.0_f64;
    let mut damage_calls = 0;
    while queue.busy() {
        if queue.waiting_edit() {
            queue.wait_edit();
        }
        let started = std::time::Instant::now();
        let progress = queue.advance_until(
            &mut doc,
            &mut history,
            4096,
            started + std::time::Duration::from_millis(2),
        )?;
        max_owner_ms = max_owner_ms.max(started.elapsed().as_secs_f64() * 1000.0);
        calls += 1;
        if calls > 1_000_000 {
            return Err("Background edit probe did not finish".into());
        }
        damage_calls += usize::from(!progress.damage.is_empty());
        for (id, result) in progress.edits {
            result?;
            if id == ticket {
                completed = true;
            }
        }
        if let Some(value) = progress.snapshot {
            snapshot = Some(value);
        }
    }
    let wall_ms = wall.elapsed().as_secs_f64() * 1000.0;
    let (id, saved) = snapshot.ok_or("Missing background edit snapshot")?;
    if !completed || id != snapshot_ticket || !crate::document::same_snapshot(&saved, &doc) {
        return Err("Background edit completion/snapshot differs".into());
    }
    Ok((
        doc,
        history,
        serde_json::json!({"admission_ms":admission_ms,"max_owner_advance_ms":max_owner_ms,"owner_calls":calls,"wall_ms":wall_ms,"damage_calls":damage_calls,"completion_and_snapshot_verified":true}),
    ))
}

/// Headless scheduling fixture. This is a CPU queue check, not UI/tablet latency.
pub fn benchmark(path: &std::path::Path) -> Result<(), String> {
    #[derive(serde::Serialize)]
    struct Measurement {
        canvas_size: usize,
        dense_source: bool,
        flat_source: bool,
        brush_size: f32,
        radial_copies: u8,
        work_budget: usize,
        deadline_ms: f64,
        calls: usize,
        max_advance_ms: f64,
        total_wall_ms: f64,
        queued_input_bytes: usize,
        completed_strokes: usize,
        canceled_strokes: usize,
        snapshots_verified: usize,
        pixels_verified_per_snapshot: usize,
        phase_timings: std::collections::BTreeMap<&'static str, Timing>,
        rollback_calls: usize,
        rollback_wall_ms: f64,
        rollback_max_advance_ms: f64,
        rollback_verified: bool,
        settlement_payload_peak_bytes: usize,
        tracked_live_document_bytes_peak: usize,
        tracked_history_and_settlement_bytes_beyond_live_peak: usize,
        settlement_bytes_beyond_live_and_history_peak: usize,
        stroke_working_bytes_peak: usize,
        tracked_document_history_settlement_and_stroke_bytes_peak: usize,
        finished_scratch_released_bytes: usize,
        edit_pipeline_verified: bool,
    }
    let mut measurements = vec![];
    for (canvas_size, dense_source, flat_source, size, radial_copies) in [
        (512, false, false, 18.0, 1),
        (512, false, false, 300.0, 4),
        (2048, true, true, 18.0, 1),
        (2048, true, true, 300.0, 4),
        (2048, true, false, 18.0, 1),
        (2048, true, false, 300.0, 4),
    ] {
        let mut doc = Document::new(canvas_size, canvas_size);
        if dense_source {
            let packed = vec![255; canvas_size * canvas_size * 4];
            doc.layers[0].pixels = if flat_source {
                packed.into()
            } else {
                crate::pixels::Pixels::read_rgba(&mut packed.as_slice(), canvas_size, canvas_size)
                    .map_err(|e| e.to_string())?
            };
        }
        let mut history = History::new();
        let make_settings = |color| {
            Arc::new(Settings {
                target: crate::paint_target::Target::Raster,
                brush: crate::brush::Brush {
                    size,
                    pressure_size: false,
                    hardness: if size == 18.0 { 1.0 } else { 0.0 },
                    taper_out: 80.0,
                    flow: 0.3,
                    opacity: 0.6,
                    ..Default::default()
                },
                color,
                secondary: color,
                erase: false,
                layer: 0,
                symmetry: crate::symmetry::Symmetry {
                    mode: if radial_copies == 1 {
                        crate::symmetry::Mode::Off
                    } else {
                        crate::symmetry::Mode::Radial
                    },
                    segments: radial_copies.max(2),
                    ..Default::default()
                },
            })
        };
        let first = make_settings([15, 70, 130]);
        let second = make_settings([190, 50, 25]);
        let samples = |settings: &Settings, reverse: bool| {
            let mut stroke = crate::stroke::Stroke::default();
            let mut dabs = vec![];
            for i in 0..40 {
                let sample = Sample {
                    x: if reverse {
                        410.0 - i as f32 * 8.0
                    } else {
                        100.0 + i as f32 * 8.0
                    },
                    y: 245.0 + (i as f32 * 0.2).sin() * 30.0,
                    pressure: 1.0,
                    time: Some(i as f64 * 0.01),
                    ..Default::default()
                };
                dabs.extend(stroke.sample(sample, &settings.brush));
                if i == 39 {
                    dabs.extend(stroke.finish(sample, &settings.brush));
                }
            }
            dabs
        };
        let a = samples(&first, false);
        let b = samples(&second, true);
        let paint_reference = |doc: &mut Document, settings: &Settings, samples: &[Sample]| {
            let mut buffer = StrokeBuffer::default();
            for &sample in samples {
                buffer.paint(doc, sample, settings);
            }
            buffer.finish_tail(doc, settings);
            buffer.reclaim_empty_tiles(doc);
        };
        let mut expected_a = doc.clone();
        paint_reference(&mut expected_a, &first, &a);
        let mut expected_ab = expected_a.clone();
        paint_reference(&mut expected_ab, &second, &b);
        let mut queue = Queue {
            profile: true,
            ..Default::default()
        };
        let id_a = queue.begin(first.clone())?;
        for &sample in &a {
            queue.dab(id_a, sample)?;
        }
        queue.end(id_a)?;
        let snapshot_a = queue.snapshot()?;
        let id_b = queue.begin_current(second.clone())?;
        for &sample in &b {
            queue.dab(id_b, sample)?;
        }
        queue.end(id_b)?;
        queue.undo()?;
        let snapshot_undo = queue.snapshot()?;
        queue.redo()?;
        let barrier = queue.barrier()?;
        let queued_input_bytes = queue.queued_input_bytes();
        let work_budget = 32;
        let deadline_ms = 2.0;
        let mut calls = 0;
        let mut max_advance_ms = 0.0f64;
        let mut completed_strokes = 0;
        let mut canceled_strokes = 0;
        let mut snapshots_verified = 0;
        let mut timings = [Timing::default(); 8];
        let mut settlement_payload_peak_bytes = 0;
        let mut tracked_live_document_bytes_peak = 0;
        let mut tracked_history_and_settlement_bytes_beyond_live_peak = 0;
        let mut settlement_bytes_beyond_live_and_history_peak = 0;
        let mut stroke_working_bytes_peak = 0;
        let mut tracked_document_history_settlement_and_stroke_bytes_peak = 0;
        let mut finished_scratch_released_bytes = 0;
        let started = std::time::Instant::now();
        while queue.busy() {
            if !queue.needs_work() {
                return Err("Queue unexpectedly waits for input or a fence".into());
            }
            let call = std::time::Instant::now();
            let progress = queue.advance_until(
                &mut doc,
                &mut history,
                work_budget,
                call + std::time::Duration::from_millis(2),
            )?;
            max_advance_ms = max_advance_ms.max(call.elapsed().as_secs_f64() * 1000.0);
            // Allocation sampling is outside owner-call timing, inside drain wall.
            let mut registry = crate::snapshot_memory::Registry::new(&doc);
            history.register_allocations(&mut registry);
            let retained = registry.additional_bytes();
            queue.register_settlement_allocations(&mut registry);
            tracked_live_document_bytes_peak =
                tracked_live_document_bytes_peak.max(registry.live_bytes());
            tracked_history_and_settlement_bytes_beyond_live_peak =
                tracked_history_and_settlement_bytes_beyond_live_peak
                    .max(registry.additional_bytes());
            settlement_bytes_beyond_live_and_history_peak =
                settlement_bytes_beyond_live_and_history_peak
                    .max(registry.additional_bytes() - retained);
            calls += 1;
            let stroke_bytes = queue.stroke_working_bytes();
            stroke_working_bytes_peak = stroke_working_bytes_peak.max(stroke_bytes);
            registry.add_owned(stroke_bytes);
            tracked_document_history_settlement_and_stroke_bytes_peak =
                tracked_document_history_settlement_and_stroke_bytes_peak
                    .max(registry.live_bytes() + registry.additional_bytes());
            finished_scratch_released_bytes += progress.finished_scratch_released_bytes;
            settlement_payload_peak_bytes =
                settlement_payload_peak_bytes.max(progress.settlement_payload_peak_bytes);
            for (total, sample) in timings.iter_mut().zip(progress.timings) {
                total.units += sample.units;
                total.total_ms += sample.total_ms;
                total.max_ms = total.max_ms.max(sample.max_ms);
            }
            if let Some(error) = progress.render_error.or(progress.history_error) {
                return Err(error);
            }
            for completed in progress.completed {
                if completed.id != id_a && completed.id != id_b {
                    return Err("Unexpected completed stroke identifier".into());
                }
                if completed.cancelled {
                    canceled_strokes += 1;
                } else if completed.changed {
                    completed_strokes += 1;
                }
            }
            if let Some((ticket, snapshot)) = progress.snapshot {
                if ticket != snapshot_a && ticket != snapshot_undo {
                    return Err("Unexpected snapshot ticket".into());
                }
                if snapshot.layers[0].pixels != expected_a.layers[0].pixels {
                    return Err("Queued snapshot differs from completed first stroke".into());
                }
                snapshots_verified += 1;
            }
            if let Some(ticket) = progress.barrier {
                if ticket != barrier || doc.layers[0].pixels != expected_ab.layers[0].pixels {
                    return Err("Queued redo/barrier differs from both strokes".into());
                }
                queue.release_barrier(ticket)?;
            }
        }
        let total_wall_ms = started.elapsed().as_secs_f64() * 1000.0;
        if completed_strokes != 2 || canceled_strokes != 0 || snapshots_verified != 2 {
            return Err("Missing queued completion/snapshot".into());
        }
        let original = doc.layers[0].pixels.clone();
        let cancelled = queue.begin(make_settings([70, 200, 40]))?;
        for &sample in &a {
            queue.dab(cancelled, sample)?;
        }
        // Finish the accepted live input, retaining an open logical contact.
        // Preparation and painting are excluded from the rollback-only timer.
        while queue.needs_work() {
            let step = queue.advance_until(
                &mut doc,
                &mut history,
                work_budget,
                std::time::Instant::now() + std::time::Duration::from_millis(2),
            )?;
            if let Some(error) = step.render_error.or(step.history_error) {
                return Err(error);
            }
        }
        if doc.layers[0].pixels == original || !queue.cancel_open() {
            return Err("Rollback fixture did not modify pixels before cancel".into());
        }
        let rollback_started = std::time::Instant::now();
        let mut rollback_calls = 0;
        let mut rollback_max_advance_ms = 0.0f64;
        let mut cancellations = 0;
        while queue.needs_work() {
            let call = std::time::Instant::now();
            let step = queue.advance_until(
                &mut doc,
                &mut history,
                work_budget,
                call + std::time::Duration::from_millis(2),
            )?;
            rollback_max_advance_ms =
                rollback_max_advance_ms.max(call.elapsed().as_secs_f64() * 1000.0);
            rollback_calls += 1;
            if let Some(error) = step.render_error.or(step.history_error) {
                return Err(error);
            }
            for completed in step.completed {
                if completed.id != cancelled || !completed.cancelled || completed.changed {
                    return Err("Unexpected rollback completion".into());
                }
                cancellations += 1;
            }
        }
        let rollback_wall_ms = rollback_started.elapsed().as_secs_f64() * 1000.0;
        if cancellations != 1
            || queue.busy()
            || history.has_pending_stroke()
            || doc.layers[0].pixels != original
        {
            return Err("Rollback differs from original completed artwork".into());
        }
        // Check typed document commands and the active target inherited by a
        // following stroke. This sequence is outside preceding timing scopes.
        use crate::document_edits::{Edit, Property, Target};
        let failed = queue.edit(Edit::SelectLayer(usize::MAX))?;
        let (new_layer, layer_id) = queue.new_layer()?;
        let rename = queue.edit(Edit::Property {
            target: Target::Identity(layer_id),
            property: Property::Name("Queued ink".into()),
        })?;
        let opacity = queue.edit(Edit::Property {
            target: Target::Active,
            property: Property::Opacity(0.5),
        })?;
        let inherited = queue.begin_layer(first.clone(), layer_id)?;
        queue.dab(inherited, a[0])?;
        queue.end(inherited)?;
        let edited_snapshot = queue.snapshot()?;
        let mut expected = doc.clone();
        expected.insert_layer_above()?;
        expected.layers[1].name = "Queued ink".into();
        expected.layers[1].opacity = 0.5;
        let mut inherited_settings = (*first).clone();
        inherited_settings.layer = 1;
        paint_reference(&mut expected, &inherited_settings, &a[..1]);
        let mut edit_count = 0;
        let mut inherited_completed = false;
        let mut snapshot_verified = false;
        while queue.needs_work() {
            let step = queue.advance_until(
                &mut doc,
                &mut history,
                work_budget,
                std::time::Instant::now() + std::time::Duration::from_millis(2),
            )?;
            if let Some(error) = step.render_error.or(step.history_error) {
                return Err(error);
            }
            for (ticket, result) in step.edits {
                if ticket == failed {
                    if result.is_ok() {
                        return Err("Invalid edit unexpectedly committed".into());
                    }
                } else if [new_layer, rename, opacity].contains(&ticket) {
                    if !result?.changed {
                        return Err("Expected edit did not commit".into());
                    }
                } else {
                    return Err("Unknown edit ticket".into());
                }
                edit_count += 1;
            }
            for completed in step.completed {
                if completed.id != inherited || completed.cancelled || !completed.changed {
                    return Err("Inherited stroke did not commit".into());
                }
                inherited_completed = true;
            }
            if let Some((ticket, snapshot)) = step.snapshot {
                if ticket != edited_snapshot
                    || snapshot.layers.len() != 2
                    || snapshot.active != 1
                    || snapshot.layers[1].id != layer_id
                    || snapshot.layers[1].name != "Queued ink"
                    || snapshot.layers[1].opacity != 0.5
                    || snapshot.layers[0].pixels != expected.layers[0].pixels
                    || snapshot.layers[1].pixels != expected.layers[1].pixels
                {
                    return Err("Edited snapshot differs from ordered reference".into());
                }
                snapshot_verified = true;
            }
        }
        if edit_count != 4 || !inherited_completed || !snapshot_verified || queue.busy() {
            return Err("Missing edit pipeline result".into());
        }
        measurements.push(Measurement {
            canvas_size,
            dense_source,
            flat_source,
            brush_size: size,
            radial_copies,
            work_budget,
            deadline_ms,
            calls,
            max_advance_ms,
            total_wall_ms,
            queued_input_bytes,
            completed_strokes,
            canceled_strokes,
            snapshots_verified,
            pixels_verified_per_snapshot: doc.width * doc.height,
            phase_timings: PHASES.into_iter().zip(timings).collect(),
            rollback_calls,
            rollback_wall_ms,
            rollback_max_advance_ms,
            rollback_verified: true,
            settlement_payload_peak_bytes,
            tracked_live_document_bytes_peak,
            tracked_history_and_settlement_bytes_beyond_live_peak,
            settlement_bytes_beyond_live_and_history_peak,
            stroke_working_bytes_peak,
            tracked_document_history_settlement_and_stroke_bytes_peak,
            finished_scratch_released_bytes,
            edit_pipeline_verified: true,
        });
    }
    let data = serde_json::to_vec_pretty(&serde_json::json!({"version": 9, "measurements": measurements,
        "stroke_scope": "Exclusive active stroke buffers and cursors are sampled outside owner-call timing, inside drain wall: remaining original/coverage blocks, vector capacities, prefix/tail storage, cursor payload and rollback key-table entries. Excludes map control bytes and shared captured Settings. Finished prefix/tail allocations retire incrementally during compaction while original pixels remain available for rollback. Released bytes record those work units; remaining scratch may drop at completion. Separate peaks must not be added: the combined tracked field samples live, history, settlement and stroke together. Sampling can miss short-lived states; excludes emitted snapshots, fill working buffers, worker scratch/output, descriptor/registry bookkeeping and allocator overhead. Not RSS or a global memory budget.",
        "allocation_scope": "Common registry samples live document, retained history and active stroke/fill settlement after each owner advance. Shared pixel allocations count once; settlement includes embedded snapshot metadata, partial raw copying, prepared Arc patch buffers, region-vector and fill-plan capacities. Peaks for separate fields may occur at different calls and must not be added. Sampling is outside owner timing but inside drain wall, can miss short-lived settlement, and excludes emitted snapshots, other active stroke/fill buffers, worker scratch/output, descriptor/registry bookkeeping and allocator overhead. This is not RSS or a global memory budget.",
        "scope": "Supported 512-square empty tiled and 2048-square dense single-layer raster documents in both flat compatibility and tiled storage. Profiling is enabled for per-unit phase counts/timings and adds clock overhead. Two overlapping timestamped strokes, completed-stroke snapshot, undo snapshot, redo and document fence. CPU advance timings include checkpoint/settlement/reclamation, undo/redo and snapshot control units; full pixel checks are outside each call timer, inside total wall. Input generation and reference painting precede all timers. A two-millisecond deadline is checked between units, with a 32-unit cap; synchronous controls and individual rows can overrun it. Rollback-only timings cover cancellation cursor construction, original row restoration, incremental scratch release, tile reclamation and final checkpoint discard/accounting; they exclude preparation/live painting and final pixel verification. Rollback must restore every pixel of both completed strokes. Settlement scans and copies one tile row per unit, retaining the full checkpoint until atomic publication; final trimming/accounting and buffer release remain synchronous. Settlement payload peak estimates count partial region buffers and region-vector capacity, excluding shared snapshot pixels, metadata clones, allocator overhead and stack fields. Input bytes exclude active buffers/history/snapshots, vacant queue capacity and allocator overhead. An additional typed-command correctness sequence runs after all reported timers: invalid target, reserved-identity new layer, identity-target rename, opacity, captured-identity stroke and snapshot, checking both layers in full against synchronous reference output. Typed raster commands and their history settlement remain synchronous; legacy target indices remain execution-relative; new identity targets survive reorder and reject missing layers. Drains without UI waits; does not measure driver/UI/display or save filesystem commits."})).map_err(|e| e.to_string())?;
    crate::storage::atomic_write(path, &data)
}

#[cfg(test)]
mod tests {
    #[test]
    fn compact_retires_finished_prefixes_without_changing_pixels_or_undo() {
        let mut doc = Document::new(75, 73);
        doc.layers[0]
            .pixels
            .writer()
            .rgba_mut(0)
            .copy_from_slice(&[9, 11, 13, 0]);
        let original = doc.layers[0].pixels.clone();
        let captured = settings([17, 83, 201]);
        let mut expected = doc.clone();
        reference(&mut expected, &captured);
        let mut history = History::new();
        let mut queue = Queue::default();
        submit(&mut queue, captured);
        let mut reduced = false;
        let mut calls = 0;
        while queue.busy() {
            calls += 1;
            assert!(calls < 10000);
            let compact = queue
                .active
                .as_ref()
                .is_some_and(|active| matches!(active.operation, Operation::Compact(_)));
            let before = queue.stroke_working_bytes();
            checked(&mut queue, &mut doc, &mut history, 1);
            if compact {
                assert_eq!(doc.layers[0].pixels, expected.layers[0].pixels);
                if queue.active.is_some() && queue.stroke_working_bytes() < before {
                    reduced = true;
                }
            }
        }
        assert!(
            reduced,
            "prefix allocations must retire before the active stroke is dropped"
        );
        assert_eq!(queue.stroke_working_bytes(), 0);
        assert!(history.undo(&mut doc));
        assert_eq!(doc.layers[0].pixels, original);
        assert!(history.redo(&mut doc));
        assert_eq!(doc.layers[0].pixels, expected.layers[0].pixels);
    }
    #[test]
    fn edit_settlement_yields_before_snapshot_and_retains_a_cheaper_sparse_original() {
        let mut doc = Document::new(513, 257);
        doc.layers[0]
            .pixels
            .writer()
            .rgba_mut(512)
            .copy_from_slice(&[9, 11, 13, 0]);
        let original = doc.clone();
        let mut history = History::new();
        let mut queue = Queue::default();
        let ticket = queue.edit(fill_edit(&doc)).unwrap();
        checked(&mut queue, &mut doc, &mut history, 1);
        queue.wait_edit();
        let snapshot = queue.snapshot().unwrap();
        let publication = checked(&mut queue, &mut doc, &mut history, 1);
        assert!(!publication.damage.is_empty() && publication.edits.is_empty());
        assert!(publication.snapshot.is_none());
        assert!(queue.edit_settlement.is_some() && queue.busy());
        let mut registry = crate::snapshot_memory::Registry::new(&doc);
        history.register_allocations(&mut registry);
        let retained = registry.additional_bytes();
        queue.register_settlement_allocations(&mut registry);
        let mut direct = crate::snapshot_memory::Registry::new(&doc);
        history.register_allocations(&mut direct);
        queue
            .edit_settlement
            .as_ref()
            .unwrap()
            .cursor
            .register_allocations(&mut direct);
        assert_eq!(registry.additional_bytes(), direct.additional_bytes());
        assert!(registry.additional_bytes() > retained);
        assert!(history.has_pending_stroke());
        assert!(
            !queue.cancel_background_edit(),
            "Published fills must be undone, not canceled"
        );
        let first = checked(&mut queue, &mut doc, &mut history, 1);
        assert!(first.snapshot.is_none() && first.edits.is_empty());
        let mut completion = None;
        let mut saved = None;
        let mut peak = 0;
        let mut calls = 0;
        while queue.busy() {
            let step = checked(&mut queue, &mut doc, &mut history, 1);
            peak = peak.max(step.settlement_payload_peak_bytes);
            if !step.edits.is_empty() {
                completion = Some(step.edits[0].0);
            }
            if let Some(value) = step.snapshot {
                saved = Some(value);
            }
            calls += 1;
            assert!(calls < 10000);
        }
        assert_eq!(completion, Some(ticket));
        assert_eq!(saved.unwrap().0, snapshot);
        assert!(!history.has_pending_stroke());
        let mut registry = crate::snapshot_memory::Registry::new(&doc);
        history.register_allocations(&mut registry);
        let retained = registry.additional_bytes();
        queue.register_settlement_allocations(&mut registry);
        assert_eq!(registry.additional_bytes(), retained);
        assert!(
            peak < 300 * 1024,
            "Sparse original must not expand to a canvas-sized patch: peak={peak}, retained={}",
            history.retained_bytes()
        );
        assert!(history.retained_bytes() < 300 * 1024);
        assert!(history.undo(&mut doc));
        assert_eq!(doc.layers[0].pixels, original.layers[0].pixels);
        assert!(history.redo(&mut doc));
        assert!(
            doc.layers[0]
                .pixels
                .chunks_exact(4)
                .all(|rgba| rgba == [17, 83, 201, 255])
        );
    }

    #[test]
    fn small_selected_fill_compacts_a_dense_original_and_preserves_other_pixels() {
        for budget in [1, 17, 4096] {
            let mut doc = Document::new(513, 257);
            doc.layers[0].pixels = [9, 11, 13, 255].repeat(doc.width * doc.height).into();
            doc.selection = Some(crate::selection::Selection {
                left: 10,
                top: 10,
                right: 18,
                bottom: 18,
            });
            let original = doc.clone();
            let mut history = History::new();
            let mut queue = Queue::default();
            queue
                .edit(crate::document_edits::Edit::FillTarget {
                    target: doc.layers[0].id,
                    source: crate::fill::Source::Active,
                    point: [10, 10],
                    color: [17, 83, 201],
                    opacity: 1.0,
                    tolerance: 0,
                    options: crate::fill::Options::default(),
                })
                .unwrap();
            checked(&mut queue, &mut doc, &mut history, 1);
            queue.wait_edit();
            let mut peak = 0;
            while queue.busy() {
                let progress = checked(&mut queue, &mut doc, &mut history, budget);
                peak = peak.max(progress.settlement_payload_peak_bytes);
                assert!(progress.edits.iter().all(|(_, result)| result.is_ok()));
            }
            assert!(peak < 16 * 1024 && history.retained_bytes() < 16 * 1024);
            for y in 0..doc.height {
                for x in 0..doc.width {
                    let expected = if doc.selection.unwrap().contains(x, y) {
                        [17, 83, 201, 255]
                    } else {
                        [9, 11, 13, 255]
                    };
                    assert_eq!(doc.layers[0].pixels.rgba(y * doc.width + x), expected);
                }
            }
            let filled = doc.clone();
            assert!(history.undo(&mut doc));
            assert_eq!(doc.layers[0].pixels, original.layers[0].pixels);
            assert_eq!(doc.selection, original.selection);
            assert!(history.redo(&mut doc));
            assert_eq!(doc.layers[0].pixels, filled.layers[0].pixels);
        }
    }

    #[test]
    fn interrupted_fill_compaction_keeps_its_original_full_undo_entry() {
        let mut doc = Document::new(75, 73);
        let original = doc.clone();
        let mut history = History::new();
        let mut queue = Queue::default();
        queue.edit(fill_edit(&doc)).unwrap();
        checked(&mut queue, &mut doc, &mut history, 1);
        queue.wait_edit();
        checked(&mut queue, &mut doc, &mut history, 1);
        assert!(queue.edit_settlement.is_some());
        doc.layers[0].name = "External change".into();
        let step = checked(&mut queue, &mut doc, &mut history, 1);
        assert!(
            step.edits[0]
                .1
                .as_ref()
                .unwrap_err()
                .contains("compaction failed")
        );
        assert_eq!(doc.layers[0].name, "External change");
        assert!(
            doc.layers[0]
                .pixels
                .chunks_exact(4)
                .all(|rgba| rgba == [17, 83, 201, 255])
        );
        assert!(!queue.busy() && !history.has_pending_stroke());
        assert!(history.undo(&mut doc));
        assert_eq!(doc.layers[0].pixels, original.layers[0].pixels);
        assert_eq!(doc.layers[0].name, original.layers[0].name);
    }
    fn fill_edit(doc: &Document) -> crate::document_edits::Edit {
        crate::document_edits::Edit::FillTarget {
            target: doc.layers[doc.active].id,
            source: crate::fill::Source::Active,
            point: [0, 0],
            color: [17, 83, 201],
            opacity: 1.0,
            tolerance: 0,
            options: crate::fill::Options::default(),
        }
    }
    fn polygon_edit(mode: crate::selection_mask::Combine) -> crate::document_edits::Edit {
        crate::document_edits::Edit::SelectionPolygon {
            points: vec![[1.0, 1.0], [5.0, 1.0], [5.0, 4.0], [1.0, 4.0]],
            mode,
        }
    }
    #[test]
    fn move_finish_publication_and_sliced_history_precede_snapshot_and_undo() {
        let mut doc = Document::new(75, 73);
        doc.layers[0].pixels = [17, 83, 201, 255].repeat(75 * 73).into();
        let mask = crate::selection_mask::Mask::rectangle(
            75,
            73,
            crate::selection::Selection {
                left: 0,
                top: 0,
                right: 32,
                bottom: 73,
            },
        )
        .unwrap();
        doc.selection = mask.bounds();
        doc.selection_mask = Some(mask);
        let before = doc.clone();
        let mut history = History::new();
        let mut queue = Queue::default();
        let (engine, release) = crate::move_preview::Engine::gated(&doc, [1, 0]);
        queue
            .finish_move(engine, false, false)
            .unwrap_or_else(|_| panic!("Move admission failed"));
        let ticket = queue.snapshot().unwrap();
        queue.undo().unwrap();
        assert!(queue.waiting_edit());
        release.send(()).unwrap();
        queue.wait_edit();
        let progress = checked(&mut queue, &mut doc, &mut history, 1);
        assert!(!progress.damage.is_empty() && progress.snapshot.is_none());
        assert!(
            queue.finishing_move.as_ref().unwrap().cursor.is_some() && history.has_pending_stroke()
        );
        let mut calls = 0;
        let mut snapshot = None;
        let mut completion = false;
        while queue.busy() {
            let progress = checked(&mut queue, &mut doc, &mut history, 1);
            calls += 1;
            if progress.move_done.is_some() {
                completion = true;
                assert!(progress.snapshot.is_none());
            }
            if let Some(value) = progress.snapshot {
                assert!(completion);
                snapshot = Some(value);
            }
        }
        assert!(calls > 10, "Settlement must yield between row units");
        let (id, saved) = snapshot.unwrap();
        assert_eq!(id, ticket);
        assert_eq!(saved.layers[0].pixels.rgba(0), [0; 4]);
        assert_eq!(saved.selection_mask.as_ref().unwrap().at(32, 0), 255);
        assert_eq!(doc.layers[0].pixels, before.layers[0].pixels);
        assert_eq!(doc.selection_mask, before.selection_mask);
        assert!(!history.can_undo());
        assert!(history.redo(&mut doc));
        assert_eq!(doc.layers[0].pixels, saved.layers[0].pixels);
        assert_eq!(doc.selection_mask, saved.selection_mask);
    }
    #[test]
    fn selection_workers_combine_feather_flip_then_snapshot_with_exact_undo() {
        use crate::{document_edits::Edit, selection_mask::Combine};
        for mode in [
            Combine::Replace,
            Combine::Add,
            Combine::Subtract,
            Combine::Intersect,
        ] {
            let mut doc = Document::new(8, 5);
            doc.selection = Some(crate::selection::Selection {
                left: 3,
                top: 0,
                right: 7,
                bottom: 5,
            });
            for x in 0..8 {
                doc.layers[0]
                    .pixels
                    .writer()
                    .rgba_mut(8 + x)
                    .copy_from_slice(&[x as u8 * 25, 17, 83, 255]);
            }
            let before = doc.clone();
            let mut history = History::new();
            let mut queue = Queue::default();
            queue.edit(polygon_edit(mode)).unwrap();
            queue.edit(Edit::FeatherSelection(1)).unwrap();
            queue
                .edit(Edit::TransformTarget {
                    target: doc.layers[0].id,
                    transform: crate::transform::Transform::FlipHorizontal,
                })
                .unwrap();
            let ticket = queue.snapshot().unwrap();
            let mut snapshot = None;
            let mut damage_count = 0;
            while queue.busy() {
                if queue.waiting_edit() {
                    queue.wait_edit();
                }
                let progress = checked(&mut queue, &mut doc, &mut history, 1);
                assert!(progress.edits.iter().all(|(_, result)| result.is_ok()));
                damage_count += usize::from(!progress.damage.is_empty());
                if let Some(value) = progress.snapshot {
                    snapshot = Some(value);
                }
            }
            assert_eq!(snapshot.unwrap().0, ticket);
            let mut raw = vec![0; 40];
            for y in 0..5 {
                for x in 0..8 {
                    let old = (3..7).contains(&x);
                    let new = (1..5).contains(&x) && (1..4).contains(&y);
                    let selected = match mode {
                        Combine::Replace => new,
                        Combine::Add => old || new,
                        Combine::Subtract => old && !new,
                        Combine::Intersect => old && new,
                    };
                    raw[y * 8 + x] = if selected { 255 } else { 0 };
                }
            }
            let selected = crate::selection_mask::Mask::from_coverage(8, 5, raw).unwrap();
            let feathered = selected.feather(1, None).unwrap();
            let mut expected = before.clone();
            expected.selection = feathered.bounds();
            expected.selection_mask = Some(feathered.clone());
            expected
                .transform_active(crate::transform::Transform::FlipHorizontal)
                .unwrap();
            assert_eq!(doc.selection_mask, expected.selection_mask);
            assert_eq!(doc.layers[0].pixels, expected.layers[0].pixels);
            assert_eq!(
                damage_count, 1,
                "Only the pixel transform invalidates the artwork display"
            );
            assert!(history.undo(&mut doc));
            assert_eq!(doc.layers[0].pixels, before.layers[0].pixels);
            assert_eq!(doc.selection_mask, Some(feathered));
            assert!(history.undo(&mut doc));
            assert_eq!(doc.selection_mask, Some(selected));
            assert!(history.undo(&mut doc));
            assert_eq!(doc.layers[0].pixels, before.layers[0].pixels);
            let mut metadata = doc.clone();
            metadata.layers[0].pixels = before.layers[0].pixels.clone();
            assert!(crate::document::same_snapshot(&metadata, &before));
            assert!(!history.can_undo());
            for _ in 0..3 {
                assert!(history.redo(&mut doc));
            }
            assert_eq!(doc.layers[0].pixels, expected.layers[0].pixels);
        }
    }
    #[test]
    fn canceled_and_stale_mask_workers_preserve_artwork_and_redo() {
        for phase in [0, 1, 2] {
            let mut doc = Document::new(8, 5);
            let mut history = History::new();
            history.checkpoint(&doc);
            doc.layers[0].opacity = 0.5;
            history.settle(&doc);
            assert!(history.undo(&mut doc));
            let mut queue = Queue::default();
            let ticket = queue
                .edit(polygon_edit(crate::selection_mask::Combine::Replace))
                .unwrap();
            if phase != 0 {
                checked(&mut queue, &mut doc, &mut history, 1);
                queue.wait_edit();
            }
            if phase == 2 {
                doc.layers[0].name = "Newer state".into();
            } else {
                assert!(queue.cancel_background_edit());
            }
            let progress = checked(&mut queue, &mut doc, &mut history, 4096);
            assert_eq!(progress.edits[0].0, ticket);
            assert!(progress.edits[0].1.is_err());
            assert!(doc.selection_mask.is_none() && doc.selection.is_none());
            assert!(history.can_redo() && !history.can_undo());
            assert!(doc.layers[0].pixels.capacity() == 0);
            if phase == 2 {
                assert_eq!(doc.layers[0].name, "Newer state");
            }
        }
    }
    #[test]
    fn fill_worker_publishes_before_snapshot_and_keeps_later_edits_separate() {
        let mut doc = Document::new(75, 73);
        let target = doc.layers[0].id;
        let mut history = History::new();
        let mut queue = Queue::default();
        let ticket = queue.edit(fill_edit(&doc)).unwrap();
        checked(&mut queue, &mut doc, &mut history, 1); // Starts worker only.
        assert!(queue.busy() && !history.can_undo());
        assert!(doc.layers[0].pixels.iter().all(|byte| *byte == 0));
        let snapshot = queue.snapshot().unwrap();
        queue
            .edit(crate::document_edits::Edit::Property {
                target: crate::document_edits::Target::Identity(target),
                property: crate::document_edits::Property::Name("After snapshot".into()),
            })
            .unwrap();
        queue.wait_edit();
        let progress = checked(&mut queue, &mut doc, &mut history, 4096);
        assert_eq!(progress.edits[0].0, ticket);
        let (captured, saved) = progress.snapshot.unwrap();
        assert_eq!(captured, snapshot);
        assert_ne!(saved.layers[0].name, "After snapshot");
        assert!(
            saved.layers[0]
                .pixels
                .chunks_exact(4)
                .all(|rgba| rgba == [17, 83, 201, 255])
        );
        checked(&mut queue, &mut doc, &mut history, 4096);
        assert_eq!(doc.layers[0].name, "After snapshot");
        assert!(history.undo(&mut doc));
        assert_eq!(doc.layers[0].pixels, saved.layers[0].pixels);
        assert!(history.undo(&mut doc));
        assert!(doc.layers[0].pixels.iter().all(|byte| *byte == 0));
        assert!(!history.can_undo());
    }
    #[test]
    fn stale_fill_source_rejects_without_touching_newer_state_or_redo() {
        let mut doc = Document::new(75, 73);
        let mut history = History::new();
        history.checkpoint(&doc);
        doc.layers[0].opacity = 0.5;
        history.settle(&doc);
        assert!(history.undo(&mut doc));
        let mut queue = Queue::default();
        queue.edit(fill_edit(&doc)).unwrap();
        checked(&mut queue, &mut doc, &mut history, 1);
        doc.layers[0].name = "External change".into();
        queue.wait_edit();
        let progress = checked(&mut queue, &mut doc, &mut history, 4096);
        assert!(
            progress.edits[0]
                .1
                .as_ref()
                .unwrap_err()
                .contains("source changed")
        );
        assert_eq!(doc.layers[0].name, "External change");
        assert!(doc.layers[0].pixels.iter().all(|byte| *byte == 0));
        assert!(history.can_redo() && !history.can_undo());
    }
    #[test]
    fn canceled_queued_or_ready_fill_leaves_snapshot_and_history_unchanged() {
        for ready in [false, true] {
            let mut doc = Document::new(75, 73);
            let mut history = History::new();
            let mut queue = Queue::default();
            let ticket = queue.edit(fill_edit(&doc)).unwrap();
            if ready {
                checked(&mut queue, &mut doc, &mut history, 1);
                queue.wait_edit();
            }
            assert!(queue.cancel_background_edit());
            queue.snapshot().unwrap();
            let progress = checked(&mut queue, &mut doc, &mut history, 4096);
            assert_eq!(progress.edits[0].0, ticket);
            assert!(
                progress.edits[0]
                    .1
                    .as_ref()
                    .unwrap_err()
                    .contains("canceled")
            );
            assert!(
                progress.snapshot.unwrap().1.layers[0]
                    .pixels
                    .iter()
                    .all(|byte| *byte == 0)
            );
            assert!(!queue.busy() && !history.can_undo());
        }
    }
    #[test]
    fn waiting_edit_blocks_later_commands_and_cooperative_cancel_is_atomic() {
        let mut doc = Document::new(75, 73);
        let mut history = History::new();
        let mut queue = Queue::default();
        let before = doc.clone();
        let source = before.clone();
        let edit = fill_edit(&doc);
        let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let control = cancel.clone();
        let (send, receive) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            receive.recv().unwrap();
            edit.prepare_control(&source, Some(&control))
        });
        queue.background_edit = Some(EditJob {
            ticket: 1,
            label: "Fill",
            before,
            cancel,
            worker: Some(worker),
            result: None,
        });
        queue.snapshot().unwrap();
        assert!(queue.waiting_edit() && !queue.needs_work());
        let progress = checked(&mut queue, &mut doc, &mut history, 4096);
        assert_eq!(progress.work_units, 0);
        assert!(progress.snapshot.is_none());
        assert!(queue.cancel_background_edit());
        send.send(()).unwrap();
        queue.wait_edit();
        let progress = checked(&mut queue, &mut doc, &mut history, 4096);
        assert!(
            progress.edits[0]
                .1
                .as_ref()
                .unwrap_err()
                .contains("canceled")
        );
        assert!(doc.layers[0].pixels.iter().all(|byte| *byte == 0));
        assert!(!history.has_pending_stroke() && !history.can_undo());
        assert!(!queue.busy());
    }
    #[test]
    fn unrendered_opacity_round_trip_never_suspends_redo() {
        let mut doc = Document::new(8, 8);
        let target = doc.layers[0].id;
        let mut history = History::new();
        history.checkpoint(&doc);
        doc.layers[0].opacity = 0.5;
        history.settle(&doc);
        assert!(history.undo(&mut doc));
        let mut queue = Queue::default();
        let ticket = queue.start_opacity(target, 0.3).unwrap();
        queue.update_opacity(ticket, target, 1.0).unwrap();
        queue.end_opacity(ticket).unwrap();
        let event = checked(&mut queue, &mut doc, &mut history, 1)
            .opacity
            .unwrap();
        assert!(event.started && !event.changed);
        assert!(history.can_redo() && !history.has_pending_stroke());
        let event = checked(&mut queue, &mut doc, &mut history, 1)
            .opacity
            .unwrap();
        assert!(event.ended && !event.changed);
        assert!(!queue.busy() && history.can_redo() && !history.can_undo());
        assert!(history.redo(&mut doc));
        assert_eq!(doc.layers[0].opacity, 0.5);
    }
    #[test]
    fn opacity_admission_coalesces_and_reserves_end_under_queue_limits() {
        let mut doc = Document::new(8, 8);
        let target = doc.layers[0].id;
        let mut history = History::new();
        let mut queue = Queue {
            command_limit: 2,
            byte_limit: 2 * std::mem::size_of::<Command>(),
            ..Default::default()
        };
        let ticket = queue.start_opacity(target, 0.5).unwrap();
        let bytes = queue.queued_input_bytes();
        for value in 0..1000 {
            queue
                .update_opacity(ticket, target, value as f32 / 1000.0)
                .unwrap();
        }
        assert_eq!(queue.queued_input_bytes(), bytes);
        assert_eq!(queue.commands.len(), 1);
        assert!(queue.undo().is_err());
        assert!(queue.start_opacity(target, 0.3).is_err());
        assert!(
            queue
                .update_opacity(ticket, crate::document::LayerId::new(), 0.3)
                .is_err()
        );
        assert!(queue.update_opacity(ticket, target, f32::NAN).is_err());
        queue.end_opacity(ticket).unwrap();
        assert_eq!(queue.commands.len(), 2);
        let step = checked(&mut queue, &mut doc, &mut history, 4096);
        assert!(step.opacity.unwrap().started);
        assert_eq!(doc.layers[0].opacity, 0.999);
        assert!(history.has_pending_stroke());
        let step = checked(&mut queue, &mut doc, &mut history, 4096);
        let event = step.opacity.unwrap();
        assert!(event.ended && event.changed);
        assert!(!queue.busy() && !history.has_pending_stroke());
        assert!(history.undo(&mut doc));
        assert_eq!(doc.layers[0].opacity, 1.0);
        assert!(!history.can_undo());
    }

    #[test]
    fn missing_opacity_target_finishes_without_artwork_or_history_changes() {
        let mut doc = Document::new(8, 8);
        let pixels = doc.layers[0].pixels.clone();
        let mut history = History::new();
        let mut queue = Queue::default();
        let ticket = queue
            .start_opacity(crate::document::LayerId::new(), 0.3)
            .unwrap();
        queue.end_opacity(ticket).unwrap();
        let event = checked(&mut queue, &mut doc, &mut history, 4096)
            .opacity
            .unwrap();
        assert!(event.error.is_some() && !event.changed);
        let event = checked(&mut queue, &mut doc, &mut history, 4096)
            .opacity
            .unwrap();
        assert!(event.ended && !event.changed);
        assert!(!queue.busy() && !history.has_pending_stroke() && !history.can_undo());
        assert_eq!(doc.layers[0].opacity, 1.0);
        assert_eq!(doc.layers[0].pixels, pixels);
    }
    use super::*;
    #[test]
    fn duplicate_identity_capacity_and_collisions_preserve_input_bounds_and_redo() {
        use crate::document_edits::Edit;
        let mut doc = Document::new(32, 32);
        let target = doc.layers[0].id;
        let mut history = History::new();
        history.checkpoint(&doc);
        doc.layers[0]
            .pixels
            .writer()
            .rgba_mut(0)
            .copy_from_slice(&[17, 83, 201, 255]);
        history.settle(&doc);
        assert!(history.undo(&mut doc));
        let before = doc.layers[0].pixels.clone();
        let mut queue = Queue::default();
        let mut copies = Vec::with_capacity(
            MAX_INPUT_BYTES / std::mem::size_of::<crate::document::LayerId>() + 1,
        );
        copies.push(crate::document::LayerId::new());
        assert!(
            queue
                .edit(Edit::DuplicateTarget { target, copies })
                .is_err()
        );
        assert!(!queue.busy());
        assert!(history.can_redo());
        let ticket = queue
            .edit(Edit::DuplicateTarget {
                target,
                copies: vec![target],
            })
            .unwrap();
        let progress = checked(&mut queue, &mut doc, &mut history, 17);
        assert_eq!(progress.edits.len(), 1);
        assert_eq!(progress.edits[0].0, ticket);
        assert!(progress.edits[0].1.is_err());
        assert_eq!(doc.layers.len(), 1);
        assert_eq!(doc.layers[0].pixels, before);
        assert!(history.can_redo() && !history.can_undo());
    }
    #[test]
    fn identity_reorder_and_delete_keep_or_cancel_captured_strokes() {
        use crate::document_edits::Edit;
        for delete in [false, true] {
            let mut doc = Document::new(75, 73);
            let target = doc.layers[0].id;
            doc.add_layer();
            let other = doc.layers[1].id;
            doc.active = 0;
            let before = doc.clone();
            let mut queue = Queue::default();
            let mut history = History::new();
            if delete {
                queue.edit(Edit::Delete).unwrap();
            } else {
                queue
                    .edit(Edit::ReorderTarget {
                        from: target,
                        before: None,
                        depth: 0,
                    })
                    .unwrap();
            }
            let stroke = queue.begin_layer(settings([17, 83, 201]), target).unwrap();
            queue.dab(stroke, sample(0)).unwrap();
            queue.end(stroke).unwrap();
            let mut finished = false;
            while queue.needs_work() {
                let step = queue.advance(&mut doc, &mut history, 1).unwrap();
                assert!(step.history_error.is_none());
                for (_, result) in step.edits {
                    assert!(result.unwrap().changed);
                }
                if step.render_error.is_some() {
                    assert!(delete);
                }
                for done in step.completed {
                    assert_eq!(done.id, stroke);
                    assert_eq!(done.cancelled, delete);
                    assert_eq!(done.changed, !delete);
                    finished = true;
                }
            }
            assert!(finished);
            if delete {
                assert_eq!(doc.layers.len(), 1);
                assert_eq!(doc.layers[0].id, other);
                assert_eq!(doc.layers[0].pixels, before.layers[1].pixels);
            } else {
                assert_eq!(doc.layers[0].id, other);
                assert_eq!(doc.layers[1].id, target);
                assert_ne!(doc.layers[1].pixels, before.layers[0].pixels);
                assert!(history.undo(&mut doc));
            }
            assert!(history.undo(&mut doc));
            assert_eq!(doc.layers[0].id, target);
            assert_eq!(doc.layers[1].id, other);
            assert_eq!(doc.layers[0].pixels, before.layers[0].pixels);
        }
    }
    #[test]
    fn reserved_layer_targets_and_failed_creation_do_not_retarget_artwork() {
        use crate::document_edits::{Edit, Property, Target};
        for full in [false, true] {
            let mut doc = Document::new(75, 73);
            if full {
                for _ in 1..crate::document::MAX_RASTER_LAYERS {
                    doc.add_layer();
                }
            }
            let before = doc.clone();
            let mut history = History::new();
            let mut queue = Queue::default();
            let (creation, target) = queue.new_layer().unwrap();
            let rename = queue
                .edit(Edit::Property {
                    target: Target::Identity(target),
                    property: Property::Name("Reserved".into()),
                })
                .unwrap();
            let stroke = queue.begin_layer(settings([17, 83, 201]), target).unwrap();
            queue.dab(stroke, sample(0)).unwrap();
            queue.end(stroke).unwrap();
            let snapshot = queue.snapshot().unwrap();
            let mut results = Vec::new();
            let mut finished = false;
            let mut saved = false;
            while queue.needs_work() {
                let progress = queue.advance(&mut doc, &mut history, 17).unwrap();
                assert!(progress.history_error.is_none());
                if progress.render_error.is_some() {
                    assert!(full);
                }
                results.extend(progress.edits);
                for done in progress.completed {
                    assert_eq!(done.id, stroke);
                    assert_eq!(done.cancelled, full);
                    finished = true;
                }
                if let Some((ticket, document)) = progress.snapshot {
                    assert_eq!(ticket, snapshot);
                    assert_eq!(
                        document.layers.len(),
                        if full { before.layers.len() } else { 2 }
                    );
                    saved = true;
                }
            }
            assert!(finished && saved);
            assert_eq!(results.len(), 2);
            assert_eq!(results[0].0, creation);
            assert_eq!(results[1].0, rename);
            assert!(results.iter().all(|(_, result)| result.is_err() == full));
            if full {
                assert!(!history.can_undo());
                for (after, before) in doc.layers.iter().zip(&before.layers) {
                    assert_eq!(after.id, before.id);
                    assert_eq!(after.name, before.name);
                    assert_eq!(after.pixels, before.pixels);
                }
            } else {
                assert_eq!(doc.layers[1].id, target);
                assert_eq!(doc.layers[1].name, "Reserved");
                assert_ne!(doc.layers[1].pixels, before.layers[0].pixels);
                assert!(history.undo(&mut doc));
                assert_eq!(doc.layers[1].id, target);
                assert!(history.redo(&mut doc));
                assert_eq!(doc.layers[1].id, target);
            }
        }
    }
    #[test]
    fn typed_edits_and_inherited_targets_preserve_stroke_snapshot_and_undo_order() {
        use crate::document_edits::{Edit, Property, Target};
        for budget in [1, 17, 64] {
            let mut doc = Document::new(75, 73);
            doc.layers[0]
                .pixels
                .writer()
                .rgba_mut(0)
                .copy_from_slice(&[9, 8, 7, 0]);
            let mut history = History::new();
            let mut queue = Queue::default();
            let first = settings([1, 2, 3]);
            submit(&mut queue, first.clone());
            let new_layer = queue.edit(Edit::NewLayer).unwrap();
            let rename = queue
                .edit(Edit::Property {
                    target: Target::Active,
                    property: Property::Name("Queued ink".into()),
                })
                .unwrap();
            let second = settings([200, 70, 20]);
            let inherited = queue.begin_current(second.clone()).unwrap();
            for i in 0..15 {
                queue.dab(inherited, sample(i)).unwrap();
            }
            queue.end(inherited).unwrap();
            let snapshot = queue.snapshot().unwrap();
            queue.edit(Edit::SelectLayer(0)).unwrap();
            queue
                .edit(Edit::Selection(Some(crate::selection::Selection {
                    left: 0,
                    top: 0,
                    right: 75,
                    bottom: 73,
                })))
                .unwrap();
            queue
                .edit(Edit::Transform(crate::transform::Transform::FlipHorizontal))
                .unwrap();
            queue.edit(Edit::Clear { whole_layer: false }).unwrap();
            let mut expected = doc.clone();
            reference(&mut expected, &first);
            expected.insert_layer_above().unwrap();
            expected.layers[1].name = "Queued ink".into();
            let mut target = (*second).clone();
            target.layer = 1;
            reference(&mut expected, &target);
            let mut observed = false;
            let mut completions = vec![];
            let mut edits = vec![];
            while queue.needs_work() {
                let step = checked(&mut queue, &mut doc, &mut history, budget);
                completions.extend(step.completed);
                for (ticket, result) in step.edits {
                    assert!(result.is_ok());
                    edits.push(ticket);
                }
                if let Some((ticket, saved)) = step.snapshot {
                    assert_eq!(ticket, snapshot);
                    assert_eq!(saved.active, 1);
                    assert_eq!(saved.layers[1].name, "Queued ink");
                    assert_eq!(saved.layers[0].pixels, expected.layers[0].pixels);
                    assert_eq!(saved.layers[1].pixels, expected.layers[1].pixels);
                    observed = true;
                }
            }
            assert!(observed);
            assert_eq!(completions.len(), 2);
            assert_eq!(completions[1].id, inherited);
            assert_eq!(edits[..2], [new_layer, rename]);
            expected.active = 0;
            expected.selection = doc.selection;
            expected
                .transform_active(crate::transform::Transform::FlipHorizontal)
                .unwrap();
            let transformed = expected.layers[0].pixels.clone();
            expected.clear_selected().unwrap();
            assert_eq!(doc.layers[0].pixels, expected.layers[0].pixels);
            assert_eq!(doc.layers[1].pixels, expected.layers[1].pixels);
            assert!(history.undo(&mut doc));
            assert_eq!(doc.layers[0].pixels, transformed);
            assert!(history.redo(&mut doc));
            assert_eq!(doc.layers[0].pixels, expected.layers[0].pixels);
        }
    }
    #[test]
    fn noops_failed_edits_and_payload_limits_preserve_redo_and_artwork() {
        use crate::document_edits::{Edit, Property, Target};
        let mut queue = Queue::default();
        let mut doc = Document::new(75, 73);
        let mut history = History::new();
        submit(&mut queue, settings([1, 2, 3]));
        while queue.needs_work() {
            checked(&mut queue, &mut doc, &mut history, 64);
        }
        assert!(history.undo(&mut doc));
        let before = doc.layers[0].pixels.clone();
        let invalid = queue.edit(Edit::SelectLayer(usize::MAX)).unwrap();
        queue.edit(Edit::Rotate(0)).unwrap();
        queue
            .edit(Edit::Property {
                target: Target::Active,
                property: Property::Opacity(1.0),
            })
            .unwrap();
        queue.edit(Edit::Selection(None)).unwrap();
        let mut results = vec![];
        while queue.needs_work() {
            results.extend(checked(&mut queue, &mut doc, &mut history, 1).edits);
        }
        assert_eq!(results[0].0, invalid);
        assert!(results[0].1.is_err());
        assert!(
            results[1..]
                .iter()
                .all(|(_, r)| !r.as_ref().unwrap().changed)
        );
        assert_eq!(doc.layers[0].pixels, before);
        assert!(history.can_redo() && !history.can_undo());
        let mut oversized = String::with_capacity(MAX_INPUT_BYTES + 1);
        oversized.push('x');
        assert!(
            queue
                .edit(Edit::Property {
                    target: Target::Active,
                    property: Property::Name(oversized)
                })
                .is_err()
        );
        assert!(
            queue
                .edit(Edit::Property {
                    target: Target::Active,
                    property: Property::Opacity(f32::NAN)
                })
                .is_err()
        );
        assert!(!queue.busy());
        assert!(history.can_redo());
    }
    #[test]
    fn partial_rollback_finishes_before_snapshot_and_following_stroke() {
        let mut doc = Document::new(75, 73);
        doc.layers[0]
            .pixels
            .writer()
            .rgba_mut(0)
            .copy_from_slice(&[9, 8, 7, 0]);
        let before = doc.clone();
        let mut history = History::new();
        let mut queue = Queue::default();
        let canceled = queue.begin(settings([200, 40, 20])).unwrap();
        for i in 0..15 {
            queue.dab(canceled, sample(i)).unwrap();
        }
        while doc.layers[0].pixels == before.layers[0].pixels {
            checked(&mut queue, &mut doc, &mut history, 1);
        }
        assert!(queue.cancel_open());
        let snapshot = queue.snapshot().unwrap();
        let following = submit(&mut queue, settings([20, 40, 200]));
        let mut completed = vec![];
        let mut snapshots = 0;
        while queue.needs_work() {
            let step = checked(&mut queue, &mut doc, &mut history, 1);
            completed.extend(step.completed);
            if let Some((ticket, document)) = step.snapshot {
                assert_eq!(ticket, snapshot);
                assert_eq!(document.layers[0].pixels, before.layers[0].pixels);
                snapshots += 1;
            }
        }
        assert_eq!(snapshots, 1);
        assert_eq!(
            completed.iter().map(|c| c.id).collect::<Vec<_>>(),
            [canceled, following]
        );
        assert!(completed[0].cancelled);
        assert!(completed[1].changed);
        let mut expected = before.clone();
        reference(&mut expected, &settings([20, 40, 200]));
        assert_eq!(doc.layers[0].pixels, expected.layers[0].pixels);
        assert!(history.undo(&mut doc));
        assert_eq!(doc.layers[0].pixels, before.layers[0].pixels);
        assert!(!history.can_undo());
    }
    #[test]
    fn expired_deadline_and_unit_boundary_yields_preserve_queue_progress() {
        let mut queue = Queue::default();
        let config = settings([10, 80, 160]);
        let id = submit(&mut queue, config.clone());
        let mut doc = Document::new(75, 73);
        let before = doc.clone();
        let mut history = History::new();
        let bytes = queue.queued_input_bytes();
        let expired = queue
            .advance_until(&mut doc, &mut history, 64, std::time::Instant::now())
            .unwrap();
        assert_eq!(expired.work_units, 0);
        assert_eq!(queue.queued_input_bytes(), bytes);
        assert!(!history.has_pending_stroke());
        assert_eq!(doc.layers[0].pixels, before.layers[0].pixels);
        let mut completed = vec![];
        while queue.needs_work() {
            // Deterministic deadline surrogate: allow only three unit boundaries.
            let mut boundaries = 0;
            let progress = queue
                .advance_while(&mut doc, &mut history, 64, || {
                    boundaries += 1;
                    boundaries <= 3
                })
                .unwrap();
            assert!(progress.work_units <= 3);
            assert!(progress.work_units > 0);
            assert!(progress.render_error.is_none());
            completed.extend(progress.completed);
        }
        assert_eq!(completed.len(), 1);
        assert_eq!(completed[0].id, id);
        assert!(completed[0].changed && !completed[0].cancelled);
        let mut expected = before;
        reference(&mut expected, &config);
        assert_eq!(doc.layers[0].pixels, expected.layers[0].pixels);
        assert!(history.undo(&mut doc));
        assert!(doc.layers[0].pixels.iter().all(|v| *v == 0));
    }
    fn settings(color: [u8; 3]) -> Arc<Settings> {
        Arc::new(Settings {
            target: crate::paint_target::Target::Raster,
            brush: crate::brush::Brush {
                size: 13.0,
                pressure_size: false,
                flow: 0.3,
                opacity: 0.6,
                taper_out: 20.0,
                ..Default::default()
            },
            color,
            secondary: color,
            erase: false,
            layer: 0,
            symmetry: Default::default(),
        })
    }
    fn sample(i: usize) -> Sample {
        Sample {
            x: 8.0 + i as f32 * 3.0,
            y: 25.5,
            pressure: 1.0,
            time: Some(i as f64 * 0.01),
            distance: i as f64 * 3.0,
            ..Default::default()
        }
    }
    #[test]
    #[ignore = "timing probe; run with --release --ignored --nocapture"]
    fn probe_large_brush_stroke() {
        for (size, hardness) in [(100.0, 1.0), (300.0, 1.0), (300.0, 0.3), (300.0, 0.0)] {
            let mut doc = Document::new(3000, 3000);
            let mut history = History::new();
            let mut queue = Queue::default();
            let settings = Arc::new(Settings {
                target: crate::paint_target::Target::Raster,
                brush: crate::brush::Brush {
                    size,
                    hardness,
                    pressure_size: false,
                    ..Default::default()
                },
                color: [200, 40, 40],
                secondary: [200, 40, 40],
                erase: false,
                layer: 0,
                symmetry: Default::default(),
            });
            let id = queue.begin(settings.clone()).unwrap();
            let dabs = 40;
            let step = size * settings.brush.spacing.max(0.02);
            for i in 0..dabs {
                queue
                    .dab(
                        id,
                        Sample {
                            x: 400.0 + i as f32 * step,
                            y: 1500.0,
                            pressure: 1.0,
                            time: Some(i as f64 * 0.01),
                            distance: (i as f32 * step) as f64,
                            ..Default::default()
                        },
                    )
                    .unwrap();
            }
            queue.end(id).unwrap();
            let start = std::time::Instant::now();
            while queue.needs_work() {
                checked(&mut queue, &mut doc, &mut history, 64);
            }
            let total = start.elapsed();
            let mut direct_doc = Document::new(3000, 3000);
            let mut buffer = StrokeBuffer::default();
            let started = std::time::Instant::now();
            for i in 0..dabs {
                direct_doc.dab_sample_buffered(
                    Sample {
                        x: 400.0 + i as f32 * step,
                        y: 1500.0,
                        pressure: 1.0,
                        ..Default::default()
                    },
                    &settings.brush,
                    settings.color,
                    false,
                    crate::symmetry::IDENTITY,
                    Some(&mut buffer),
                );
            }
            eprintln!(
                "  direct paint {:.2} ms per dab",
                started.elapsed().as_secs_f64() * 1000.0 / dabs as f64
            );
            let side = (size as usize).min(3000);
            let started = std::time::Instant::now();
            let mut sum = 0.0;
            for y in 0..side {
                for x in 0..side {
                    let (u, v) = (
                        (x as f32 / side as f32) * 2.0 - 1.0,
                        (y as f32 / side as f32) * 2.0 - 1.0,
                    );
                    sum += crate::brush_edge::coverage_with_profile(
                        u,
                        v,
                        size / 2.0,
                        1.0,
                        hardness,
                        crate::brush::Tip::Round,
                        Default::default(),
                    );
                }
            }
            eprintln!(
                "size {size} hardness {hardness}: {:.2} ms per dab (spacing {step}px); coverage alone {:.2} ms ({sum:.0})",
                total.as_secs_f64() * 1000.0 / dabs as f64,
                started.elapsed().as_secs_f64() * 1000.0
            );
        }
    }
    fn submit(queue: &mut Queue, settings: Arc<Settings>) -> StrokeId {
        let id = queue.begin(settings).unwrap();
        for i in 0..15 {
            queue.dab(id, sample(i)).unwrap();
        }
        queue.end(id).unwrap();
        id
    }
    fn reference(doc: &mut Document, settings: &Settings) {
        let mut buffer = StrokeBuffer::default();
        for i in 0..15 {
            buffer.paint(doc, sample(i), settings);
        }
        buffer.finish_tail(doc, settings);
        buffer.reclaim_empty_tiles(doc);
    }
    fn checked(
        queue: &mut Queue,
        doc: &mut Document,
        history: &mut History,
        work: usize,
    ) -> Progress {
        let progress = queue.advance(doc, history, work).unwrap();
        assert!(progress.work_units <= work);
        assert!(
            progress.render_error.is_none(),
            "{:?}",
            progress.render_error
        );
        assert!(
            progress.history_error.is_none(),
            "{:?}",
            progress.history_error
        );
        progress
    }
    #[test]
    fn overlapping_strokes_snapshot_undo_redo_and_fence_preserve_fifo() {
        for budget in [1, 17, 64] {
            let mut doc = Document::new(75, 73);
            let base = doc.clone();
            let a = settings([10, 50, 100]);
            let b = settings([200, 70, 20]);
            let mut first = base.clone();
            reference(&mut first, &a);
            let mut both = first.clone();
            reference(&mut both, &b);
            let mut history = History::new();
            let mut queue = Queue::default();
            let id_a = submit(&mut queue, a);
            let ticket_a = queue.snapshot().unwrap();
            let id_b = submit(&mut queue, b);
            queue.undo().unwrap();
            let ticket_undo = queue.snapshot().unwrap();
            queue.redo().unwrap();
            let barrier = queue.barrier().unwrap();
            let mut completed = vec![];
            let mut snapshots = 0;
            while queue.needs_work() {
                let progress = checked(&mut queue, &mut doc, &mut history, budget);
                for done in progress.completed {
                    assert!(done.changed && !done.cancelled);
                    completed.push(done.id);
                }
                if let Some((ticket, snapshot)) = progress.snapshot {
                    assert!(ticket == ticket_a || ticket == ticket_undo);
                    assert_eq!(snapshot.layers[0].pixels, first.layers[0].pixels);
                    snapshots += 1;
                }
                if let Some(ticket) = progress.barrier {
                    assert_eq!(ticket, barrier);
                }
            }
            assert_eq!(completed, [id_a, id_b]);
            assert_eq!(snapshots, 2);
            assert_eq!(doc.layers[0].pixels, both.layers[0].pixels);
            assert!(queue.busy());
            assert!(!history.has_pending_stroke());
            assert!(queue.release_barrier(barrier + 1).is_err());
            let stalled = checked(&mut queue, &mut doc, &mut history, 100);
            assert_eq!(stalled.work_units, 0);
            assert_eq!(stalled.barrier, Some(barrier));
            // The fence permits document changes before the next queued stroke.
            history.checkpoint(&doc);
            doc.add_layer();
            history.settle(&doc);
            queue.release_barrier(barrier).unwrap();
            let mut next = (*settings([1, 2, 3])).clone();
            next.layer = doc.active;
            submit(&mut queue, Arc::new(next));
            while queue.needs_work() {
                checked(&mut queue, &mut doc, &mut history, budget);
            }
            assert!(!queue.busy());
            assert_eq!(doc.layers[0].pixels, both.layers[0].pixels);
            assert!(history.undo(&mut doc));
            assert_eq!(doc.layers[1].pixels.capacity(), 0);
        }
    }
    #[test]
    fn overflow_keeps_completed_queued_taps_and_reserves_end_markers() {
        let mut queue = Queue {
            command_limit: 8,
            ..Default::default()
        };
        let first = queue.begin(settings([1, 2, 3])).unwrap();
        queue.dab(first, sample(0)).unwrap();
        queue.end(first).unwrap();
        let second = queue.begin(settings([200, 70, 20])).unwrap();
        while queue.dab(second, sample(1)).is_ok() {}
        assert!(queue.commands.len() <= queue.command_limit);
        assert!(queue.queued_input_bytes() <= queue.byte_limit);
        assert!(queue.input_open.is_none());
        assert!(queue.end(second).is_err());
        let mut doc = Document::new(75, 73);
        let mut history = History::new();
        let mut completed = vec![];
        while queue.needs_work() {
            completed.extend(checked(&mut queue, &mut doc, &mut history, 1).completed);
        }
        assert_eq!(completed.len(), 2);
        assert_eq!(completed[0].id, first);
        assert!(!completed[0].cancelled);
        assert_eq!(completed[1].id, second);
        assert!(completed[1].cancelled);
        let mut expected = Document::new(75, 73);
        let settings = settings([1, 2, 3]);
        let mut buffer = StrokeBuffer::default();
        buffer.paint(&mut expected, sample(0), &settings);
        buffer.finish_tail(&mut expected, &settings);
        assert_eq!(doc.layers[0].pixels, expected.layers[0].pixels);
        assert!(history.undo(&mut doc));
        assert!(!history.can_undo());
    }
    #[test]
    fn streaming_pause_partial_cancel_and_noop_preserve_redo() {
        let mut doc = Document::new(75, 73);
        let mut history = History::new();
        let mut queue = Queue::default();
        submit(&mut queue, settings([1, 2, 3]));
        while queue.needs_work() {
            checked(&mut queue, &mut doc, &mut history, 64);
        }
        queue.undo().unwrap();
        while queue.needs_work() {
            checked(&mut queue, &mut doc, &mut history, 64);
        }
        let before = doc.layers[0].pixels.clone();
        let id = queue.begin(settings([200, 70, 20])).unwrap();
        queue.dab(id, sample(0)).unwrap();
        for step in 0..1000 {
            let progress = checked(&mut queue, &mut doc, &mut history, 1);
            if doc.layers[0].pixels != before {
                assert!(!progress.damage.is_empty());
                break;
            }
            assert!(step < 999, "queued dab never changed pixels");
        }
        assert!(queue.snapshot().is_err());
        assert!(queue.undo().is_err());
        assert!(queue.cancel_open());
        let started = checked(&mut queue, &mut doc, &mut history, 1);
        assert!(started.completed.is_empty());
        assert!(history.has_pending_stroke());
        let mut completed = vec![];
        while queue.needs_work() {
            let step = checked(&mut queue, &mut doc, &mut history, 1);
            if step.completed.is_empty() {
                assert!(history.has_pending_stroke());
            }
            completed.extend(step.completed);
        }
        assert_eq!(completed.len(), 1);
        assert_eq!(completed[0].id, id);
        assert!(completed[0].cancelled);
        assert_eq!(doc.layers[0].pixels, before);
        assert!(history.can_redo());
        assert!(!history.has_pending_stroke());
        let mut noop = (*settings([1, 2, 3])).clone();
        noop.brush.flow = 0.0;
        let id = queue.begin(Arc::new(noop)).unwrap();
        queue.dab(id, sample(0)).unwrap();
        while queue.needs_work() {
            checked(&mut queue, &mut doc, &mut history, 16);
        }
        assert!(queue.busy());
        assert!(!queue.needs_work());
        assert_eq!(
            checked(&mut queue, &mut doc, &mut history, 16).work_units,
            0
        );
        queue.end(id).unwrap();
        while queue.needs_work() {
            checked(&mut queue, &mut doc, &mut history, 16);
        }
        assert_eq!(doc.layers[0].pixels, before);
        assert!(history.can_redo());
        assert!(!queue.busy());
    }
    #[test]
    fn invalid_input_large_capacity_and_stale_targets_fail_without_losing_prior_results() {
        let mut queue = Queue::default();
        let mut large = (*settings([1, 2, 3])).clone();
        let mut points = Vec::with_capacity(MAX_INPUT_BYTES / 8 + 1);
        points.extend([[0.0, 0.0], [1.0, 1.0]]);
        large.brush.dynamics.size = Some(crate::dynamics::Dynamics {
            inputs: vec![crate::dynamics::Effector {
                input: crate::dynamics::Input::Pressure,
                curve: crate::dynamics::Curve { points },
            }],
            ..Default::default()
        });
        assert!(queue.begin(Arc::new(large)).is_err());
        assert!(!queue.busy());
        let good = submit(&mut queue, settings([1, 2, 3]));
        let invalid = queue.begin(settings([200, 70, 20])).unwrap();
        assert!(
            queue
                .dab(
                    invalid,
                    Sample {
                        x: f32::NAN,
                        ..sample(0)
                    }
                )
                .is_err()
        );
        let mut stale = (*settings([20, 200, 70])).clone();
        stale.layer = 1;
        let bad_target = queue.begin(Arc::new(stale)).unwrap();
        queue.dab(bad_target, sample(0)).unwrap();
        queue.end(bad_target).unwrap();
        let mut doc = Document::new(75, 73);
        let mut history = History::new();
        let mut completed = vec![];
        let mut error = None;
        while queue.needs_work() {
            let progress = queue.advance(&mut doc, &mut history, 4096).unwrap();
            completed.extend(progress.completed);
            error = error.or(progress.render_error);
        }
        assert!(error.is_some());
        assert_eq!(
            completed.iter().map(|c| c.id).collect::<Vec<_>>(),
            [good, invalid, bad_target]
        );
        assert!(completed[0].changed);
        assert!(completed[1].cancelled && completed[2].cancelled);
        let mut expected = Document::new(75, 73);
        reference(&mut expected, &settings([1, 2, 3]));
        assert_eq!(doc.layers[0].pixels, expected.layers[0].pixels);
        assert!(!history.has_pending_stroke());
    }
}
