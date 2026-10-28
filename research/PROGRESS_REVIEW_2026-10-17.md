# Progress and branch synchronization review

## Scope and branch state

The default branch is `master`, not `main`. At review start, `master` was
`c34542f`; fetching advanced `origin/linux` to `5bd1db5`. The default branch
was an ancestor of the Linux branch: Linux already contained all Windows-side
work. A fast-forward integrates the port without dropping commits or resolving
conflicts. Both branches should point to the same review commit after validation.
Future platform changes should stay behind platform configuration in this shared
source tree; avoid maintaining separate copies of drawing and persistence code.

## Progress since the earlier engineering candidate

- Replaced the string-pull/catch-up stabilizer with time-based smoothing.
  Earlier artist approval applies to the removed design; current retest is open.
- Added canvas reference images with placement, scaling, rotation, opacity,
  locking and project/recovery persistence.
- Expanded blending from 12 to 26 modes, including component modes, and added
  wheel/arrow navigation for the selector.
- Added transparency checkerboard, flush sidebars, left-side color controls,
  and Pointer-tool reference management.
- Pinned Rust 1.97.1 and aligned the non-macOS command modifier in workflow tests.
- Added native Wayland tablet pressure/tool packets and pen-driven widgets,
  plus a fix for hidden-window presentation stalls on Wayland.
- Retained earlier save cancellation/publication safety, startup recovery,
  bounded display workers, resource diagnostics and GPL/matching-source packaging.

Implemented features and historical measurements do not establish acceptance
of the newly combined build. Linux baseline evidence is recorded in LINUX_PORT.md;
local Windows verification is recorded separately in RELEASE_CHECKLIST.md.

## Verification of the combined code

`scripts/Verify.ps1 -ReleaseTargetDirectory target/qa/job-peak-target-20261017`
passed on Windows with Rust 1.97.1: 802 default tests, 811 all-feature tests,
28 Python checks, formatting, both warnings-denied Clippy configurations and
release compilation. Two large-memory tests remain ignored in each normal suite;
they were not rerun for this review. Log:
`target/qa/branch-sync-verification-20261017.log`.

The available Ubuntu WSL environment lacks Cargo and Wayland development tools,
so this review does not independently verify Linux compilation or runtime behavior.
No new release package, resource soak or physical-tablet acceptance was performed.

## Remaining work, in order

1. Freeze a combined candidate and run the current verification/package pipeline.
   Verify its binary/source hashes and perform a fresh offline source rebuild.
   Confirm hosted Windows checks; add Linux CI and a reproducible Linux verifier.
2. Close sustained performance gates: three standard 30-round repeats and a
   complete two-hour maximum/mixed workload on the current candidate. Record
   matching-idle growth, lifetime peaks, completed save/open duration, GPU ownership
   and drawing frame/queue timing. Historical standard memory repeats passed;
   the earlier maximum run was incomplete and does not pass this gate.
3. Repeat artwork safety on the new reference/blend state: exact save/reopen,
   recovery while busy/on large documents, later edits during saves, cancellation,
   replacement, storage failures and supported-format migration. Exercise actual
   file dialogs, clipboard and exports on disposable artwork.
4. Complete an Artist 12 sketch/ink/color/shade/save/reopen/export session and
   retest the new stabilizer. Record exact build, active driver, scaling and
   duration; check pen lift, fast curves, focus changes and pan/zoom. Complete
   DPI/short-window, keyboard/accessibility and clean-profile brush portability.
5. Close Linux-specific gaps: physical Wayland pressure/tilt/buttons/hover cursor,
   fractional scaling/UI zoom, image paste, portals, paths and multi-instance
   settings locks. X11 pressure is not implemented. Add desktop/icon integration,
   a release archive and Linux build/run documentation before promoting support.
6. Finish distribution acceptance: outstanding third-party notice reviews,
   clean Windows install/start/upgrade/artwork preservation, supported hardware/OS
   matrix, signing/channel decision, release notes and final artist acceptance.

Keep feature expansion paused during candidate validation. CSP remains a restricted
import subset; PSD parity, animation and larger document limits are outside this
release scope. Selling official GPL builds and supplying free buildable matching
source remains the distribution model.
