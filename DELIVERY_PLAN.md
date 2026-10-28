# Dudel completion and release plan

Updated 2026-10-28. This is the execution-plan history.
[The current assessment](research/PROJECT_ASSESSMENT_2026-10-17.md)
records source findings. [The previous delivery history](research/DELIVERY_HISTORY_2026-10-17.md)
is preserved verbatim; its next steps and verification totals are historical.

Use [RELEASE_STATUS.md](RELEASE_STATUS.md) for the concise current source,
candidate and gate summary. Dated checkpoints below retain their original scope;
an earlier checkpoint's pending task may be closed by a later one.

**Active goal (owner-approved 2026-10-27): prepare a dependable Windows 0.1 release.**
Freeze feature scope, review and checkpoint the pending implementation, close the
integration and documentation findings in the [release assessment](research/RELEASE_ASSESSMENT_2026-10-27.md),
then verify one identified candidate and matching source. Reuse unchanged evidence;
retest resource and workflow behavior affected by newer changes. G1-G4 remain the
acceptance gates. Unsigned initial distribution and independent first-use testing
on the first public build remain the owner's accepted decisions.

**Adjusted completion decision, 2026-10-28:** the owner asked to adjust and
complete this goal after confirming existing hardware and performance testing.
[RELEASE_DECISION.md](RELEASE_DECISION.md) is now the authoritative first-release
acceptance record. It closes preparation using the existing evidence and explicit
support limits. The detailed matrices below retain engineering targets and
history; unmeasured profiling, exhaustive OS/device matrices and post-publication
first-use feedback are follow-ups rather than blanket demands for more testing.
Actual website/payment setup and public upload remain launch actions, not claims
made by completing this preparation goal.

**Evidence-use correction, 2026-10-28:** the owner confirms the Artist 12 has
worked across repeated development testing and Explorer previews work with the
DLL. Accept that qualitative feedback. The October 21 unattended study already
passed, and the later viewport/timelapse/fill code matches its focused benchmark
records. [Evidence reconciliation](benchmarks/release-evidence-reuse-20261028.json)
supersedes blanket requests for another hardware session or full unattended study.
Before proposing any new test, identify a concrete unanswered question, explain
why the existing evidence cannot answer it, and specify the smallest test whose
result changes a release decision. Missing measurements remain unclaimed; they
are not an automatic instruction to repeat the same study. No repeat study is
currently requested or scheduled.

## Product goal and definition of finished

Deliver a dependable Windows illustration application that an artist can use on
the XP-Pen Artist 12 for a sustained sketch -> ink -> color -> shade -> save -> reopen ->
export workflow. Drawing feels responsive, editing is predictable, and interruption
does not silently destroy artwork. Professional readiness means measured reliability
and complete workflows within an explicit support envelope.

Release 1 covers bounded project tabs with one actively edited document, pressure brushes and portable brush packs,
raster layers/folders/blending/clipping/masks, selections/fill/transforms, canvas
size/crop, adjustments, native projects, recovery, PNG exchange and existing bounded
filter extensions. These capabilities largely exist; finish their interactions.

Current limits (raised 2026-10-22 from 4,194,304 pixels): 1-4096 pixels per side, **16,777,216 pixels total**, 32 raster layers,
64 total entries and folder depth 6. Mask-aware projects share a 512 MiB budget
between raster payloads and layer-mask coverage. A full 4096 x 4096 canvas is supported;
each dense RGBA layer at that size uses 64 MiB, so eight consume that mask-aware
payload budget before masks. The 32-layer ceiling does not promise 32 dense maximum-size
layers. Define
minimum/recommended hardware and supported Windows/driver versions from testing.

Defer animation, cloud collaboration, general PSD/CSP editing
parity, larger limits, expanded plugin APIs and color-managed print claims. Keep
CSP import restricted to its proven subset and indexed `.didx` storage opt-in.
Ordinary native mask-aware v5 is distinct from the experimental indexed container.

## Current assessment (2026-10-19)

The current improvements candidate is `dudel-0.1.0-candidate-20261019-233855-419`,
EXE `FEBE2687...B30D`. [Its proof](benchmarks/release-improvements-20261019.json)
records passing local full verification, native visual review, both archive
checksums/CRCs, 160 compiled inputs, 881 matching notice files, packaged DX12
startup and study CheckOnly. Hosted CI, fresh archive recompilation and physical/
resource/distribution acceptance remain open. No long study was started. The
source-stage and archive identities preserve the authorized uncommitted changes.

The earlier workflow candidate `dudel-0.1.0-candidate-20261019-210656-046` freezes
documentation revision `6ba76c3` and application inputs from `bc3515f`.
[Package verification](benchmarks/workflow-candidate-20261019.json) passes binary
and matching-source archive hashes/CRCs, all 160 compiled inputs, notice parity,
offline documentation links, the included short workflow checklist and packaged
DX12 startup/capture review. Full Windows/Linux CI passes on `bc3515f`; the
checklist commit changes no application inputs. Both this and the older frozen
study package pass CheckOnly; no long study was started. Independent offline
recompilation of this archive, physical workflow and release acceptance remain
open. Temporary source staging was retained; distributable ZIPs are complete.

The [workflow release audit](research/WORKFLOW_RELEASE_AUDIT_2026-10-19.md) now
closes cross-tab Save as overwrites and identifies/fixes global symmetry/snapping
leaking across project tabs. The latter passes 54 focused workflow/tab/guide
tests, formatting, default Clippy, release compilation and three reviewed native
DX12 captures. [Build-specific evidence](benchmarks/tab-guide-isolation-20261019.json)
keeps this engineering EXE separate from tonight's frozen study candidate.
Application source `bc3515f`, including the guide fix, passes complete
Windows/Linux CI in the workflow candidate proof. Physical and distribution
gates stay open.

[Prebuilt source-role accounting](research/PREBUILT_SOURCE_ROLES_2026-10-19.md)
now identifies platform import libraries, upstream test DLLs, WASI helpers and
empty placeholders, with their provided sources and selection boundaries. The
Windows binary package contains no additional native program beyond dudel.exe.
This is documented role/source review, not independent regeneration of upstream
binaries or final distribution acceptance. No application/build inputs changed.

[Native component review](research/NATIVE_COMPONENT_REVIEW_2026-10-19.md) records
47 original native-source/header files, corresponding notice preservation,
prebuilt-named payloads and default feature selection for the frozen candidate.
SQLite 3.53.2's source ID matches the executable; the engine's own terms are kept
distinct from wrapper/SQLCipher terms. This closes inventory uncertainty, not
prebuilt source-form/whole-program compatibility or clean-machine acceptance.

[The four manifest-only licence items](research/LEGACY_LICENSE_DECLARATIONS_2026-10-19.md)
now have a version-specific review disposition for the 8D2C768E candidate bundle:
accept the explicit upstream MIT/CC0 declarations with unchanged source,
attribution and clearly labelled canonical references. Full original-file
checksums and pinned upstream source/manifest bytes were reverified. Broader
native-subcomponent compatibility and public-distribution acceptance remain open.

[Native Windows save sharing failures](benchmarks/native-save-sharing-failure-20261019.json)
now pass for v4/v5 on engineering source `2afdaaa`. A real exclusive file handle
causes Windows error 32 during save publication. Existing destination bytes,
recovered editable state and unsaved/path state survive; after unlocking, the
same destination saves/reopens exactly. Original project/snapshot bytes remain
unchanged. Formatting, default Clippy and release compilation pass. This remains
diagnostic-build evidence. [Linux CI](benchmarks/linux-hosted-save-sharing-20261019.json)
passes 869 default / 873 all-feature tests, 34 Python checks, both lints, release
and verified X11 artifacts. [Windows CI](benchmarks/windows-hosted-save-sharing-20261019.json)
also passes 877 default / 886 all-feature tests, 34 Python checks, both lints,
release, both maximum fixtures and hosted packaging/upload. Hosted artifact bytes
are not audited; local native and frozen-candidate proofs remain separately
identified. Broader G1 failures and physical/distribution acceptance remain open.

[Large/busy native recovery](benchmarks/large-busy-native-recovery-20261019.json)
now passes on engineering source `57eead3`, whose only compiled-input change from
the frozen candidate is the recovery probe. Four large-area v4/v5 cases cover
two noisy raster layers, masks and a 20-second live adjustment. Recovery defers
temporary preview state, resumes after cancellation and restores/saves/reopens
exact editable state without changing original files. This narrows G1; maximum
layer count, physical strokes/queued work, slow/failing devices and final RC
acceptance remain open. Focused verification passes; later source `2afdaaa` also
has passing hosted verification above. The actual large/busy runtime observations
retain their engineering build identity.

The earlier study candidate, `dudel-0.1.0-candidate-20261019-111249-426` on `fda875f`,
keeps adjustment popups stable across preview preparation and updates, without
fade-in. [Focused verification](benchmarks/adjustment-popup-verification-20261019.json)
passes 10 adjustment/menu checks, formatting, default all-target Clippy, release
compilation, native 100%/200% visual review and archive/input/notice checks.
[Current Linux CI](benchmarks/linux-hosted-adjustment-20261019.json) passes
868 default / 872 all-feature tests, 34 Python checks, both lints, release and
verified X11 artifacts. Its captures are byte-identical to previously reviewed
ones. [Current Windows CI](benchmarks/windows-hosted-adjustment-20261019.json)
passes 876 default / 885 all-feature tests, 34 Python checks, both lints, release,
both dedicated maximum fixtures and hosted development packaging/upload.
Hosted artifact bytes are not audited; local package proofs remain separate.

[Latest independent rebuild and native safety](benchmarks/adjustment-offline-rebuild-native-safety-20261019.json)
also pass: the matching source ZIP rebuilds in a fresh offline target, the rebuilt
executable starts on DX12, and packaged v4/v5 interruption/restore/save/reopen plus
all six settings phases preserve expected state. Small-idle recovery snapshots
completed in 15.75-16.47 seconds. These results remain bounded to the recorded
fixtures/reference PC; broader G1 and physical/distribution gates stay open.
No long resource study was started.

[Bundled asset follow-up](research/BUNDLED_ASSET_REVIEW_2026-10-19.md) verifies
four unchanged font payloads within the executable and matching source archive,
their metadata/packaged terms, and original checksums for both MPL dependencies.
It records source-based conditions and the remaining legacy review rather than treating
notice inclusion alone as final dependency compatibility acceptance.

The preceding bucket candidate, `dudel-0.1.0-candidate-20261019-110448-355` on `f90e3af`,
adds default-on soft-edge bucket filling while preserving original linework.
[Current verification](benchmarks/fill-edges-verification-20261019.json) records
875 default / 884 all-feature tests, 34 Python checks, formatting, both lints,
release compilation, reviewed native 100%/200% captures and matching-source
archive checks. Fill controls now scroll above Color in short panels. Hosted CI
and independent rebuild of this candidate remain separate from the older proofs.

The packaged `RunPerformanceStudy.ps1` passes a default-path CheckOnly launch.
The user will run the full study when this PC is idle, or on a laptop whose
hardware is recorded separately. No long study was launched here. The command,
Python requirement and output location are in the packaged README; reports stay
under `diagnostics/target/qa/study-*`. Completing and reviewing those reports is
the next resource gate, followed by physical workflow and distribution acceptance.

Recent source work adds layer/global symmetry and guide controls, compact status
controls, per-canvas timelapse playback, brush-shaped cursors and independently
remembered eraser sizes, including flipped-pen hover. Preset deletion and
favoriting now preserve the separate paint/eraser slots. The brush/eraser audit
passes 18 focused tests, formatting and default all-target Clippy; that is not a
complete release verification. Current source changes need their own frozen
candidate, native captures and performance/artist acceptance.

The short-inspector layout fix passes five focused workflow checks, formatting,
default all-target Clippy and release compilation. [Native and package evidence](benchmarks/short-inspector-verification-20261019.json)
for candidate `dudel-0.1.0-candidate-20261019-101430-664` verifies matching compiled
inputs, archive checksums/CRCs and 880 matching notices. Reviewed DX12 captures at
100% and 200%, including 960 x 640 physical pixels, show the inspector overlap
resolved. Short inspectors expose Navigator/Color as popups; the smallest layout
moves layer actions/properties into Layer to keep the selected row visible.
Windows CI for that inspector source subsequently passed (run 39236445681,
872 default / 881 all-feature tests, 34 Python checks). Broader FLOW acceptance remains open; one transient
toolbar glyph anomaly in an initial capture is recorded without a claimed fix.

[Current offline rebuild and native safety checks](benchmarks/current-offline-rebuild-native-safety-20261019.json)
now pass on this candidate: fresh extracted-source compilation in Cargo offline
mode without a reused target cache, rebuilt DX12 startup, native v4/v5 interruption and
Restore copy/save/reopen equality, and six settings restart/corrupt/repair/error/
retry phases. Four observed small-idle snapshot times span 15.76-16.54 seconds.
Original project/snapshot bytes and the corrupt-settings backup remain exact.
These checks do not establish large/busy recovery, clean-machine, physical-input
or power-loss acceptance. Sustained measurements await an available idle test
window; the user's separate editor remains untouched.

[Linux CI for the inspector fix](benchmarks/linux-hosted-inspector-20261019.json)
passes on `ffcf959`: 864 default / 868 all-feature tests, 34 Python checks, both
lints, release compilation and verified/reviewed X11 captures. The 200% capture
confirms the inspector overlap is resolved on this tested Linux path. Physical
Wayland/tablet and distribution acceptance remain separate.

Candidate `dudel-0.1.0-candidate-20261019-100137-448` now freezes application
revision `548ad7a` with the packaging changes recorded as dirty inputs.
[Archive and native evidence](benchmarks/offline-notices-candidate-20261019.json)
verifies binary/source checksums, ZIP CRCs, all compiled inputs and 880 identical
notice files. Normal-scale guide and eraser captures were reviewed. The forced-open
inspector in this older candidate overlaps Navigator/Layers at 1280 x 800 with
200% scaling and clips the mask action horizontally; the newer fix is above.
FLOW acceptance remains open. This older candidate has no
independent rebuild or sustained resource acceptance yet.

Packaging now has hash-checked local upstream notices bound to exact crate
checksums and manifests. The [offline collection audit](benchmarks/offline-notice-collection-20261019.json)
checks all 458 resolved registry packages with notice network requests disabled,
restoring 139 files for 68 packages byte-for-byte from the prior verified
candidate. All 34 Python checks pass. A fresh hosted package still needs
inspection; license compatibility and bundled-subcomponent review stay open.
Both [Windows](benchmarks/windows-hosted-offline-notices-20261019.json) and
[Linux](benchmarks/linux-hosted-preset-notices-20261019.json) CI pass on `9e979aa`,
including the preset corrections and fixed notice inputs. Windows passes 871
default / 880 all-feature tests, both explicit maximum fixtures and packaging;
Linux passes 863 / 867 and verified X11/software-OpenGL startup captures. Both
run 34 Python checks and both lints. Windows' package log reports no missing
notice collections; its hosted archive bytes have not been audited. These checks
predate the inspector fix and do not close physical or resource acceptance.

The previously frozen engineering executable `D41C0072F8FA9D15C33160F5973B382E8884BEEE5FB0536D39A1CEF32173011A`
replaces rate-sized diagnostic frame retention with a fixed 64 KiB temporary-file
buffer and streaming report publication. Every callback is retained; logging IO
is measured separately. Full local verification passes 856 default / 865
all-feature tests, both Clippy configurations, release compilation and 32 Python
checks. [Recorder verification](benchmarks/frame-log-fix-verification-20261018.json)
records exact source inputs. Native integration passes six rounds / 198.31 seconds,
32,004 retained callbacks, constant 64 KiB staging and twelve completed save/open
jobs. Continuity and power restoration pass; the capture was visually reviewed.
This four-layer semantic probe does not evaluate resource targets. Candidate
`dudel-0.1.0-candidate-20261018-231406-661` passes archive integrity and matching
source checks, fresh offline release compilation and rebuilt DX12 startup (capture
reviewed). [Candidate packaging evidence](benchmarks/frame-log-candidate-packaging-20261018.json)
records archive hashes, 174 resolving local documentation links and independent
rebuild identity. [One completed standard repeat](benchmarks/frame-log-standard-first-repeat-20261018.json)
passes its per-run memory, completed-job/peak and continuity targets. The second
repeat was intentionally stopped to prioritize requested UI work; power settings
were restored. No study is currently running, and no three-repeat or
maximum-duration acceptance is claimed. Windows CI for recorder commit
`52ffee9aacc463085dd068f4296079ff7de0cca7` also passes. The older resource results
below belong to the earlier identified binary.

Linux CI now runs full verification plus an X11/software-OpenGL startup capture
on synchronized `linux` pushes, complementing Windows checks on `master`. The
[hosted Linux baseline](benchmarks/linux-hosted-baseline-20261018.json) on `aca86c0`
passes 849 default / 853 all-feature tests, both Clippy configurations, 32 Python
checks, release compilation and a visually reviewed X11/software-OpenGL capture.
The later tab-recovery and brush work passes the [hosted Windows baseline](benchmarks/brush-eraser-hosted-baseline-20261019.json)
on `8539447`: 869 default / 878 all-feature tests, 32 Python checks, both lints,
release compilation, both dedicated maximum fixtures and package upload. Its
package log reports no missing notice collections; artifact bytes were not audited.
Latest-source Linux and physical Wayland/tablet acceptance stay open.
The older Linux diagnostic cfg cleanup did not change the frozen default Windows
resource candidate. The owner selected an own-site store and
has no signing credentials; the identified unsigned candidate has a local download
manifest with matching free source, but publication and launch acceptance remain open.
The [store handoff](benchmarks/store-downloads-candidate-20261018.json) has no public
URLs or payment integration. [Observed reference hardware](benchmarks/reference-environment-20261018.json)
records this PC and installed Pentablet application version without claiming active
driver verification, minimum requirements or physical input acceptance.

The [current candidate notice inventory](benchmarks/candidate-notice-inventory-20261018.json)
matches all 458 dependency declarations to vendored manifests and verifies 880
indexed notice files byte-for-byte between the binary package and source archive,
including ten font notices. Preserved upstream provenance hashes match packaged
files. Four legacy declaration/reference items and bundled-subcomponent distribution
review remain open; this mechanical audit does not establish license compatibility.

Windows CI for the Linux workflow introduction (`d69b929`) passes full verification,
both dedicated maximum fixtures and development artifact upload. Its hosted package
reports unresolved notice collection entries, so that artifact has no notice or
distribution acceptance. The successful local notice audit belongs to the separate
identified local candidate. [Hosted evidence](benchmarks/linux-ci-introduction-windows-20261018.json)
preserves this distinction; investigate deterministic notice recovery for hosted
packaging before any public distribution.

Earlier application inputs from `581971a043f947606e4f6bd5d35151b3753adeff` were synchronized
in local and remote `master` and `linux`; subsequent documentation-only checkpoints
retain those compiled inputs. The release executable is
`331640509A0A1326527637F397ABD95FA376AFAA5D6E1F2B4D277BE9D0C202DD`.
The complete local verification passes 853 default and 862 all-feature tests,
28 Python checks, formatting, both all-target warnings-denied Clippy configurations
and Windows release compilation. Two maximum-memory tests are ignored normally;
hosted CI runs them explicitly. Native DX12 startup and a separate read-only
clipboard probe pass. [Exact inputs and verification](benchmarks/clipboard-file-reader-verification-20261018.json)
also preserve the captured clipboard access violation and its fix; serial test
success was not used to replace a failing parallel gate.

The candidate `dudel-0.1.0-candidate-20261018-170922-110` has a matching-source
archive; checksums, ZIP CRC and compiled input hashes pass. Its fresh independent
offline rebuild and rebuilt DX12 startup pass. Three 30-round/20-layer sparse
mixed-brush runs pass memory, completed-job/peak and continuity subtargets.
The maximum 32-layer noisy run stopped after 7,585 observed seconds, 28 completed
rounds and part of round 29: its 1,152,000-frame diagnostic buffer filled before
the required 30 rounds. Logs and partial checkpoints are retained; no maximum
workload acceptance is inferred. The controller is terminal and was not restarted.
See [current resource evidence](benchmarks/release-gates-clipboard-fix-20261018.json).
This failure motivated the bounded frame recorder fix described above; its
remaining repeated and maximum-duration studies are still required. Native measurements ran after local
compiler workloads finished.

Hosted Windows CI passed for the preceding rope/edge source `7f0fe36`; its
[build-specific evidence](benchmarks/rope-edge-hosted-ci-20261018.json) does not
certify the later clipboard fix. CI run 39148261858 for `581971a` also completed
successfully, including explicit maximum-memory fixtures, packaging and upload.

Packaging follow-up fixes offline documentation dependencies: staged binary/source
documentation has no missing file links, private/untracked material is excluded,
and recorded compiled inputs stay unchanged. All 31 Python checks pass. Seven
legacy encoding bytes in the internal product plan were normalized without changing its
text. This packaging fix is for the next archive; the measured archives are
immutable. See [documentation verification](benchmarks/package-doc-links-verification-20261018.json).

The user's newer workflow scope is implemented: timelapse, editable themes and
canvas surround, compact tool properties, borderless window controls, ordered
multi-image reference boards, audience preview/exclusions, bounded tabs, grouped
tools and configurable project/asset folders with ZIP interchange. Rope now has
independent distance, and painting contacts can cross the paper boundary. These
features still need the candidate-specific runtime/artist acceptance described
below. The current time-based stabilizer supersedes historical 450 ms guidance.

Current open work is MEM/SAFE/DRAW/FLOW/SHIP/RC acceptance, not a toolkit rewrite.
Physical XP-Pen Artist 12 driver/scaling and sustained-session evidence, Linux
runtime, native multi-monitor/OBS capture, clean-profile exchange and clean Windows
installation remain open. OBS is installed, but its UI test could not start:
Computer Use returned `GetCursorPos failed: Access is denied (0x80070005)` twice.
The owner chose our own website/store and has no signing credentials yet. The
public-release signing decision and website/payment setup remain open.
Historical build counts and resource findings are retained in
[archived checkpoints](research/DELIVERY_CHECKPOINTS_2026-10-18.md); they do not
close acceptance on this candidate. [RELEASE_CHECKLIST.md](RELEASE_CHECKLIST.md)
retains the full gate record.

## Delivery sequence

Work one primary package at a time. Artwork-loss and correctness defects interrupt
the queue. Sizes are relative effort, not dates: S = bounded task, M = several
tasks, L = investigation or cross-cutting work. Developer owns implementation and
evidence; artist owns physical tablet acceptance. Use a clean Windows environment
for installation and independent acceptance.

| Order / ID | Deliverable | Dependency | Size | Completion evidence |
| --- | --- | --- | --- | --- |
| 0 / BASE | Frozen baseline, build/patch identity, fixture inventory, current checks and gate checklist | None | S | Reproducible baseline and explicit open defects |
| 1 / MEM | Attribute remaining DX12 commitment; fix retention or prove a bounded plateau | BASE | L | Three repeated workloads and a two-hour session meet G2 with exact artwork/reopen checks |
| 2 / SAFE | Verify native save/open/save-as/recovery/export and settings through actual editor failure paths | BASE; repeat after MEM changes | M | G1 matrix passes; old files survive pre-publication failures; stale results cannot replace newer work |
| 3 / DRAW | Current-build input quality and measured responsiveness on the XP-Pen | MEM, SAFE candidate | M | Build-identified hardware checklist, latency/frame data and sustained artist session |
| 4 / FLOW | Complete illustration UX, brush portability, scaling, keyboard focus, OS clipboard/dialogs | SAFE; DRAW findings | M | G3 workflow and clean-profile portability pass |
| 5 / SHIP | Windows CI, reproducible packaging, dependency/license inventory, install/upgrade docs | BASE for automation; MEM-SAFE-DRAW-FLOW for candidate | M | Versioned installable candidate, hashes, compatibility matrix and G4 evidence |
| 6 / RC | Feature freeze, defect fixes and independent candidate acceptance | SHIP | M | All gates pass on the same candidate; no open blocker/critical defects |

CI and checklist preparation can proceed while MEM evidence is collected. Avoid
unrelated engine refactors. Estimate dates only after BASE/MEM reveal the remaining
workload, then revise after artist testing.

## G1: artwork and persistence release gate

- Zero known artwork-loss defects. Verify exact metadata, layer RGBA, masks,
  selection and guides after supported save/reopen and undo/redo workflows.
- Test save-as with pending edits, later edits during save, replacement, cancel,
  worker error/panic, read-only destinations, full-disk simulation, damaged inputs
  and process termination before/after publication. Pre-publication failure must
  preserve the previous valid destination. Stale completion cannot change a newer
  document's path, dirty state or artwork.
- Recover the latest completed snapshot through the actual startup UI; identify
  recovered work and require an explicit project save. Measure completed recovery
  cadence against the configured interval and explain its limits in the quick start.
- Exercise ordinary v4, mask-aware v5 and supported older readers with fixtures.
  Publish the compatibility matrix; old application versions may not read v5 masks.
  Evaluate indexed storage separately before any default-format promotion.
- Use disposable fixtures for interruption tests. Process termination alone does
  not prove power-loss durability; state the actual guarantees and test boundary.

## G2: responsiveness and resource release gate

These are proposed acceptance targets, not current measurements. BASE records the
reference CPU/RAM/GPU/driver/Windows/tablet setup and fixture hashes. A target change
requires a documented measured reason and an explicit support-envelope decision.

| Measurement | Acceptance target / method |
| --- | --- |
| Standard drawing fixture | 2048 x 2048, 20 layers, masks/clipping, ink/pencil/textured presets; also sparse and dense/noisy maximum supported 32-layer fixtures |
| End-to-end frame interval while drawing | p95 <=16.7 ms, p99 <=33.3 ms on the standard fixture; measure without diagnostic oracle/report overhead |
| First visible feedback | <=50 ms p95 on reference tablet; use a suitable capture method, disclose uncertainty; synthetic timing is not physical pen latency |
| Background job interaction | Visible progress and UI response <=100 ms; report full job duration and actual cancellation boundaries, including noninterruptible codec work |
| Sustained workload | >=30 draw/transform/undo/save/reopen rounds per run; exclude five warm-up rounds; repeat three times and include a two-hour mixed session |
| Matching-idle process-private growth | <=32 MiB net after warm-up and fitted slope <=1 MiB/round; investigate sustained positive trends; a larger driver cache requires demonstrated plateau and a documented bound |
| Ownership | No monotonic increase in live document/GPU ownership for a fixed workload; sample working set, Rust requests, GPU allocation/reserve, history and pending snapshots separately |
| Standard peak process-private commitment | <=2 GiB on the 2048 x 2048 / 20-layer fixture, including history and in-flight save/reopen; measure actual peak separately from matching-idle growth |
| Maximum dense/noisy peak process-private commitment | <=4 GiB on the maximum supported 32-layer fixture, including masks/history and one save/open job; this is an engineering test budget, not a raised document limit or a minimum-RAM claim |
| Completed native save/open job | Standard fixture p95 <=5 s per save or open; maximum dense/noisy p95 <=30 s per job; >=5 repetitions each, cached/cold results separate, include worker execution through publication/read completion |
| Undo/redo interaction | UI accepts the action and shows feedback within 100 ms; report full settlement duration separately and preserve ordered edits |
| Regression review | Record peak and job p50/p95 on each fixture; investigate unexplained >10% regressions; command dispatch time alone cannot pass a completed-job target |

Peak/job budgets were frozen on 2026-10-17 before the dense/noisy acceptance runs.
The 4 GiB target reserves headroom beyond the maximum 512 MiB base RGBA payload
for masks, history, snapshots, display caches, decoding and driver commitment;
actual peaks must be measured. The standard target keeps ordinary illustration
well below that maximum-workload envelope. The 30 s maximum-job budget permits
bounded bulk I/O while the existing 100 ms interaction target keeps the editor
usable. Cancellation acknowledges the request promptly and reports its actual
worker/codec/publication boundary. Neither target is evidence of a pass or a promise about an
untested disk/GPU/RAM configuration. Cold-cache methodology must be disclosed;
restarting the app alone does not flush the Windows filesystem cache.

Separate UI CPU, queue delay, worker execution and GPU presentation. Record actual
pen behavior independently. No missing/duplicate marks on fast curves, focus loss,
hover, navigation or pen/mouse switching. Do not raise document limits until dense
workloads meet budgets on recommended hardware.

## G3: complete artist workflow release gate

- Complete New -> brush/color -> sketch -> ink -> fill -> clipping/mask shading ->
  select/transform -> undo/redo -> save -> restart/reopen -> opaque/transparent PNG.
  Confirm expected pixels and unchanged editable state after export.
- Check apply/cancel, locks, active mask/layer feedback, hidden layers, folders,
  preview settlement and following commands for every destructive tool.
- New-user target: New -> draw -> add layer -> save -> export within ten minutes
  without developer coaching. Record friction and fix blocking findings.
- Verify 1280 x 800 and 1920 x 1080 physical windows at 100/150/200% scaling;
  record actual logical area. Keep controls reachable in short windows and >=65%
  canvas width in the default 1280-logical-width brush layout.
- Text entry owns shortcuts; popups/modals own Escape. Visible keyboard focus,
  reachable common actions, usable pen targets. Audit contrast, focus order and
  screen-reader exposure; disclose remaining accessibility gaps.
- Create ink/pencil/textured presets, restart, export/import into clean preferences
  and compare deterministic strokes. Test native dialogs and system image clipboard
  using disposable artwork. Complete [XP_PEN_VALIDATION.md](XP_PEN_VALIDATION.md)
  with candidate hash, driver version, scaling, brush and session duration.

## G4: distribution and support release gate

- Windows CI runs locked default/all-feature tests, formatting, warnings-denied
  all-target Clippy and release build. Give the ignored high-memory test an adequately
  provisioned pre-RC run, or resolve its support implications explicitly.
- Produce a versioned package from a recorded revision, with checksums, build
  instructions, license/third-party notices and dependency review. Decide signing
  and distribution requirements for the intended public channel before launch.
- Test clean non-developer Windows install/launch, save/recovery/export, settings,
  project migration from previous versions and uninstall preserving user artwork.
- Publish requirements, quick start, shortcuts, compatible formats, known issues,
  recovery limitations and bug-report template. Diagnostic collection stays opt-in.
- Restricted CSP import matches controlled fixtures for the advertised subset;
  unsupported content gives useful errors. Full folder/clipping parity becomes
  required only if that subset is promoted into supported import behavior.

## Architecture and scope discipline

Preserve ordered edits, immutable worker snapshots, coherent undo, stale-result
rejection and atomic publication. Improve boundaries when a measured defect or the
current package requires it. Gradually separate active-document/job state from
workspace preferences and centralize shared actions where menus/shortcuts diverge.
The existing replacement Action enum is not yet a complete command registry.

A toolkit rewrite, GPU brush rewrite, expanded extension platform or new default
storage format needs evidence before entering this release's critical path.

## Current completion and launch sequence

The original implementation/testing backlog is closed for the bounded Windows
0.1 preparation decision in RELEASE_DECISION.md. Keep its dated evidence above.

1. Produce the public-name package with `scripts/Package.ps1 -Release`; accepted
   app/DLL hashes are enforced. Verify both archives, notices and source identity.
2. Reuse the completed offline rebuild after verifying identical compiled inputs,
   Cargo settings and artifact bytes. Do not rerun the study or Full suite for prose.
3. Deliver the binary, corresponding source, checksums and publication handoff.
4. Launch separately on the owner's website/store; verify the uploaded downloads
   and begin the already-agreed independent first-use feedback.

Unmeasured diagnostic targets below/above are not reported as passes. Any new
investigation must identify an actionable unanswered question; publication and
follow-up feature work do not silently restart this completed preparation goal.

## Creator tools implementation follow-up

The requested timelapse/themes/reference/brush/plugin SDK pass is implemented; see [creator features](CREATOR_FEATURES.md) and [implementation/verification evidence](research/CREATOR_WORKFLOWS_2026-10-17.md). These additions require current-candidate artist/runtime acceptance and do not close historical release gates. Routine development uses Quick verification and focused regressions; Full release/CI retains diagnostic-feature checks and explicit large-memory fixtures. Prioritize user-reported defects and measured UI bottlenecks over expanding regression permutations.

## Multi-project and presentation workflows (2026-10-18)

The requested scope now includes a separate audience/OBS artwork viewport with
reference and layer exclusions; project tabs with a configurable limit; Pencil,
Pen, Brush and Airbrush libraries and a consolidated lasso; configurable user
library folders; and ZIP export/import with layer directories. Reference boards
must retain their normal-stack/folder placement and up to twelve images.

Implementation exists in preview_window.rs, tabs.rs, locations.rs,
project_package.rs and the grouped tool UI. Integration verification must cover
masked artwork and folder references in both preview exclusions and ZIP rebuilds,
unsaved work and background jobs when switching/closing tabs, saved folder/tab
preferences, and compact native layouts. ZIP packages retain an exact native
project; removing project.dudel deliberately selects reconstruction from edited
PNGs and manifest metadata. Older manifests lacking reference placement metadata
must fail explicitly instead of silently discarding images.

Performance acceptance remains measured: shared artwork snapshots, cached damage
tiles, bounded reference imports, and inactive tab scheduling reduce work but do
not prove unchanged frame latency or constant memory with many projects. Native
Windows/Wayland multi-monitor movement and an actual OBS capture, large-photo
reference responsiveness, and representative multi-tab memory/frame measurements
remain acceptance work. Keep these separate from focused automated checks and
keep the original MEM/SAFE/DRAW/FLOW/SHIP/RC gates open until their evidence passes.
