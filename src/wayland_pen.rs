//! Read Wayland tablet-v2 tool events on eframe's own display connection.
//! Binding a tablet seat ends the compositor's pointer emulation for the pen,
//! so egui widgets get pointer events rebuilt from the same tool frames;
//! Input suppresses those for painting, as it does for Windows pen history.
use crate::{input::Packet, stroke::Sample};
use eframe::egui;
use raw_window_handle::{HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle};
use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex, MutexGuard,
        atomic::{AtomicBool, Ordering},
    },
    thread::JoinHandle,
};
use wayland_client::{
    Connection, Dispatch, Proxy, QueueHandle, WEnum,
    backend::{self, ObjectId},
    delegate_noop, event_created_child,
    globals::{GlobalListContents, registry_queue_init},
    protocol::{
        wl_callback::WlCallback, wl_registry::WlRegistry, wl_seat::WlSeat, wl_surface::WlSurface,
    },
};
use wayland_protocols::wp::tablet::zv2::client::{
    zwp_tablet_manager_v2::ZwpTabletManagerV2,
    zwp_tablet_pad_dial_v2::ZwpTabletPadDialV2,
    zwp_tablet_pad_group_v2::{self, ZwpTabletPadGroupV2},
    zwp_tablet_pad_ring_v2::ZwpTabletPadRingV2,
    zwp_tablet_pad_strip_v2::ZwpTabletPadStripV2,
    zwp_tablet_pad_v2::{self, ZwpTabletPadV2},
    zwp_tablet_seat_v2::{self, ZwpTabletSeatV2},
    zwp_tablet_tool_v2::{self, ZwpTabletToolV2},
    zwp_tablet_v2::ZwpTabletV2,
};

const MAX_QUEUE: usize = 8192;
const MAX_UI_EVENTS: usize = 1024;
/// linux/input-event-codes.h barrel buttons.
const BTN_STYLUS: u32 = 0x14b;
const BTN_STYLUS2: u32 = 0x14c;

/// Pointer events for egui widgets, in surface-local logical coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
enum UiEvent {
    Moved([f32; 2]),
    Button {
        pos: [f32; 2],
        button: egui::PointerButton,
        pressed: bool,
    },
    Gone,
}

struct Shared {
    /// Packets in surface-local logical coordinates.
    queue: Vec<Packet>,
    ui: Vec<UiEvent>,
    active: bool,
    in_range: bool,
    /// The tool in proximity is an eraser.
    inverted: bool,
    failed: bool,
    enabled: bool,
    status: String,
}
impl Default for Shared {
    fn default() -> Self {
        Self {
            queue: vec![],
            ui: vec![],
            active: false,
            in_range: false,
            inverted: false,
            failed: false,
            enabled: true,
            status: String::new(),
        }
    }
}
impl Shared {
    fn cancel(&mut self) {
        if std::mem::take(&mut self.active) {
            self.push(Packet::Cancel);
        }
    }
    fn push(&mut self, packet: Packet) {
        // Leave one slot for cancellation. Retain completed strokes even when
        // an unfinished burst exceeds the bound between UI drains.
        if self.queue.len() >= MAX_QUEUE - 1 {
            let completed = self
                .queue
                .iter()
                .rposition(|p| matches!(p, Packet::End(_) | Packet::Cancel))
                .map_or(0, |i| i + 1);
            self.queue.truncate(completed);
            if !matches!(self.queue.last(), Some(Packet::Cancel)) {
                self.queue.push(Packet::Cancel);
            }
            self.active = false;
            self.failed = true;
        } else {
            self.queue.push(packet);
        }
    }
    fn push_ui(&mut self, event: UiEvent) {
        // Hover motion is only useful as the latest position; drop the oldest.
        if self.ui.len() >= MAX_UI_EVENTS {
            self.ui.remove(0);
        }
        self.ui.push(event);
    }
}

/// One tablet tool; axis values persist between frames, flags are per frame.
#[derive(Default)]
struct Tool {
    eraser: bool,
    on_surface: bool,
    pos: [f32; 2],
    pressure: Option<f32>,
    tilt: Option<[f32; 2]>,
    /// The first tilt reading, kept until one differs from it.
    first_tilt: Option<[f32; 2]>,
    rotation: Option<f32>,
    barrel: bool,
    painting: bool,
    entered: bool,
    changed: bool,
    down: bool,
    up: bool,
    left: bool,
    buttons: Vec<(egui::PointerButton, bool)>,
}
impl Tool {
    /// A pen without a tilt sensor can still report one fixed tilt, as the
    /// XP-Pen driver's virtual pen does, so tilt counts once a reading moves.
    fn set_tilt(&mut self, tilt: [f32; 2]) {
        let first = *self.first_tilt.get_or_insert(tilt);
        if self.tilt.is_some() || tilt != first {
            self.tilt = Some(tilt);
        }
    }
    /// Close one tool frame: queue paint packets and widget pointer events.
    fn finish_frame(&mut self, shared: &mut Shared, time: Option<f64>) {
        if self.on_surface || self.left {
            let sample = Sample {
                x: self.pos[0],
                y: self.pos[1],
                pressure: self.pressure.unwrap_or(1.0).clamp(0.0, 1.0),
                pressure_known: self.pressure.is_some(),
                time,
                tilt: self.tilt,
                twist: self.rotation,
                eraser: self.eraser,
                barrel: self.barrel,
                ..Default::default()
            };
            if shared.enabled {
                // A barrel-held contact is a secondary gesture: UI only, never paint.
                if self.down && !self.barrel && !shared.active {
                    shared.failed = false;
                    shared.active = true;
                    self.painting = true;
                    shared.push(Packet::Start(sample));
                } else if self.painting && self.changed && !self.up && !self.left {
                    shared.push(Packet::Move(sample));
                }
                if self.painting && self.up {
                    shared.push(Packet::End(sample));
                    shared.active = false;
                    self.painting = false;
                }
            }
            if self.painting && (self.left || !shared.enabled || !shared.active) {
                self.painting = false;
                shared.cancel();
            }
            if self.entered || self.changed || self.down {
                shared.push_ui(UiEvent::Moved(self.pos));
            }
            for &(button, pressed) in &self.buttons {
                shared.push_ui(UiEvent::Button {
                    pos: self.pos,
                    button,
                    pressed,
                });
            }
            if self.down || self.up {
                shared.push_ui(UiEvent::Button {
                    pos: self.pos,
                    button: egui::PointerButton::Primary,
                    pressed: self.down && !self.up,
                });
            }
            if self.left {
                shared.push_ui(UiEvent::Gone);
                self.on_surface = false;
                self.pressure = None;
            }
        }
        self.entered = false;
        self.changed = false;
        self.down = false;
        self.up = false;
        self.left = false;
        self.buttons.clear();
    }
}

struct Dispatcher {
    shared: Arc<Mutex<Shared>>,
    ctx: egui::Context,
    /// Protocol id of eframe's window surface.
    surface: u32,
    tools: HashMap<u32, Tool>,
}
impl Dispatcher {
    fn shared(&self) -> MutexGuard<'_, Shared> {
        lock(&self.shared)
    }
}

fn lock(shared: &Mutex<Shared>) -> MutexGuard<'_, Shared> {
    shared.lock().unwrap_or_else(|e| e.into_inner())
}

struct Reader {
    conn: Connection,
    qh: QueueHandle<Dispatcher>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

pub struct Backend {
    shared: Arc<Mutex<Shared>>,
    ctx: egui::Context,
    reader: Option<Reader>,
}
impl Backend {
    #[cfg(test)]
    pub(crate) fn with_test_packets(packets: Vec<Packet>) -> Self {
        Self {
            shared: Arc::new(Mutex::new(Shared {
                queue: packets,
                ..Default::default()
            })),
            ctx: Default::default(),
            reader: None,
        }
    }
    pub fn attach(cc: &eframe::CreationContext<'_>) -> Result<Self, String> {
        let display = cc.display_handle().map_err(|e| e.to_string())?;
        let RawDisplayHandle::Wayland(display) = display.as_raw() else {
            return Err("Not a Wayland session".into());
        };
        let window = cc.window_handle().map_err(|e| e.to_string())?;
        let RawWindowHandle::Wayland(window) = window.as_raw() else {
            return Err("No Wayland window surface".into());
        };
        // SAFETY: eframe keeps its event loop, and so the wl_display, alive past
        // the app; Drop joins the reader before this Connection is released.
        let conn = Connection::from_backend(unsafe {
            backend::Backend::from_foreign_display(display.display.as_ptr().cast())
        });
        // SAFETY: the window surface is a live wl_surface proxy owned by winit.
        let surface =
            unsafe { ObjectId::from_ptr(WlSurface::interface(), window.surface.as_ptr().cast()) }
                .map_err(|e| e.to_string())?
                .protocol_id();
        let (globals, mut queue) =
            registry_queue_init::<Dispatcher>(&conn).map_err(|e| e.to_string())?;
        let qh = queue.handle();
        let manager: ZwpTabletManagerV2 = globals
            .bind(&qh, 1..=1, ())
            .map_err(|_| "Compositor has no tablet protocol (zwp_tablet_manager_v2)")?;
        let seats: Vec<_> = globals.contents().with_list(|list| {
            list.iter()
                .filter(|g| g.interface == WlSeat::interface().name)
                .map(|g| g.name)
                .collect()
        });
        if seats.is_empty() {
            return Err("No Wayland seat".into());
        }
        for name in seats {
            let seat: WlSeat = globals.registry().bind(name, 1, &qh, ());
            manager.get_tablet_seat(&seat, &qh, ());
        }
        let shared = Arc::new(Mutex::new(Shared {
            status: "Wayland tablet input ready; mouse/egui fallback until a pen is detected"
                .into(),
            ..Default::default()
        }));
        let mut dispatcher = Dispatcher {
            shared: shared.clone(),
            ctx: cc.egui_ctx.clone(),
            surface,
            tools: HashMap::new(),
        };
        // Receive the tablets and tools that already exist before reporting ready.
        queue
            .roundtrip(&mut dispatcher)
            .map_err(|e| e.to_string())?;
        let stop = Arc::new(AtomicBool::new(false));
        let thread = std::thread::Builder::new()
            .name("wayland-pen".into())
            .spawn({
                let stop = stop.clone();
                move || {
                    while !stop.load(Ordering::Acquire) {
                        if let Err(error) = queue.blocking_dispatch(&mut dispatcher) {
                            let mut shared = dispatcher.shared();
                            shared.cancel();
                            shared.in_range = false;
                            shared.failed = true;
                            shared.status = format!("Wayland tablet input stopped: {error}");
                            dispatcher.ctx.request_repaint();
                            break;
                        }
                    }
                }
            })
            .map_err(|e| e.to_string())?;
        Ok(Self {
            shared,
            ctx: cc.egui_ctx.clone(),
            reader: Some(Reader {
                conn,
                qh,
                stop,
                thread: Some(thread),
            }),
        })
    }
    pub fn enabled(&self) -> bool {
        lock(&self.shared).enabled
    }
    pub fn set_enabled(&mut self, enabled: bool) {
        let mut shared = lock(&self.shared);
        if shared.enabled != enabled {
            shared.cancel();
            shared.enabled = enabled;
        }
    }
    fn zoom(&self) -> f32 {
        let zoom = self.ctx.zoom_factor();
        if zoom.is_finite() && zoom > 0.0 {
            zoom
        } else {
            1.0
        }
    }
    /// Surface coordinates are logical pixels; egui points also divide by zoom.
    /// The tool near the screen is an eraser (or a flipped pen).
    pub fn eraser_in_range(&self) -> bool {
        let shared = lock(&self.shared);
        shared.enabled && shared.in_range && shared.inverted
    }
    pub fn drain(&mut self, pixels_per_point: f32) -> (Vec<Packet>, bool, String) {
        let _ = pixels_per_point;
        let zoom = self.zoom();
        let mut shared = lock(&self.shared);
        let suppress = shared.enabled && (shared.active || shared.in_range) && !shared.failed;
        let mut packets = std::mem::take(&mut shared.queue);
        for packet in &mut packets {
            if let Packet::Start(s) | Packet::Move(s) | Packet::End(s) = packet {
                s.x /= zoom;
                s.y /= zoom;
            }
        }
        // A fault still delivers cancellation before fallback resumes next update.
        let suppress = suppress || !packets.is_empty();
        (
            packets,
            suppress,
            if shared.enabled {
                shared.status.clone()
            } else {
                "egui input fallback selected".into()
            },
        )
    }
    /// Widget pointer events for this frame, which the compositor no longer
    /// emulates once the tablet seat is bound.
    pub fn pointer_events(&mut self, modifiers: egui::Modifiers) -> Vec<egui::Event> {
        let zoom = self.zoom();
        let pos = |[x, y]: [f32; 2]| egui::pos2(x / zoom, y / zoom);
        std::mem::take(&mut lock(&self.shared).ui)
            .into_iter()
            .map(|event| match event {
                UiEvent::Moved(p) => egui::Event::PointerMoved(pos(p)),
                UiEvent::Button {
                    pos: p,
                    button,
                    pressed,
                } => egui::Event::PointerButton {
                    pos: pos(p),
                    button,
                    pressed,
                    modifiers,
                },
                UiEvent::Gone => egui::Event::PointerGone,
            })
            .collect()
    }
}
impl Drop for Backend {
    fn drop(&mut self) {
        let Some(reader) = &mut self.reader else {
            return;
        };
        reader.stop.store(true, Ordering::Release);
        // A sync reply on the reader's queue wakes its blocking dispatch.
        reader.conn.display().sync(&reader.qh, ());
        if reader.conn.flush().is_ok()
            && let Some(thread) = reader.thread.take()
        {
            let _ = thread.join();
        }
    }
}

impl Dispatch<ZwpTabletToolV2, ()> for Dispatcher {
    fn event(
        state: &mut Self,
        tool: &ZwpTabletToolV2,
        event: zwp_tablet_tool_v2::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use zwp_tablet_tool_v2::{ButtonState, Event, Type};
        let id = tool.id().protocol_id();
        if let Event::Removed = event {
            if let Some(mut removed) = state.tools.remove(&id)
                && removed.painting
            {
                removed.painting = false;
                state.shared().cancel();
            }
            let in_range = state.tools.values().any(|t| t.on_surface);
            let inverted = state.tools.values().any(|t| t.on_surface && t.eraser);
            state.shared().in_range = in_range;
            state.shared().inverted = inverted;
            tool.destroy();
            return;
        }
        let surface = state.surface;
        let t = state.tools.entry(id).or_default();
        match event {
            Event::Type {
                tool_type: WEnum::Value(kind),
            } => t.eraser = kind == Type::Eraser,
            Event::ProximityIn { surface: s, .. } => {
                t.on_surface = s.id().protocol_id() == surface;
                t.entered = t.on_surface;
            }
            Event::ProximityOut => t.left = t.on_surface,
            Event::Down { .. } => t.down = true,
            Event::Up => t.up = true,
            Event::Motion { x, y } => {
                t.pos = [x as f32, y as f32];
                t.changed = true;
            }
            Event::Pressure { pressure } => {
                t.pressure = Some(pressure as f32 / 65535.0);
                t.changed = true;
            }
            Event::Tilt { tilt_x, tilt_y } => {
                t.set_tilt([tilt_x as f32, tilt_y as f32]);
                t.changed = true;
            }
            Event::Rotation { degrees } => {
                t.rotation = Some(degrees as f32);
                t.changed = true;
            }
            Event::Button { button, state, .. } if matches!(button, BTN_STYLUS | BTN_STYLUS2) => {
                let pressed = state == WEnum::Value(ButtonState::Pressed);
                if button == BTN_STYLUS {
                    t.barrel = pressed;
                }
                t.buttons.push((
                    if button == BTN_STYLUS {
                        egui::PointerButton::Secondary
                    } else {
                        egui::PointerButton::Middle
                    },
                    pressed,
                ));
            }
            Event::Frame { time } => {
                let mut shared = lock(&state.shared);
                let pressure = t.pressure.is_some();
                t.finish_frame(&mut shared, Some(time as f64 / 1000.0));
                if shared.enabled && t.on_surface {
                    shared.status = format!(
                        "Wayland tablet input · {}",
                        if pressure {
                            "pressure 0–65535"
                        } else {
                            "no pressure reported"
                        }
                    );
                }
                shared.in_range = state.tools.values().any(|t| t.on_surface);
                shared.inverted = state.tools.values().any(|t| t.on_surface && t.eraser);
                drop(shared);
                state.ctx.request_repaint();
            }
            _ => {}
        }
    }
}

impl Dispatch<ZwpTabletSeatV2, ()> for Dispatcher {
    fn event(
        _: &mut Self,
        _: &ZwpTabletSeatV2,
        _: zwp_tablet_seat_v2::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
    event_created_child!(Dispatcher, ZwpTabletSeatV2, [
        zwp_tablet_seat_v2::EVT_TABLET_ADDED_OPCODE => (ZwpTabletV2, ()),
        zwp_tablet_seat_v2::EVT_TOOL_ADDED_OPCODE => (ZwpTabletToolV2, ()),
        zwp_tablet_seat_v2::EVT_PAD_ADDED_OPCODE => (ZwpTabletPadV2, ()),
    ]);
}

impl Dispatch<ZwpTabletPadV2, ()> for Dispatcher {
    fn event(
        _: &mut Self,
        _: &ZwpTabletPadV2,
        _: zwp_tablet_pad_v2::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
    event_created_child!(Dispatcher, ZwpTabletPadV2, [
        zwp_tablet_pad_v2::EVT_GROUP_OPCODE => (ZwpTabletPadGroupV2, ()),
    ]);
}

impl Dispatch<ZwpTabletPadGroupV2, ()> for Dispatcher {
    fn event(
        _: &mut Self,
        _: &ZwpTabletPadGroupV2,
        _: zwp_tablet_pad_group_v2::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
    event_created_child!(Dispatcher, ZwpTabletPadGroupV2, [
        zwp_tablet_pad_group_v2::EVT_RING_OPCODE => (ZwpTabletPadRingV2, ()),
        zwp_tablet_pad_group_v2::EVT_STRIP_OPCODE => (ZwpTabletPadStripV2, ()),
        zwp_tablet_pad_group_v2::EVT_DIAL_OPCODE => (ZwpTabletPadDialV2, ()),
    ]);
}

impl Dispatch<WlRegistry, GlobalListContents> for Dispatcher {
    fn event(
        _: &mut Self,
        _: &WlRegistry,
        _: wayland_client::protocol::wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

delegate_noop!(Dispatcher: ZwpTabletManagerV2);
delegate_noop!(Dispatcher: ignore WlSeat);
delegate_noop!(Dispatcher: ignore WlCallback);
delegate_noop!(Dispatcher: ignore ZwpTabletV2);
delegate_noop!(Dispatcher: ignore ZwpTabletPadRingV2);
delegate_noop!(Dispatcher: ignore ZwpTabletPadStripV2);
delegate_noop!(Dispatcher: ignore ZwpTabletPadDialV2);

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(tool: &mut Tool, shared: &mut Shared, edit: impl FnOnce(&mut Tool)) {
        edit(tool);
        tool.finish_frame(shared, Some(0.0));
    }
    fn hovering() -> (Tool, Shared) {
        let mut tool = Tool::default();
        let mut shared = Shared::default();
        frame(&mut tool, &mut shared, |t| {
            t.on_surface = true;
            t.entered = true;
            t.pos = [10.0, 20.0];
        });
        (tool, shared)
    }

    #[test]
    fn pressure_stroke_starts_moves_and_ends_with_axes() {
        let (mut tool, mut shared) = hovering();
        frame(&mut tool, &mut shared, |t| {
            t.down = true;
            t.pressure = Some(0.25);
            t.tilt = Some([10.0, -5.0]);
            t.changed = true;
        });
        frame(&mut tool, &mut shared, |t| {
            t.pos = [12.0, 22.0];
            t.pressure = Some(0.5);
            t.changed = true;
        });
        frame(&mut tool, &mut shared, |t| t.up = true);
        assert!(matches!(shared.queue[0], Packet::Start(s)
            if s.pressure == 0.25 && s.pressure_known && s.tilt == Some([10.0, -5.0])));
        assert!(matches!(shared.queue[1], Packet::Move(s) if s.x == 12.0 && s.pressure == 0.5));
        assert!(matches!(shared.queue[2], Packet::End(_)));
        assert_eq!(shared.queue.len(), 3);
        assert!(!shared.active && !tool.painting);
        assert_eq!(
            shared
                .ui
                .iter()
                .filter(|e| matches!(e, UiEvent::Button { .. }))
                .count(),
            2
        );
    }

    #[test]
    fn fixed_tilt_is_ignored_until_it_moves() {
        let mut tool = Tool::default();
        tool.set_tilt([-1.0, -1.0]);
        tool.set_tilt([-1.0, -1.0]);
        assert_eq!(tool.tilt, None);
        tool.set_tilt([-2.0, -1.0]);
        assert_eq!(tool.tilt, Some([-2.0, -1.0]));
        tool.set_tilt([-1.0, -1.0]);
        assert_eq!(tool.tilt, Some([-1.0, -1.0]));
    }

    #[test]
    fn barrel_contact_clicks_widgets_but_never_paints() {
        let (mut tool, mut shared) = hovering();
        frame(&mut tool, &mut shared, |t| {
            t.barrel = true;
            t.buttons.push((egui::PointerButton::Secondary, true));
        });
        frame(&mut tool, &mut shared, |t| t.down = true);
        frame(&mut tool, &mut shared, |t| t.up = true);
        assert!(shared.queue.is_empty());
        assert!(shared.ui.contains(&UiEvent::Button {
            pos: [10.0, 20.0],
            button: egui::PointerButton::Secondary,
            pressed: true,
        }));
    }

    #[test]
    fn leaving_proximity_cancels_the_open_stroke_and_drops_pressure() {
        let (mut tool, mut shared) = hovering();
        frame(&mut tool, &mut shared, |t| {
            t.down = true;
            t.pressure = Some(0.7);
        });
        frame(&mut tool, &mut shared, |t| t.left = true);
        assert!(matches!(shared.queue.last(), Some(Packet::Cancel)));
        assert_eq!(shared.ui.last(), Some(&UiEvent::Gone));
        assert!(!tool.on_surface && tool.pressure.is_none() && !shared.active);
    }

    #[test]
    fn other_surfaces_are_ignored() {
        let mut tool = Tool::default();
        let mut shared = Shared::default();
        frame(&mut tool, &mut shared, |t| {
            t.down = true;
            t.changed = true;
        });
        assert!(shared.queue.is_empty() && shared.ui.is_empty());
    }

    #[test]
    fn disabled_backend_only_forwards_widget_events() {
        let (mut tool, mut shared) = hovering();
        shared.enabled = false;
        frame(&mut tool, &mut shared, |t| t.down = true);
        assert!(shared.queue.is_empty());
        let mut backend = Backend::with_test_packets(vec![]);
        lock(&backend.shared).enabled = false;
        lock(&backend.shared).ui = shared.ui.clone();
        let (_, suppress, status) = backend.drain(1.0);
        assert!(!suppress);
        assert_eq!(status, "egui input fallback selected");
        assert!(matches!(
            backend.pointer_events(Default::default()).last(),
            Some(egui::Event::PointerButton { pressed: true, .. })
        ));
    }

    #[test]
    fn drain_suppresses_fallback_while_the_pen_is_in_range() {
        let mut backend = Backend::with_test_packets(vec![Packet::Start(Sample {
            x: 30.0,
            ..Default::default()
        })]);
        let (packets, suppress, _) = backend.drain(2.0);
        assert!(suppress);
        assert!(matches!(packets[0], Packet::Start(s) if s.x == 30.0));
        assert!(!backend.drain(2.0).1);
        lock(&backend.shared).in_range = true;
        assert!(backend.drain(2.0).1);
        backend.set_enabled(false);
        assert!(!backend.drain(2.0).1);
    }

    #[test]
    fn overflow_keeps_completed_strokes_and_cancels_the_open_one() {
        let (mut tool, mut shared) = hovering();
        frame(&mut tool, &mut shared, |t| t.down = true);
        frame(&mut tool, &mut shared, |t| t.up = true);
        frame(&mut tool, &mut shared, |t| t.down = true);
        for i in 0..MAX_QUEUE {
            frame(&mut tool, &mut shared, |t| {
                t.pos = [i as f32, 0.0];
                t.changed = true;
            });
        }
        assert!(shared.failed && !tool.painting);
        assert_eq!(shared.queue.len(), 3);
        assert!(matches!(shared.queue[1], Packet::End(_)));
        assert!(matches!(shared.queue[2], Packet::Cancel));
    }
}
