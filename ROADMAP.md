> Historical product roadmap. Follow [UPGRADE_PLAN.md](UPGRADE_PLAN.md) for the
> current 0.2.0 priorities and release gates. [DELIVERY_PLAN.md](DELIVERY_PLAN.md)
> records the completed 0.1 release preparation.

# Dudel product goal

The earlier roadmap below is retained as context; the current release plan is
[UPGRADE_PLAN.md](UPGRADE_PLAN.md).
The user has confirmed working pressure on their XP-Pen Artist 12 with installed drivers.

Build an extensible, reliable desktop painting application in Rust for tablet
artists, initially on Windows. The target is comfortable illustration workflows
with responsive pressure strokes, nondestructive document editing, user-created
brush libraries and portable extensions. Feature parity with mature painting apps
requires a multi-stage project; the first delivered milestone is a usable foundation.

## Milestone 1 — Working foundation (implemented)

- Native application with paint/erase, brush editing and reusable presets.
- Layers, undo/redo, project persistence and PNG export.
- Pressure event path with visible diagnostics and honest fallback.
- Separate painting, brush and extension modules; bounded scripted filter API.
- Automated checks for compositing, pressure footprint, erasing, undo/redo,
  malformed documents and atomic plugin failure.

The user confirmed changing pressure and stroke thickness on their XP-Pen Artist 12.
Remaining hardware validation is manual: draw slow and fast curves, compare exports
and save/reopen a multilayer project.

## Milestone 2 — Tablet feel and reliability

- Add an input abstraction for timestamped pen samples: position, pressure,
  tilt, rotation, proximity, buttons and eraser state.
- Validate Windows Ink on real hardware; add WinTab as an optional backend
  where needed. Record tablet model, driver version and backend in diagnostics.
- Process every packet, preserve dab spacing across packets, resample curves,
  add configurable pressure curves and stabilization without excessive latency.
- Add recoverable autosave, crash recovery and robust atomic project replacement.
- Move filters and file work off the UI thread with cancellation and progress.

Acceptance: no gaps on fast strokes, smooth thin-to-thick curves, no ink during
hover or pan, stroke recovery after focus loss, safe interruption and recovery.
Measure input-to-display latency on target hardware; target a stable 60 FPS view.

## Milestone 3 — Core illustration workflow

- New document sizes, PNG import, transparent PNG export and reference images.
- Selection masks, fill, eyedropper, transform, clipping and alpha lock.
- Blend modes, groups, masks and nondestructive adjustments.
- Tiled raster storage, dirty-region GPU compositing and tile-delta history.
- Compressed project container with thumbnails, schema migrations and recovery.

Acceptance: meaningful operations undo as one action; large canvases stay responsive;
save/reopen preserves every layer and setting; malformed files fail cleanly.

## Milestone 4 — Brush ecosystem

- Brush tip images, texture/grain, scatter, angle, dual brushes and dynamics.
- Live stroke preview, preset library, tags and portable versioned brush packs.
- Separate brush engine interface so new dab behavior can be added independently.

Acceptance: deterministic replay of recorded input; stable preset serialization;
brush packs validate and import with explicit compatibility errors.

## Milestone 5 — General plugin platform

API 2 now provides brush/tool presets, guarded commands and declarative panels.
Custom stroke engines, import/export hooks, arbitrary UI layouts, dependencies
and plugin-specific settings remain beyond this first contribution API.

- Versioned manifests and a documented WASM host ABI.
- Explicit capabilities for filters, brushes, commands, panels and document access.
- Fuel/memory/time limits, cancellation, isolated execution and transactional edits.
- Plugin manager for install, enable/disable, update and compatibility reporting.
- SDK examples and compatibility tests; no unrestricted native code by default.

Acceptance: faulty plugins cannot corrupt the document or hang the interface;
extensions participate in undo and work against a stable, versioned SDK.

## Milestone 6 — Release quality

- Configurable shortcuts, accessible UI, localization, packaging and updates.
- Benchmarks, integration coverage and a tablet compatibility matrix.
- Color management and display profiles before claiming professional print fidelity.
- Explore macOS/Linux only after input backend and rendering contracts stabilize.

Success is reliable drawing on the user's actual tablet, preservation and recovery
of artwork, reusable brush packs, and documented extension compatibility.
