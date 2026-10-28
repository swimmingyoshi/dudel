# Next milestone: a dependable everyday drawing workflow

Completion disposition, 2026-10-28: the bounded workflow fixes, candidate
verification and owner feedback are recorded in [RELEASE_DECISION.md](RELEASE_DECISION.md).
That decision supersedes the provisional gate language in this historical plan.

Created 2026-10-19. This is the next bounded work package under
[DELIVERY_PLAN.md](DELIVERY_PLAN.md), not a replacement for its release gates.

## Goal

Prepare a candidate on which an artist can create an illustration, switch between
projects, customize the workspace, save/reopen and export without losing edits or
getting stuck in the UI. Close demonstrated workflow defects and provide a short,
build-identified acceptance handoff. The unattended DX12 study is a parallel gate,
not a prerequisite for beginning this work.

Completion means the five deliverables below have recorded outcomes. An audit
may find an interaction already correct: record that result and reuse existing
coverage instead of adding a redundant fix or test. External acceptance remains
explicitly pending until someone performs it.

## Work order and deliverables

| Order | Work package | Concrete outcome | Completion evidence |
| --- | --- | --- | --- |
| 1 | Document and tab safety | Audit new/open/close/quit, tab-limit replacement, unsaved prompts, save failures, in-flight jobs and returning to an inactive project. Fix any path that loses edits, applies a result to the wrong document or leaves a misleading dirty/path state. | Scenario-to-existing-coverage map; focused regressions only for uncovered defects; disposable-project exercise and recorded limitations. |
| 2 | Theme and brush portability | Audit repeated theme edits, color picker interaction, Apply/Save/Cancel, restart and theme import/export; verify independent brush/eraser state and brush-pack transfer into clean preferences. | Exact starting profile, actions and resulting files recorded; settings persist and editable artwork remains unchanged. Reuse existing deterministic brush-pack tests. |
| 3 | Workspace polish | Walk the sketch/ink/fill/mask/transform/export path in short and normal windows. Check popup stability, status clipping, focus/Escape/text shortcuts, layer/reference selection and symmetry feedback. Fix the highest-impact findings first. | A concise defect list with reproduction, severity and resolution; reviewed captures for changed layouts. Include 100/150/200% layouts, distinguishing simulated UI scale from actual OS scaling. |
| 4 | Targeted UI efficiency | Inspect repaint scheduling, preview regeneration and inactive-tab/reference work encountered during the workflow audit. Measure before changing a suspected hotspot; fix confirmed avoidable work. | Before/after observations on the same build setup and fixture, with visual/output equivalence. A documented no-change finding is valid; no speculative engine refactor. |
| 5 | Candidate handoff | Prepare a fresh candidate only after the fixes settle, update the quick start/known issues and provide a compact manual acceptance sheet covering the audited workflow. Keep master and linux aligned after validation. | Build/source identity, appropriate passing checks, reviewed changed UI and branch comparison. Each physical, Linux-runtime or clean-machine check has an honest pending/pass/fail status. |

Developer owns source review, fixes, focused verification and the handoff.
The artist owns physical Artist 12 acceptance. A release tester owns independent
first-use and clean-machine checks. Do not mark the milestone release-accepted
using developer checks alone.

## Efficient validation

- Start by mapping existing tests and native evidence to the scenario. Do not
  repeat completed storage, codec or licensing audits for unchanged inputs.
- Run affected tests and formatting for each code change; run the appropriate
  lint/build checks at a coherent package boundary. Do not run full suites for
  prose-only edits.
- Use existing Windows/Linux CI for full suites and dedicated maximum fixtures
  once a coherent source checkpoint is ready. Re-run failures or affected checks
  after fixes, rather than repeating green checks without a reason.
- Profile representative drawing, layer/reference and tab operations. Keep
  diagnostic overhead separate from user-facing performance measurements.
- Use disposable profiles/projects. Preserve the user's artwork and open editor.

## Scope boundaries

Prioritize artwork-loss defects, blocked interactions, misleading state, then
layout and measured performance. Keep existing document limits and formats.
Do not add unrelated tools, expand plugin APIs, redesign storage or raise limits
during this milestone. Capture lower-priority ideas in a follow-up backlog.

## Tonight's study and the release boundary

Outcome, 2026-10-21:
- The frozen candidate below passed its standard repeats but failed the two-hour
  maximum session on heap retention.
- After the heap-compaction fix, `dudel-0.1.0-candidate-20261021-190309-093` passed
  the full study ([summary](benchmarks/heap-compaction-study-20261021.json)).
- The remaining G1-G4 gates below still apply.

The text below is the original plan. Do not launch the study automatically while the
PC is in use. The frozen study candidate is
`dudel-0.1.0-candidate-20261019-111249-426` (EXE SHA-256
`8D2C768E9179B61A9D05FD884E791C763465C6A3C8FB46E763399670ABE9279C`).
Leave that package immutable. Attribute its results to that build and hardware;
do not silently transfer a pass to a newer candidate. Review application changes
and determine the affected resource checks needed on the final candidate.

Release still requires the existing G1-G4 gates: remaining artwork safety cases,
sustained performance, current-build Artist 12 acceptance, independent illustration
and first-use exercises, Linux acceptance for advertised support, clean Windows
distribution checks and final package/notice/channel review. This milestone moves
those gates forward; it does not declare the product ready to sell.

## Tracking

First checkpoint: [workflow source audit](research/WORKFLOW_RELEASE_AUDIT_2026-10-19.md).
Document/tab review found and fixed cross-tab Save as overwrites. Two regressions
cover rejection/preservation and ordinary save/new-copy compatibility; 68 focused
tests and default all-target Clippy pass. Existing theme/brush/ZIP and selected
workflow coverage were reused. Independent/native-dialog acceptance, runtime UI
profiling, candidate packaging and the remaining release gates are still pending.

Second checkpoint: [brush preview cache review](research/BRUSH_PREVIEW_CACHE_2026-10-19.md)
removes repeated dynamics JSON serialization from cache checks. An optimized
component microbenchmark records 6.08x / 9.58x speedups for default/dense settings;
this is not a frame-time or physical-latency claim. A cache regression covers
unchanged settings and nested-curve edits. The audience preview's Workspace
background now follows the customized canvas surround. Final checks and source
checkpoint identity are recorded with the implementation outcome.

Third checkpoint: [complete workflow follow-up](research/WORKFLOW_RELEASE_AUDIT_2026-10-19.md)
fixes global symmetry and snapping leaking across project tabs. A regression
reproduced the defect before the fix; all 54 existing workflow/tab/guide checks
and default all-target Clippy pass afterward. The preceding `fcfcca1` Windows and
Linux CI both completed successfully, including Windows maximum fixtures and
packaging/upload.
New-source release compilation and three reviewed native DX12 captures also pass
([evidence](benchmarks/tab-guide-isolation-20261019.json)); full hosted acceptance
is tracked separately. Independent
artist, actual OS dialogs, DPI/tablet and clean-machine acceptance remain open.

Candidate handoff: `dudel-0.1.0-candidate-20261019-210656-046`, clean revision
`6ba76c3` with application source `bc3515f`. Both platforms' full CI passes for
those application inputs. [Package proof](benchmarks/workflow-candidate-20261019.json)
records verified binary/source ZIPs, compiled inputs, notices, documentation links,
and a reviewed packaged DX12 capture. The [short checklist](CANDIDATE_TEST_CHECKLIST.md)
is bundled in both archives. This completes preparation of the developer handoff;
artist feedback and the existing sustained/clean-machine release gates are pending.

Release follow-up: [preview context and open-project accounting](research/RELEASE_IMPROVEMENTS_2026-10-19.md)
implements the bounded findings from the subsequent CSP/release review. See
[current release status](RELEASE_STATUS.md) for its validation and candidate
boundary; the earlier workflow and study packages remain immutable.
