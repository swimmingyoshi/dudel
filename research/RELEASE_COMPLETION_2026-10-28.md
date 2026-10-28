# Release-preparation completion audit — 2026-10-28

The owner asked to adjust and complete the goal using existing testing, repeated
Artist 12 use and working Explorer previews. Windows 0.1 preparation is complete
under [the release decision](../RELEASE_DECISION.md). This is not a claim of public
launch or completion of every proposed research measurement.

| Goal requirement | Final evidence and disposition |
| --- | --- |
| Freeze and checkpoint source | Accepted application revision `0d8e429`; release packaging/documentation revision `3d06255`. Both app/DLL hashes are pinned in RELEASE_ACCEPTANCE.json. No feature expansion in this completion pass. |
| Fix preview/release integration defects | Strict 256-side decode bound and regression coverage; DLL tests/lints/build in verification; both artifacts recorded in packaging; both workspaces covered by source/vendor dependency checks and offline rebuild. Current Full checks passed. |
| Reconcile support docs and demonstrated polish | Current release decision, notes, format limits, quick start and handoff distinguish supported features, experimental paths and tested configurations. Existing tab/save/theme/brush/layout fixes and focused audits remain evidence. Historical pending lists no longer silently reset acceptance. |
| Verified identifiable download set | `dudel-0.1.0-windows-x64.zip`, matching `-source.zip`, individual sidecars and CHECKSUMS.sha256. Final hashes and sizes are in [the proof](../benchmarks/release-final-20261028.json) and local DOWNLOADS.json. |
| Matching buildable source | Both deliverables previously rebuilt from extracted vendored source offline. Final-package verification compares both accepted binary hashes, all 166 compiled inputs, Cargo/toolchain settings and all 23,859 vendored files. Prose/packaging changes do not cause another full rebuild. |
| Impact-based G1–G4 acceptance | RELEASE_DECISION.md records each area's evidence, accepted scope and unmeasured limits. Current safety checks and maximum fixtures, prior native safety/laptop evidence, completed study and owner feedback are used. No unperformed diagnostic is marked passed. |
| Reuse hardware/performance results | Owner's Artist 12/Explorer confirmations accepted. Three standard repeats and the two-hour maximum study passed; heap fix unchanged and later resource-related code matches focused benchmarks. No repeat study, tablet session or laptop campaign scheduled. |
| Public artifacts and handoff | Verified copies collected in `artifacts/release-handoff-20261028`, with DOWNLOADS.json, CHECKSUMS.sha256 and PUBLISHING.md. The public-name package has corrected documentation and retains matching source/notices. |
| Existing owner decisions | Initial unsigned distribution retained. Independent first-use feedback remains after publication. Existing laptop installation/runtime results are reused. |

## Final artifact identities

| File | SHA-256 |
| --- | --- |
| `dudel-0.1.0-windows-x64.zip` | `95C04690D2BA067EC4861C2F0E71508FFCC8D6B21F3A0350D6D599A0ADFE2DE7` |
| `dudel-0.1.0-windows-x64-source.zip` | `E6AC200A5DB5F9003EAE47AAA8B1A4D4912A12878B0A663646F213E9D9FFCCB5` |

The final notices check passes for 458 dependencies, 880 notice files and 139
provenance hashes, with no changed reviewed notice/dependency inputs. Both
archives' integrity and matching-source relationship pass. Packaged Markdown
checks resolve 187 binary-archive and 203 source-archive local links.

## Separate launch work and retained limits

The website/domain, purchase configuration and public support route still need
actual deployment values. The repository is private; its issue page is not a
public support route. Upload/re-download and public availability are not claimed.
PUBLISHING.md gives the exact launch sequence and DOWNLOADS.json leaves missing
URLs null instead of inventing them. No GitHub release or public tag was published.

The optional old Actions-artifact cleanup is prepared with verified backups but
not authorized or performed. Its upload quota does not block delivery of the
independently verified local release files. No successful test suite was rerun to
retry uploads.

GPU commitment, precise pen latency, exhaustive OS/device/accessibility matrices
and broader interop remain disclosed follow-up investigations under the adjusted
first-release scope. They are not concealed passes or requests for another
blanket testing campaign. A demonstrated safety/blocking regression reopens its
specific acceptance area.
