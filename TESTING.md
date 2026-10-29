# Fast development checks and release verification

Use the existing behavior tests. Add a focused regression only for a meaningful
new behavior or bug; avoid tests that repeat implementation details or duplicate
the same check through several layers. Batch checks after a coherent change.

On Windows `./scripts/Verify.ps1` defaults to Quick: formatting, the normal suite
once, default Clippy and the small Python suite. It also formats, tests and lints
the independent Explorer thumbnail workspace, including when an app test filter
is supplied. Python 3.11+ is required. It skips diagnostic feature builds,
release compilation, large-memory fixtures and native soak runs.

For a small area use `./scripts/Verify.ps1 -TestFilter timelapse,themes`.
On Linux use `bash scripts/verify.sh quick` or
`bash scripts/verify.sh quick reference_layers`. A filter is a developer check,
not a claim that the complete suite passed. A documentation-only change needs
link/content review, not a fresh engine build.

Both verify scripts turn on `.githooks/pre-push` for the clone they run in
(`git config core.hooksPath .githooks` does the same by hand). It refuses a push
while `cargo fmt --check` fails, the commonest way both CI workflows broke;
`git push --no-verify` skips it once.

The repository is public. Verify, the pre-push hook and packaging also run
`scripts/check_public_tree.py`, which rejects Windows user-profile paths and the
checkout's absolute path in tracked files, built binaries and release ZIPs. Record
paths relative to the repository (or as `<repo>`) in new reports. A maintainer
can keep a private deny-list of further terms outside the repository; see the
script's header. Private notes belong outside this repository, never in it.

Multiple filters run together in one Cargo invocation, matching any listed filter
and running overlapping tests only once. Linux accepts additional space-separated
filters; PowerShell accepts arrays or comma-separated values. This keeps focused
development checks from repeatedly rebuilding or running shared regressions.

A reviewed commit containing only Markdown or recorded verification evidence may
use `[skip ci]` to avoid repeating the full Windows build. Do not use it for source,
build, workflow or test changes, or a pull request requiring fresh checks.
[GitHub's skip behavior](https://docs.github.com/en/actions/how-tos/manage-workflow-runs/skip-workflow-runs)
applies to that commit; it does not cancel an already-running candidate check.

Clipboard transaction tests inject the copied-file list and use in-memory raster
IO. They must not read or replace the developer's system clipboard. Windows
file-list decoding uses bounded synthetic CF_HDROP fixtures; actual clipboard
interoperability remains a separate native workflow check.

Before release/CI use `./scripts/Verify.ps1 -Mode Full` or
`bash scripts/verify.sh full`. This retains both normal and diagnostic-feature
tests/lints and release compilation (both the app and thumbnail DLL on Windows).
Windows CI separately runs the two ignored
maximum-memory fixtures and matching-source packaging. Full verification refuses
test filters. Resource soaks and tablet acceptance stay explicit release gates;
do not run them on routine UI edits or infer acceptance from unit tests.

After the relevant checks pass, repeat only after code changes, a failure or an
unresolved concern. Keep UI work focused on visible rows and changed state, and
measure native performance on a frozen build independently of compiler workloads.

Hosted Windows checks run for default-branch (`master`) pushes, pull requests and
manual dispatch. Mirroring the same commit to `linux` does not start a duplicate
Windows release job. Superseded runs on the same ref are canceled. Dependency
builds use [Rust Cache](https://github.com/Swatinem/rust-cache) with the selected
compiler; these workflow changes still require hosted execution evidence.

Linux checks run on synchronized `linux` pushes, PRs and manual dispatch using
Ubuntu 24.04. Full verification is followed by an Xvfb/software-OpenGL startup
capture; its artifact records executable/capture hashes and the commit. This
checks compilation and startup, not physical Wayland/tablet or GPU performance.
Push the same commit to `master` and `linux` (`git push origin master master:linux`)
to run each platform once.

Both workflows keep uploaded artifacts for 7 days. A private repository shares
500 MB of artifact storage across the account and each Windows candidate is about
110 MB; releases are packaged locally, so CI artifacts are only for inspection.
The [first complete hosted Linux baseline](benchmarks/linux-hosted-baseline-20261018.json)
passes on `aca86c0`; subsequent source changes still require their own checks.

For the timelapse/display-memory optimization probes, build first and run without
concurrent compilers or other performance workloads:

```powershell
cargo test --locked --release --no-run
cargo test --locked --release -- --ignored --test-threads=1 benchmark_banded_frames benchmark_sparse_four_k_tiles
```

These write `artifacts/timelapse-bands.json` and `artifacts/compact-display.json`.
The timelapse probe checks exact RGB equivalence against the previous thumbnail
path outside timing. Texture counts describe RGBA payload, excluding driver
allocation overhead. For coherent display-publication and snapshot checks, run
`target/release/dudel.exe --benchmark-display-memory artifacts/display-memory.json`
on a matching release build. These are CPU probes, not physical pen-latency or
GPU-memory measurements.

The direct display-packing probe is separate:
`cargo test --locked --release benchmark_direct_upload_packing -- --ignored --test-threads=1`.
It writes `artifacts/direct-upload-packing.json`, comparing the old padded-RGBA
copy plus conversion with direct packing in the same build. It excludes shared
compositing and GPU work; run it without competing performance workloads.

Viewport residency and scheduling checks:

```powershell
cargo test --locked --release benchmark_viewport_residency -- --ignored --test-threads=1
target/release/dudel.exe --benchmark-moving-display artifacts/moving-display.json
target/release/dudel.exe --capture-large-canvas artifacts/large-fit.png 1280 800 1
target/release/dudel.exe --capture-large-canvas-detail artifacts/large-detail.png 1280 800 1
```

The viewport probe writes `artifacts/viewport-residency.json`. It checks atomic
publication and sampled pixels against image-library box reduction while
measuring dense 4K texture payload at fit, detail, and rotated/mirrored views.
Run captures with `DUDEL_RENDERER=opengl` and `DUDEL_RENDERER=dx12` on Windows.
The capture modes use isolated fixtures. In the application, View > Performance
shows the last 256 UI/queue samples (p50/p95/p99), longest profiled queue units,
and compositing-cache hits, misses, bypasses and evictions. Unit profiling runs
only while that window is open; these are CPU timings, not physical input latency.


For reduced-tile dirty patches and cached viewport planning, use the isolated
release probe after compilation has finished:

```powershell
cargo test --locked --release --no-run
$env:DUDEL_VIEWPORT_PATCH_REPORT = "artifacts/viewport-patches.json"
cargo test --locked --release benchmark_viewport_patches -- --ignored --test-threads=1
Remove-Item Env:DUDEL_VIEWPORT_PATCH_REPORT
```

It checks dense partial-alpha 4K artwork at fit, 50% and 100%, with 18- and
80-square damage crossing tile gutters. Each of 24 edits settles before the
next; setup, writes and pixel validation are excluded from owner timing.
Reports distinguish the initial owner call from total owner work through
publication, polling wall time, same-call completion and queued RGBA bytes.
Use alternating trials of frozen builds for comparisons. The normal suite
separately covers blank compact tiles, tiny marks whose navigator average is
zero, canvas edges, masked compositing, filter changes, multiple damage regions,
and plan invalidation/frozen generations. These checks do not measure physical
pen latency or actual GPU allocation.

For bucket fills at the supported 4K maximum, run the release executable after
compilation has finished:

```powershell
$env:DUDEL_FILL_BENCH_SIZE = "4096"
$env:DUDEL_FILL_BENCH_EDGES = "1"
target/release/dudel.exe --benchmark-fill artifacts/fill-4k-edges.json
Remove-Item Env:DUDEL_FILL_BENCH_EDGES
target/release/dudel.exe --benchmark-fill artifacts/fill-4k-plain.json
Remove-Item Env:DUDEL_FILL_BENCH_SIZE
```

The edge run uses the application's initial settings (Fill to edges enabled,
tolerance 12). The plain run uses tolerance zero with edge preservation off.
The size override accepts 1–4096; without it the existing 512/2048 cases run.
Each case checks every destination pixel, the reference layer and worker undo,
with three direct and three queued fills. Sparse, dense flat, dense tiled and
tiled-with-history fixtures cover active, specific and visible references.
The separate gap/expansion cases still use their fixed 512/2048 sizes.
Direct time excludes history and display; worker wall time includes history
settlement and 1 ms polling sleeps, but excludes UI/GPU refresh. The reported
visitation payload is one bitset, not all edge masks, allocations or RSS.
The normal tests compare scanline connectivity and painting with independent
scalar oracles, including tile boundaries, soft selections and cancellation.

Layer effects and the mask view have an isolated UI capture: a masked layer with
a drop shadow and stroke, the Layer Effects window open, the sparkle badge in its
row and the mask shown on the canvas:

```powershell
target/release/dudel.exe --capture-layer-effects artifacts/layer-effects.png 1280 800 1
```

PSD import checks: `--verify-import FILE.psd NEW_REPORT.json` reports the layer
stack and the difference from the file's saved composite; with
`DUDEL_VERIFY_IMAGES` set it also writes the rendering, the saved image and every
layer as PNGs beside the report.

Mirrored free transforms have an isolated UI capture:

```powershell
target/release/dudel.exe --capture-mirror-pair artifacts/mirror-pair.png 1280 800 1
```

The fixture contains differently colored eyes so accidental duplication is
visible. Check the blue driving handles, orange partner outline, dashed axis,
Mirror pair controls and Apply/Cancel. Normal tests cover paired raster/mask
edits, independent affine/perspective/mesh oracles, reflected curved mesh
triangles, tilted axes, source/destination crossings, clipping, frozen preview
sources, cancellation, a single undo step, legacy defaults and real UI clicks
with the properties sidebar shown or hidden. Paired drags use the coalescing
background transform worker; the single-region GPU overlay is bypassed.
