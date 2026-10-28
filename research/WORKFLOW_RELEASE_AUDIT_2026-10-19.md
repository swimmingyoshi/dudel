# Workflow release audit: first source checkpoint

Base revision: `41017d850e896a175aad362256c2338e16d64b8d`.
Scope: document/tab safety and reuse of existing workflow coverage. This audit
does not certify the frozen study EXE or a new packaged release.

## Defect fixed

With first.dudel open in tab A, Save as from tab B could select first.dudel and
start an ordinary background write. Tab A retained its old in-memory state even
though its file was being replaced by B. Existing Open deduplication did not
protect Save as.

The save destination is now checked against open tabs before gesture settlement,
snapshot admission or a pending New/Close continuation. A conflicting filename is
rejected with the owning tab's name and instructions to choose another name or
close the tab first. Existing canonical-path comparison also recognizes resolved
path aliases. This is an in-app ownership check, not interprocess file locking;
other programs and independently launched Dudel processes remain outside it.

The new regression failed on the old implementation because a save worker had
started. It now passes for both clean and modified parked projects, original and
canonical path forms, preserves destination bytes and both editable states, and
prevents the requested close continuation. A complementary real-file test checks
normal saving and a new copy still save/reopen correctly with another tab parked.

## Coverage map and results

| Scenario | Existing evidence reused / new evidence | Result |
| --- | --- | --- |
| Independent documents, history, paths and views | tabs_keep_independent_documents_history_views_and_paths | Pass |
| Switch while rendering or while a transform needs a decision | switching_waits_for_rendering_and_refuses_open_transforms | Pass |
| Recovery before parking; failure leaves artwork active | new_tab_waits_for_recovery_of_pixels_masks_guides_and_folder_references; failed_snapshot_cancels_tab_navigation_without_parking_or_replacing_artwork | Pass |
| Close last/parked/dirty tabs; quit across dirty tabs | closing_tabs_prompts_only_for_unsaved_work_and_keeps_one_canvas; quitting_visits_each_unsaved_tab_before_closing | Pass |
| Save cancellation, failure, newer edits and deferred replacement | save_jobs::tests and document_actions::tests | Pass |
| Canceled/stale/failed loads; real-file reopen | load_jobs::tests | Pass |
| Cross-tab Save as and unaffected ordinary saving | Two new save_jobs regressions described above | Pass |
| Theme picker interactions, invalid edits, draft save, Apply/Cancel, persistence | themes::tests | Pass in automated egui/file fixtures |
| Brush/eraser independence; clean-preference brush-pack transfer with matching strokes | brush_library::tests | Pass |
| ZIP project/folder roundtrip, reconstruction and useful rejection | project_package::tests | Pass |
| New-canvas keyboard/unsaved/failure paths, short inspector, layered illustration save/reopen/export | Selected workflow_tests | Pass in automated egui/file fixtures |

Commands completed after the fix:

```
cargo test --locked -- tabs::tests document_actions::tests save_jobs::tests load_jobs::tests
# 41 passed, 0 failed, 0 ignored
cargo test --locked -- themes::tests brush_library::tests project_package::tests workflow_tests::new_canvas workflow_tests::layered_illustration workflow_tests::short_inspector
# 27 passed, 0 failed, 0 ignored
cargo clippy --locked --all-targets -- -D warnings
# passed
```

Formatting was applied with cargo fmt; diff whitespace is checked separately.
No unchanged Python suite, maximum fixture, source archive rebuild or long study
was repeated for this bounded Rust save-path change. Full Windows/Linux CI belongs
to the subsequent source checkpoint; no hosted pass is claimed here yet.

## UI efficiency review boundary

Source review confirms brush previews already cache appearance fingerprints;
layer/mask thumbnails use stamps and a 200 ms update throttle; inactive tabs park
settled work; the audience preview reuses main tiles when exclusions permit it.
These are existing mechanisms, not newly measured improvements. No performance
gain is claimed. Representative runtime profiling and independent short-window,
actual OS DPI, keyboard and Artist 12 acceptance remain milestone work.

## Next acceptance exercise

Use disposable artwork and record the build, OS and scaling. A tester should:

1. Create a sketch, ink and color layer; fill a soft edge, apply/cancel an
   adjustment, add a mask and export both PNG variants. Reopen and compare.
2. Open two distinct projects; try Save as onto the other tab's file. Confirm
   rejection, then save a distinct copy. Cancel an unsaved close; confirm both
   projects remain. Quit and answer each project's prompt deliberately.
3. Repeatedly edit Paper's text/accent/canvas surround, including an invalid
   contrast combination; recover the draft, Apply, save a theme and restart.
   Cancel a second edit and verify the applied theme survives.
4. Transfer a custom brush pack, theme and project ZIP to clean preferences;
   compare brush/eraser settings, layers/references and resulting artwork.
5. Repeat common actions in short windows and actual 100/150/200% OS scaling;
   check keyboard focus, text input, Escape, status visibility and file dialogs.

Record failures with exact steps instead of marking broad gates complete.
Physical tablet, real OS-dialog and independent first-use results remain pending.

## Follow-up: complete workflow and project guide isolation

Follow-up base revision: `fcfcca16360fb26ca06f08b37c6ddd58446015ae`.
Review found that the tab Session held per-layer symmetry and guide baselines but
omitted global symmetry, its drag state, ruler snapping and grid snapping. Those
fields consequently remained live across session swaps. A newly opened tab could
inherit another canvas's global symmetry/snapping, while ordinary switching could
incorrectly advance a saved document's revision and unsaved state. A later save
could persist the wrong guide configuration.

Those four fields now travel with each tab. A regression first failed with a
radial/8-segment/rotated symmetry and disabled ruler snapping leaking into a new
blank canvas. It now verifies default new-tab guides, two saved projects with
distinct guide settings, repeated round trips without dirty/revision changes,
closing a clean tab back to its neighbor, and closing the last tab to a default
blank canvas. It uses the complete saved Guides record rather than inspecting
only the new Session fields.

The full existing workflow module plus tab and guide tests passes:

```
cargo test --locked -- tabs::tests guides::tests workflow_tests
# 54 passed, zero failed, zero ignored
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
# both passed
```

The workflow coverage includes pressure-packet mask painting, mask copy/transform,
save/reopen/recovery/PNG export, reference-board folder reordering and individual
image selection, symmetry badges, fill controls, keyboard/modal interactions,
window controls, compact status/inspector/tool settings and illustration history.
No additional broad UI redesign was justified by these passing checks. Automated
egui input is distinct from independent artist or real operating-system testing.

## CI review for the preceding cache/safety checkpoint

[Linux run 39320259832](https://github.com/swimmingyoshi/dudel/actions/runs/39320259832)
completed successfully on `fcfcca1`: 872 default and 876 all-feature tests,
three ignored in each (including the manual component microbenchmark), 34 Python
checks, both lints, release build and X11 startup/artifact upload. Job
118292227820's completed logs were reviewed. Artifact bytes were not downloaded
or independently reviewed in this follow-up; no physical Wayland claim is made.

[Windows run 39320260242](https://github.com/swimmingyoshi/dudel/actions/runs/39320260242)
also completed successfully. Completed job 118292435889's logs show 880 default
and 889 all-feature tests, three ignored in each, 34 Python checks, both lints,
release compilation, both dedicated maximum fixtures (81.06 s / 13.29 s) and
packaging/upload. The package reports 458 dependencies and an empty notice
collection manual_review list; that is not a new compatibility certification.
Hosted archive bytes were not downloaded/audited. Both hosted results predate the
new tab-guide isolation fix, whose own full CI remains a separate checkpoint.

## Native follow-up for tab-guide isolation

The release build passes. Engineering EXE SHA-256:
`ABF4F9DC93FFA1FD554D5EAA3B3DF8029C784EB8BCD0E03E2990D079E7A58EEA`.
Its changed compiled input is src/tabs.rs, hashed in the
[native verification record](../benchmarks/tab-guide-isolation-20261019.json).
Three isolated DX12 captures exit successfully and were visually reviewed:
tab strip at 1280x800 / 100% UI scale, guide controls at 1280x800 / 100%, and guide
controls at 1280x800 / 200%. The selected layer and guide badges remain visible;
the compact layout exposes Navigator/Color buttons and keeps the wrapped status
controls reachable. These captures use app UI zoom, not a verified OS DPI setting.

An initial capture harness check lost the exited process's ExitCode under Windows
PowerShell despite producing a valid image. Retaining the child handle fixed the
harness; all three final captures returned exit 0. No application crash was found.
The isolated profiles leave the user's artwork, settings and editor untouched.
This is an engineering executable, not a newly accepted distribution package.
