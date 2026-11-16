# Crates

Dudel is a Cargo workspace of three crates. `cargo build`, `cargo test` and
`cargo clippy` at the repository root cover all three.

| Crate | Folder | What it holds | Depends on |
|---|---|---|---|
| `dudel-core` | `crates/dudel-core` | The engine: documents and layers, brushes and painting, wet paint and materials, selections, transforms, filters, text rendering, and the project, PSD and CSP file formats | `emath` (egui's geometry types) and the file and font libraries; no window toolkit |
| `dudel-ui` | `crates/dudel-ui` | The interface kit: interface styles, icon art and icons, widgets | egui |
| `dudel` | repository root, `src/` | The app: the window, the `Dudel` state, panels, dialogs, input, jobs, packaging | both of the above, eframe |

Neither library names the app, and `dudel-ui` doesn't name the engine, so the
two build side by side. A change to a panel rebuilds only the app.

## Rules for the engine

- No egui or eframe. Geometry uses `emath` (`Pos2`, `Vec2`, `Rect`), the
  same types egui re-exports, so values pass between the crates unchanged.
- No `Dudel` and no app modules. An engine feature with a panel keeps the
  panel in the app (see below).
- Items the app uses are `pub`; everything else stays private.

## One module name, two halves

Twenty-five modules have both an engine part and an app part: for example
`fill`, `material`, `paper`, `wet`, `guides`, `rulers`, `reference_layers`,
`vector_layer` and `canvas_size`. The engine part is
`crates/dudel-core/src/<name>.rs`. The app keeps `src/<name>.rs` for its
`impl Dudel` blocks, dialogs and painting, starting with

```rust
pub(crate) use dudel_core::<name>::*;
```

so `crate::fill::Request` still names the engine type everywhere in the app.
Engine modules with no app half are reached through `use dudel_core::*;` in
`main.rs`; a local `mod` of the same name takes precedence over that glob.

## Tests

- Engine tests live in the engine and run without a window:
  `cargo test -p dudel-core`.
- Tests that drive the app (`Dudel`, egui input) live in the app half of their
  module.
- The engine's `testing` feature gives the app's tests the built-in test font
  (`text_render::TEST_FONT`) and the CSP fixture builders. The app turns it on
  only for its tests, through `[dev-dependencies]`; a release build never
  contains it. An engine item that tests need from the app is gated
  `#[cfg(any(test, feature = "testing"))]`.

## Adding code

- Pixel work, file formats and anything a script or a test should do without
  a window go in the engine.
- A dialog or panel for an engine feature goes in the app half of its module.
- A widget or style that needs nothing from Dudel goes in `dudel-ui`.
