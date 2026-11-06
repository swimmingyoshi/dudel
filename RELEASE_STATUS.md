# Windows 0.1 release status

Updated 2026-11-05. **Release preparation for the 2026-11-04 rebuild of 0.1.0 is
complete.** It replaces the unpublished 2026-10-31 package and is built from
`bffcbea`; its evidence and untested areas are in
[the release decision](RELEASE_DECISION.md). The owner requested goal
completion using the existing testing, confirmed Artist 12 use and working
Explorer previews. [RELEASE_DECISION.md](RELEASE_DECISION.md) defines the accepted
scope and explicitly distinguishes unperformed diagnostics from passing tests.

## Release identity

- Application source: commit `bffcbea`; later commits change documents only. The
  `v0.1.0` tag names the final release-records commit.
- Version/platform: 0.1.0, Windows x64, portable, unsigned.
- Public-package name: `dudel-0.1.0-windows-x64.zip`.
- Matching source: `dudel-0.1.0-windows-x64-source.zip`.
- Each ZIP has a SHA-256 sidecar; the GitHub release publishes both with the ZIPs.
- [Accepted binaries](RELEASE_ACCEPTANCE.json): EXE `D53A9ED9...725ECD30`,
  thumbnail DLL `A0FCCA48...EE41DEC1`. Release packaging rejects different bytes.

The 2026-11-04 rebuild adds the vector, shortcut, search, layer and stabilization
work in the release notes. It passed Full verification with 1,137 default / 1,146
all-feature tests, three DLL tests and 42 Python tests. The owner's pen passes on
2026-11-04 and 2026-11-05 passed, the second covering selection limits, fill to line
centres, line handles and SVG export.
Hosted Windows CI passed on `e20c76b` (run 41948291941), including the maximum
project fixtures and candidate packaging. Hosted Linux did not run on it. The
table and results below describe earlier builds unless they say otherwise.

The 2026-10-31 rebuild adds the import, export, effects, adjustment and
large-document work in the release notes. The earlier final build added path-remapped binaries, a fixed-layout brush toolbar and
low latency drawing by default to the previously accepted source `0d8e429`
(private development history); see [the release decision](RELEASE_DECISION.md).
The earlier [final archive/equivalence proof](benchmarks/release-final-20261028.json)
and [candidate proof](benchmarks/release-candidate-20261027.json) describe that
previous build and its archives.

## Acceptance disposition

| Area | Result |
| --- | --- |
| G1 artwork safety | Accepted within the documented atomic-write/recovery boundary. Current Full verification, dedicated maximum fixtures and existing native safety/portability evidence cover the release workflow. [Safety coverage](benchmarks/release-safety-coverage-20261027.json). Exhaustive device/power-loss guarantees are not claimed. |
| G2 resources | Completed three standard repeats and a two-hour maximum study passed. Heap-compaction code is unchanged; later viewport/timelapse/dab/fill/document sources match their focused benchmark records. [Reconciliation](benchmarks/release-evidence-reuse-20261028.json). No repeated study is scheduled. |
| G3 artist workflow | Owner confirms Artist 12 works after repeated development testing and Explorer previews work with the DLL. Existing workflow and UI evidence is retained. Exact physical latency is not inferred. Independent first-use feedback remains post-publication. |
| G4 release preparation | Both artifacts, ZIP/source integrity, notices and offline source rebuild pass. Static runtime/laptop evidence is reused. Final distribution files are checked against accepted inputs; website/store publication is a separate launch action. |

The 2026-10-31 rebuild passed local Full verification with 1,030 default /
1,039 all-feature application tests, Clippy, Python checks and release builds,
as recorded in [the release decision](RELEASE_DECISION.md). The earlier
[2026-10-27 evidence](benchmarks/release-stabilization-20261027.json) records
916 default / 925 all-feature tests, three DLL tests, 37 Python tests and both
dedicated maximum fixtures; those are historical results, not counts for the
rebuilt package or current HEAD.

The 2026-11-02 review of `f21c494` passed Quick verification: 1,076 application
tests (12 ignored), three DLL tests, 42 Python tests, formatting, default Clippy
and the public-tree check. This was not a Full build, packaging acceptance,
physical pen session or performance soak. See [stabilization plan](STABILIZATION_PLAN.md)
for subsequent fixes and their separate verification.

The stabilization fixes subsequently passed a complete Quick run in an isolated
checkout: 1,079 application tests (12 ignored), three DLL tests, 42 Python tests,
formatting, Clippy and public-tree checks. On 2026-11-03, integration with vector
stages 1-3 through `7a30db3` passed 1,092 application tests (12 ignored) and
formatting. Integrated Clippy was canceled while waiting for a concurrent release
build, as further vector work continued; no complete Quick pass is claimed for
that changing tree. [The plan](STABILIZATION_PLAN.md) records the scope and logs.

## Delivery and follow-ups

The subsequent workflow improvements passed 1,109 default application tests
(14 ignored), strict all-target Clippy, formatting, 42 Python tests and privacy
checks on 2026-11-03. The 64 targeted workflow/controller checks also passed.
See [the workflow plan](UX_WORKFLOW_PLAN.md) for scope and local evidence. This
development verification does not replace the packaged-release acceptance above.

The 2026-11-04 shortcut increment adds saved bindings, conflict detection,
resets and profile import/export. Its final default-feature suite passed 1,118
tests (14 ignored) with four test workers, including the corrected F6 focus
navigation path. Strict all-target Clippy, formatting, source privacy scans and
the development build passed. Native desktop and scaled compact shortcut UI
captures were reviewed. The initial concurrent-run failures and final evidence are recorded in the
[workflow plan](UX_WORKFLOW_PLAN.md); this remains source-development evidence.

The subsequent multi-layer increment passed 1,125 default-feature application
tests (14 ignored) with four workers, including selection UI, atomic batch
edits, lock rejection and single-step undo. Strict all-target Clippy, formatting,
public-tree checks, whitespace checks and the development build passed. Native
desktop and scaled compact selection controls were visually reviewed. Details
and validation logs are in the [workflow plan](UX_WORKFLOW_PLAN.md).

- 0.1.0 was published on 2026-11-05 as a GitHub pre-release, with Windows and
  experimental Linux packages. For later releases, upload the binary ZIP, source
  ZIP and both sidecars together. Include
  the release decision, known limits and support/reporting route on the download page.
- The own-site/store channel is selected; the website/domain and purchase setup
  are not configured by this repository. The pre-release has not been announced;
  first testers are the next step ([tester guide](TESTER_GUIDE.md)).
- Windows/Linux hosted verification steps passed on `0d8e429`. Their artifact
  uploads failed due to GitHub storage quota. This does not invalidate the
  independently verified local release files. [CI/package evidence](benchmarks/release-candidate-20261027.json).
- [45 older CI artifacts were backed up and verified](benchmarks/ci-artifact-backup-20261028.json);
  the owner later judged them superseded and the local copies were deleted
  (2026-10-28). The hosted copies remain with the archived development history.
  Do not rerun successful suites to probe quota recalculation.
- Actual GPU commitment, precise physical latency, exhaustive OS/accessibility
  matrices and broader compatibility remain scoped follow-up research. A new
  test needs a concrete unanswered question whose answer changes a decision.

The tested systems, 16,384 px / 100 MP / 1,000-layer / 2 GiB document envelope, format limits, unsigned
status and recovery boundaries are documented in [release notes](RELEASE_NOTES.md),
[format support](FORMAT_SUPPORT.md) and [quick start](QUICK_START.md).

[Completion audit](research/RELEASE_COMPLETION_2026-10-28.md) maps the adjusted
preparation goal to the final deliverables and states the separate launch actions.

Historical results retain their original build identities. This release decision
uses those results through explicit evidence review; it does not rewrite them
as measurements of a different binary or claim every old checklist entry passed.
