# New canvas form and safe replacement

2026-10-16. Follow-up to `RESPONSIVE_BRUSH_TOOLBAR.md`.

New canvas is a modal form with initial width focus, normal Tab navigation,
explicit Cancel, Escape/backdrop cancellation, and Enter submission. Enter on
the focused Cancel button cancels. Width and height accept whole pixel counts
from 1 to 4096, subject to the existing 4,194,304-pixel area limit. Empty,
fractional, negative, oversized and overflowing inputs cannot create a document.
Invalid input stays available for correction with a visible explanation.

The dimension fields scroll independently of the creation/cancellation controls
on short viewports. The form owns keyboard input even before a field acquires
focus. Canvas/tool shortcuts and the global Tab panel toggle cannot alter artwork
or the workspace behind it. Display limits, artwork formats and brush preferences
are unchanged.
Ctrl+N takes priority over canvas actions delivered in the same input frame;
tool switching and the raw Tab handler also recognize that opening frame.

Submission goes through the existing document replacement guard. Unsaved artwork
remains intact until Save or Discard resolves that guard. Cancelling the guard
returns to the form with the dimensions retained. Save waits for a completed
save of the old painting before applying the new document. A failed save cancels
the pending replacement, preserves the document/path/undo history, and returns
to the form with the error shown in its scrollable content. Successful creation
clears the form, establishes the normal new-document history/view state, and
retains the drawing tools and brushes.
An already-running save follows the same rule: creation waits for its outcome.
If newer edits invalidate the saved snapshot, the replacement stays cancelled
and the form explains that the latest drawing needs review.

Ten focused regression groups cover dimension boundaries, pre-focus keyboard
isolation, field editing and Tab/Enter/cancel-button behavior, invalid input and
cancellation at 960x700/640x400/480x320, unsaved Cancel/Discard, actual asynchronous
save and reopen of the original, injected save failures against an owned
fixture directory (including an existing save), newer-edit cancellation, and
same-frame Ctrl+N/Undo/Delete/tool-key isolation.
Existing header routing now closes the form with its actual
Cancel button. Workflow tests render the shared unsaved modal as the real app does.

Native capture commands use generated documents and private session settings:

```text
--capture-new-canvas NEW_IMAGE.png WIDTH HEIGHT SCALE
--capture-new-canvas-invalid NEW_IMAGE.png WIDTH HEIGHT SCALE
```

## Verified checkpoint

- Full suite: **758 passed, zero failed, one existing high-memory test ignored**.
  All-target warnings-denied Clippy, formatting, Git whitespace and optimized
  offline build pass. Logs: `target/qa/new-canvas-{tests,clippy,diff-check,release}.log`.
- Current release and identical frozen `target/qa/dudel-new-canvas.exe` SHA-256:
  `977EBAF5E2B6BA96DC7D85BDB5089F140AD437609D47BC121AE293624730AC6B`.
- Five separate native captures were produced and viewed. Each image below has
  adjacent `.stdout.log` and `.stderr.log` files under `target/qa/`.

| Image | Logical viewport | Zoom / pixels per point |
| --- | --- | --- |
| `new-canvas-100.png` | 1280x800 | 1 / 1 |
| `new-canvas-150.png` | 853.3x533.3 | 1.5 / 1.5 |
| `new-canvas-200.png` | 640x400 | 2 / 2 |
| `new-canvas-min-200.png` | 480x320 | 2 / 2 |
| `new-canvas-invalid-min-200.png` | 480x320 | 2 / 2 |

The native images verify fields, constraints, validation feedback, disabled
creation for an over-limit size, and visible Create/Cancel controls. Keyboard
and replacement decisions are exercised through actual egui event/hit testing,
and the asynchronous file tests write/reopen generated projects. These images
do not certify physical keyboard/tablet delivery or a manually operated native
file dialog. Diagnostics do not read or modify actual user settings or drawings.

Physical pen, sustained native resource/GPU interaction, OS clipboard, supported
CSP exchange and packaging remain separate open release gates. This form work
does not establish professional-release readiness on its own.
