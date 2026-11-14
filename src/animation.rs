//! Frame-by-frame animation with frames as layers, the way Procreate's
//! Animation Assist works: each top-level layer or folder of the stack is a
//! frame, bottom first. A frame shows for its layer's `hold` count of
//! frame periods. The bottom entry can stay under every frame as a
//! background and the top one over every frame as a foreground.
//!
//! The document's layers are never hidden or shown to play a frame: the
//! canvas and exports composite a cheap copy (pixel tiles are shared) in
//! which only that frame, the background, the foreground and the onion skin
//! are visible (`frame_view`). Saving, undo and recovery therefore work as
//! for any layered picture, and older versions open an animation as layers.

use crate::document::{Document, Layer};
use serde::{Deserialize, Serialize};

/// Frames per second Dudel offers.
pub const FPS: std::ops::RangeInclusive<u8> = 1..=60;
/// Longest hold, in frame periods.
pub const MAX_HOLD: u16 = 240;
/// Frames shown before and after the current one, at most.
pub const MAX_ONION: u8 = 5;

/// How playback continues after the last frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Playback {
    #[default]
    Loop,
    /// Forward, then backward, then forward again.
    PingPong,
    /// Stop on the last frame.
    Once,
}
impl Playback {
    pub const ALL: [Self; 3] = [Self::Loop, Self::PingPong, Self::Once];
    pub fn name(self) -> &'static str {
        match self {
            Self::Loop => "Loop",
            Self::PingPong => "Ping-pong",
            Self::Once => "Once",
        }
    }
}

/// A document's animation settings; saved with the project.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Animation {
    pub fps: u8,
    pub playback: Playback,
    /// The bottom entry stays visible under every frame.
    pub background: bool,
    /// The top entry stays visible over every frame.
    pub foreground: bool,
    /// Frames shown faintly before and after the current one while drawing.
    pub onion_before: u8,
    pub onion_after: u8,
    /// Opacity of the nearest onion frame; farther ones fade.
    pub onion_opacity: f32,
    /// Earlier frames tinted red and later ones green, so they are easy to
    /// tell apart.
    pub onion_tint: bool,
}
impl Default for Animation {
    fn default() -> Self {
        Self {
            fps: 12,
            playback: Playback::Loop,
            background: false,
            foreground: false,
            onion_before: 1,
            onion_after: 1,
            onion_opacity: 0.35,
            onion_tint: true,
        }
    }
}
impl Animation {
    pub fn validate(&self) -> Result<(), String> {
        if !FPS.contains(&self.fps)
            || self.onion_before > MAX_ONION
            || self.onion_after > MAX_ONION
            || !self.onion_opacity.is_finite()
            || !(0.0..=1.0).contains(&self.onion_opacity)
        {
            return Err("Animation settings are outside their supported range".into());
        }
        Ok(())
    }
}

/// One frame: a top-level entry and the stack indices it covers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    /// The entry's own index: a layer, or a folder's header.
    pub entry: usize,
    /// The entry and, for a folder, everything inside it.
    pub block: std::ops::Range<usize>,
    /// Frame periods it shows for, at least one.
    pub hold: u16,
}

/// The top-level entries, bottom first. Reference boards shown among the
/// layers on the canvas are not frames.
fn entries(doc: &Document) -> Vec<usize> {
    (0..doc.layers.len())
        .filter(|&index| {
            doc.layers[index].depth == 0 && doc.layers[index].display_reference.is_none()
        })
        .collect()
}

/// The background and foreground entries, when the settings ask for them
/// and the stack has room for at least one frame between them.
pub fn fixed(doc: &Document, animation: &Animation) -> (Option<usize>, Option<usize>) {
    let entries = entries(doc);
    let spare = entries.len().saturating_sub(1);
    let background = (animation.background && spare >= 1)
        .then(|| entries.first().copied())
        .flatten();
    let reserved = usize::from(background.is_some());
    let foreground = (animation.foreground && entries.len() > reserved + 1)
        .then(|| entries.last().copied())
        .flatten();
    (background, foreground)
}

/// The frames in playing order: the top-level entries, bottom first,
/// without the background and foreground and without hidden entries.
pub fn frames(doc: &Document, animation: &Animation) -> Vec<Frame> {
    let (background, foreground) = fixed(doc, animation);
    entries(doc)
        .into_iter()
        .filter(|&entry| Some(entry) != background && Some(entry) != foreground)
        .filter(|&entry| doc.layers[entry].visible)
        .map(|entry| Frame {
            entry,
            block: doc.block(entry),
            hold: doc.layers[entry].hold.clamp(1, MAX_HOLD),
        })
        .collect()
}

/// The frame holding the stack entry at `index`, if it is in one.
pub fn frame_of(frames: &[Frame], index: usize) -> Option<usize> {
    frames.iter().position(|frame| frame.block.contains(&index))
}

/// Frame periods in one pass through the frames.
pub fn length(frames: &[Frame]) -> u32 {
    frames.iter().map(|frame| u32::from(frame.hold)).sum()
}

/// The frame showing `seconds` after playback started on frame `start`. A
/// Once animation stays on its last frame when it ends.
pub fn frame_at(frames: &[Frame], animation: &Animation, start: usize, seconds: f64) -> usize {
    if frames.is_empty() {
        return 0;
    }
    let start = start.min(frames.len() - 1);
    // Periods from the beginning of the animation to the start frame.
    let offset: u32 = frames[..start].iter().map(|f| u32::from(f.hold)).sum();
    let period = (seconds.max(0.0) * f64::from(animation.fps)).floor() as u64 + u64::from(offset);
    let total = u64::from(length(frames)).max(1);
    let position = match animation.playback {
        Playback::Loop => period % total,
        Playback::Once => period.min(total - 1),
        Playback::PingPong => {
            // Forward through every period, then back without repeating
            // the last and first frames.
            let last = u64::from(frames.last().unwrap().hold);
            let first = u64::from(frames[0].hold);
            let back = total.saturating_sub(last + first);
            let cycle = total + back;
            let at = period % cycle.max(1);
            if at < total {
                at
            } else {
                // Walk backward from the end of the second-to-last frame.
                total - last - 1 - (at - total)
            }
        }
    };
    let mut remaining = position;
    for (index, frame) in frames.iter().enumerate() {
        if remaining < u64::from(frame.hold) {
            return index;
        }
        remaining -= u64::from(frame.hold);
    }
    frames.len() - 1
}

/// Seconds one frame period lasts.
pub fn period(animation: &Animation) -> f64 {
    1.0 / f64::from(animation.fps.max(1))
}

/// Tints for the onion skin: earlier frames red, later frames green.
const BEFORE_TINT: [u8; 3] = [220, 60, 60];
const AFTER_TINT: [u8; 3] = [50, 170, 80];

/// A copy of `doc` showing only `frame` (an index into `frames`), the
/// background and the foreground, plus the onion skin when `onion` is set.
/// Pixel tiles are shared with `doc`, so the copy is cheap.
pub fn frame_view(
    doc: &Document,
    animation: &Animation,
    frames: &[Frame],
    frame: Option<usize>,
    onion: bool,
) -> Document {
    let mut view = doc.clone();
    let (background, foreground) = fixed(doc, animation);
    for entry in entries(doc) {
        let shown = Some(entry) == background
            || Some(entry) == foreground
            || frame.is_some_and(|f| frames.get(f).is_some_and(|f| f.entry == entry));
        view.layers[entry].visible = shown;
    }
    let Some(current) = frame.filter(|_| onion) else {
        return view;
    };
    let mut skin = |offset: usize, before: bool, count: u8| {
        let index = if before {
            current.checked_sub(offset)?
        } else {
            current + offset
        };
        let frame = frames.get(index)?;
        // The nearest is the strongest; farther ones fade evenly.
        let fade = 1.0 - (offset as f32 - 1.0) / f32::from(count.max(1));
        let layer: &mut Layer = &mut view.layers[frame.entry];
        layer.visible = true;
        layer.opacity *= animation.onion_opacity * fade;
        if animation.onion_tint {
            layer.effects.push(crate::layer_effects::LayerEffect::new(
                crate::layer_effects::Effect {
                    kind: crate::layer_effects::Kind::ColorOverlay,
                    paint: crate::layer_effects::Paint::Color(if before {
                        BEFORE_TINT
                    } else {
                        AFTER_TINT
                    }),
                    blend: crate::blend::BlendMode::Normal,
                    opacity: 0.6,
                },
            ));
        }
        Some(())
    };
    for offset in 1..=usize::from(animation.onion_before) {
        skin(offset, true, animation.onion_before);
    }
    for offset in 1..=usize::from(animation.onion_after) {
        skin(offset, false, animation.onion_after);
    }
    view
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Five entries bottom to top: three layers, a folder with two layers,
    /// and a top layer.
    fn stack() -> Document {
        let mut doc = Document::new(4, 4);
        doc.layers = vec![
            Layer::raster("A".into(), 4, 4),
            Layer::raster("B".into(), 4, 4),
            Layer::raster("C".into(), 4, 4),
            Layer {
                depth: 1,
                ..Layer::raster("D1".into(), 4, 4)
            },
            Layer {
                depth: 1,
                ..Layer::raster("D2".into(), 4, 4)
            },
            Layer::new_folder("D".into()),
            Layer::raster("E".into(), 4, 4),
        ];
        doc.layers[1].hold = 3;
        doc
    }
    fn names(doc: &Document, frames: &[Frame]) -> Vec<String> {
        frames
            .iter()
            .map(|f| doc.layers[f.entry].name.clone())
            .collect()
    }

    #[test]
    fn top_level_entries_are_frames_bottom_first_with_background_and_foreground() {
        let mut doc = stack();
        let mut animation = Animation::default();
        let frames = frames(&doc, &animation);
        assert_eq!(names(&doc, &frames), ["A", "B", "C", "D", "E"]);
        assert_eq!(frames[3].block, 3..6);
        assert_eq!(frames[1].hold, 3);
        assert_eq!(frame_of(&frames, 4), Some(3));
        animation.background = true;
        animation.foreground = true;
        let frames = super::frames(&doc, &animation);
        assert_eq!(names(&doc, &frames), ["B", "C", "D"]);
        assert_eq!(frame_of(&frames, 0), None);
        // Hidden entries are left out of the animation.
        doc.layers[2].visible = false;
        assert_eq!(names(&doc, &super::frames(&doc, &animation)), ["B", "D"]);
        // A single entry is a frame, never a background.
        let single = Document::new(4, 4);
        assert_eq!(super::frames(&single, &animation).len(), 1);
    }

    #[test]
    fn playback_follows_holds_and_each_playing_mode() {
        let doc = stack();
        let mut animation = Animation {
            fps: 10,
            ..Default::default()
        };
        let frames = frames(&doc, &animation);
        // Periods: A, B, B, B, C, D, E (seven per pass).
        let at = |animation: &Animation, start, s| frame_at(&frames, animation, start, s);
        let sequence: Vec<usize> = (0..9).map(|p| at(&animation, 0, p as f64 / 10.0)).collect();
        assert_eq!(sequence, [0, 1, 1, 1, 2, 3, 4, 0, 1]);
        assert_eq!(at(&animation, 2, 0.0), 2, "starts on the chosen frame");
        assert_eq!(at(&animation, 2, 0.35), 0, "and wraps around");
        animation.playback = Playback::Once;
        assert_eq!(at(&animation, 0, 100.0), 4);
        animation.playback = Playback::PingPong;
        let sequence: Vec<usize> = (0..13)
            .map(|p| at(&animation, 0, p as f64 / 10.0))
            .collect();
        assert_eq!(sequence, [0, 1, 1, 1, 2, 3, 4, 3, 2, 1, 1, 1, 0]);
    }

    #[test]
    fn frame_view_shows_one_frame_and_the_onion_skin_without_changing_the_document() {
        let doc = stack();
        let animation = Animation {
            background: true,
            onion_before: 2,
            onion_after: 1,
            ..Default::default()
        };
        let frames = frames(&doc, &animation);
        let current = frame_of(&frames, 2).unwrap();
        let plain = frame_view(&doc, &animation, &frames, Some(current), false);
        let shown: Vec<&str> = plain
            .layers
            .iter()
            .filter(|l| l.visible && l.depth == 0)
            .map(|l| l.name.as_str())
            .collect();
        assert_eq!(shown, ["A", "C"], "the background and the frame");
        let onion = frame_view(&doc, &animation, &frames, Some(current), true);
        let b = &onion.layers[1];
        assert!(b.visible && (b.opacity - 0.35).abs() < 1e-6);
        assert_eq!(b.effects.len(), 1, "tinted");
        let d = &onion.layers[5];
        assert!(d.visible && d.opacity < 1.0);
        assert!(!onion.layers[6].visible, "only one frame after");
        // The document itself is untouched.
        assert!(doc.layers.iter().all(|l| l.visible && l.effects.is_empty()));
        assert!(
            Animation {
                fps: 0,
                ..animation
            }
            .validate()
            .is_err()
        );
    }

    #[test]
    fn projects_keep_their_animation_and_holds_and_stills_stay_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("walk.dudel");
        let mut doc = stack();
        doc.animation = Some(Animation {
            fps: 8,
            playback: Playback::PingPong,
            background: true,
            onion_before: 2,
            ..Default::default()
        });
        crate::storage::save_project(&path, &doc).unwrap();
        let loaded = crate::storage::load_project(&path).unwrap();
        assert_eq!(loaded.animation, doc.animation);
        let holds: Vec<u16> = loaded.layers.iter().map(|l| l.hold).collect();
        assert_eq!(holds, [1, 3, 1, 1, 1, 1, 1]);
        // A still picture writes neither field, so older versions read it
        // exactly as before.
        let still = Document::new(4, 4);
        let metadata = crate::binary_project::metadata(&still).unwrap();
        let text = String::from_utf8_lossy(&metadata);
        assert!(!text.contains("animation") && !text.contains("hold"));
    }
}
