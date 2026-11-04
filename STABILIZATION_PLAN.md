# Current-source stabilization (2026-11-02)

Scope: reconcile documentation after `f21c494`, then resolve the two issues
identified in the current-state review. Preserve unrelated vector-layer work
already present in the checkout. This is not a new acceptance of the 0.1.0 ZIPs.

## Documentation

- [x] Separate current source features/checks from the accepted 2026-10-31 package.
- [x] Correct format-version protection, native folder/fill effects, satin and
  patterns, hue-range and gradient-map editing, and historical verification counts.
- [x] Update this plan and affected user/SDK documentation with final behavior
  and actual verification results.

## Fill opacity and inside effects

- [x] Add a small independent pixel regression for soft alpha with reduced fill,
  including full/partial overlay opacity and more than one inside effect.
- [x] Combine the owner's colors and inside effects in layer space before
  applying the original coverage once. Preserve masks, layer opacity/blend,
  clipping, outer effects, cached redraws and exported composites.
- [x] Check cached versus fresh rendering, native save/load and export behavior.

## Scripted-tool generated work

- [x] Bound native dab generation before allocating large output arrays, sharing
  the allowance across tracks and begin/point/end processing for one input event.
- [x] Cover stabilization catch-up and final tails as well as ordinary points.
  End an over-budget tool with an actionable error; keep earlier artwork undoable.
- [x] Test a long segment, many individually short segments/tracks, finish and
  catch-up, plus successful ordinary example strokes. Avoid destructive OOM probes.
- [x] Document the exact limit and error behavior in the extension SDK.

## Validation

- [x] Run focused regressions, then repository Quick verification and diff checks.
- [x] Record any failures or unrelated-work limitations accurately. Do not rerun
  physical tablet acceptance, maximum-memory fixtures or soaks without a new need.

Baseline review: Quick passed on `f21c494`: 1,076 application tests, 12 ignored,
three thumbnail tests, 42 Python tests, formatting, default Clippy and public-tree
checks. Existing thumbnail COM export linker warnings were non-fatal.

Area verification passed: 71 tests, zero failures, one ignored timing probe
(`layer_effects`, `scripted_tool`, `psd_export`, `stroke::`). The two initial
fixture failures (mask schema version and History construction) were corrected.
The shared checkout's Quick run stopped at formatting in the concurrently edited
vector-layer implementation. An isolated checkout at `9725903` (the review source
plus a CSP checklist update), with only this task's source changes, passed Quick:
1,079 application tests, zero failures, 12 ignored; three thumbnail tests;
42 Python tests; formatting and Clippy for both workspaces; public-tree check.
The existing thumbnail COM export linker warnings were non-fatal. Log:
`artifacts/stabilization-isolated-verify.log`.

2026-11-03: vector-layer stages 1-3 were separately committed through `7a30db3`.
Integration on that shared source passed formatting and **1,092 application
tests, zero failures, 12 ignored** (`artifacts/stabilization-integrated-verify.log`).
The following Clippy invocation waited for another release build's lock; it was
canceled without interrupting that build. Further vector-picking edits were
underway, so this is not a complete Quick pass for that changing shared tree.
The complete isolated Quick result above validates this task's source changes.
Documentation
also identifies that work's v7 header and editable CSP stroke imports; those
features were not implemented by this stabilization task.

Final documentation links and public-tree checks passed, including this new
plan explicitly. Diff whitespace checks passed with CRLF recognized as line
endings (the existing upgrade plan is stored with CRLF). No release build,
packaging, physical pen session or resource soak was performed for these fixes.

Implementation details: native output is limited to 4096 dabs per sample/lift/
catch-up update across hooks and tracks. Reduced-fill combinations are generated
on the existing effect workers, reuse changed regions, and participate in cache
memory accounting. Editable PSD styles preserve their original data; when baked,
the owner, inside effects and mask share a combined raster and the export report
names that conversion. No file-format change was required by these fixes.
