//! Filters that look at neighboring pixels or position: Gaussian blur,
//! sharpen (unsharp mask) and add noise. They run on the active layer inside
//! the selection, like the color adjustments, through the same preview.
use crate::{document::Layer, plugins::FilterControl, selection::Selection, selection_mask::Mask};
use std::sync::atomic::Ordering;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Filter {
    /// Gaussian blur with this radius (standard deviation) in pixels.
    Blur { radius: f32 },
    /// Unsharp mask: push each pixel away from its blurred surroundings.
    Sharpen { amount: f32, radius: f32 },
    /// Grain: `amount` is the largest change, as a share of full brightness.
    Noise { amount: f32, monochrome: bool },
}

/// Fixed so the preview and the applied result match exactly.
const NOISE_SEED: u32 = 0x5EED_D0D1;

fn canceled(control: &FilterControl) -> bool {
    control.cancel.load(Ordering::Relaxed)
}

/// A deterministic value in -1..=1 for this pixel and channel.
fn noise(x: usize, y: usize, channel: u32) -> f32 {
    let mut h = (x as u32).wrapping_mul(0x9E37_79B1)
        ^ (y as u32).wrapping_mul(0x85EB_CA77)
        ^ channel.wrapping_mul(0xC2B2_AE3D)
        ^ NOISE_SEED;
    h ^= h >> 16;
    h = h.wrapping_mul(0x7FEB_352D);
    h ^= h >> 15;
    h = h.wrapping_mul(0x846C_A68B);
    h ^= h >> 16;
    h as f32 / u32::MAX as f32 * 2.0 - 1.0
}

/// Apply `filter` to `layer` inside the selection. Soft selection edges blend
/// the result in; Lock transparency keeps each pixel's alpha.
pub(crate) fn apply(
    layer: &mut Layer,
    selection: Option<Selection>,
    mask: Option<&Mask>,
    [width, height]: [usize; 2],
    filter: Filter,
    control: &FilterControl,
) -> Result<(), String> {
    let cancel = || canceled(control);
    if cancel() {
        return Err("Filter canceled".into());
    }
    let mut rect = selection.map_or([0, 0, width, height], |s| {
        [
            s.left.min(width),
            s.top.min(height),
            s.right.min(width),
            s.bottom.min(height),
        ]
    });
    if let Some(bounds) = mask.and_then(|m| m.bounds()) {
        rect = [
            rect[0].max(bounds.left),
            rect[1].max(bounds.top),
            rect[2].min(bounds.right),
            rect[3].min(bounds.bottom),
        ];
    }
    let [left, top, right, bottom] = rect;
    if left >= right || top >= bottom {
        return Ok(());
    }
    // Blurs read pixels around the selection too: about three radii.
    let reach = match filter {
        Filter::Blur { radius } | Filter::Sharpen { radius, .. } => (radius * 3.0).ceil() as usize,
        Filter::Noise { .. } => 0,
    };
    let source = [
        left.saturating_sub(reach),
        top.saturating_sub(reach),
        (right + reach).min(width),
        (bottom + reach).min(height),
    ];
    let (sw, sh) = (source[2] - source[0], source[3] - source[1]);
    let original = layer.pixels.clone();
    let at = |x: usize, y: usize| original.rgba(y * width + x);
    // Premultiplied, blurred planes for the blurs: alpha, then red, green, blue.
    let blurred: Option<[Vec<f32>; 4]> = match filter {
        Filter::Blur { radius } | Filter::Sharpen { radius, .. } => {
            let plane = |channel: usize| -> Result<Vec<f32>, String> {
                let mut values = Vec::with_capacity(sw * sh);
                for y in source[1]..source[3] {
                    if cancel() {
                        return Err("Filter canceled".into());
                    }
                    for x in source[0]..source[2] {
                        let p = at(x, y);
                        let alpha = f32::from(p[3]) / 255.0;
                        values.push(if channel == 3 {
                            alpha
                        } else {
                            f32::from(p[channel]) * alpha
                        });
                    }
                }
                crate::layer_effects::gaussian(&mut values, sw, sh, f64::from(radius), &cancel)?;
                Ok(values)
            };
            Some([plane(3)?, plane(0)?, plane(1)?, plane(2)?])
        }
        Filter::Noise { .. } => None,
    };
    let mut writer = layer.pixels.writer();
    let mut changed = false;
    for y in top..bottom {
        if cancel() {
            return Err("Filter canceled".into());
        }
        for x in left..right {
            if selection.is_some_and(|s| !s.contains(x, y)) {
                continue;
            }
            let coverage = mask.map_or(255, |m| m.at(x, y));
            if coverage == 0 {
                continue;
            }
            let before = at(x, y);
            let mut after = match (filter, &blurred) {
                (Filter::Blur { .. }, Some([a, r, g, b])) => {
                    let i = (y - source[1]) * sw + (x - source[0]);
                    let alpha = a[i];
                    if alpha * 255.0 < 0.5 {
                        // Still clear: keep the hidden color untouched.
                        before
                    } else {
                        let c = |v: f32| (v / alpha).round().clamp(0.0, 255.0) as u8;
                        [c(r[i]), c(g[i]), c(b[i]), (alpha * 255.0).round() as u8]
                    }
                }
                (Filter::Sharpen { amount, .. }, Some([a, r, g, b])) => {
                    if before[3] == 0 {
                        continue;
                    }
                    let i = (y - source[1]) * sw + (x - source[0]);
                    let alpha = a[i].max(1.0 / 512.0);
                    let around = [r[i] / alpha, g[i] / alpha, b[i] / alpha];
                    let mut p = before;
                    for c in 0..3 {
                        let v = f32::from(before[c]);
                        p[c] = (v + amount * (v - around[c])).round().clamp(0.0, 255.0) as u8;
                    }
                    p
                }
                (Filter::Noise { amount, monochrome }, _) => {
                    if before[3] == 0 {
                        continue;
                    }
                    let mut p = before;
                    for c in 0..3 {
                        let n = noise(x, y, if monochrome { 0 } else { c as u32 });
                        let v = f32::from(before[c]) + n * amount * 255.0;
                        p[c] = v.round().clamp(0.0, 255.0) as u8;
                    }
                    p
                }
                _ => unreachable!("blurred planes exist for blur and sharpen"),
            };
            if layer.alpha_locked {
                after[3] = before[3];
            }
            let after = crate::selection_mask::mix_rgba(before, after, coverage);
            if after != before {
                writer.rgba_mut(y * width + x).copy_from_slice(&after);
                changed = true;
            }
        }
    }
    if changed {
        writer.reclaim_region(width, rect);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;

    fn control() -> FilterControl {
        FilterControl::default()
    }
    /// A 21 x 21 layer with one opaque red pixel in the middle.
    fn dot() -> Document {
        let mut doc = Document::new(21, 21);
        doc.layers[0]
            .pixels
            .writer()
            .rgba_mut(10 * 21 + 10)
            .copy_from_slice(&[255, 0, 0, 255]);
        doc
    }

    #[test]
    fn blur_spreads_alpha_without_darkening_the_color() {
        let mut doc = dot();
        let [w, h] = [doc.width, doc.height];
        apply(
            &mut doc.layers[0],
            None,
            None,
            [w, h],
            Filter::Blur { radius: 2.0 },
            &control(),
        )
        .unwrap();
        let center = doc.layers[0].pixels.rgba(10 * 21 + 10);
        let near = doc.layers[0].pixels.rgba(10 * 21 + 12);
        assert!(center[3] < 255 && near[3] > 0, "{center:?} {near:?}");
        // Premultiplied blurring keeps the spread paint red, not dark.
        assert!(near[0] > 240 && near[1] == 0, "{near:?}");
        let far = doc.layers[0].pixels.rgba(0);
        assert_eq!(far[3], 0);
    }

    #[test]
    fn sharpen_raises_edge_contrast_and_noise_is_repeatable() {
        let mut doc = Document::new(8, 1);
        for x in 0..8 {
            let v = if x < 4 { 100 } else { 150 };
            doc.layers[0]
                .pixels
                .writer()
                .rgba_mut(x)
                .copy_from_slice(&[v, v, v, 255]);
        }
        let [w, h] = [doc.width, doc.height];
        let sharpen = Filter::Sharpen {
            amount: 1.0,
            radius: 1.0,
        };
        apply(&mut doc.layers[0], None, None, [w, h], sharpen, &control()).unwrap();
        assert!(doc.layers[0].pixels.rgba(3)[0] < 100);
        assert!(doc.layers[0].pixels.rgba(4)[0] > 150);
        let grain = Filter::Noise {
            amount: 0.2,
            monochrome: true,
        };
        let (mut a, mut b) = (doc.clone(), doc.clone());
        apply(&mut a.layers[0], None, None, [w, h], grain, &control()).unwrap();
        apply(&mut b.layers[0], None, None, [w, h], grain, &control()).unwrap();
        assert!((0..8).all(|i| a.layers[0].pixels.rgba(i) == b.layers[0].pixels.rgba(i)));
        assert!((0..8).any(|i| a.layers[0].pixels.rgba(i) != doc.layers[0].pixels.rgba(i)));
        let p = a.layers[0].pixels.rgba(2);
        assert!(
            p[0] == p[1] && p[1] == p[2],
            "monochrome grain keeps gray gray"
        );
    }

    #[test]
    fn selection_and_alpha_lock_limit_the_change() {
        let mut doc = dot();
        let [w, h] = [doc.width, doc.height];
        let selection = Some(Selection {
            left: 0,
            top: 0,
            right: 11,
            bottom: 21,
        });
        apply(
            &mut doc.layers[0],
            selection,
            None,
            [w, h],
            Filter::Blur { radius: 2.0 },
            &control(),
        )
        .unwrap();
        assert!(doc.layers[0].pixels.rgba(10 * 21 + 9)[3] > 0);
        assert_eq!(doc.layers[0].pixels.rgba(10 * 21 + 12)[3], 0);
        let mut locked = dot();
        locked.layers[0].alpha_locked = true;
        apply(
            &mut locked.layers[0],
            None,
            None,
            [w, h],
            Filter::Blur { radius: 2.0 },
            &control(),
        )
        .unwrap();
        assert_eq!(locked.layers[0].pixels.rgba(10 * 21 + 10)[3], 255);
        assert_eq!(locked.layers[0].pixels.rgba(10 * 21 + 12)[3], 0);
    }

    #[test]
    fn canceling_leaves_the_layer_untouched() {
        let mut doc = dot();
        let [w, h] = [doc.width, doc.height];
        let control = control();
        control.cancel.store(true, Ordering::Relaxed);
        assert!(
            apply(
                &mut doc.layers[0],
                None,
                None,
                [w, h],
                Filter::Blur { radius: 2.0 },
                &control
            )
            .is_err()
        );
        assert_eq!(doc.layers[0].pixels.rgba(10 * 21 + 12)[3], 0);
    }
}
