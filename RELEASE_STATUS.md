# Windows 0.1 release status

Updated 2026-10-28. **Release preparation is complete and accepted.** The owner requested goal
completion using the existing testing, confirmed Artist 12 use and working
Explorer previews. [RELEASE_DECISION.md](RELEASE_DECISION.md) defines the accepted
scope and explicitly distinguishes unperformed diagnostics from passing tests.

## Release identity

- Application source: the `v0.1.0` tag of the public repository.
- Version/platform: 0.1.0, Windows x64, portable, unsigned.
- Public-package name: `dudel-0.1.0-windows-x64.zip`.
- Matching source: `dudel-0.1.0-windows-x64-source.zip`.
- Each ZIP has a SHA-256 sidecar; the GitHub release publishes both with the ZIPs.
- [Accepted binaries](RELEASE_ACCEPTANCE.json): EXE `8E6E1944...134CDC8B`,
  thumbnail DLL `AD08C857...314ABD2`. Release packaging rejects different bytes.

The final build adds path-remapped binaries, a fixed-layout brush toolbar and
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

The local Full run passed 916 default / 925 all-feature application tests, three
DLL tests, 37 Python tests, both workspaces' formatting/lints and release builds.
Both dedicated maximum fixtures passed. [Local evidence](benchmarks/release-stabilization-20261027.json).
No full suite is repeated for release prose or packaging metadata.

## Delivery and follow-ups

- Prepare/upload the binary ZIP, source ZIP and both sidecars together. Include
  the release decision, known limits and support/reporting route on the download page.
- The own-site/store channel is selected; the website/domain and purchase setup
  are not configured by this repository. **No public launch has been performed.**
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

The tested systems, 4096-axis/512 MiB document envelope, format limits, unsigned
status and recovery boundaries are documented in [release notes](RELEASE_NOTES.md),
[format support](FORMAT_SUPPORT.md) and [quick start](QUICK_START.md).

[Completion audit](research/RELEASE_COMPLETION_2026-10-28.md) maps the adjusted
preparation goal to the final deliverables and states the separate launch actions.

Historical results retain their original build identities. This release decision
uses those results through explicit evidence review; it does not rewrite them
as measurements of a different binary or claim every old checklist entry passed.
