# Repository and release assessment — 2026-10-27

## Recommendation

Dudel has enough core functionality for a useful first Windows illustration
release. Freeze feature scope, consolidate the current source, close the concrete
integration and presentation gaps below, and accept one identified candidate.
Do not expand the engine or pursue general CSP parity as a prerequisite.

This is a planning review, not release acceptance. It supplements the existing
G1–G4 gates in DELIVERY_PLAN.md; it does not create another independent checklist.
The release should remain 0.1, with a controlled public beta first if useful for
collecting wider feedback. A beta label does not waive artwork-safety requirements.

## Review boundary and fresh evidence

Reviewed the working tree based on `a3170d045df6bdb43288aaeb5a2f4bd464a4e4b7`,
the product/research/release records, selected implementation and packaging paths,
and the saved October 22 brush and New Canvas captures. No fresh native UI or
physical tablet session was performed. This was not a line-by-line audit of all
application code or a new CSP runtime comparison.

Earlier comparison notes about established drawing apps are kept privately.
The comparison below summarizes their recommendations, checked against
implementation/progress records and current source.

Fresh checks on this working tree:

| Check | Result |
| --- | --- |
| `cargo test --locked --quiet` | 916 passed, zero failed, eight explicitly ignored |
| Root formatting and default all-target Clippy with warnings denied | Pass |
| Python research/tooling suite | 36 passed |
| Thumbnail workspace tests | Two passed |
| Thumbnail workspace formatting and all-target Clippy with warnings denied | Pass |
| `git diff --check` | Pass; Git also reports LF/CRLF conversion notices |
| Thumbnail lockfile registry packages versus application lockfile | All thumbnail package/version/source tuples are present in the application lockfile |

The thumbnail test link emitted LNK4104 export warnings; tests still passed.
All-feature suites, ignored maximum fixtures, a fresh release build, hosted CI,
archive rebuild, long resource studies and physical acceptance were not repeated
for this review. Earlier results retain their original build scope.

## What went well

1. **Artwork safety became architecture.** Atomic saves/recovery, stale worker
   rejection, transactional edits, coherent undo and cross-tab ownership checks
   address the failures that would actually cost an artist work. Existing audits
   found and fixed concrete issues, including cross-tab Save as ownership.
2. **Research translated into useful engineering.** Native pen history,
   timestamped samples, brush dynamics, sparse copy-on-write tiles, compressed
   history and cooperative background work substantially improve the original
   prototype. We did not need an exact copy of CSP's unknown algorithms.
3. **Optimization has unusually good evidence.** Reports retain fixtures,
   hashes, before/after timings, output equivalence and caveats. Recent work
   targets repeated tile lookup, upload preparation, reduced-resolution patches,
   timelapse buffers and fill traversal rather than speculative rewrites.
4. **Real testing changed the product.** Laptop testing exposed the missing
   Visual C++ runtime and workspace problems. Static CRT packaging and tool/status
   fixes directly respond to those findings. This is stronger than another
   synthetic test count by itself.
5. **Distribution is taken seriously.** Locked dependencies, pinned compiler,
   source/input identities, matching-source archives, offline rebuild tooling,
   notices and portable packaging already provide a substantial release foundation.
6. **The product now has a coherent illustration workflow.** Brushes, masks,
   selections, fills, transforms, tabs, references, PNG exchange and recovery are
   present. Timelapse, portable themes/packs and mirrored feature transforms add
   useful personality beyond a minimal drawing prototype.

## Repository health and concrete findings

### P1 — Consolidate the actual release source

At review start, 44 tracked files were modified, with 3,374 additions and 463
deletions, excluding untracked content. New content includes mirrored transforms,
project previews, the Explorer DLL and performance evidence. These are meaningful
changes across rendering, filling, storage and packaging, not a small final polish.

Remote master still identifies `a3170d0`. Remote linux identifies `d5547f3` and is
11 commits behind master in the locally available ancestry; the local linux branch
is older again. Resolve the intended branch policy and verify the final shared
source rather than treating historical Linux CI as current coverage.

Review and checkpoint the pending work in coherent changes, then freeze an RC
commit. Source hashing already makes dirty builds traceable, but a clean tagged
release is simpler to reproduce, support and compare.

### P1 — Bring the Explorer DLL into the release contract

Package.ps1 builds/copies the DLL and checks its PE imports. However:

- Verify.ps1 does not test/lint/format its independent Cargo workspace. Hosted
  Windows packaging compiles it, but that does not execute its tests.
- check_source_archive.py rebuilds only `--bin dudel`.
- BUILD.json records the application executable hash/imports, but has no matching
  DLL artifact hash/import identity.
- Generated BUILDING.md describes rebuilding the application only.
- package_source.py assumes thumbnail dependencies remain a subset of the app's.
  That assumption is true today, but should be checked or replaced with combined
  dependency collection when either lockfile changes.

Extend these existing paths to cover both deliverables. Include real registration,
thumbnail display, upgrade/move and unregistration behavior in the final portable
package smoke test. Registration points to a DLL path, so moving or removing a
portable installation needs a clear user-facing procedure.

### P1 — Bound preview decoding before shipping the shell integration

`shell/thumbnails/src/lib.rs:64` calls `image::load_from_memory` before resizing.
The writer emits previews no larger than 256 pixels per side, but the reader does
not enforce that contract before decoding. Its roughly 64 KiB encoded-header cap
does not itself limit decoded dimensions. The current image dependency's default
reader has no dimension cap and a 512 MiB allocation allowance.

Use explicit preview dimensions and a small decode allocation budget before full
decode, and cover oversized/truncated preview rejection. This is a source-level
resource-bounding finding; this review did not reproduce a crash or claim an
exploit. Correct the module comment promising damaged projects cannot slow or
crash the host: bounded work is the guarantee the implementation can establish.

### P1 — Make support and acceptance documentation agree

DELIVERY_PLAN.md lines 32–35 simultaneously permit 16,777,216 pixels and say a
4096 × 4096 canvas is unsupported. The adjacent statement about 32 fully populated
maximum-area rasters fitting a 512 MiB mask-aware budget is also stale: one dense
4096-square RGBA raster is 64 MiB, so eight consume that payload budget before
masks. Layer-count limits are not a promise of that many dense maximum-size layers.

README still references a Windows runtime prerequisite, while release notes
describe static linking. RELEASE_STATUS has an October 21 update heading with
later entries and does not describe the newest local features. The old CSP review
also preserves the former canvas limit. Historical reports can remain historical;
current support pages must agree.

Keep RELEASE_STATUS as the concise index: final commit/artifact identities,
supported envelope, open issues, and accepted/pending evidence per gate. Preserve
detailed dated reports as evidence, rather than continually appending competing
"current" narratives. There are already 20 root Markdown documents and 54 research
Markdown documents before this review.

### P2 — Improve maintainability incrementally after the release

The repository has 146 Rust files, but substantial orchestration still lives in
large modules: canvas.rs is about 3,063 lines; main.rs 2,400. canvas_textures.rs is
4,673 lines, of which about 2,213 precede its test module, so total file size alone
overstates production complexity.

Good next boundaries are display residency/publication, document session state,
and shared command/brush-property metadata. tabs.rs already generates parked-session
exchange through a macro; retain that improvement. Active project state still
lives on Dudel separately from parked sessions. Move toward one owned session
model gradually, using existing tab isolation regressions. Avoid a broad rewrite
during stabilization.

## Comparison with the original research

| Original recommendation | Current outcome | What remains |
| --- | --- | --- |
| Own WM_POINTER backend and per-sample pressure/sensors | Substantially implemented, with history, timestamps and fallback | Build-identified hardware acceptance; WinTab only if a real driver need appears |
| Screen-space stabilization, velocity and richer dynamics | Implemented with bounded typed dynamics, Smooth/Rope and speed handling | Global calibration and a universal property model; exact CSP behavior remains unknown |
| Better softness, buffered flow/opacity, spacing compensation and taper | Implemented with compatibility paths and equivalence coverage | Polish physical feel and heavy-brush latency from measurements |
| Sparse storage and compressed idle data | Sparse COW tiles and compressed history implemented | General cold-document compression, paging and aggregate memory-pressure policy |
| Hold-to-switch tool shortcuts | Implemented | Customizable bindings and easier command discovery |
| Indexed records, mipmaps and thumbnails | Experimental indexed storage; reduced display residency; new native preview/Explorer work | Default-format promotion and persisted per-layer pyramids are separate future decisions |
| CSP raster migration | Restricted importer implemented, rejecting unsupported content | Ordinary folders, clipping, masks and broader blend compatibility need controlled fixtures |
| Linear-light fill distance | Not adopted; current fill semantics differ | No evidence that CSP's bucket uses it; compare visible output before changing behavior |
| Integer display compositing | Targeted optimizations, not a wholesale integer compositor | Only pursue if profiling establishes a material end-to-end gain |
| ID-based property registry | Typed settings and shared UI helpers, but no universal registry | Gradual consolidation to reduce duplicate UI/serialization/shortcut logic |

We did well at applying the research to Dudel's most immediate problems. We have
not established feature, rendering, speed or reliability parity with CSP. Several
original effort estimates were optimistic: compressed active documents, indexed
storage and interop carry substantial lifecycle and validation work beyond their
core algorithms. Their partial/deferred status is not a reason to delay a bounded
illustration release.

## Polish with the highest artist value

- **Clarify the three brush surfaces.** Use the top bar for frequent drawing
  adjustments, properties for contextual tuning, and Studio for preset authoring.
  The saved capture shows a coherent dark workspace, but competing brush controls
  and dense side panels still consume attention. This is an assessment of the
  saved October 22 captures, not a current physical usability result.
- **Make settings easier to find.** The laptop owner requested a project-folder
  setting that already existed. Improve labels/help entry points and the route
  from the relevant workflow. A searchable command interface is a useful later
  feature; a simple contextual link may solve this release's problem.
- **Make save state obvious.** Keep project save, recovery backup and pending
  background work visibly distinct. The new backup cue is the right direction.
- **Use real artwork in acceptance.** One normal ink/fill/masked shading project,
  one dense 4K project, and several tabs with references expose different costs.
  Exercise timelapse on/off and transform previews without adding an exhaustive
  new permutation matrix.
- **Prepare the public handoff.** Include a small redistributable example project,
  a short illustrated quick start, accurate known limitations and an easy way to
  collect build/system diagnostics without including artwork automatically.

## Proposed release sequence

| Stage | Work | Exit condition |
| --- | --- | --- |
| 1. Scope and checkpoint | Freeze Windows 0.1 features; review current diff; decide final supported envelope; reconcile branch and status records | One reviewed RC commit and one authoritative open-issue list |
| 2. Bounded polish | Close thumbnail integration/decode bounds, contradictory docs, and demonstrated workflow defects | No known artwork-loss/blocking issues; focused checks pass |
| 3. Candidate verification | Run Full checks and dedicated maximum fixtures once at the coherent checkpoint; include DLL workspace; package and offline-rebuild both deliverables | Identified EXE/DLL/source archives, checksums, notices and matching-build results |
| 4. Affected acceptance | Retest render residency, timelapse/fill/transform interaction and new preview save costs; short build-identified Artist 12 pass; portable laptop/Explorer smoke | G1–G4 rows have explicit outcomes or an owner-approved scope/target decision |
| 5. Public handoff | Version/tag, concise release notes, downloads/source/checksums, tested-system matrix, support route and known issues | Re-download the public artifact and confirm identity/startup; begin beta feedback |

Do not restart every historical investigation. The October 21 packaged DX12
study passed its standard repeats and two-hour maximum memory workload. Static CRT
and the laptop round-two pass are also real evidence. Reuse unchanged results.
Rendering/timelapse/storage changes since those builds do require an impact-based
retest before claiming equivalent resource behavior. Prefer the normal unprofiled
application for user-facing latency claims.

Existing owner decisions remain in force: unsigned first distribution is allowed,
and independent ten-minute first-use testing was moved to the first public build.
Do not reintroduce signing or that first-use test as new pre-publication blockers.
If the existing numeric performance targets or long physical-session requirements
are revised, record the decision explicitly; do not mark unperformed checks passed.

## What to add after 0.1

1. **0.1.x: feedback and usability.** Address actual artist friction, preset
   quality, confusing states and regressions. Add diagnostic export if the current
   reporting workflow proves cumbersome.
2. **Next workflow milestone: shortcuts and calibration.** Customizable commands,
   quick access and global pressure calibration offer broad tablet value without
   requiring a new document format. Build these on a modest command/property model.
3. **Choose one larger capability from user work.** Smudge/blending for painting,
   or layered interchange for moving between apps. PSD/OpenRaster or broader CSP
   support needs its own compatibility scope and fixtures; do not promise them all.
4. **Scale only when needed.** Use multi-document/dense-art measurements to choose
   cold-tile compression, aggregate cache budgets or persisted reduced-resolution
   data. Raise canvas/layer limits only after their full save/recovery/undo envelope
   is validated.

ICC/print workflows, larger documents, broad interop and full Linux distribution
acceptance remain separate milestones. For this release, the most valuable outcome
is an understandable, dependable drawing app with an honest support promise.
