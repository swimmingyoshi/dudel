# Windows 0.1 release decision — 2026-10-28

**Release preparation is accepted for the Windows scope below.** The owner asked
to adjust and complete the release goal using the existing testing and confirmed
Artist 12 and Explorer results. This decision replaces the earlier blanket
retest requests and provisional engineering checklist as the current acceptance
record. It does not claim every proposed diagnostic was performed.

The accepted application source is the `v0.1.0` tag of the public repository.
[RELEASE_ACCEPTANCE.json](RELEASE_ACCEPTANCE.json) pins both binary hashes, and
release packaging refuses different executable or DLL bytes.

**Final changes, 2026-10-28.** The evidence below was gathered on the earlier
accepted application source (`0d8e429`, private development history). The final
build differs from it in three reviewed ways: binaries are built with path
remapping so they no longer embed the build machine's folders; the brush toolbar
keeps a fixed layout in Smooth and Rope mode (follow options moved into a menu);
and low latency drawing is on by default. Full local verification passes on the
final source (918 Rust tests, Python checks, clippy, privacy check), as do both
packaging gates. The owner smoke-tested the path-remapped candidate
`20261028-205750-323` and confirmed that enabling Low latency removed the
visible line lag ([tablet record](XP_PEN_VALIDATION.md)). These changes do not
alter painting, documents, storage or recovery, so the evidence below carries
over; no new resource study or timed session was run on the final binary.

## Accepted evidence

| Area | Decision and evidence |
| --- | --- |
| G1 artwork safety | Accept the existing exact native/legacy/mask round trips, ordered save/load/undo coverage, atomic failure/cancellation/interruption checks and native recovery evidence. Current Full verification and maximum fixtures pass. See [current safety map](benchmarks/release-safety-coverage-20261027.json), [native recovery](benchmarks/large-busy-native-recovery-20261019.json), [Windows failure/retry](benchmarks/native-save-sharing-failure-20261019.json) and [workflow audit](research/WORKFLOW_RELEASE_AUDIT_2026-10-19.md). No known artwork-loss defect is being waived. |
| G2 resources | Accept the passed three-repeat/two-hour study, unchanged heap fix and subsequent focused performance results. Current viewport/timelapse/dab/fill/document sources match those later records. [Evidence reconciliation](benchmarks/release-evidence-reuse-20261028.json). No repeat study is required without a concrete new regression question. |
| G3 artist workflow | Accept the owner's repeated successful Artist 12 use and working Explorer previews, together with automated illustration/portability checks and reviewed UI captures. [Tablet record](XP_PEN_VALIDATION.md), [workflow coverage](research/WORKFLOW_RELEASE_AUDIT_2026-10-19.md). The independent first-use exercise remains after publication. |
| G4 release preparation | Accept local binary/source integrity, both deliverables' offline rebuilds, static runtime imports, reviewed unchanged notices, hosted verification steps and the earlier laptop install/upgrade/recovery/removal results. [Candidate proof](benchmarks/release-candidate-20261027.json), [laptop record](research/CLEANPC_LAPTOP_2026-10-22.md). The final public-package checks record equivalence to those accepted binary/source inputs. |

The historical studies retain their actual build identities. Acceptance uses
reviewed carry-over and source comparisons; it does not invent a new run on the
current binary, a timed physical session, or measurements the tools did not make.

## Support scope and known limits

- First release: portable Windows x64, with tested Windows 11 desktop/laptop
  configurations in [release notes](RELEASE_NOTES.md). A universal minimum OS,
  GPU or RAM specification is not established by these machines.
- Up to 4096 pixels per axis, 32 raster layers, 64 entries, folder depth six and
  a shared 512 MiB raster/mask payload budget. These are simultaneous budgets,
  not a promise of 32 densely painted 4K layers.
- Ordinary native v4/v5 projects and flattened PNG exchange are the primary
  workflow. CSP import is restricted; unsupported features are rejected.
  Indexed storage and Linux remain experimental. See [format support](FORMAT_SUPPORT.md).
- Exact physical pen latency, full GPU commitment, exhaustive OS/device failure,
  accessibility and multi-monitor matrices remain unmeasured or incomplete.
  They are disclosed engineering follow-ups, not advertised guarantees or
  instructions to repeat the owner's completed tests. A demonstrated artwork-loss
  or blocking regression reopens the relevant acceptance area.
- Atomic replacement/interruption evidence is not power-loss certification.
  Recovery preserves completed snapshots, not every uncommitted pen event.
  [Quick start](QUICK_START.md) explains saving and recovery limits.
- Preview enablement works with the DLL. Move/removal edge cases were not
  separately reported; keep the DLL beside the app, re-enable from a moved
  folder, and remove registration before deleting the app folder.
- Unsigned initial distribution is the owner's accepted choice. No trusted
  publisher signature or absence of SmartScreen prompts is promised.

These dispositions close release preparation; they do not label unperformed
tests as passes. The old exhaustive measurement matrix remains useful research
history, rather than a continually expanding first-release gate.

## Publication handoff

The prepared download set contains the Windows ZIP, matching vendored-source ZIP,
both checksum files and this decision. The handoff manifest records names, sizes,
hashes, support scope and publication state. Source must remain available with
the corresponding binary through the intended own-site/store channel.

**Not published by this task:** website/domain, purchase setup, upload and public
re-download are launch actions. No live download URL or payment configuration is
invented. The site must present the actual version, Windows x64/unsigned status,
matching-source download and known limits before launch.

GitHub Actions verification passed but artifact uploads hit storage quota. Local
release files are independently verified and do not depend on those uploads.
The 45 old CI artifacts were backed up and verified locally; on 2026-10-28 the
owner judged those superseded builds unneeded and the local copies were deleted.
The hosted copies remain with the archived development history.
