# Windows 0.3 release decision — 2026-11-10

> This is the maintainers' acceptance record for the packaged release. The
> measurement files and audits it names were moved to the project's archive on
> 2026-11-09; their names are kept so each claim stays traceable.

**Release preparation for 0.3.0 is accepted as a pre-release for outside
testers,** at the owner's request on 2026-11-10 to make a pre-release. The 0.2.0
decision that follows remains the record for the evidence carried over.

**Application source:** commit `f6e5b5a`; later commits change documents only.
[RELEASE_ACCEPTANCE.json](RELEASE_ACCEPTANCE.json) pins the binaries, EXE
`6E92CBFC...CE234D4B` and DLL `C21A954B...31CFA0CD`, and release packaging refuses
different bytes.

**What changed since 0.2.0** (see [release notes](../../RELEASE_NOTES.md)): a
welcome window, problem reports with a message when Dudel has to close, Help
menu links, Color and Navigator as tabs, a tidier layer toolbar with the mask
row only for masked layers, one dialog style, palette color extraction and
color ideas, checkbox and radio contrast in dark themes, and the documentation
moved under `docs/`. Project and file formats are unchanged.

**Evidence on this build:**

- Local `scripts/Verify.ps1 -Mode Full` on `f6e5b5a`: default and all-feature
  test runs (1,327 tests in the all-feature run, 17 ignored), the DLL tests,
  formatting, both Clippy configurations, both release builds, the 42 Python
  tests and the public-tree check.
- Hosted Windows and Linux CI passed on `b108c3c`, whose application code is
  the same as `f6e5b5a` apart from the version number.
- Candidate `dudel-0.3.0-candidate-20261110-175240-000`: static runtime (no
  Visual C++ imports), no private paths or terms, and 0 public-tree problems in
  the package and its source ZIP.
- The packaged executable started, showed version 0.3.0 and captured its
  workspace and welcome window.
- The Linux tarball was cross-built with `cargo-zigbuild` against glibc 2.31
  (the binary needs at most 2.30). It was not started; Linux stays experimental.

**Not run on this build:** the private CSP corpus comparison (import code is
unchanged since 0.2.0), the resource and two-hour studies, an owner pen pass
over the new panels and dialogs, and a launch of the Linux build. The painting,
storage, recovery and import evidence carries over from 0.2.0 where those areas
are unchanged; the new interface code is covered by the automated workflow
tests above. Outside testers are the next check.

**Known limits** are listed under "Known limits" in the release notes.

# Windows 0.2 release decision — 2026-11-09

**Release preparation for 0.2.0 is accepted for the Windows scope below,** at
the owner's request to start the 0.2.0 release on 2026-11-09. The 0.1.0
decision that follows remains the record for the evidence carried over.

**Application source:** commit `531da84`; later commits change documents only.
[RELEASE_ACCEPTANCE.json](RELEASE_ACCEPTANCE.json) pins the binaries, EXE
`86734496...37164C3A` and DLL `CA9E3C10...E4204EC8`, and release packaging
refuses different bytes.

**What changed since 0.1.0** (see [release notes](../../RELEASE_NOTES.md)): editable
text layers saved as project format v8, the Export image window with Export
again, the pen quick menu (the Windows side button now arrives as a right-click),
one-click Add mask, the blend-mode wheel and image-paste fixes, and the Linux
packaging scripts from the `linux` branch.

**Evidence on this build:**

- Local `scripts/Verify.ps1 -Mode Full` on `43d114e` (same application source
  as `531da84`, whose two extra commits change packaging scripts and docs):
  1,312 default / 1,321 all-feature tests (17 ignored each), three DLL tests,
  formatting, both Clippy configurations and both release builds. The 42 Python
  tests and the public-tree check were re-run on `531da84`.
- All 69 files of the owner's private CSP corpus import exactly as with the
  baseline build, including the three with text layers.
- Candidate `dudel-0.2.0-candidate-20261109-072207-335`: static runtime (no
  Visual C++ imports), no private paths or terms, notices passed with 0 problems
  and binary/source parity. Against the reviewed 0.1.0 package, 18 notice files
  changed, all new MIT, Apache-2.0 or Unicode terms for text shaping and
  bidirectional text (rustybuzz, unicode-bidi and related crates).
- Offline rebuild of the source ZIP passed (`check_source_archive.py`).
- The packaged executable started and captured its workspace.
- The owner's checks during development: they used per-selection text styles,
  fit to box and the export window on 2026-11-08 and reported them working, and
  confirmed the quick menu with the pen's side button on 2026-11-09 after the
  Windows fix.

**Not run on this build:** hosted CI (planned once on the pushed candidate),
the resource and two-hour studies, the laptop install, a fresh owner pen pass
over every text item in TEXT_TOOL_PLAN.md, and Linux
packaging, which is built separately on Linux and stays experimental. The
painting, storage and recovery evidence carries over from 0.1.0 where those
areas are unchanged; the new text and export code is covered by the automated
tests above.

**Known limits** are listed under "Limits in 0.2.0" in the release notes.

# Windows 0.1 release decision — 2026-10-28

**Release preparation is accepted for the Windows scope below.** The owner asked
to adjust and complete the release goal using the existing testing and confirmed
Artist 12 and Explorer results. This decision replaces the earlier blanket
retest requests and provisional engineering checklist as the current acceptance
record. It does not claim every proposed diagnostic was performed.

The accepted application source is the `v0.1.0` tag of the public repository.
[RELEASE_ACCEPTANCE.json](RELEASE_ACCEPTANCE.json) pins both binary hashes, and
release packaging refuses different executable or DLL bytes.

**Rebuilt again, 2026-11-04.** The 2026-10-31 build was never published either.
The owner kept the version at 0.1.0 and asked for this rebuild, which adds vector
layers and line editing, SVG export, command search, custom shortcuts, layer
filtering, multi-layer batches, editing feedback and the stabilization fixes (see
[release notes](../../RELEASE_NOTES.md)). The current binaries are EXE `D53A9ED9...725ECD30`
and DLL `A0FCCA48...EE41DEC1`, built from `bffcbea`; later commits change
documents only. Evidence on this build: local `scripts/Verify.ps1 -Mode Full`
(1,137 default / 1,146 all-feature tests, three DLL tests, 42 Python tests, Clippy,
release builds), the packaging gates and a packaged startup. All 16 of the owner's
existing projects (formats v4 and v6) opened and composited. A real SVG export
parsed as well-formed XML. The owner's pen pass on 2026-11-04 passed, and a second on 2026-11-05 passed
selection limits, fill to line centres, line handles and SVG export; it names no
particular vector editor for the SVG. Hosted Windows CI passed on `e20c76b` (run
41948291941), including the maximum project fixtures and candidate packaging. Not
run on this build: hosted Linux CI, the resource and two-hour studies, offline
source rebuilds and the laptop install. The tag
`v0.1.0` names the final release-records commit. The paragraphs below describe
earlier builds.

**Rebuilt, 2026-10-31.** The first 0.1.0 build was prepared but never
published; the owner held it back until CSP and PSD import were solid, then
asked for 0.1.0 to be rebuilt from current source. The rebuild adds wider
CSP and PSD import, PSD export, layer effects, adjustment layers, canvases up
to 16,384 px per side and 100 MP, up to 1,000 layers, and many-layer speed
work (see [release notes](../../RELEASE_NOTES.md)). Unlike the 2026-10-28 changes,
these touch documents, compositing and file formats. Evidence on this build:
local `scripts/Verify.ps1 -Mode Full` (1,030 default / 1,039 all-feature
tests, Clippy, Python checks, release builds), both packaging gates (static
runtime, no private paths or terms), a packaged PSD export round trip of a
146-layer model that reopens pixel-exact, a packaged startup and clean close,
and the owner's checks that exports open in Live2D Cubism, Affinity and
Photopea. Not run on this build: hosted CI (refused for account billing on
2026-10-31), the resource and two-hour studies, offline source rebuilds and
the laptop install. The evidence table below describes the earlier build and
carries over only where those areas are unchanged.

**Final changes, 2026-10-28.** The evidence below was gathered on the earlier
accepted application source (`0d8e429`, private development history). The final
build differs from it in three reviewed ways: binaries are built with path
remapping so they no longer embed the build machine's folders; the brush toolbar
keeps a fixed layout in Smooth and Rope mode (follow options moved into a menu);
and low latency drawing is on by default. Full local verification passes on the
final source (918 Rust tests, Python checks, clippy, privacy check), as do both
packaging gates. The owner smoke-tested the path-remapped candidate
`20261028-205750-323` and confirmed that enabling Low latency removed the
visible line lag (tablet record). These changes do not
alter painting, documents, storage or recovery, so the evidence below carries
over; no new resource study or timed session was run on the final binary.

## Accepted evidence

| Area | Decision and evidence |
| --- | --- |
| G1 artwork safety | Accept the existing exact native/legacy/mask round trips, ordered save/load/undo coverage, atomic failure/cancellation/interruption checks and native recovery evidence. Current Full verification and maximum fixtures pass. See current safety map, native recovery, Windows failure/retry and workflow audit. No known artwork-loss defect is being waived. |
| G2 resources | Accept the passed three-repeat/two-hour study, unchanged heap fix and subsequent focused performance results. Current viewport/timelapse/dab/fill/document sources match those later records. Evidence reconciliation. No repeat study is required without a concrete new regression question. |
| G3 artist workflow | Accept the owner's repeated successful Artist 12 use and working Explorer previews, together with automated illustration/portability checks and reviewed UI captures. Tablet record, workflow coverage. The independent first-use exercise remains after publication. |
| G4 release preparation | Accept local binary/source integrity, both deliverables' offline rebuilds, static runtime imports, reviewed unchanged notices, hosted verification steps and the earlier laptop install/upgrade/recovery/removal results. Candidate proof, laptop record. The final public-package checks record equivalence to those accepted binary/source inputs. |

The historical studies retain their actual build identities. Acceptance uses
reviewed carry-over and source comparisons; it does not invent a new run on the
current binary, a timed physical session, or measurements the tools did not make.

## Support scope and known limits

- First release: portable Windows x64, with tested Windows 11 desktop/laptop
  configurations in [release notes](../../RELEASE_NOTES.md). A universal minimum OS,
  GPU or RAM specification is not established by these machines.
- Up to 16,384 pixels per side and 100 megapixels, 1,000 raster layers, 2,000
  entries, folder depth ten and a shared 2 GiB raster/mask payload budget.
  These are simultaneous budgets, not a promise of 1,000 densely painted
  maximum-size layers.
- Ordinary native v4/v5 projects, PNG exchange and PSD export are the primary
  workflow. CSP and PSD import cover the subsets in format support; skipped
  layers and approximations are listed in Import details.
  Indexed storage and Linux remain experimental. See [format support](../FORMAT_SUPPORT.md).
- Exact physical pen latency, full GPU commitment, exhaustive OS/device failure,
  accessibility and multi-monitor matrices remain unmeasured or incomplete.
  They are disclosed engineering follow-ups, not advertised guarantees or
  instructions to repeat the owner's completed tests. A demonstrated artwork-loss
  or blocking regression reopens the relevant acceptance area.
- Atomic replacement/interruption evidence is not power-loss certification.
  Recovery preserves completed snapshots, not every uncommitted pen event.
  [Quick start](../QUICK_START.md) explains saving and recovery limits.
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
