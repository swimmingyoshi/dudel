# Responsive brush toolbar

2026-10-16. Follow-up to `SETTINGS_STORE_DURABILITY.md`, whose native fixture
exposed clipping of Snap/symmetry at 1280x800 with a longer current brush name.

## Resulting behavior

The current brush button has a bounded width, proportional to the visible
toolbar and capped at 192 logical pixels. It truncates the displayed name while
preserving the entire name in its hover text and widget text. Choosing a brush
and opening Brush settings still use the same picker workflow.

The toolbar uses direct size/opacity fields below 1050 logical pixels and moves
those controls into Stroke options below 700. After laying out the primary
controls, it measures the inline stabilizer/snapping/symmetry group using the
current font, values, spacing and active symmetry label. If that group exceeds
the remaining visible space, Stroke options provides the same controls. Visible
viewport bounds remain authoritative when a parent retains an older, wider layout.

These choices apply to Brush and Eraser and to artwork and mask coverage targets.
Brush names and stored preferences are not modified by display truncation.

Stroke options uses a height-bounded scroll area. In compact layouts, Symmetry
expands within that popover, keeping scope, mode, copies, rotation and guide
controls together. Wide layouts retain the direct symmetry popover. The compact
flow avoids a second hover-sensitive flyout competing for the same small viewport.

Below 900 logical pixels, the document header puts its name and save state above
a compact command row. File, Edit, Canvas, Layer, Select, Filter, View and Help
remain directly visible. The right-hand document name is bounded and truncated
with the full filename available on hover; Saved/Unsaved changes remains visible.
Main command menus scroll when their contents exceed the viewport height and
reset their scroll position on reopening, including after a viewport resize.

## Verification scope

The workflow regression repeatedly resizes through fourteen widths from 1920 to
480 logical pixels. It uses long ASCII and multibyte names, Brush and Eraser,
artwork and mask targets, and an active symmetry mode. It checks that toolbar text
stays inside the viewport and hit-tests actual Snap, preset selection and
Color/Coverage popup actions at representative widths. Existing narrow-mask and
retained-parent toolbar regressions also pass.

A second workflow exercises a long document filename with saved and unsaved
states across six widths. It checks actual aligned text bounds for clipping and
overlap, opens File through the exposed header layout, and invokes New canvas.
A third runs at 480x320 logical pixels: opens Stroke options, expands Symmetry,
scrolls to Radial, verifies the mode change, reaches Copies, then scrolls to and
toggles Lock guides. This validates the complete settings flow through hit testing.
The header test checks command routing; New canvas's full cancellation and
keyboard workflow was a separate next work item at that checkpoint. It is now
implemented and verified in `NEW_CANVAS_WORKFLOW.md`.

Isolated native capture entry points:

```text
--capture-long-toolbar NEW_IMAGE.png WIDTH HEIGHT SCALE
--capture-long-toolbar-options NEW_IMAGE.png WIDTH HEIGHT SCALE
--capture-long-toolbar-brushes NEW_IMAGE.png WIDTH HEIGHT SCALE
```

They use the existing generated illustration fixture and private settings path.
They do not read or write the real application preference store. Capture popup
requests are applied through the first layout passes so a control that appears
after initial viewport sizing is included in the screenshot.

Full-suite verification: **748 passed, zero failed, one existing high-memory test
ignored**. All 17 end-to-end workflow regressions pass. All-target Clippy with
warnings denied, formatting and Git whitespace checks pass. Logs:
`target/qa/toolbar-{tests,workflows,clippy,diff-check}.log`.

## Native evidence and build identity

Optimized offline build passes (`target/qa/toolbar-release.log`). An earlier
attempt could not replace an executable in use; that session was left alone.
The final normal release build completed once the file was available. Current
`target/release/dudel.exe` and frozen
`target/qa/dudel-responsive-toolbar-settled.exe` have identical SHA-256:

`789644A48F40B20CA9F00B44C8AF02EAAE43C896C1E7FFF2DF3197DD198BF72D`.

Seven separate native captures from that frozen binary were produced and viewed:

| Image under `target/qa/` | Observed logical viewport | Zoom / pixels per point |
| --- | --- | --- |
| `toolbar-settled-100.png` | 1280x800 | 1 / 1 |
| `toolbar-settled-150.png` | 853.3x533.3 | 1.5 / 1.5 |
| `toolbar-settled-200.png` | 640x400 | 2 / 2 |
| `toolbar-settled-options-200.png` | 640x400 | 2 / 2 |
| `toolbar-settled-options-min-200.png` | 480x320 | 2 / 2 |
| `toolbar-settled-picker-200.png` | 640x400 | 2 / 2 |
| `toolbar-settled-wide-100.png` | 1920x1000 | 1 / 1 |

Each has adjacent `.stdout.log` and `.stderr.log` files. The images verify the
bounded brush name, directly visible command row, document state, compact
primary controls, inline wide controls, and the visible brush/stroke popovers.
The minimum-size stroke popover includes size, opacity, stabilizer, Snap and the
Symmetry section header within the viewport. The expanded Radial/scrolling flow
is verified by the hit-testing regression, rather than these collapsed-section
native images. Captures wait at least 400ms before requesting a screenshot so
transient popup fade-in is not mistaken for the final appearance.

The professional release gates remain open: physical pen input, sustained
memory/GPU interaction, native clipboard exchange, supported CSP fixtures and
packaging. Toolbar checks do not establish those broader requirements.
