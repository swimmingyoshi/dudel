use crate::stroke::Sample;
use eframe::egui::{Event, PointerButton, TouchId, TouchPhase};

#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize)]
pub enum Packet {
    Start(Sample),
    Move(Sample),
    End(Sample),
    Cancel,
}

pub struct Input {
    pen: Option<TouchId>,
    mouse_down: bool,
    pressure: f32,
    pub native_requested: bool,
    pub diagnostics: Diagnostics,
    #[cfg(any(windows, target_os = "linux"))]
    native: Option<crate::native_pen::Backend>,
    /// Until when a driver's own right-click is the side button's echo.
    #[cfg(windows)]
    side_echo_until: f64,
    /// Tests stand in for a pen flipped to its eraser end.
    #[cfg(test)]
    pub(crate) test_eraser_in_range: bool,
}
impl Default for Input {
    fn default() -> Self {
        Self {
            pen: None,
            mouse_down: false,
            pressure: 0.0,
            native_requested: true,
            diagnostics: Default::default(),
            #[cfg(any(windows, target_os = "linux"))]
            native: None,
            #[cfg(windows)]
            side_echo_until: f64::NEG_INFINITY,
            #[cfg(test)]
            test_eraser_in_range: false,
        }
    }
}

#[derive(Default)]
pub struct Diagnostics {
    pub packets: u64,
    pub pressure: Option<f32>,
    pub last: Option<Sample>,
    pub native: bool,
    pub status: String,
    recent: std::collections::VecDeque<Packet>,
}

impl Diagnostics {
    fn record(&mut self, packets: &[Packet]) {
        for packet in packets {
            self.packets += 1;
            match packet {
                Packet::Start(s) | Packet::Move(s) | Packet::End(s) => {
                    self.last = Some(*s);
                    self.pressure = s.pressure_known.then_some(s.pressure);
                }
                Packet::Cancel => self.pressure = None,
            }
            if matches!(packet, Packet::End(_)) {
                self.pressure = None;
            }
            if self.recent.len() == 4096 {
                self.recent.pop_front();
            }
            self.recent.push_back(*packet);
        }
    }
    pub fn export(&self, path: &std::path::Path) -> Result<(), String> {
        #[derive(serde::Serialize)]
        struct Recording<'a> {
            version: u8,
            coordinates: &'static str,
            native: bool,
            truncated: bool,
            status: &'a str,
            packets: &'a std::collections::VecDeque<Packet>,
        }
        let bytes = serde_json::to_vec_pretty(&Recording {
            version: 1,
            coordinates: "egui logical screen points; native monotonic seconds, fallback time absent",
            native: self.native,
            truncated: self.packets > self.recent.len() as u64,
            status: &self.status,
            packets: &self.recent,
        }).map_err(|e| e.to_string())?;
        crate::storage::atomic_write(path, &bytes)
    }
}

impl Input {
    pub fn native_enabled(&self) -> Option<bool> {
        #[cfg(any(windows, target_os = "linux"))]
        if let Some(native) = &self.native {
            return Some(native.enabled());
        }
        None
    }
    /// Tests stand in for the Windows pen's side button at `at` pixels.
    #[cfg(all(test, windows))]
    pub(crate) fn test_side_button(&mut self, pressed: bool, at: [f32; 2]) {
        self.native
            .get_or_insert_with(|| crate::native_pen::Backend::with_test_packets(vec![]))
            .test_side_button(pressed, at);
    }
    /// A pen hovering or touching with its eraser end.
    pub fn eraser_in_range(&self) -> bool {
        #[cfg(test)]
        if self.test_eraser_in_range {
            return true;
        }
        #[cfg(any(windows, target_os = "linux"))]
        if let Some(native) = &self.native {
            return native.eraser_in_range();
        }
        false
    }
    pub fn set_native_enabled(&mut self, enabled: bool) {
        self.native_requested = enabled;
        #[cfg(any(windows, target_os = "linux"))]
        if let Some(native) = &mut self.native {
            native.set_enabled(enabled);
        }
        self.pen = None;
        self.mouse_down = false;
    }
    #[cfg(any(windows, target_os = "linux"))]
    pub fn attach(&mut self, cc: &eframe::CreationContext<'_>) {
        match crate::native_pen::Backend::attach(cc) {
            Ok(mut backend) => {
                backend.set_enabled(self.native_requested);
                self.native = Some(backend);
            }
            Err(error) => self.diagnostics.status = format!("egui fallback: {error}"),
        }
    }

    /// Wayland stops emulating a pointer for the pen once its tablet seat is
    /// bound; give egui widgets the pen's pointer events instead.
    #[cfg(target_os = "linux")]
    pub fn raw_input_hook(&mut self, raw_input: &mut eframe::egui::RawInput, _: f32) {
        if let Some(native) = &mut self.native {
            let events = native.pointer_events(raw_input.modifiers);
            raw_input.events.extend(events);
        }
    }

    /// Windows gives pen messages to winit as touches, so the side button
    /// never reaches egui as a right-click; add it from the pen reader.
    #[cfg(windows)]
    pub fn raw_input_hook(
        &mut self,
        raw_input: &mut eframe::egui::RawInput,
        pixels_per_point: f32,
    ) {
        let Some(native) = &mut self.native else {
            return;
        };
        let (events, busy) = native.side_button_events(pixels_per_point, raw_input.modifiers);
        let now = raw_input.time.unwrap_or(0.0);
        if busy {
            self.side_echo_until = now + 0.4;
        }
        if busy || now < self.side_echo_until {
            // A driver set to send a right-click as well would toggle twice.
            raw_input.events.retain(|e| {
                !matches!(
                    e,
                    Event::PointerButton {
                        button: PointerButton::Secondary,
                        ..
                    }
                )
            });
        }
        raw_input.events.extend(events);
    }
    pub fn frame_packets(&mut self, events: &[Event], pixels_per_point: f32) -> Vec<Packet> {
        #[cfg(any(windows, target_os = "linux"))]
        if let Some(native) = &mut self.native {
            let (mut packets, suppress, status) = native.drain(pixels_per_point);
            self.diagnostics.native = native.enabled();
            self.diagnostics.status = status;
            if suppress {
                // Completed taps keep their order; cancel only the unfinished
                // contact after native packets have been delivered.
                if events
                    .iter()
                    .any(|e| matches!(e, Event::WindowFocused(false)))
                    && !matches!(packets.last(), Some(Packet::Cancel))
                {
                    packets.push(Packet::Cancel);
                }
                self.mouse_down = false;
                self.pen = None;
                self.diagnostics.record(&packets);
                return packets;
            }
        }
        let _ = pixels_per_point;
        let packets = self.packets(events);
        self.diagnostics.record(&packets);
        packets
    }
    /// Prefer touch/pen events when present, excluding their synthesized mouse
    /// duplicates. Preserve ordering, including taps completed within one frame.
    pub fn packets(&mut self, events: &[Event]) -> Vec<Packet> {
        let use_pen = self.pen.is_some() || events.iter().any(|e| matches!(e, Event::Touch { .. }));
        let mut packets = vec![];
        for event in events {
            match event {
                Event::Touch {
                    id,
                    phase,
                    pos,
                    force,
                    ..
                } => {
                    let sample = Sample {
                        x: pos.x,
                        y: pos.y,
                        pressure: force
                            .unwrap_or(if self.pen.is_some() {
                                self.pressure
                            } else {
                                1.0
                            })
                            .clamp(0.0, 1.0),
                        pressure_known: force.is_some(),
                        ..Default::default()
                    };
                    match phase {
                        TouchPhase::Start if self.pen.is_none() => {
                            self.pen = Some(*id);
                            self.pressure = sample.pressure;
                            packets.push(Packet::Start(sample));
                        }
                        TouchPhase::Move if self.pen == Some(*id) => {
                            self.pressure = sample.pressure;
                            packets.push(Packet::Move(sample));
                        }
                        TouchPhase::End if self.pen == Some(*id) => {
                            packets.push(Packet::End(sample));
                            self.pen = None;
                        }
                        TouchPhase::Cancel if self.pen == Some(*id) => {
                            self.pen = None;
                            packets.push(Packet::Cancel);
                        }
                        _ => {}
                    }
                }
                Event::PointerButton {
                    pos,
                    button: PointerButton::Primary,
                    pressed,
                    ..
                } if !use_pen => {
                    let sample = Sample {
                        x: pos.x,
                        y: pos.y,
                        pressure: 1.0,
                        ..Default::default()
                    };
                    self.mouse_down = *pressed;
                    packets.push(if *pressed {
                        Packet::Start(sample)
                    } else {
                        Packet::End(sample)
                    });
                }
                Event::PointerMoved(pos) if !use_pen && self.mouse_down => {
                    packets.push(Packet::Move(Sample {
                        x: pos.x,
                        y: pos.y,
                        pressure: 1.0,
                        ..Default::default()
                    }));
                }
                Event::WindowFocused(false) | Event::PointerGone => {
                    self.pen = None;
                    self.mouse_down = false;
                    packets.push(Packet::Cancel);
                }
                _ => {}
            }
        }
        if use_pen {
            self.mouse_down = false;
        }
        packets
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::{Modifiers, TouchDeviceId, pos2};
    #[cfg(any(windows, target_os = "linux"))]
    #[test]
    fn native_focus_loss_keeps_completed_taps_and_cancels_the_open_contact() {
        let sample = Sample {
            x: 40.0,
            y: 20.0,
            pressure: 0.8,
            pressure_known: true,
            time: Some(1.0),
            ..Default::default()
        };
        for already_cancelled in [false, true] {
            let mut packets = vec![
                Packet::Start(sample),
                Packet::End(sample),
                Packet::Start(sample),
            ];
            if already_cancelled {
                packets.push(Packet::Cancel);
            }
            let mut input = Input {
                native: Some(crate::native_pen::Backend::with_test_packets(packets)),
                ..Default::default()
            };
            let result = input.frame_packets(&[Event::WindowFocused(false)], 2.0);
            assert_eq!(result.len(), 4);
            // Windows reports physical pixels; Wayland surface coordinates are
            // already logical and only divide by egui zoom (1.0 here).
            let x = if cfg!(windows) { 20.0 } else { 40.0 };
            assert!(matches!(result[0], Packet::Start(s) if s.x == x && s.time == sample.time));
            assert!(matches!(result[1], Packet::End(_)));
            assert!(matches!(result[2], Packet::Start(_)));
            assert!(matches!(result[3], Packet::Cancel));
            assert_eq!(input.diagnostics.pressure, None);
            assert_eq!(input.diagnostics.packets, 4);
            assert!(input.frame_packets(&[], 2.0).is_empty());
        }
    }
    #[test]
    fn diagnostic_recording_is_bounded_and_preserves_native_sample_metadata() {
        let mut diagnostic = Diagnostics::default();
        let sample = Sample {
            pressure: 0.0,
            pressure_known: true,
            time: Some(1.25),
            tilt: Some([-12.0, 8.0]),
            eraser: true,
            ..Default::default()
        };
        diagnostic.record(&[Packet::Start(sample), Packet::Move(sample)]);
        assert_eq!(diagnostic.pressure, Some(0.0));
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("input.json");
        diagnostic.export(&path).unwrap();
        let value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        let packets: Vec<Packet> = serde_json::from_value(value["packets"].clone()).unwrap();
        assert!(matches!(packets[0], Packet::Start(s) if s == sample));
        for _ in 0..5000 {
            diagnostic.record(&[Packet::Move(sample)]);
        }
        assert_eq!(diagnostic.recent.len(), 4096);
        diagnostic.record(&[Packet::Cancel]);
        assert_eq!(diagnostic.pressure, None);
    }
    #[test]
    fn a_tap_and_all_moves_survive_a_single_frame() {
        let mut input = Input::default();
        let button = |pressed| Event::PointerButton {
            pos: pos2(10.0, 10.0),
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        };
        let p = input.packets(&[
            button(true),
            Event::PointerMoved(pos2(20.0, 20.0)),
            Event::PointerMoved(pos2(30.0, 30.0)),
            button(false),
        ]);
        assert_eq!(p.len(), 4);
        assert!(matches!(p[0], Packet::Start(_)));
        assert!(matches!(p[3], Packet::End(_)));
    }
    #[test]
    fn pen_pressure_is_preserved_without_duplicate_mouse_marks() {
        let mut input = Input::default();
        let touch = |phase, force| Event::Touch {
            device_id: TouchDeviceId(0),
            id: TouchId(1),
            phase,
            pos: pos2(10.0, 10.0),
            force,
        };
        let p = input.packets(&[
            touch(TouchPhase::Start, Some(0.2)),
            Event::PointerButton {
                pos: pos2(10.0, 10.0),
                button: PointerButton::Primary,
                pressed: true,
                modifiers: Modifiers::NONE,
            },
            touch(TouchPhase::Move, Some(0.8)),
            touch(TouchPhase::End, None),
        ]);
        assert_eq!(p.len(), 3);
        if let Packet::End(sample) = p[2] {
            assert_eq!(sample.pressure, 0.8);
        } else {
            panic!("Missing pen end");
        }
        assert!(
            input
                .packets(&[Event::PointerMoved(pos2(12.0, 12.0))])
                .is_empty()
        );
    }
}
