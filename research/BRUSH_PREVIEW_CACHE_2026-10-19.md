# Brush preview cache-check efficiency

Base revision: `7aa990d26a51b949ba882dc734efe81d09908c03`.

## Change and scope

The brush library, live Brush Studio preview and preset-change comparison call
brush_previews::fingerprint. Previously each call serialized BrushDynamics into a
new JSON byte vector, then hashed that vector, even when the settings and cached
image had not changed. Default dynamics serialize four absent parameters; dense
curves serialize every input and point.

The fingerprint now visits the four parameters directly, hashing presence,
minimum/mode, input kinds and curve point bits, including sequence boundaries.
This removes JSON formatting and temporary byte-vector allocation from that part
of cache checks. Existing tip/grain hashing and preview rendering are unchanged.
Cache keys are process-local; no project, brush-pack or settings format changes.

The library already limits previews to visible rows. This patch preserves that
behavior; it does not introduce a new virtualization system or change artwork.

## Measurement protocol

The explicitly ignored component benchmark compares the prior
serde_json::to_vec(...).hash(...) operation with direct dynamics hashing in one
process. Each fixture runs five pairs of 10,000 iterations, alternating which
implementation runs first. Inputs and hash results cross black_box boundaries.
Fixtures are default dynamics and all four parameters with four inputs and
sixteen-point curves each. Timings cover dynamics key generation only, not the
remaining brush fingerprint, preview rendering, UI frames or pen latency.

```
cargo test --locked brush_previews::tests::benchmark_dynamics_cache_key -- --ignored --exact --nocapture
cargo test --locked --release brush_previews::tests::benchmark_dynamics_cache_key -- --ignored --exact --nocapture
```

Raw local output is retained in target/qa/brush-cache-20261019. The benchmark is
skipped in normal/CI suites and has no timing assertion. Running it is optional
when investigating this component; it is not a new release gate.

Development-build median nanoseconds per operation:

| Fixture | JSON | Direct hashing |
| --- | ---: | ---: |
| Default | 3975.44 | 316.88 |
| Dense | 489057.33 | 41547.19 |

These unoptimized timings demonstrate component overhead, not production frame
improvements. Optimized-build medians on the same reference PC:

| Fixture | JSON (ns) | Direct hashing (ns) | Component speedup |
| --- | ---: | ---: | ---: |
| Default | 146.05 | 24.03 | 6.08x |
| Dense | 20266.02 | 2115.45 | 9.58x |

Default savings are about 0.12 microseconds per check; dense settings save about
18.15 microseconds. These are modest absolute costs, not claims of a perceptible
whole-application speedup. [Recorded samples](../benchmarks/brush-preview-cache-20261019.json)
preserve every measured pair and the measured module hash. Later test-only edits
improve nested-curve coverage and fixture construction; the measured hashing
implementation is unchanged. The benchmark-only release test executable is not
a packaged application candidate.

## Correctness checks

A cache behavior regression checks initial upload, no upload for unchanged
settings, refresh of the same texture after changing dynamics, changed rendered
pixels, another refresh after editing an existing curve, then no upload once
settled. Initial egui font-atlas upload is excluded
from the brush-texture identity check. Existing live-preview, dynamics renderer,
visible-row library and deterministic brush-pack transfer checks are reused.

Final focused verification: 13 passed, zero failed, one ignored manual benchmark;
cargo fmt --all -- --check and default all-target warnings-denied Clippy pass.
The focused filter includes brush_previews, brush_studio, dynamics, preview_window,
the visible-row library test and the deterministic clean-preference brush-pack
test. Full hosted verification belongs to the new source checkpoint and remains
separate from these local checks.

No sustained DX12 study, physical-input acceptance or distribution acceptance is
claimed. The frozen study package remains unchanged; this source checkpoint needs
its own candidate identity for final acceptance.

## Small workspace consistency fix

The audience preview's Workspace background previously read the theme's control
background (extreme_bg_color). It now reads theme.canvas_background, matching the
main canvas surround and the Theme editor's Outside canvas setting. Explicit
Black/White/Chroma green choices are unchanged. This is a direct color-source fix;
no dedicated test suite or performance claim is added for it.
