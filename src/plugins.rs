use crate::document::Layer;
use rhai::packages::{Package, StandardPackage};
use rhai::{Engine, Scope};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
pub enum FilterProgram {
    Rhai(String),
    Wasm(Vec<u8>),
}

#[derive(Clone, Default)]
pub struct FilterControl {
    pub cancel: Arc<AtomicBool>,
    pub progress: Arc<AtomicUsize>,
}

/// Atomic copy-on-write filter output. Stateful WASM programs retain the original
/// row-major call order, and hidden RGB in transparent pixels stays untouched.
#[cfg(test)]
pub(crate) fn filter_pixels(
    layer: &mut Layer,
    selection: Option<crate::selection::Selection>,
    width: usize,
    control: &FilterControl,
    filter: impl FnMut([u8; 4]) -> Result<[u8; 4], String>,
) -> Result<(), String> {
    filter_pixels_masked(layer, selection, None, width, control, filter)
}
pub(crate) fn filter_pixels_masked(
    layer: &mut Layer,
    selection: Option<crate::selection::Selection>,
    mask: Option<&crate::selection_mask::Mask>,
    width: usize,
    control: &FilterControl,
    mut filter: impl FnMut([u8; 4]) -> Result<[u8; 4], String>,
) -> Result<(), String> {
    let stride = width
        .checked_mul(4)
        .filter(|stride| *stride > 0)
        .ok_or("Invalid document dimensions")?;
    if !layer.pixels.len().is_multiple_of(stride) {
        return Err("Invalid document dimensions".into());
    }
    if control.cancel.load(Ordering::Relaxed) {
        return Err("Filter canceled".into());
    }
    let height = layer.pixels.len() / stride;
    if mask.is_some_and(|m| m.dimensions() != [width, height]) {
        return Err("Selection mask dimensions differ from filter target".into());
    }
    let rect = selection.map_or([0, 0, width, height], |s| {
        [
            s.left.min(width),
            s.top.min(height),
            s.right.min(width).max(s.left.min(width)),
            s.bottom.min(height).max(s.top.min(height)),
        ]
    });
    let mut pixels = layer.pixels.clone();
    let mut changed = false;
    {
        let mut writer = pixels.writer();
        layer
            .pixels
            .try_visit_rows(width, height, rect, |left, y, row| -> Result<(), String> {
                for (offset, original) in row.chunks_exact(4).enumerate() {
                    if control.cancel.load(Ordering::Relaxed) {
                        return Err("Filter canceled".into());
                    }
                    let index = y * width + left + offset;
                    control.progress.store(index + 1, Ordering::Relaxed);
                    if original[3] == 0 {
                        continue;
                    }
                    let coverage = mask.map_or(255, |m| m.at(left + offset, y));
                    if coverage == 0 {
                        continue;
                    }
                    let mut result = filter(original.try_into().unwrap())?;
                    if layer.alpha_locked {
                        result[3] = original[3];
                    }
                    result = crate::selection_mask::mix_rgba(
                        original.try_into().unwrap(),
                        result,
                        coverage,
                    );
                    if result != original {
                        writer.rgba_mut(index).copy_from_slice(&result);
                        changed = true;
                    }
                }
                Ok(())
            })?;
        if changed {
            writer.reclaim_region(width, rect);
        }
    }
    if control.cancel.load(Ordering::Relaxed) {
        return Err("Filter canceled".into());
    }
    control.progress.store(width * height, Ordering::Relaxed);
    if changed {
        layer.pixels = pixels;
    }
    Ok(())
}

/// Scripts get channel values only: no filesystem, network, UI, or document access.
/// Results commit atomically after all pixels succeed.
#[cfg(test)]
pub fn apply_filter(layer: &mut Layer, source: &str) -> Result<(), String> {
    apply_filter_selected(layer, source, None, 1)
}

#[cfg(test)]
pub fn apply_filter_selected(
    layer: &mut Layer,
    source: &str,
    selection: Option<crate::selection::Selection>,
    width: usize,
) -> Result<(), String> {
    apply_filter_controlled(layer, source, selection, width, &FilterControl::default())
}

pub fn apply_filter_controlled(
    layer: &mut Layer,
    source: &str,
    selection: Option<crate::selection::Selection>,
    width: usize,
    control: &FilterControl,
) -> Result<(), String> {
    apply_filter_masked(layer, source, selection, None, width, control)
}
pub fn apply_filter_masked(
    layer: &mut Layer,
    source: &str,
    selection: Option<crate::selection::Selection>,
    mask: Option<&crate::selection_mask::Mask>,
    width: usize,
    control: &FilterControl,
) -> Result<(), String> {
    if layer.locked {
        return Err("Layer is locked; unlock it before applying a filter".into());
    }
    if source.len() > 32_768 {
        return Err("Plugin exceeds 32 KB limit".into());
    }
    let mut engine = Engine::new_raw();
    let cancel = control.cancel.clone();
    engine.on_progress(move |_| {
        if cancel.load(Ordering::Relaxed) {
            Some("Canceled".into())
        } else {
            None
        }
    });
    StandardPackage::new().register_into_engine(&mut engine);
    engine.set_max_operations(2_000);
    engine.set_max_expr_depths(32, 16);
    engine.set_max_call_levels(8);
    engine.set_max_array_size(64);
    engine.set_max_map_size(32);
    engine.set_max_string_size(4096);
    let ast = engine.compile(source).map_err(|e| e.to_string())?;
    filter_pixels_masked(layer, selection, mask, width, control, |mut pixel| {
        let mut scope = Scope::new();
        let result = engine
            .call_fn::<rhai::Array>(
                &mut scope,
                &ast,
                "filter",
                (
                    pixel[0] as i64,
                    pixel[1] as i64,
                    pixel[2] as i64,
                    pixel[3] as i64,
                ),
            )
            .map_err(|e| e.to_string())?;
        if result.len() != 4 {
            return Err("filter must return [r, g, b, a]".into());
        }
        for (dst, value) in pixel.iter_mut().zip(result) {
            *dst = value
                .as_int()
                .map_err(|_| "Channels must be integers")?
                .clamp(0, 255) as u8;
        }
        Ok(pixel)
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn masked_rhai_skips_holes_blends_alpha_and_keeps_late_errors_atomic() {
        let mut doc = crate::document::Document::new(4, 1);
        doc.layers[0].pixels =
            vec![0, 0, 255, 255, 255, 0, 0, 255, 0, 0, 255, 128, 5, 7, 9, 0].into();
        let before = doc.layers[0].pixels.clone();
        let mask =
            crate::selection_mask::Mask::from_coverage(4, 1, vec![255, 0, 128, 255]).unwrap();
        let source = "fn filter(r,g,b,a) { if r == 255 { throw \"hole called\"; } [255,0,0,0] }";
        super::apply_filter_masked(
            &mut doc.layers[0],
            source,
            None,
            Some(&mask),
            4,
            &super::FilterControl::default(),
        )
        .unwrap();
        assert_eq!(doc.layers[0].pixels.rgba(0), [255, 0, 0, 0]);
        assert_eq!(doc.layers[0].pixels.rgba(1), [255, 0, 0, 255]);
        assert_eq!(doc.layers[0].pixels.rgba(2), [0, 0, 255, 64]);
        assert_eq!(doc.layers[0].pixels.rgba(3), [5, 7, 9, 0]);
        doc.layers[0].pixels = before.clone();
        doc.layers[0].alpha_locked = true;
        super::apply_filter_masked(
            &mut doc.layers[0],
            source,
            None,
            Some(&mask),
            4,
            &super::FilterControl::default(),
        )
        .unwrap();
        assert_eq!(doc.layers[0].pixels.rgba(2), [128, 0, 127, 128]);
        doc.layers[0].pixels = before.clone();
        doc.layers[0].alpha_locked = false;
        assert!(
            super::apply_filter_masked(
                &mut doc.layers[0],
                "fn filter(r,g,b,a) { if a == 128 { throw \"late\"; } [255,0,0,0] }",
                None,
                Some(&mask),
                4,
                &super::FilterControl::default()
            )
            .is_err()
        );
        assert!(doc.layers[0].pixels.shares(&before));
        let canceled = super::FilterControl::default();
        canceled
            .cancel
            .store(true, std::sync::atomic::Ordering::Relaxed);
        assert!(
            super::apply_filter_masked(&mut doc.layers[0], source, None, Some(&mask), 4, &canceled)
                .is_err()
        );
        assert!(doc.layers[0].pixels.shares(&before));
    }
    #[test]
    fn sparse_filter_preserves_hidden_rgb_sharing_and_atomic_late_failure() {
        let mut doc = crate::document::Document::new(517, 259);
        for (i, pixel) in [
            (0, [10, 20, 30, 170]),
            (256, [40, 50, 60, 230]),
            (258 * 517 + 516, [70, 80, 90, 128]),
            (517, [7, 8, 9, 0]),
        ] {
            doc.layers[0]
                .pixels
                .writer()
                .rgba_mut(i)
                .copy_from_slice(&pixel);
        }
        let original = doc.layers[0].pixels.clone();
        let original_capacity = original.capacity();
        apply_filter_selected(
            &mut doc.layers[0],
            "fn filter(r,g,b,a) {[r,g,b,a]}",
            None,
            517,
        )
        .unwrap();
        assert!(doc.layers[0].pixels.shares(&original));
        assert_eq!(original.capacity(), original_capacity);
        let control = FilterControl::default();
        assert!(
            apply_filter_controlled(
                &mut doc.layers[0],
                "fn filter(r,g,b,a) { if r == 70 { throw \"late failure\"; } [0,0,0,0] }",
                None,
                517,
                &control
            )
            .is_err()
        );
        assert!(doc.layers[0].pixels.shares(&original));
        assert_eq!(original.capacity(), original_capacity);
        doc.layers[0].alpha_locked = true;
        apply_filter_selected(
            &mut doc.layers[0],
            "fn filter(r,g,b,a) {[255-r,255-g,255-b,0]}",
            Some(crate::selection::Selection {
                left: 255,
                top: 0,
                right: 517,
                bottom: 259,
            }),
            517,
        )
        .unwrap();
        assert_eq!(doc.layers[0].pixels.rgba(256), [215, 205, 195, 230]);
        assert_eq!(
            doc.layers[0].pixels.rgba(258 * 517 + 516),
            [185, 175, 165, 128]
        );
        assert_eq!(doc.layers[0].pixels.rgba(0), [10, 20, 30, 170]);
        assert_eq!(doc.layers[0].pixels.rgba(517), [7, 8, 9, 0]);
        assert_eq!(doc.layers[0].pixels.capacity(), original_capacity);
        assert_eq!(original.capacity(), original_capacity);
    }

    #[test]
    fn filter_can_reclaim_tiles_and_cancel_after_partial_output_without_commit() {
        let mut doc = crate::document::Document::new(517, 259);
        for i in [0, 256, 517, 773] {
            doc.layers[0]
                .pixels
                .writer()
                .rgba_mut(i)
                .copy_from_slice(&[20, 30, 40, 255]);
        }
        let original = doc.layers[0].pixels.clone();
        let control = FilterControl::default();
        let mut calls = 0;
        assert!(
            filter_pixels(&mut doc.layers[0], None, 517, &control, |_| {
                calls += 1;
                if calls == 2 {
                    control.cancel.store(true, Ordering::Relaxed);
                }
                Ok([0; 4])
            })
            .is_err()
        );
        assert_eq!(calls, 2);
        assert!(doc.layers[0].pixels.shares(&original));
        control.cancel.store(false, Ordering::Relaxed);
        apply_filter_controlled(
            &mut doc.layers[0],
            "fn filter(r,g,b,a) {[0,0,0,0]}",
            None,
            517,
            &control,
        )
        .unwrap();
        assert_eq!(doc.layers[0].pixels.capacity(), 0);
        assert_eq!(control.progress.load(Ordering::Relaxed), 517 * 259);
        assert_eq!(
            original.capacity(),
            2 * crate::pixels::SIDE * crate::pixels::SIDE * 4
        );
    }

    use super::*;
    #[test]
    fn canceled_execution_is_atomic() {
        let mut layer = Layer {
            name: "Canceled".into(),
            visible: true,
            opacity: 1.0,
            blend: crate::blend::BlendMode::Normal,
            clipped: false,
            locked: false,
            alpha_locked: false,
            pixels: vec![10, 20, 30, 255].into(),
            ..Layer::default()
        };
        let original = layer.pixels.clone();
        let control = FilterControl::default();
        control.cancel.store(true, Ordering::Relaxed);
        assert!(
            apply_filter_controlled(
                &mut layer,
                "fn filter(r,g,b,a) {[0,0,0,0]}",
                None,
                1,
                &control
            )
            .is_err()
        );
        assert_eq!(layer.pixels, original);
    }
    #[test]
    fn selected_filters_never_visit_or_change_excluded_pixels() {
        let mut layer = Layer {
            name: "Selected".into(),
            visible: true,
            opacity: 1.0,
            blend: crate::blend::BlendMode::Normal,
            clipped: false,
            locked: false,
            alpha_locked: false,
            pixels: vec![10, 20, 30, 255, 100, 120, 140, 255].into(),
            ..Layer::default()
        };
        let selection = crate::selection::Selection {
            left: 0,
            top: 0,
            right: 1,
            bottom: 1,
        };
        apply_filter_selected(&mut layer,"fn filter(r,g,b,a) { if r == 100 { throw \"outside selection\"; } [255-r,255-g,255-b,a] }",Some(selection),2).unwrap();
        assert_eq!(layer.pixels, vec![245, 235, 225, 255, 100, 120, 140, 255]);
    }
    #[test]
    fn filter_is_atomic_and_inverts() {
        let mut layer = Layer {
            name: "Test".into(),
            visible: true,
            opacity: 1.0,
            blend: crate::blend::BlendMode::Normal,
            clipped: false,
            locked: false,
            alpha_locked: false,
            pixels: vec![10, 20, 30, 255].into(),
            ..Layer::default()
        };
        apply_filter(&mut layer, "fn filter(r,g,b,a) { [255-r,255-g,255-b,a] }").unwrap();
        assert_eq!(layer.pixels, [245, 235, 225, 255]);
        let before = layer.pixels.clone();
        assert!(apply_filter(&mut layer, "fn filter(r,g,b,a) { [r] }").is_err());
        assert_eq!(layer.pixels, before);
        assert!(apply_filter(&mut layer, "fn filter(r,g,b,a) { loop {} }").is_err());
        layer.locked = true;
        assert!(apply_filter(&mut layer, "fn filter(r,g,b,a) { [0,0,0,0] }").is_err());
        layer.locked = false;
        layer.alpha_locked = true;
        apply_filter(&mut layer, "fn filter(r,g,b,a) { [0,0,0,0] }").unwrap();
        assert_eq!(layer.pixels[3], 255);
    }
}
