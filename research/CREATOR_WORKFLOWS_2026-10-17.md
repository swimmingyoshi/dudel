# Creator workflow implementation and verification

Application source: `3f00525` (following feature commit `b9079af`).
Windows executable SHA-256:
`BB327D1EAEE5323D9FD4FFC5B0F1344015C9287368BC7DBD2F8C00E4103CC5DB`.

Candidate: `artifacts/releases/dudel-0.1.0-candidate-20261018-023323-112/`.
Binary/source ZIP checksums and CRC checks pass; all 148 recorded source inputs
match the source ZIP. The archive includes 454 dependency notice inventories.
See `benchmarks/creator-package-verification.json`. The subsequent report/CI-only
commit does not alter the application source identified above.

## Delivered

- Opt-in artwork interval recording: one COW snapshot in flight, background
  composition/PNG encoding, pause/resume, settled final frame, persistent frame
  inventory, reopening for export, and cancellable/progress-reporting animated
  GIF export. New/Open and dimension changes end capture. PNG sequences remain
  available for full-color video editing. Recording status remains visible when
  its window is closed. Capture is bounded to 1280 px, 7,200 frames and about 1 GiB.
- Persistent Dark/Paper/Graphite themes and validated custom JSON palette
  import/export. Style setup is cached rather than repeated every frame.
- Compact, grouped reference rows with bounded labels, independent visibility and
  lock controls, within-group ordering, selection reveal and accessible row labels.
  Selecting a reference no longer highlights/reveals the underlying artwork row.
  Reference settings collapse so the actual list remains usable in short panels.
- Dry gouache, Charcoal, Round wash and Flat bristle presets with embedded,
  portable textures. Existing libraries add these through Restore built-in brushes.
- Developer package builder and host CLI validation, working from the existing
  versioned Rhai/WASM pixel-filter sandbox; installed-package export for sharing.
- Quick/focused development checks; Full mode retained explicitly in Windows CI.
  Dependency caching, superseded-run cancellation and canonical-branch push
  triggers avoid redundant jobs when the same commit is mirrored to Linux.

## Evidence and scope

The first complete Quick pass passed 804 tests (two ignored), warnings-denied
default Clippy, formatting and 28 Python tests. Subsequent refinements used focused
checks rather than repeatedly running the complete suite:

- `target/qa/creator-features-verification.log`: complete baseline pass.
- `target/qa/creator-focused-verification.log`: recording/export, palettes,
  references, extensions and visible brush previews; 15 focused tests pass.
- `target/qa/creator-final-focused-verification.log`: 26 focused reference,
  layer-panel and brush-preview/library tests pass, plus Clippy/fmt/Python.
- `target/qa/creator-reference-layout-verification-final.log`: all nine reference
  tests pass after the final collapsed-settings layout, plus Clippy/fmt/Python.

Meaningful new regressions cover frame order/size and protected/canceled GIF
publication, settlement/idle/pause recording and durable inventory, palette
round-trip/contrast, reference group ordering/artwork preservation, selected-row
reveal, and visible-only uploads/searching the last entry in a 512-brush library.
The reveal test advances virtual time so animated scrolling can actually settle.
An initial failing layout check and the corrected result are retained locally.

The SDK built `warm-paper-1.0.0.dudelplugin` against the native release validator.
Unsupported API metadata was rejected and existing SDK output was preserved.
`verify.sh` passes Bash syntax checking in Ubuntu WSL; that environment still
lacks the Rust/Wayland development prerequisites for a Linux build/runtime check.

Native screenshot fixtures exit successfully. Final Dark and Paper reference
captures were visually inspected: selected references, adjacent references and
artwork rows remain readable and selectable below compact settings.
Local captures: `target/qa/creator-ui-20261017/polished-references-{dark,paper}.png`;
`POLISHED_RESULTS.json` records their executable identity. Timelapse window
captures and earlier light-theme workspace captures also complete successfully.

## Remaining acceptance

This is a development candidate. Current-build artist acceptance, long recordings,
resource/latency measurement, Linux runtime/tablet behavior, hosted CI execution,
fresh offline rebuild of the new matching source and clean-machine installation
remain open. Historical resource studies do not certify this update. GIF is the
in-app animated output; native MP4 encoding, recording embedded in project files,
resuming capture into old sessions and arbitrary UI/tool plugin APIs are not
implemented. The release checklist's outstanding safety/distribution gates remain.
