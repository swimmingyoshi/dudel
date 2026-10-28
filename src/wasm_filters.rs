use crate::{document::Layer, plugins::FilterControl, selection::Selection};
use std::{io::Read, path::Path, sync::atomic::Ordering};
use wasmi::{Config, Engine, Linker, Module, Store, StoreLimits, StoreLimitsBuilder};

pub const MODULE_LIMIT: usize = 1024 * 1024;
pub fn validate(bytes: &[u8]) -> Result<(), String> {
    if bytes.len() > MODULE_LIMIT || !bytes.starts_with(b"\0asm") {
        return Err("Expected binary WASM, maximum 1 MiB".into());
    }
    let mut config = Config::default();
    config.allow_start_fn(false);
    let engine = Engine::new(&config);
    let module = Module::new(&engine, bytes).map_err(|e| format!("Invalid WASM: {e}"))?;
    if module.imports().next().is_some() {
        return Err("WASM filters cannot import host functions or WASI".into());
    }
    for (name, params) in [
        ("dudel_api_version", &[][..]),
        ("dudel_filter_rgba", &[wasmi::ValType::I32][..]),
    ] {
        let valid = module.exports().any(|export| {
            export.name() == name && matches!(export.ty(), wasmi::ExternType::Func(function) if function.params() == params && function.results() == [wasmi::ValType::I32])
        });
        if !valid {
            return Err(format!("Missing or invalid WASM export: {name}"));
        }
    }
    Ok(())
}
pub fn read_module(path: &Path) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .map_err(|e| e.to_string())?
        .take((MODULE_LIMIT + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > MODULE_LIMIT {
        return Err("WASM module exceeds 1 MiB".into());
    }
    Ok(bytes)
}

pub fn apply(
    layer: &mut Layer,
    bytes: &[u8],
    selection: Option<Selection>,
    width: usize,
    control: &FilterControl,
) -> Result<(), String> {
    apply_masked(layer, bytes, selection, None, width, control)
}
pub fn apply_masked(
    layer: &mut Layer,
    bytes: &[u8],
    selection: Option<Selection>,
    mask: Option<&crate::selection_mask::Mask>,
    width: usize,
    control: &FilterControl,
) -> Result<(), String> {
    if layer.locked {
        return Err("Layer is locked".into());
    }
    if bytes.len() > MODULE_LIMIT || !bytes.starts_with(b"\0asm") {
        return Err("Expected a binary WASM module, maximum 1 MiB".into());
    }
    if width == 0 {
        return Err("Invalid document width".into());
    }
    if control.cancel.load(Ordering::Relaxed) {
        return Err("Filter canceled".into());
    }
    let mut config = Config::default();
    config.consume_fuel(true).allow_start_fn(false);
    config
        .set_max_recursion_depth(64)
        .set_max_stack_height(64 * 1024);
    let engine = Engine::new(&config);
    let module = Module::new(&engine, bytes).map_err(|e| format!("Invalid WASM module: {e}"))?;
    if module.imports().next().is_some() {
        return Err("WASM pixel filters may not import host functions or WASI".into());
    }
    let limits = StoreLimitsBuilder::new()
        .memory_size(16 * 1024 * 1024)
        .memories(1)
        .tables(1)
        .table_elements(1024)
        .instances(1)
        .trap_on_grow_failure(true)
        .build();
    let mut store = Store::new(&engine, limits);
    store.limiter(|limits: &mut StoreLimits| limits);
    store.set_fuel(10000).map_err(|e| e.to_string())?;
    let instance = Linker::<StoreLimits>::new(&engine)
        .instantiate_and_start(&mut store, &module)
        .map_err(|e| format!("WASM instantiation: {e}"))?;
    let version = instance
        .get_typed_func::<(), i32>(&store, "dudel_api_version")
        .map_err(|_| "Missing dudel_api_version() -> i32".to_owned())?;
    if version.call(&mut store, ()).map_err(|e| e.to_string())? != 1 {
        return Err("Incompatible WASM API; expected 1".into());
    }
    let filter = instance
        .get_typed_func::<i32, i32>(&store, "dudel_filter_rgba")
        .map_err(|_| "Missing dudel_filter_rgba(i32) -> i32".to_owned())?;
    let mut budget = 100_000_000_u64;
    crate::plugins::filter_pixels_masked(layer, selection, mask, width, control, |pixel| {
        let fuel = budget.min(10000);
        store.set_fuel(fuel).map_err(|e| e.to_string())?;
        let packed = i32::from_le_bytes(pixel);
        let result = filter
            .call(&mut store, packed)
            .map_err(|e| format!("WASM filter trapped: {e}"))?
            .to_le_bytes();
        budget -= fuel - store.get_fuel().map_err(|e| e.to_string())?;
        Ok(result)
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn masked_stateful_wasm_preserves_selected_row_order_and_discards_late_trap() {
        for trap in [false, true] {
            let conditional = if trap {
                "global.get $calls i32.const 2 i32.eq if unreachable end"
            } else {
                ""
            };
            let module = wat::parse_str(format!(
                "(module
                (global $calls (mut i32) (i32.const 0))
                (func (export \"dudel_api_version\") (result i32) i32.const 1)
                (func (export \"dudel_filter_rgba\") (param i32) (result i32)
                global.get $calls i32.const 1 i32.add global.set $calls
                {conditional}
                global.get $calls i32.const 40 i32.mul i32.const -16777216 i32.or))"
            ))
            .unwrap();
            let mut doc = crate::document::Document::new(5, 1);
            doc.layers[0].pixels = vec![
                0, 0, 0, 255, 9, 11, 13, 255, 0, 0, 0, 255, 5, 7, 9, 0, 0, 0, 0, 255,
            ]
            .into();
            let before = doc.layers[0].pixels.clone();
            let mask =
                crate::selection_mask::Mask::from_coverage(5, 1, vec![255, 0, 128, 255, 255])
                    .unwrap();
            let result = super::apply_masked(
                &mut doc.layers[0],
                &module,
                None,
                Some(&mask),
                5,
                &crate::plugins::FilterControl::default(),
            );
            if trap {
                assert!(result.is_err());
                assert!(doc.layers[0].pixels.shares(&before));
            } else {
                result.unwrap();
                assert_eq!(doc.layers[0].pixels.rgba(0), [40, 0, 0, 255]);
                assert_eq!(doc.layers[0].pixels.rgba(1), [9, 11, 13, 255]);
                assert_eq!(doc.layers[0].pixels.rgba(2), [40, 0, 0, 255]);
                assert_eq!(doc.layers[0].pixels.rgba(3), [5, 7, 9, 0]);
                assert_eq!(doc.layers[0].pixels.rgba(4), [120, 0, 0, 255]);
            }
        }
    }
    #[test]
    fn sparse_stateful_wasm_retains_row_major_order_across_tile_boundaries() {
        let module = wat::parse_str(
            r#"(module
            (global $calls (mut i32) (i32.const 0))
            (func (export "dudel_api_version") (result i32) i32.const 1)
            (func (export "dudel_filter_rgba") (param i32) (result i32)
                global.get $calls i32.const 1 i32.add global.set $calls
                global.get $calls i32.const -16777216 i32.or))"#,
        )
        .unwrap();
        let mut doc = crate::document::Document::new(517, 259);
        for i in [0, 256, 517, 773, 258 * 517 + 516] {
            doc.layers[0]
                .pixels
                .writer()
                .rgba_mut(i)
                .copy_from_slice(&[40, 50, 60, 128]);
        }
        doc.layers[0]
            .pixels
            .writer()
            .rgba_mut(2)
            .copy_from_slice(&[7, 8, 9, 0]);
        let original = doc.layers[0].pixels.clone();
        let capacity = original.capacity();
        let control = FilterControl::default();
        apply(&mut doc.layers[0], &module, None, 517, &control).unwrap();
        for (call, i) in [0, 256, 517, 773, 258 * 517 + 516].into_iter().enumerate() {
            assert_eq!(doc.layers[0].pixels.rgba(i), [call as u8 + 1, 0, 0, 255]);
        }
        assert_eq!(doc.layers[0].pixels.rgba(2), [7, 8, 9, 0]);
        assert_eq!(original.capacity(), capacity);
        assert_eq!(doc.layers[0].pixels.capacity(), capacity);
        assert_eq!(control.progress.load(Ordering::Relaxed), 517 * 259);
        assert_eq!(original.rgba(0), [40, 50, 60, 128]);
    }
    use super::*;
    #[test]
    fn late_traps_growth_and_layer_locks_preserve_pixels() {
        let late_trap = wat::parse_str(
            r#"(module
            (global $calls (mut i32) (i32.const 0))
            (func (export "dudel_api_version") (result i32) i32.const 1)
            (func (export "dudel_filter_rgba") (param i32) (result i32)
                global.get $calls i32.const 1 i32.add global.set $calls
                global.get $calls i32.const 2 i32.eq if unreachable end
                i32.const -1))"#,
        )
        .unwrap();
        let growth = wat::parse_str(
            r#"(module (memory 1)
            (func (export "dudel_api_version") (result i32) i32.const 1)
            (func (export "dudel_filter_rgba") (param i32) (result i32)
                i32.const 300 memory.grow drop local.get 0))"#,
        )
        .unwrap();
        let mut doc = crate::document::Document::new(2, 1);
        doc.layers[0].pixels = vec![10, 30, 200, 128, 40, 50, 60, 255].into();
        let before = doc.layers[0].pixels.clone();
        let control = FilterControl::default();
        assert!(
            apply(&mut doc.layers[0], &late_trap, None, 2, &control)
                .unwrap_err()
                .starts_with("WASM filter trapped:")
        );
        assert_eq!(control.progress.load(Ordering::Relaxed), 2);
        assert_eq!(doc.layers[0].pixels, before);
        assert!(
            apply(&mut doc.layers[0], &growth, None, 2, &control)
                .unwrap_err()
                .starts_with("WASM filter trapped:")
        );
        assert_eq!(doc.layers[0].pixels, before);
        doc.layers[0].locked = true;
        assert!(
            apply(
                &mut doc.layers[0],
                &bytes("i32.const -1", 1),
                None,
                2,
                &control
            )
            .is_err()
        );
        assert_eq!(doc.layers[0].pixels, before);
        doc.layers[0].locked = false;
        doc.layers[0].alpha_locked = true;
        apply(
            &mut doc.layers[0],
            &bytes("i32.const -1", 1),
            None,
            2,
            &control,
        )
        .unwrap();
        assert_eq!(
            doc.layers[0].pixels,
            vec![255, 255, 255, 128, 255, 255, 255, 255]
        );
    }
    fn bytes(body: &str, version: i32) -> Vec<u8> {
        wat::parse_str(format!("(module (func (export \"dudel_api_version\") (result i32) i32.const {version}) (func (export \"dudel_filter_rgba\") (param i32) (result i32) {body}))")).unwrap()
    }
    #[test]
    fn wasm_filters_preserve_alpha_selection_and_atomicity() {
        let mut doc = crate::document::Document::new(2, 1);
        doc.layers[0].pixels = vec![10, 30, 200, 128, 40, 50, 60, 255].into();
        let control = FilterControl::default();
        apply(
            &mut doc.layers[0],
            &bytes("local.get 0 i32.const 16777215 i32.xor", 1),
            Some(Selection {
                left: 0,
                top: 0,
                right: 1,
                bottom: 1,
            }),
            2,
            &control,
        )
        .unwrap();
        assert_eq!(
            doc.layers[0].pixels,
            vec![245, 225, 55, 128, 40, 50, 60, 255]
        );
        let before = doc.layers[0].pixels.clone();
        assert!(
            apply(
                &mut doc.layers[0],
                &bytes("(loop br 0) i32.const 0", 1),
                None,
                2,
                &control
            )
            .is_err()
        );
        assert_eq!(doc.layers[0].pixels, before);
        assert!(
            apply(
                &mut doc.layers[0],
                &bytes("local.get 0", 2),
                None,
                2,
                &control
            )
            .is_err()
        );
        assert_eq!(doc.layers[0].pixels, before);
        control.cancel.store(true, Ordering::Relaxed);
        assert!(
            apply(
                &mut doc.layers[0],
                &bytes("local.get 0", 1),
                None,
                2,
                &control
            )
            .is_err()
        );
        assert_eq!(doc.layers[0].pixels, before);
    }
    #[test]
    fn imports_memory_excess_and_wrong_signatures_are_rejected() {
        let mut doc = crate::document::Document::new(1, 1);
        for source in [
            "(module (import \"wasi_snapshot_preview1\" \"fd_write\" (func)))",
            "(module (memory 300))",
            "(module (func (export \"dudel_api_version\") (result i64) i64.const 1))",
        ] {
            assert!(
                apply(
                    &mut doc.layers[0],
                    &wat::parse_str(source).unwrap(),
                    None,
                    1,
                    &FilterControl::default()
                )
                .is_err()
            );
        }
        let oversized = wat::parse_str("(module (memory 300) (func (export \"dudel_api_version\") (result i32) i32.const 1) (func (export \"dudel_filter_rgba\") (param i32) (result i32) local.get 0))").unwrap();
        assert!(
            apply(
                &mut doc.layers[0],
                &oversized,
                None,
                1,
                &FilterControl::default()
            )
            .unwrap_err()
            .starts_with("WASM instantiation:")
        );
    }
}
