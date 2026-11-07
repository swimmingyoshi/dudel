# Workflow improvements and 0.2.0 priorities

Updated 2026-11-07. The implemented workflow below records development changes
and their original evidence. The active next steps for 0.2.0 are the pen quick
menu and export workflow in [UPGRADE_PLAN.md](UPGRADE_PLAN.md). Do not interpret
historical test totals here as verification of the 0.2.0 release candidate.

This work covers the first three priorities from the workflow review and the
subsequent shortcut increment below. It changes
development source, not the previously accepted 0.1.0 release package.

## Implemented scope

- Editing feedback beside the canvas identifies artwork, vector, mask and
  reference targets, offers explicit unlock/show/new-raster/paint-mask actions,
  and exposes switching back to artwork and deselecting. Recovery goes through
  existing document actions and checks projected properties during queued work.
- Layer navigation searches names without case sensitivity, combines search
  words, filters types (including folders and references), finds entries inside
  collapsed folders, and provides a clear empty state. Filtering preserves the
  selected layer and stack. Reveal-active clears filters, opens parent folders
  and scrolls to the selected entry. Compact rows retain details in tooltips.
  Drag reordering is disabled while filtered because hidden rows obscure drop targets.
- Command search opens from Edit or Ctrl+K / Ctrl+Shift+P. Common file, layer,
  selection, view and tool actions include aliases and existing shortcut hints.
  Up/Down chooses, Enter executes, Escape closes, and unavailable actions explain
  why. Commands call the existing application actions. Search owns keyboard and
  canvas input while open.

Search text, type filters and row density are session UI state. They are not
stored in artwork files. The search list currently covers common actions, not
every extension or plugin command.

## Validation

Regression coverage exercises real egui hit testing at desktop and compact
window sizes, command execution/undo, keyboard selection, disabled and empty
results, Escape preserving canvas selection, collapsed-folder filtering,
reveal-active, compact rows, and explicit unlock/deselect actions. Controller
tests cover projected locks/visibility, undo, vector retention and mask targeting.

The final targeted run passed 64 workflow/controller tests. The full default
application suite then passed 1,109 tests (14 ignored), followed by strict
all-target Clippy with warnings denied. Formatting, 42 Python tests and the
public-tree scan also passed; the new modules and this plan were scanned
explicitly because they were not yet tracked. Local logs are
`artifacts/ux-workflow-tests.log`, `artifacts/ux-full-tests.log` and
`artifacts/ux-clippy.log`.

The development executable builds successfully. Native UI captures at 1280 × 800
were reviewed for command search and the locked-layer/selection/filter workflow;
the visual review replaced unsupported arrow glyphs in the search hint with
plain text. Captures are `artifacts/ux-command-search.png` and
`artifacts/ux-workflow.png`; the build log is `artifacts/ux-build.log`.

## Shortcut increment — 2026-11-04

The shared command catalog now drives common menu actions, command search,
keyboard dispatch, shortcut reference and tool hints. Settings has a Shortcuts
page with command filtering, key capture, conflict explanations, explicit
unbinding, per-command defaults and a confirmed reset of all shortcuts.

Bindings persist in application preferences. Existing preferences without
bindings retain defaults. Versioned `.dudelkeys` profiles export the full
effective mapping; imports validate the entire bounded file before replacing
anything. Invalid files, unknown commands, reserved gestures and conflicts
leave the current mapping intact. Assignment consumes keyboard and clipboard
events before artwork input; Escape cancels without clearing a canvas selection.
Tool bindings retain tap/hold and group cycling. Standard clipboard, contextual
editing, project navigation and canvas gesture keys remain reserved.

This increment covers common built-in commands. It does not introduce extension
command bindings, mouse gesture remapping or a tablet-driver configuration UI.
The assignment UI replaces a command's existing aliases with one shortcut;
profiles support up to three bindings per command.

The focused shortcut/tool/new-canvas/preferences run passed 28 tests. The first
full run passed 1,110 tests, failed eight and ignored 14. Seven failures were
background viewport/history worker deadlines; the eighth exposed F6's synthetic
Tab being consumed as a panel toggle. The dispatcher now leaves that event to
egui's focus navigation. The original failure evidence remains in
`artifacts/shortcuts-full-tests.log`.

The final full suite passed 1,118 tests (14 ignored) with four workers in 68.81
seconds, including F6 navigation and all seven previously timed-out tests.
The test executable was built with application debug information disabled
(`cargo rustc --locked --bin dudel --profile test -- -C debuginfo=0`) and then
run with `--test-threads=4`. This changes neither assertions nor the default
feature set; dependencies retain their usual build settings. Results are in
`artifacts/shortcuts-full-tests-limited.log`. Formatting and public-tree scans
also passed, including explicit scans of the new, untracked source files.
Strict all-target Clippy with warnings denied passed; its log is
`artifacts/shortcuts-clippy.log`.

The development executable builds with application debug information disabled
(`cargo rustc --locked --bin dudel -- -C debuginfo=0`); the build log is
`artifacts/shortcuts-build.log`. Native captures were reviewed for Settings and
the conflict modal at 1280 × 800, plus Settings at 960 × 640 with 150% UI scaling.
The compact page scrolls, and conflicting bindings show their owner with Apply
disabled. Captures are `artifacts/shortcuts-settings.png`,
`artifacts/shortcuts-conflict.png` and `artifacts/shortcuts-compact.png`.
These checks do not constitute a new physical-pen session or release-package
acceptance.

## Multi-layer increment — 2026-11-04

Implemented the next increment as session-local selection by stable layer
identity, with Ctrl-click toggles, Shift-click visible ranges, active-target
highlighting and a hidden-selection count. Filtering preserves selection;
reference selection clears it. Plain row selection and Only active restore a
single target. Painting and non-batch tools retain their active-layer behavior.

The Batch menu provides visibility, locks, stack movement, grouping and deletion.
Existing group/delete/step entry points also handle a multi-selection. Selected
folders subsume selected descendants. Move/group require siblings; movement
stops at parent boundaries and preserves order, while noncontiguous groups are
inserted at the highest selected position. Structural operations reject locked
targets, descendants and ancestors. Deleting the last artwork layer is rejected.
Multi-layer dragging is disabled; up/down and grouping provide explicit moves.

Earlier queued work settles before a batch. One identity-targeted document edit
builds a copy-on-write candidate and records one undo checkpoint. Failed batches
discard the candidate; no-ops create no checkpoint. Masks and native layer
payloads remain attached to their layers. Selection is neither project data nor
undo state. See [Quick Start](QUICK_START.md) for artist-facing instructions.

The first focused run passed eight batch-related tests, including actual layer
panel clicks at desktop and compact sizes, filtered/collapsed ranges, order,
locks, folder deduplication, one-step undo/redo and preceding queued edits.
The final default-feature suite passed 1,125 tests (14 ignored), using four test
workers, in 88.23 seconds. The log is `artifacts/layer-batch-full-tests.log`;
focused results are in `artifacts/layer-batch-targeted.log`. Public-tree and
whitespace scans passed, including an explicit scan of the new batch module.
Strict all-target Clippy with warnings denied and formatting passed. The
development executable builds with `cargo build --locked`; lint and build logs
are `artifacts/layer-batch-clippy.log` and `artifacts/layer-batch-build.log`.
Native selection highlighting and Batch controls were reviewed at 1280 × 800
and 960 × 640 with 150% UI scaling. Captures are
`artifacts/layer-batch-desktop.png` and `artifacts/layer-batch-compact.png`.
The active row remains distinct and the controls fit the compact inspector.
These are source-development checks, not new package acceptance or physical-pen
measurements.

## 0.2.0 next increments

### Pen quick menu — release priority

- [ ] Put a compact configurable menu near the canvas for favorite brushes and
      a small set of common actions. Reuse existing favorite brushes and command
      actions so the menu does not create a second source of truth.
- [ ] Define pen-sized targets, placement near an edge/corner, open/close and
      dismissal gestures, and behavior while drawing, panning or using a text
      editor. Closed-menu input must pass through to the canvas normally.
- [ ] Check keyboard focus, accessibility labels, project-tab isolation and
      compact window sizes. The menu is preference/workspace UI state, not
      artwork project data.
- [ ] Owner's pen review on the current candidate before release.

### Export workflow — release priority

See [export options](docs/EXPORT_OPTIONS.md) for the detailed behavior to
implement. Keep one captured document revision for each export; failure or
cancellation must not change the project or replace a protected destination.

### Named workspaces — stretch goal

- [ ] Consider named workspace layouts and optional pen-friendly control density
      after the release priorities are complete. Do not block 0.2.0 on this.

## Reprioritization evidence

Before reprioritizing, observe artists finding a layer in a crowded document,
switching between artwork and masks, and exporting successive revisions. Record
wrong-target edits, time spent finding commands and recovery steps. Automated
UI checks do not establish physical pen usability.
