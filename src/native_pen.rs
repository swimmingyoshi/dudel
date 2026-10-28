//! Observe pen messages before winit reads them. Leave message dispatch intact
//! for egui widgets; Input suppresses the duplicate painting events.
use crate::{input::Packet, stroke::Sample};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::{collections::HashMap, ptr};
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
    Graphics::Gdi::ClientToScreen,
    System::Performance::QueryPerformanceFrequency,
    UI::{
        Controls::POINTER_DEVICE_PROPERTY,
        Input::Pointer::*,
        Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass},
        WindowsAndMessaging::*,
    },
};

const SUBCLASS: usize = 0x44555045;
const MAX_HISTORY: usize = 4096;
const MAX_QUEUE: usize = 8192;

#[derive(Default)]
struct Stream {
    active: Option<u32>,
    queue: Vec<Packet>,
    in_range: bool,
    /// The pen in range is flipped to its eraser end, hovering or touching.
    inverted: bool,
    failed: bool,
    last: Option<(u32, u32, u32)>,
    pressure: Option<f32>,
}
impl Stream {
    fn cancel(&mut self) {
        if self.active.take().is_some() {
            self.push(Packet::Cancel);
        }
        self.last = None;
        self.pressure = None;
    }
    fn push(&mut self, packet: Packet) {
        // Leave one slot for cancellation. Retain completed strokes even when
        // an unfinished history burst exceeds the bound between UI drains.
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
            self.active = None;
            self.pressure = None;
            self.failed = true;
        } else {
            self.queue.push(packet);
        }
    }
    fn ingest(&mut self, id: u32, frame: u32, flags: u32, sample: Sample) {
        self.inverted = sample.eraser;
        if let Some((last_id, last_frame, last_flags)) = self.last
            && last_id == id
            && ((frame.wrapping_sub(last_frame) as i32) < 0
                || (frame == last_frame && flags == last_flags))
        {
            return;
        }
        if flags & POINTER_FLAG_DOWN != 0 {
            // Windows marks barrel/right-button gestures as secondary input.
            // Leave their UI dispatch intact, but never start a paint stroke.
            if sample.barrel && flags & POINTER_FLAG_FIRSTBUTTON == 0 {
                self.last = Some((id, frame, flags));
                return;
            }
            if self.active.is_some() && self.active != Some(id) {
                return;
            }
            self.failed = false;
            if self.active == Some(id) {
                return;
            }
            self.active = Some(id);
            self.pressure = sample.pressure_known.then_some(sample.pressure);
            self.push(Packet::Start(sample));
        } else if self.active == Some(id) && !self.failed {
            if sample.pressure_known {
                self.pressure = Some(sample.pressure);
            }
            if flags & (POINTER_FLAG_CANCELED | POINTER_FLAG_CAPTURECHANGED) != 0 {
                self.cancel();
            } else if flags & POINTER_FLAG_UP != 0 {
                self.push(Packet::End(sample));
                self.active = None;
            } else if flags & POINTER_FLAG_INCONTACT != 0 {
                self.push(Packet::Move(sample));
            }
        }
        self.last = Some((id, frame, flags));
    }
    fn resolve_pressure(&self, value: Option<f32>) -> (f32, bool) {
        match value {
            Some(value) => (value.clamp(0.0, 1.0), true),
            None => (
                if self.active.is_some() {
                    self.pressure.unwrap_or(1.0)
                } else {
                    1.0
                },
                false,
            ),
        }
    }
}

struct State {
    stream: Stream,
    alive: bool,
    enabled: bool,
    frequency: f64,
    properties: HashMap<usize, Option<POINTER_DEVICE_PROPERTY>>,
    tilts: HashMap<usize, [f32; 2]>,
    status: String,
    ctx: eframe::egui::Context,
}

pub struct Backend {
    hwnd: HWND,
    state: Box<State>,
}
impl Backend {
    #[cfg(test)]
    pub(crate) fn with_test_packets(packets: Vec<Packet>) -> Self {
        Self {
            hwnd: ptr::null_mut(),
            state: Box::new(State {
                stream: Stream {
                    queue: packets,
                    ..Default::default()
                },
                alive: false,
                enabled: true,
                frequency: 1.0,
                properties: HashMap::new(),
                tilts: HashMap::new(),
                status: String::new(),
                ctx: Default::default(),
            }),
        }
    }
    pub fn attach(cc: &eframe::CreationContext<'_>) -> Result<Self, String> {
        let handle = cc.window_handle().map_err(|e| e.to_string())?;
        let RawWindowHandle::Win32(handle) = handle.as_raw() else {
            return Err("No Windows window handle".into());
        };
        let hwnd = handle.hwnd.get() as HWND;
        Self::install(hwnd, cc.egui_ctx.clone())
    }
    fn install(hwnd: HWND, ctx: eframe::egui::Context) -> Result<Self, String> {
        let mut frequency = 0;
        // SAFETY: output is a valid i64; attachment and callback run on the UI thread.
        unsafe { QueryPerformanceFrequency(&mut frequency) };
        let mut state = Box::new(State {
            stream: Stream::default(),
            alive: true,
            enabled: true,
            frequency: frequency as f64,
            properties: HashMap::new(),
            tilts: HashMap::new(),
            status: "Windows pen history ready; mouse/egui fallback until a pen is detected".into(),
            ctx,
        });
        // The boxed address stays stable until Drop removes this subclass.
        if unsafe {
            SetWindowSubclass(
                hwnd,
                Some(callback),
                SUBCLASS,
                (&mut *state as *mut State) as usize,
            )
        } == 0
        {
            return Err("Could not attach Windows pen history reader".into());
        }
        Ok(Self { hwnd, state })
    }
    pub fn enabled(&self) -> bool {
        self.state.enabled
    }
    pub fn set_enabled(&mut self, enabled: bool) {
        if self.state.enabled != enabled {
            self.state.stream.cancel();
            self.state.stream.in_range = false;
            self.state.stream.last = None;
            self.state.enabled = enabled;
        }
    }
    /// The pen near the screen is flipped to its eraser end.
    pub fn eraser_in_range(&self) -> bool {
        self.state.enabled && self.state.stream.in_range && self.state.stream.inverted
    }
    pub fn drain(&mut self, pixels_per_point: f32) -> (Vec<Packet>, bool, String) {
        let suppress = self.state.enabled
            && (self.state.stream.active.is_some() || self.state.stream.in_range)
            && !self.state.stream.failed;
        let mut packets = std::mem::take(&mut self.state.stream.queue);
        let scale = if pixels_per_point.is_finite() && pixels_per_point > 0.0 {
            pixels_per_point
        } else {
            1.0
        };
        for packet in &mut packets {
            if let Packet::Start(s) | Packet::Move(s) | Packet::End(s) = packet {
                s.x /= scale;
                s.y /= scale;
            }
        }
        // A fault still delivers cancellation before fallback resumes next update.
        let suppress = suppress || !packets.is_empty();
        (
            packets,
            suppress,
            if self.state.enabled {
                self.state.status.clone()
            } else {
                "egui input fallback selected".into()
            },
        )
    }
}
impl Drop for Backend {
    fn drop(&mut self) {
        if self.state.alive {
            // SAFETY: the window has not issued NCDESTROY; UI-thread ownership.
            unsafe { RemoveWindowSubclass(self.hwnd, Some(callback), SUBCLASS) };
        }
    }
}

unsafe extern "system" fn callback(
    hwnd: HWND,
    msg: u32,
    w: WPARAM,
    l: LPARAM,
    _: usize,
    data: usize,
) -> LRESULT {
    // SAFETY: data is the stable State allocation installed above. NCDESTROY
    // removes the callback before the HWND can be reused; Drop owns the allocation.
    unsafe {
        let state = &mut *(data as *mut State);
        if msg == WM_NCDESTROY {
            state.alive = false;
            RemoveWindowSubclass(hwnd, Some(callback), SUBCLASS);
        } else if msg == WM_KILLFOCUS
            || (msg == WM_POINTERCAPTURECHANGED && state.stream.active == Some((w & 0xffff) as u32))
        {
            state.stream.cancel();
            state.stream.in_range = false;
        } else if msg == WM_POINTERLEAVE {
            state.stream.in_range = false;
            state.stream.inverted = false;
        } else if state.enabled && matches!(msg, WM_POINTERDOWN | WM_POINTERUPDATE | WM_POINTERUP) {
            let id = (w & 0xffff) as u32;
            let mut kind = 0;
            if GetPointerType(id, &mut kind) != 0 && kind == PT_PEN {
                if let Err(error) = read_history(hwnd, id, state) {
                    state.stream.cancel();
                    state.stream.failed = true;
                    state.status = format!("Pen history failed; egui fallback: {error}");
                }
                state.ctx.request_repaint();
            }
        }
        // Do not keep a State reference alive through downstream window dispatch.
        DefSubclassProc(hwnd, msg, w, l)
    }
}

fn pressure_property(device: *mut std::ffi::c_void) -> Option<POINTER_DEVICE_PROPERTY> {
    let mut count = 0;
    // SAFETY: query count before allocating; all output buffers are initialized.
    unsafe {
        if GetPointerDeviceProperties(device, &mut count, ptr::null_mut()) == 0 || count > 1024 {
            return None;
        }
        let mut properties = vec![POINTER_DEVICE_PROPERTY::default(); count as usize];
        if GetPointerDeviceProperties(device, &mut count, properties.as_mut_ptr()) == 0 {
            return None;
        }
        properties.into_iter().take(count as usize).find(|p| {
            p.usagePageId == 0x0d
                && p.usageId == 0x30
                && p.logicalMax > 1024
                && p.logicalMax > p.logicalMin
        })
    }
}

fn read_history(hwnd: HWND, id: u32, state: &mut State) -> Result<(), String> {
    let mut count = 0;
    // SAFETY: history is read before forwarding the current message. Size is bounded
    // and a changing history count is rejected instead of indexing truncated data.
    unsafe {
        if GetPointerPenInfoHistory(id, &mut count, ptr::null_mut()) == 0
            || count == 0
            || count as usize > MAX_HISTORY
        {
            return Err("History unavailable or exceeds bounded sample count".into());
        }
        let mut history = vec![POINTER_PEN_INFO::default(); count as usize];
        if GetPointerPenInfoHistory(id, &mut count, history.as_mut_ptr()) == 0
            || count as usize > history.len()
        {
            return Err("Incomplete pen history".into());
        }
        history.truncate(count as usize);
        let Some(newest) = history.first() else {
            return Ok(());
        };
        let device = newest.pointerInfo.sourceDevice;
        if state.properties.len() >= 32 && !state.properties.contains_key(&(device as usize)) {
            state.properties.clear();
            state.tilts.clear();
        }
        let property = *state
            .properties
            .entry(device as usize)
            .or_insert_with(|| pressure_property(device));
        let mut raw = vec![0; count as usize];
        let raw_valid = property
            .is_some_and(|p| GetRawPointerDeviceData(id, count, 1, &p, raw.as_mut_ptr()) != 0);
        state.status = match property {
            Some(p) => format!(
                "Windows pen history · raw pressure {}–{} ({})",
                p.logicalMin,
                p.logicalMax,
                if raw_valid {
                    "available"
                } else {
                    "normalized fallback"
                }
            ),
            None => "Windows pen history · normalized pressure 0–1024".into(),
        };
        let mut origin = POINT { x: 0, y: 0 };
        if ClientToScreen(hwnd, &mut origin) == 0 {
            return Err("Client coordinate mapping failed".into());
        }
        let mut device_rect = RECT::default();
        let mut display_rect = RECT::default();
        let mapped = GetPointerDeviceRects(device, &mut device_rect, &mut display_rect) != 0
            && device_rect.right > device_rect.left
            && device_rect.bottom > device_rect.top;
        for (index, pen) in history.iter().enumerate().rev() {
            let info = &pen.pointerInfo;
            let (x, y) = if mapped && info.sourceDevice == device {
                (
                    display_rect.left as f64
                        + (info.ptHimetricLocationRaw.x as f64 - device_rect.left as f64)
                            * (display_rect.right as f64 - display_rect.left as f64)
                            / (device_rect.right as f64 - device_rect.left as f64),
                    display_rect.top as f64
                        + (info.ptHimetricLocationRaw.y as f64 - device_rect.top as f64)
                            * (display_rect.bottom as f64 - display_rect.top as f64)
                            / (device_rect.bottom as f64 - device_rect.top as f64),
                )
            } else {
                (
                    info.ptPixelLocationRaw.x as f64,
                    info.ptPixelLocationRaw.y as f64,
                )
            };
            let value = if let Some(p) =
                property.filter(|_| raw_valid && info.sourceDevice == device)
            {
                Some(
                    ((raw[index] as f64 - p.logicalMin as f64)
                        / (p.logicalMax as f64 - p.logicalMin as f64)) as f32,
                )
            } else if pen.penMask & PEN_MASK_PRESSURE != 0 {
                Some(pen.pressure as f32 / 1024.0)
            } else {
                None
            };
            let (pressure, known) = state.stream.resolve_pressure(value);
            let key = info.sourceDevice as usize;
            let mut tilt = state.tilts.get(&key).copied();
            if pressure > 0.0
                && pen.penMask & (PEN_MASK_TILT_X | PEN_MASK_TILT_Y)
                    == PEN_MASK_TILT_X | PEN_MASK_TILT_Y
            {
                tilt = Some([pen.tiltX as f32, pen.tiltY as f32]);
                if let Some(tilt) = tilt {
                    state.tilts.insert(key, tilt);
                }
            }
            let sample = Sample {
                x: (x - origin.x as f64) as f32,
                y: (y - origin.y as f64) as f32,
                pressure: pressure.clamp(0.0, 1.0),
                pressure_known: known,
                time: (info.PerformanceCount != 0 && state.frequency > 0.0)
                    .then_some(info.PerformanceCount as f64 / state.frequency),
                tilt,
                twist: (pen.penMask & PEN_MASK_ROTATION != 0)
                    .then_some((pen.rotation % 360) as f32),
                eraser: pen.penFlags & (PEN_FLAG_INVERTED | PEN_FLAG_ERASER) != 0,
                barrel: pen.penFlags & PEN_FLAG_BARREL != 0,
                ..Default::default()
            };
            state
                .stream
                .ingest(id, info.frameId, info.pointerFlags, sample);
        }
        state.stream.in_range = newest.pointerInfo.pointerFlags & POINTER_FLAG_INRANGE != 0;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_flipped_pen_is_known_while_it_hovers_and_forgotten_when_it_leaves() {
        let mut stream = Stream::default();
        let hover = |eraser| Sample {
            eraser,
            ..Default::default()
        };
        stream.in_range = true;
        stream.ingest(1, 1, POINTER_FLAG_INRANGE, hover(true));
        assert!(
            stream.inverted && stream.queue.is_empty(),
            "hover draws nothing"
        );
        stream.ingest(1, 2, POINTER_FLAG_INRANGE, hover(false));
        assert!(!stream.inverted);
    }
    #[test]
    fn barrel_secondary_click_does_not_paint_but_primary_contact_can() {
        let mut stream = Stream::default();
        let sample = Sample {
            barrel: true,
            pressure: 1.0,
            ..Default::default()
        };
        stream.ingest(1, 1, POINTER_FLAG_DOWN | POINTER_FLAG_SECONDBUTTON, sample);
        assert!(stream.queue.is_empty());
        assert_eq!(stream.active, None);
        stream.ingest(1, 2, POINTER_FLAG_DOWN | POINTER_FLAG_FIRSTBUTTON, sample);
        assert!(matches!(stream.queue[0], Packet::Start(_)));
    }
    #[test]
    fn missing_lift_pressure_holds_the_contact_force_but_zero_remains_zero() {
        let mut stream = Stream::default();
        stream.ingest(
            1,
            1,
            POINTER_FLAG_DOWN,
            Sample {
                pressure: 0.2,
                pressure_known: true,
                ..Default::default()
            },
        );
        assert_eq!(stream.resolve_pressure(None), (0.2, false));
        assert_eq!(stream.resolve_pressure(Some(0.0)), (0.0, true));
        stream.cancel();
        assert_eq!(stream.resolve_pressure(None), (1.0, false));
    }
    #[test]
    fn hidden_window_subclass_cancels_and_detaches_before_window_destruction() {
        let class: Vec<u16> = "STATIC\0".encode_utf16().collect();
        // SAFETY: a message-only test window, created/destroyed on this thread;
        // the built-in STATIC class and buffers remain valid through the calls.
        unsafe {
            let hwnd = CreateWindowExW(
                0,
                class.as_ptr(),
                class.as_ptr(),
                0,
                0,
                0,
                1,
                1,
                HWND_MESSAGE,
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null(),
            );
            assert!(!hwnd.is_null());
            let mut backend = Backend::install(hwnd, Default::default()).unwrap();
            backend.state.stream.active = Some(7);
            SendMessageW(hwnd, WM_KILLFOCUS, 0, 0);
            assert!(matches!(
                backend.state.stream.queue.last(),
                Some(Packet::Cancel)
            ));
            assert_ne!(DestroyWindow(hwnd), 0);
            assert!(!backend.state.alive);
            drop(backend);
        }
    }
    #[test]
    fn drain_scales_native_coordinates_and_allows_mouse_after_pen_leaves() {
        let mut backend = Backend {
            hwnd: ptr::null_mut(),
            state: Box::new(State {
                stream: Stream::default(),
                alive: false,
                enabled: true,
                frequency: 1.0,
                properties: HashMap::new(),
                tilts: HashMap::new(),
                status: String::new(),
                ctx: Default::default(),
            }),
        };
        backend.state.stream.queue.push(Packet::Start(Sample {
            x: 300.0,
            y: 150.0,
            ..Default::default()
        }));
        let (packets, suppress, _) = backend.drain(1.5);
        assert!(suppress);
        assert!(matches!(packets[0],Packet::Start(s) if s.x==200.0 && s.y==100.0));
        assert!(!backend.drain(1.5).1);
        backend.state.stream.active = Some(1);
        backend.state.stream.in_range = true;
        assert!(backend.drain(1.5).1);
        backend.set_enabled(false);
        let (packets, suppress, _) = backend.drain(1.5);
        assert!(suppress);
        assert!(matches!(packets[0], Packet::Cancel));
        assert!(!backend.drain(1.5).1);
    }
    #[test]
    fn sample_pressure_zero_and_order_survive_history_and_mouse_duplicates() {
        let mut stream = Stream::default();
        for (frame, flags, pressure) in [
            (1, POINTER_FLAG_DOWN, 0.0),
            (2, POINTER_FLAG_INCONTACT, 0.2),
            (3, POINTER_FLAG_UP, 0.8),
        ] {
            let sample = Sample {
                pressure,
                pressure_known: true,
                time: Some(frame as f64 * 0.01),
                ..Default::default()
            };
            stream.ingest(7, frame, flags, sample);
            stream.ingest(7, frame, flags, sample);
        }
        assert_eq!(stream.queue.len(), 3);
        assert!(matches!(stream.queue[0], Packet::Start(s) if s.pressure == 0.0));
        assert!(matches!(stream.queue[1], Packet::Move(s) if s.pressure == 0.2));
        assert!(matches!(stream.queue[2], Packet::End(s) if s.pressure == 0.8));
        assert_eq!(stream.active, None);
    }
    #[test]
    fn overlapping_history_does_not_replay_old_samples() {
        let mut stream = Stream::default();
        stream.ingest(7, 1, POINTER_FLAG_DOWN, Sample::default());
        for frame in [2, 3, 2, 3, 4] {
            stream.ingest(
                7,
                frame,
                POINTER_FLAG_INCONTACT,
                Sample {
                    x: frame as f32,
                    ..Default::default()
                },
            );
        }
        assert_eq!(stream.queue.len(), 4);
        stream.ingest(7, 4, POINTER_FLAG_UP, Sample::default());
        assert!(matches!(stream.queue.last(), Some(Packet::End(_))));
    }
    #[test]
    fn secondary_pen_and_overflow_do_not_merge_or_commit_strokes() {
        let mut stream = Stream::default();
        stream.ingest(1, 1, POINTER_FLAG_DOWN, Sample::default());
        stream.ingest(2, 2, POINTER_FLAG_DOWN, Sample::default());
        assert_eq!(stream.active, Some(1));
        for i in 2..=MAX_QUEUE as u32 + 2 {
            stream.ingest(1, i, POINTER_FLAG_INCONTACT, Sample::default());
        }
        assert!(stream.failed);
        assert_eq!(stream.queue.len(), 1);
        assert!(matches!(stream.queue[0], Packet::Cancel));
        stream.ingest(1, 10000, POINTER_FLAG_DOWN, Sample::default());
        stream.cancel();
        assert!(matches!(stream.queue.last(), Some(Packet::Cancel)));
    }

    #[test]
    fn overflow_retains_completed_taps_and_cancels_only_the_unfinished_suffix() {
        let mut stream = Stream::default();
        let tap = Sample {
            x: 17.0,
            pressure: 0.7,
            pressure_known: true,
            ..Default::default()
        };
        stream.ingest(1, 1, POINTER_FLAG_DOWN, tap);
        stream.ingest(1, 2, POINTER_FLAG_UP, tap);
        stream.ingest(1, 3, POINTER_FLAG_DOWN, Sample::default());
        for frame in 4..MAX_QUEUE as u32 + 20 {
            stream.ingest(1, frame, POINTER_FLAG_INCONTACT, Sample::default());
        }
        assert!(stream.failed);
        assert_eq!(stream.active, None);
        assert_eq!(stream.pressure, None);
        assert_eq!(stream.queue.len(), 3);
        assert!(matches!(stream.queue[0], Packet::Start(s) if s == tap));
        assert!(matches!(stream.queue[1], Packet::End(s) if s == tap));
        assert!(matches!(stream.queue[2], Packet::Cancel));
        // Later real contacts may recover without dropping those queued taps.
        stream.ingest(1, 10000, POINTER_FLAG_DOWN, tap);
        stream.ingest(1, 10001, POINTER_FLAG_UP, tap);
        assert!(!stream.failed);
        assert_eq!(stream.queue.len(), 5);
        assert!(matches!(stream.queue[4], Packet::End(s) if s == tap));
    }

    #[test]
    fn saturation_by_completed_taps_stays_bounded_across_new_contacts() {
        let mut stream = Stream::default();
        for frame in 0..MAX_QUEUE as u32 * 2 {
            stream.ingest(
                1,
                frame,
                if frame % 2 == 0 {
                    POINTER_FLAG_DOWN
                } else {
                    POINTER_FLAG_UP
                },
                Sample::default(),
            );
            assert!(stream.queue.len() <= MAX_QUEUE);
        }
        let closed = stream
            .queue
            .iter()
            .filter(|p| matches!(p, Packet::End(_)))
            .count();
        assert_eq!(closed, (MAX_QUEUE - 2) / 2);
        assert!(matches!(stream.queue.last(), Some(Packet::Cancel)));
        assert!(stream.failed);
        assert_eq!(stream.active, None);
        let old = std::mem::take(&mut stream.queue);
        stream.ingest(1, 20000, POINTER_FLAG_DOWN, Sample::default());
        stream.ingest(1, 20001, POINTER_FLAG_UP, Sample::default());
        assert_eq!(old.len(), MAX_QUEUE - 1);
        assert_eq!(stream.queue.len(), 2);
        assert!(!stream.failed);
    }
}
