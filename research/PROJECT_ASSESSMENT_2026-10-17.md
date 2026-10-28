# Project assessment: 2026-10-17

## Review boundary

Reviewed the current workspace at HEAD `d7d5fa7`, including existing uncommitted
diagnostic work. This is a source, documentation and automated-check assessment.
No new physical tablet, native GUI, cold-disk, power-loss or installer acceptance
was performed. Historical reports are evidence for their recorded builds only.

Existing modifications to `Cargo.toml`, `src/main.rs`, `src/native_session.rs`,
new `src/region_census.rs` and two region-census benchmark images predate this
review. They were preserved. The review changes planning documents only.

## What the project is today

Dudel is a substantial Windows illustration application in Rust, using
eframe/egui and wgpu, with a single active document. The working tree contains
123 Rust source files and approximately 80,763 nonblank source lines, including
tests and diagnostics. It has considerable engine and failure-test coverage;
it has not yet demonstrated every gate for a professional release.

| Area | Source evidence | Assessment |
| --- | --- | --- |
| Painting/input | `brush.rs`, `stroke.rs`, `stroke_buffer.rs`, `native_pen.rs`, `replay.rs` | Pressure, dynamics, stroke buffering, stabilization and replay already exist. Current-build physical acceptance remains open. |
| Raster/history | `pixels.rs`, `history_pixels.rs`, `document.rs`, `render_queue.rs` | Sparse/COW storage, budgeted history and ordered background edits exist. Preserve these contracts during changes. |
| Illustration tools | `selection_mask.rs`, `wand.rs`, `affine_preview.rs`, `mask_transform.rs`, `canvas_size.rs`, `adjustments.rs` | Selection masks, transforms, canvas resize/crop and adjustments already exist; old missing-feature lists are obsolete. |
| Layers/masks | `folders.rs`, `blend.rs`, `layer_mask.rs`, `mask_controller.rs` | Folders, blend modes, clipping and persistent layer masks exist. Validate their combined workflows. |
| Files/recovery | `storage.rs`, `tile_project.rs`, `save_jobs.rs`, `load_jobs.rs`, `recovery.rs`, `export_jobs.rs` | Background native save/load, recovery and PNG exchange exist. Native v4 and mask-aware v5 are distinct from the experimental indexed container. |
| Indexed storage | `indexed_commit.rs`, `save_jobs.rs`, `indexed_document.rs` | Opt-in editor save/session integration already exists; older notes describing it as entirely outside editor saves are historical. Promotion still needs lifecycle/device evidence. |
| Workspace/brushes | `workspace.rs`, `brush_studio.rs`, `brush_library.rs`, `settings_store.rs` | Responsive panels, brush creation/library and durable preferences exist. New-user and tablet validation remain release work. |
| Extensions/interop | `wasm_filters.rs`, `plugins/SDK.md`, `plugins/WASM_SDK.md`, `clip_*.rs` | Filter extension interfaces and restricted CSP import exist. Do not imply general CSP compatibility. |
| Distribution | Root manifest and repository inventory | Package version is 0.1.0. No `.github` CI directory or dedicated release packaging configuration was found. Reproducible distribution is an open work package. |

## Verified product boundaries

- `document.rs` and `canvas_size.rs`: each dimension 1-4096 pixels, total at most
  4,194,304 pixels. A 4096 x 4096 document is **not** supported.
- Up to 32 raster layers, 64 total layer/folder entries and folder depth 6.
- `tile_project.rs`: ordinary native saves use v4, with mask-aware v5 when needed;
  `storage.rs` retains older read paths. `.didx` is a separate opt-in experiment.
- Current color behavior does not establish a color-managed print workflow.
- The root `Dudel` struct still combines document, tools, views, queue state,
  settings and previews. Existing modules help, but lifecycle coupling deserves
  gradual improvement rather than a rewrite during release stabilization.
- Menus and shortcut handling remain spread across `workspace.rs`, `canvas.rs`
  and other modules. `document_actions.rs` covers replacement/close flows; it is
  not a complete central command registry.

## Highest-priority findings

1. **Sustained resources remain unresolved.** `WGPU_DEFERRED_DESTROY.md` records
   zero matching-idle Rust drift after the per-frame nonblocking poll fix, while
   reopened process-private drift remains +44.98 MiB over rounds 4-12. This is
   recorded evidence, not a fresh run or proof of the remaining owner. Existing
   region-census changes indicate investigation is already underway.
2. **Hardware evidence trails implementation.** `XP_PEN_VALIDATION.md` includes
   earlier pressure and subjective smoothness feedback, but lacks a complete
   current-build checklist with driver/scaling/build identity and long-session
   results. Automated input is insufficient to close this gap.
3. **The supported envelope is smaller than broad professional-app claims.**
   Release a dependable illustration product within explicit limits; larger
   canvases, print color and commercial-app parity are separate decisions.
4. **Plans obscure status.** `IMPROVEMENT_PLAN.md` describes the old 43-module,
   97-test prototype and lists many now-implemented features as absent.
   `DELIVERY_PLAN.md` had grown to over 1,400 lines of checkpoints. Its exact
   previous contents are preserved in `DELIVERY_HISTORY_2026-10-17.md`.
5. **Release operations need concrete deliverables.** Build/test automation,
   packaging, clean-machine installation, license/dependency inventory, rollback
   compatibility and a concise release checklist must join engine development.

## Verification

Fresh command results for this workspace are recorded in the current
[delivery plan](../DELIVERY_PLAN.md). Historical benchmark results above are
explicitly attributed to their reports. No source fixes or new application
features were attempted as part of this planning task.
