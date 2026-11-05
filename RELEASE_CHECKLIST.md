# Release acceptance checklist

Current decision, 2026-10-28: Windows 0.1 preparation is accepted under
[RELEASE_DECISION.md](RELEASE_DECISION.md). The dated development entries below
are history; their former pending tasks are not a fresh test request.

The [current release status](RELEASE_STATUS.md) separates working-source changes,
the latest packaged workflow handoff and the older frozen performance study.
Detailed dated checkpoints below remain evidence for their identified builds.

Final 0.1.0 build (rebuilt 2026-11-04): source `bffcbea`. The executable SHA-256
begins `D53A9ED9`; the thumbnail DLL begins `A0FCCA48`. Before publication: move the
`v0.1.0` tag and run hosted CI once. See RELEASE_DECISION.md.

Previous unpublished build (rebuilt 2026-10-31): tag `v0.1.0`. The executable SHA-256
begins `A3F989AF`; the thumbnail DLL begins `6302A480`. The evidence on this
rebuild is listed in RELEASE_DECISION.md. The unpublished first build
(EXE `8E6E1944`, DLL `AD08C857`) added path-remapped binaries, a fixed-layout
brush toolbar and low latency by default to the previously accepted source
`0d8e429` (candidate `20261028-003227-120`, EXE `9713876B`, DLL `7A82B099`), whose
full checks, maximum fixtures, native captures, packaging/notices checks and
offline rebuilds the dated entries below record. See RELEASE_DECISION.md.

## Earlier development checkpoints

October 19 handoff: `dudel-0.1.0-candidate-20261019-233855-419`.
[Improvements proof](benchmarks/release-improvements-20261019.json) records local
full verification (885 default / 894 all-feature tests, both Clippy configurations,
34 Python checks and release build), reviewed DX12 captures, archive/input/notice
checks, packaged startup and study CheckOnly. Its [workflow checklist](CANDIDATE_TEST_CHECKLIST.md)
includes contextual transform previews. Hosted CI and independent offline rebuild
of this archive remain pending alongside physical artist, sustained performance
and clean-machine acceptance. The preceding
[workflow candidate proof](benchmarks/workflow-candidate-20261019.json) retains its
Windows/Linux CI result for application source bc3515f; that result is not a CI
pass for the new changes. Earlier candidate identities below retain their own evidence.

Created 2026-10-17. Execute against [DELIVERY_PLAN.md](DELIVERY_PLAN.md).
An unchecked gate is open. This checklist is a starting record, not certification.

## Historical acceptance notes (beginning 2026-10-19)

[Prebuilt source-role review](research/PREBUILT_SOURCE_ROLES_2026-10-19.md) now
accounts for the sixteen upstream Windows import libraries, libloading test DLLs,
WASI helpers and empty vcpkg fixtures. Their provided import/link/helper source
and target/test roles were checked. The binary ZIP has no additional native
programs beyond dudel.exe. This records the role/source review; it does not claim
independent regeneration of every upstream binary or complete G4 acceptance.

[Native source/notice review](research/NATIVE_COMPONENT_REVIEW_2026-10-19.md)
now inventories and checksum-verifies 47 C/C++/header files in twelve crates plus
the prebuilt-named payloads. It identifies the frozen Windows binary's SQLite
3.53.2 source ID, the default bundled/non-SQLCipher selection, and disabled Linux
Wayland C logging shims. All involved collected notices match binary/source.
The 341 vcpkg library-named fixtures are empty placeholders. Prebuilt source-form
and whole-program compatibility, clean Windows and final G4 acceptance remain open.

[Four legacy declaration reviews](research/LEGACY_LICENSE_DECLARATIONS_2026-10-19.md)
are resolved for the 8D2C768E candidate's notice bundle: explicit upstream MIT/CC0
declarations, original crate checksums, pinned release source and preserved author
attribution were reverified. Canonical terms remain labelled references; no
upstream file or copyright year was invented. This is a version-specific review
decision, not complete native-subcomponent compatibility or public release acceptance.

[Windows native save failure/retry](benchmarks/native-save-sharing-failure-20261019.json)
passes on engineering source `2afdaaa`: an exclusive handle on an existing valid
destination produces actual Windows sharing error 32. Both v4/v5 retain exact
old-file bytes and recovered edits, remain unsaved with no project path, and
successfully retry/save/reopen after releasing the handle. Original project and
snapshot bytes remain intact; failure/success captures were reviewed. Formatting,
default all-target Clippy and release compilation pass.
[Linux hosted verification](benchmarks/linux-hosted-save-sharing-20261019.json)
passes on this diagnostic source (869 default / 873 all-feature tests, 34 Python
checks, both lints, release and verified X11 artifacts).
[Windows hosted verification](benchmarks/windows-hosted-save-sharing-20261019.json)
also passes (877 default / 886 all-feature tests, 34 Python checks, both lints,
release, both maximum fixtures and packaging/upload). The hosted package reports
458 dependencies and no missing notice collection; artifact bytes were not
downloaded/audited. The preceding probe-source runs are superseded.
Disk exhaustion, device errors, OS file chooser and full G1 acceptance remain open.

[Large/busy recovery follow-up](benchmarks/large-busy-native-recovery-20261019.json)
passes on engineering source `57eead3`: 2048 x 2048 noisy two-layer v4/v5 fixtures,
both idle and with a live adjustment held open for 20 seconds. Recovery excludes
the temporary preview and completes about 1.2 seconds after cancellation; all
four interruption/restore/save/reopen cases preserve exact editable state and
original files. Ten focused recovery checks, formatting, default Clippy and release
build pass. This diagnostic build differs from the frozen candidate only in the
probe module. Later source `2afdaaa` has passing hosted checks above. Maximum
layers, active physical strokes/queued edits, slow/failing storage and the full
G1 matrix remain separate.

Study result, 2026-10-21. Candidate `dudel-0.1.0-candidate-20261021-190309-093`
(EXE `7C3090D3...F659D7`) contains the 2026-10-19 improvements and heap compaction
after a document replace or tab close. Its packaged Full DX12 study passed on the
reference PC (Ryzen 5 3600X, RTX 3060):
- Three standard 20-layer sparse repeats, 30 rounds each.
- The two-hour maximum 32-layer noisy mixed-brush session: 59 rounds, 7,247 s,
  idle net growth at most 7.8 MiB and slope at most 0.11 MiB/round.
- All job and lifetime-peak targets passed. The largest unexplained callback gap
  was 86.4 ms.

The earlier `…111249-426` study passed its standard repeats, but the maximum session
failed on heap retention, not live ownership. Evidence:
[summary](benchmarks/heap-compaction-study-20261021.json) and
[research](research/MAX32_HEAP_RETENTION_2026-10-20.md). This closes the synthetic
idle-study part of G2 only.

Earlier candidate `dudel-0.1.0-candidate-20261019-111249-426` on `fda875f`
fixes adjustment-dialog movement during live previews. [Focused evidence](benchmarks/adjustment-popup-verification-20261019.json)
records 10 passing adjustment/menu checks, formatting, default all-target Clippy,
release compilation, reviewed 100%/200% captures and archive/input/notice checks.
This is focused local validation, not another full local suite.
[Latest Linux CI](benchmarks/linux-hosted-adjustment-20261019.json) now passes
868 default / 872 all-feature tests, 34 Python checks, both lints, release and
verified X11 capture artifacts. The captures match the previously reviewed bytes.
[Latest Windows CI](benchmarks/windows-hosted-adjustment-20261019.json) now passes
876 default / 885 all-feature tests, 34 Python checks, both lints, release,
both dedicated maximum fixtures and packaging/upload. The hosted package reports
458 dependencies and no missing notice collections; its artifact bytes were not
downloaded/audited. Local package checks remain separately identified.
These runs supersede the cancelled bucket-source runs.

[Latest rebuild and native safety](benchmarks/adjustment-offline-rebuild-native-safety-20261019.json)
pass on this candidate: fresh offline release compilation, rebuilt DX12 startup,
packaged v4/v5 interruption/Restore copy/save/reopen equality and all six settings
phases. Four small-idle snapshot times span 15.75-16.47 seconds. This evidence
does not close large/busy recovery, clean-machine or physical-input acceptance.
The idle study remains available and unstarted.

[Bundled asset review](research/BUNDLED_ASSET_REVIEW_2026-10-19.md) verifies exact
unchanged executable/source font payloads, font metadata and packaged terms, plus
the original-file checksums for both MPL crates. Complete native-subcomponent
compatibility review remains open; an empty notice-collection list is not final
compatibility acceptance. The four manifest-only items now have the separate
review disposition linked above.

Application source `f90e3af` adds soft-edge bucket filling, including transparent
and flattened linework, separate reference layers, selection/opacity and queued
Undo/Redo checks. Fill controls scroll within the space above Color at high
scaling. [Current bucket and package evidence](benchmarks/fill-edges-verification-20261019.json)
records candidate `dudel-0.1.0-candidate-20261019-110448-355`, local Full verification
(875 default / 884 all-feature tests, 34 Python checks, both lints and release
build), reviewed DX12 100%/200% captures and packaged study preflight. Two large
tests remain ignored in each ordinary suite. Hosted CI for this source is pending.

The package includes `RunPerformanceStudy.ps1`; the user can run it when leaving
this PC idle or on a laptop, with hardware recorded per run. Full mode performs
three standard repeats and one maximum workload lasting at least two hours.
No sustained run was started during this work. Preflight does not establish a
resource pass. Earlier candidate `20261019-105234-802` has a broken default study
path and overlapping short Fill controls; use the newer candidate above.

Application source `ffcf959` includes the responsive inspector fix. Candidate
`dudel-0.1.0-candidate-20261019-101430-664` has verified matching inputs, archive
checksums/CRCs, 880 matching notice files and reviewed Windows 100%/200% captures.
[Fresh rebuild and native safety evidence](benchmarks/current-offline-rebuild-native-safety-20261019.json)
now proves a fresh locked offline build, rebuilt DX12 startup, packaged-candidate
v4/v5 interruption/restore/save/reopen and all six isolated settings phases. The
small idle recovery fixtures completed snapshots in 15.76-16.54 seconds; this is
not large/busy recovery or physical power-loss acceptance.

[Linux CI and reviewed X11 captures](benchmarks/linux-hosted-inspector-20261019.json)
pass on `ffcf959` (864 default / 868 all-feature tests, 34 Python checks, both
lints and release build). [Windows run 39236445681](https://github.com/swimmingyoshi/dudel/actions/runs/39236445681)
also passed on `ffcf959` (872 default / 881 all-feature tests, 34 Python checks).
The full resource studies, physical XP-Pen/Wayland/OBS,
clean Windows, dependency compatibility and final RC acceptance remain open.
The sustained study is awaiting a user-run idle test window. The prior buffered-recorder study stopped after
one completed repeat and is not live.

Engineering narratives below preserve earlier build-specific observations;
their old running/pending statements are historical. Use this section,
[DELIVERY_PLAN.md](DELIVERY_PLAN.md) and the required evidence table for current status.

## Branch synchronization review (2026-10-17)

The combined Windows/Linux source based on `5bd1db5` passes 802 default and
811 all-feature tests, 28 Python checks, formatting, both warnings-denied Clippy
configurations and Windows release compilation. Two large-memory tests were not
rerun. Log: `target/qa/branch-sync-verification-20261017.log`.
See [current progress and remaining priorities](research/PROGRESS_REVIEW_2026-10-17.md).
Reference layers, expanded blend modes, new time-based stabilization and the
Wayland port supersede the earlier feature snapshot. Historical package and
hardware results below do not certify this combined source. Current stabilizer
retest, Linux runtime verification and all unchecked release gates remain open.

## Candidate identity (required before acceptance)

Earlier packaged engineering candidate: GPL/About and cancellable native saves,
executable SHA-256 `2402AE958CFBBDFA178809186FCD34E501C31C4761D619E677E7317E7D494703`,
`artifacts/releases/dudel-0.1.0-candidate-20261017-144000-843.zip`.
Matching source: the adjacent `-source.zip`; both SHA-256 sidecars are present.
[Package verification](benchmarks/gpl-package-verification.json) records checksums,
452 dependency notice entries and four legacy declaration/reference review items.
Both ZIP CRC/checksum checks pass, as do source hashes and explicit bundled-font
notice checks. An independently extracted source archive passed fresh offline
release compilation (11m 29s) and rebuilt-executable startup. Every compiled input,
Cargo configuration and all 23,547 vendored files in the corrected archive are
byte-identical to that verified build. Documentation/notice-only corrections did
not require repeating compilation. Evidence:
`target/qa/gpl-source-archive-rebuild-20261017/STATE.json` and the package report.

Default/all-feature tests: 783/792 pass, one high-memory test ignored normally.
Formatting, default/all-feature Clippy, eleven Python checks and release build pass
(`target/qa/gpl-candidate-20261017/verification.log`). The dedicated maximum-project
test passed after storage changes (100.17 seconds). Sixteen actual child
terminations and Windows read-only destination protection pass.

The artist accepted the smooth catch-up candidate (`995C5A...323E7F`) with
"Smooth now"; subsequent changes preserve that easing. This is targeted acceptance,
not the full two-hour tablet matrix. The first completed thirty-round twenty-layer
DX12 run meets per-run memory targets: final matching-idle +1.1484 MiB,
0.101884 MiB/round after five warm-up rounds. See
[summary](benchmarks/standard20-r30-dx12-memory-gates.json).
All three current-candidate repeats completed: 30 rounds each, 20 layers,
1,724.77 / 1,812.25 / 1,561.39 seconds. Every matching-idle private-memory and
tracked-ownership subtarget passes. Final matching-idle net growth is 5.21 / 5.57 /
3.15 MiB, with fitted slopes 0.208 / 0.204 / 0.106 MiB/round after five warm-up rounds.
[Aggregate evidence](benchmarks/gpl-standard20-three-run-summary.json).
These frozen runs do not contain lifetime-peak or completed-job timing counters;
dense workloads, a continuous two-hour session, additional brush presets, GPU
ownership and final-candidate acceptance remain open.

## Safety engineering checkpoint

The recovery diagnostics/shared startup-discovery extraction are verified on
executable `151CC53C708BF5681883E566C686BC355C9CCED3636D8324D948F8EE9037000F`.
[Source identity](benchmarks/native-recovery-build.json),
[native recovery results](benchmarks/native-artwork-recovery-results.json) and
[native settings results](benchmarks/native-settings-current-results.json) identify
this build separately from the packaged GPL candidate and running memory study.
All 783/792 application tests, 14 Python checks, formatting, both Clippy configurations
and the release build pass (`target/qa/recovery-probe-final-verification-20261017.log`).

Latest fixture engineering build: see [source identity](benchmarks/dense-boundary-build.json).
The 32-layer dense generator now respects the existing 512 MiB combined mask/raster
payload cap. The real maximum noisy fixture validates at that boundary (26.15 s).
All 784 default / 793 all-feature tests, 14 Python checks, formatting, both Clippy
configurations and release build pass; two large-memory tests are ignored normally.
The new boundary test passed separately. See [fixture inventory](research/NATIVE_SOAK_FIXTURES.md).
This newer diagnostic build is distinct from the still-running frozen memory study.

## Measurement and Windows GUI engineering checkpoint

Frozen executable `5B97376259EFB878ED082EFB796E2F30B8C098238D91B516E62CD5F33C1E1D22`
adds lifetime OS private/resident peaks and command-to-editor-ready save/open
measurements. Source identity: [job/peak build](benchmarks/job-peak-build.json).
The new Windows GUI release builds successfully in an isolated Cargo target; the
user's running executable remains untouched. Native DX12 smoke passes with two
completed jobs. See [Windows audit](research/WINDOWS_PACKAGE_AUDIT.md).

785 default / 794 all-feature tests, 18 Python checks, formatting and both Clippy
configurations pass; two large-memory tests remain ignored normally. The ordinary
release target was locked by the user's running app; the isolated release build
passed. Verification: `target/qa/job-peak-verification-20261017.log` and
`target/qa/job-peak-target-20261017/build.log`. A 30-round maximum supported
32-layer noisy run stopped incompletely after 24 full rounds at a diagnostic
resource limit. No complete memory/job acceptance can be calculated; see
benchmarks/maximum32-noise-incomplete-summary.json.

The user confirmed the physical tablet model as **XP-Pen Artist 12**. Prior
pressure and smooth catch-up feedback remains valid for the identified builds;
active driver, display scaling and full-session workflow evidence remain open.

## Capture-error follow-up engineering build

Frozen executable `5C2554712F13117DEC605CEA14B5EDC573D3B480F8629BC5FB90F75A3317EAE9`
adds the verified F6 control entry, longer dense diagnostic limits, partial error
reports and a guard against screenshot-stage fall-through after a diagnostic error.
789 default / 798 all-feature tests, 21 Python checks, formatting, both Clippy
configurations and release build pass. Two large-memory tests remain ignored normally.
Source identity: [capture-abort build](benchmarks/capture-abort-build.json).
The stricter native failure probe passes, including no spurious screenshot timeout;
normal native smoke is running. Packaging
and complete resource/workflow acceptance remain open. See
[diagnostic evidence](research/SOAK_PROCESS_OBSERVATION.md).

## Duration-controlled mixed workload engineering build

Frozen executable `7F765F63EEA25BCDC2B625FF2A666ECC7F4B3E09DB674D3DD0B5A5C623B654D8`
passes 791 default / 800 all-feature tests, 28 Python checks, formatting, both
warnings-denied Clippy configurations and release compilation. Two large-memory
tests remain ignored normally. [Source identity](benchmarks/duration-continuity-build.json).
The three-minute native probe completes five rounds / 181.854 observed seconds,
80 mask edits, five strokes and five save/open pairs across all three configured
brushes; continuity and power restoration pass. It is not resource acceptance.
[Protocol](research/NATIVE_DURATION_RUNS.md) and [results](benchmarks/duration-mixed-probe-results.json).
Current binary/source packaging and its fresh offline build are next. A package's
adjacent BUILD.json and SOURCE.md identify its own executable/source pair; earlier
package identities above remain historical. Full resource and manual gates remain open.

## Planning-review checks

These results apply to the current working tree on 2026-10-17. Application source
was not changed by this review. Existing diagnostic edits were present and were
not committed or frozen into a candidate by this task.

- [x] Default `cargo test --locked`: 760 passed, zero failed, one ignored.
- [x] `cargo fmt --all -- --check`: passed.
- [x] `cargo clippy --locked --all-targets -- -D warnings`: passed.
- [x] Engineering candidate: 767 default / 776 all-feature tests pass; formatting
  and default/all-feature warnings-denied Clippy pass.
- [x] Dedicated high-memory ignored test: passed separately (94.28 seconds).
- [x] Frozen engineering binaries, candidate ZIP/checksum and isolated startup capture.
- [ ] All checks on the final release candidate; hosted CI and clean-machine acceptance.

Execution evidence: [persistent display workers](research/DISPLAY_WORKER_REUSE.md).
Original planning-review results above remain historical; current source includes
the display-worker implementation. The engineering candidate is not release-certified.

## Final preparation disposition (2026-10-28)

The owner requested an adjusted, completed release-preparation goal after
confirming the repeated Artist 12/Explorer testing and completed unattended studies.
[RELEASE_DECISION.md](RELEASE_DECISION.md) is authoritative for this first release.

| Gate | Disposition | Evidence / boundary |
| --- | --- | --- |
| BASE | Complete | Final application inputs: tag `v0.1.0` (adds path remapping, fixed toolbar layout and low latency default to `0d8e429`); EXE/DLL hashes pinned in RELEASE_ACCEPTANCE.json. Packaging refuses other bytes. |
| G1 | Accepted for the documented save/recovery boundary | Current automated safety map, maximum fixtures, exact format/undo tests, native recovery and Windows sharing-failure evidence. No known artwork-loss bug is waived; physical device/power-loss certification is not claimed. |
| G2 | Accepted using completed studies and focused follow-ups | Three standard repeats and the two-hour maximum study passed; heap fix unchanged; current resource-related sources match subsequent benchmarks. No repeated study scheduled. |
| G3 | Owner usability confirmation accepted | Artist 12 works after repeated development use; previews work after the DLL. Existing illustration/brush/theme/ZIP/UI evidence retained. Formal latency/session metadata is not invented. |
| G4 preparation | Accepted; verify final packaged files against pinned inputs | Both deliverables rebuilt offline; current archives/notices verified; static-runtime laptop evidence retained. Release handoff contains matching source/checksums. Hosted upload quota is an infrastructure issue. |
| Publication | Separate launch action | Website/payment configuration, upload and public re-download are not performed by this preparation task. Independent first-use feedback follows publication. |

## Explicit follow-up limits

The earlier exhaustive OS/device, maximum-busy native recovery, physical latency,
GPU attribution, accessibility/multi-monitor and expanded CSP fixture matrices
are retained as engineering follow-ups, not represented as passed. Supported
format restrictions and known recovery/platform boundaries remain public. A new
measurement needs a specific unresolved question and the smallest useful method;
completed tests are not repeated solely because the package has a new timestamp.

## Release decision

Preparation accepted with the scope and evidence in RELEASE_DECISION.md. The
final package must match RELEASE_ACCEPTANCE.json and pass archive/source/notices
checks. Acceptance is not public availability, universal hardware compatibility
or a claim that unperformed diagnostics succeeded.

## Creator workflow development candidate

Application revision `3f00525` adds timelapse recording/GIF export, themes, compact reference organization, four brushes and plugin authoring/sharing tools. See [verification scope](research/CREATOR_WORKFLOWS_2026-10-17.md). The initial full normal suite and focused follow-ups, Clippy, formatting, Python and release compilation pass. Two large-memory fixtures and diagnostic feature suites were not repeated for routine UI work. Current-candidate long sessions, native latency, Linux runtime, hosted CI and independent release acceptance remain open.

## Historical workflow candidate baseline (2026-10-18)

Application source 5b7203e (subsequent scope-documentation commit 9290eb8 leaves
compiled inputs unchanged) passes 849 default and 858 all-feature tests, 28 Python
checks, formatting, both all-target warnings-denied Clippy configurations and
Windows release compilation. Two large-memory fixtures remain ignored in each
local suite; the hosted Windows job schedules them explicitly. Candidate executable
SHA-256: 83A749CC9BA0D99E3A36DD549702F9671AEA4A57F4696D254B5B1B2D626EA7A4.

See benchmarks/release-gates-current-20261018.json and the live execution record
target/qa/release-gates-20261018/PIPELINE_STATE.json. Matching-source packaging, archive checksums/CRC and recorded input checks pass.
The independent offline rebuild is running; sequential current-candidate DX12
studies are queued behind it. Hosted CI run 39143558188 was observed running; no hosted or sustained
pass is claimed yet. Previous physical feedback and resource studies retain their
original build identities. All unchecked acceptance gates remain open.

Current dependency inventory: benchmarks/windows-dependency-review-20261018.json
maps 223 Windows normal-edge dependency versions to the 458-entry package notice
index. 378 supplied notice files match their vendored source bytes; the bundled
font notices are present. Nine image/GIF/ZIP-related dependency versions were
added since the prior review. The four legacy notice-review items remain explicit;
this inventory is not final licensing or clean-machine acceptance.

Independent rebuild follow-up: source archive E071F860...99F13414 passed
checksum/CRC, recorded source hashes and a fresh locked offline Windows release
build (8m 57s). The rebuilt executable is E1040993...F9B7071; different build-path
bytes are expected and binary byte reproducibility is not claimed. The current
packaged executable is running the 180-second DX12 mixed-brush semantic probe.
Rebuilt-executable startup, complete current-candidate resource studies and all
other unchecked gates remain open. See the current release-gates evidence JSON.

Current DX12 probe: six rounds / 182.063 observed seconds on NVIDIA GeForce RTX
3060 pass, including 96 mask edits, six generated brush strokes and six native
save/open pairs. Continuity and power restoration pass; screenshot visually
reviewed. See benchmarks/current-dx12-mixed-probe-20261018.json. This four-layer
probe does not evaluate resource targets. The first current 30-round/20-layer
mixed-brush resource run is live; complete studies remain pending. Hosted CI
39143558188 has passed Verify and both dedicated maximum-memory fixture steps;
packaging/upload are still pending. No complete CI or release pass is claimed.

The current standard study was intentionally interrupted to prioritize the
reported rope-distance control and off-page drawing defect. Only the owned
synthetic process was stopped; its controller restored the temporary power
request and retained logs. No resource pass is inferred from that partial run.
The frozen package/rebuild/probe evidence remains tied to its recorded binary;
the drawing fix requires a separately identified candidate and later studies.

Hosted CI follow-up: runs 39143558188 (pre-edge-fix source) and 39146325725
(rope/edge source 7f0fe36) completed successfully, including dedicated maximum
fixtures, packaging and artifact upload. See the current release-gates JSON and
benchmarks/rope-edge-hosted-ci-20261018.json. This closes the observed CI execution
uncertainty for those revisions, not the entire G4 acceptance gate.

The local full run on 7f0fe36 then exposed an intermittent native access violation
while a clipboard transaction test read the real system clipboard. A debugger
reproduction identified DragQueryFileW via arboard's copied-file list reader.
Serial success was retained as diagnostic evidence, not substituted for the
parallel gate. Clipboard tests now inject file discovery; Windows production
reads validate a bounded CF_HDROP allocation directly. Subsequent full checks
and candidate identities are recorded separately. All sustained, physical,
Linux-runtime and clean-machine acceptance gates remain open.

Clipboard hardening follow-up: the complete parallel suites pass (853 default /
862 all-feature tests, two ignored in each), as do 28 Python checks, formatting,
both all-target Clippy configurations and Windows release compilation. The
release executable 33164050...D0C202DD starts on DX12; its capture was reviewed.
Twenty separate native read-only clipboard probes completed without replacing
clipboard contents. See benchmarks/clipboard-file-reader-verification-20261018.json
for exact tested input hashes and preserved crash evidence. The next identified
package/rebuild/resource pipeline reuses these matching completed checks.

Runtime follow-up for 33164050...D0C202DD: the independent source rebuild and
rebuilt DX12 startup pass. Hosted CI 39148261858 also completed successfully.
All three 30-round/20-layer sparse mixed-brush repeats pass memory, completed-job
timing, lifetime-peak and continuity subtargets. The maximum noisy run stopped at
1,152,000 recorded frames after 7,585 observed seconds, 28 complete rounds and
part of round 29. This is a diagnostic frame-storage failure, not completed
maximum-workload acceptance. The controller stopped and restored its power request;
partial evidence is retained. See benchmarks/release-gates-clipboard-fix-20261018.json.

The next package also fixes missing offline documentation dependencies; staged
binary/source link closure and unchanged build inputs are verified separately in
benchmarks/package-doc-links-verification-20261018.json. The measured archives
remain unchanged. Owner channel decision: own website/store; no signing
credentials yet. Public-release signing and all unchecked acceptance gates remain open.

Buffered recorder follow-up: engineering executable D41C0072...2173011A uses fixed
64 KiB frame staging and streams all callback records into its final JSON report.
856 default / 865 all-feature tests, both Clippy configurations, release build and
32 Python checks pass. Corrupt logs and writer errors cannot produce an accepted
completed report; logging overhead stays separate from application UI timings.
Native integration now passes six rounds / 198.31 seconds with 32,004 complete
frame records, constant 64 KiB staging, twelve save/open jobs, continuity and power
restoration. Capture reviewed; four-layer probe resource targets stay unevaluated.
New archive identity and candidate-specific resource studies remain pending.
See benchmarks/frame-log-fix-verification-20261018.json. No prior
maximum-run failure or unchecked acceptance gate has been converted into a pass.
