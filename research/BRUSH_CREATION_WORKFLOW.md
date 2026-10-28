# Brush creation, preset safety and compact studio workflow

2026-10-16. This checkpoint addresses daily brush creation under priority 6 in
`DELIVERY_PLAN.md`. It adds a direct studio creation path and fixes preset-state
and naming defects. It does not establish physical drawing feel or full release
readiness.

## Resulting behavior

Brush Studio now exposes Save as new, Update preset and Revert together. New
creation and the sidebar's existing Save as new use the same validated command.
Creation keeps the original preset and artwork intact, selects the new preset,
updates recent brushes and clears library filters so it can be found. Updating
an existing preset retains its confirmation; Revert restores its saved settings.
Invalid settings show an inline message and cannot be saved through studio
buttons. The 512-preset limit rejects creation without partial mutation.

Changed-state detection compares the complete brush, including stabilization
amount, distance space and speed behavior. The earlier preview-image hash
intentionally excluded stabilization, so a stabilization-only edit failed to
mark the preset changed and could leave sidebar Update/Reset disabled. Preview
image hashing remains separate; no stroke/sampling algorithm was changed.
Favourite state belongs to the library entry. Toggling the selected entry's
star synchronizes its live metadata without presenting a paint-setting change.
Version-1 preferences normalize an older stale live-star value from the selected
entry while preserving edited brush settings. Preference and brush-pack versions
are unchanged.

Creation/duplication generates names unique against lowercased existing names.
Copy suffixes respect the existing 200-byte name bound and UTF-8 boundaries.
Previously, appending ` copy` to a valid maximum-length name could create an
invalid library entry; preference decoding validates all entries. The new path
validates before insertion and preserves the original. It prevents that new
invalid entry; it does not repair previously invalid preference files. Imported
pack names retain their existing format behavior.

Long library captions truncate before the favourite star; hover feedback includes
the full name. Studio uses a fixed title and truncates the current-name heading
so a long name cannot force the editor wider.

## Responsive editing

Window dimensions are bounded by the current viewport, including after resizing
from a larger viewport. Wide layouts retain section navigation beside the preview
and settings. Compact layouts use a scrolling section chooser and smaller
preview. Settings scroll inside a fixed allocated rectangle; the footer is
outside that rectangle, with space reserved for actions and status/error text.
The final layout keeps both actions and state feedback visible. Native QA caught
an initial status-line clipping issue; the regression now includes that line.

The isolated `--capture-brush-studio NEW_IMAGE.png WIDTH HEIGHT SCALE` fixture
opens the Stabilizer section with an amount-only edit of 12 px. Capture waits
for the live brush preview, document display and thumbnails before saving.
It loads no user artwork/preferences and does not persist preferences.

## Automated evidence

Full suite: **732 passed, zero failed, one existing high-memory test ignored**.
Warnings-denied all-target Clippy, formatting, whitespace and optimized build pass.
Five added regressions cover:

- Repeated duplicate creation for 200-byte ASCII and multibyte names, valid UTF-8,
  lowercased-name uniqueness and preservation of the original entry.
- Stabilization-only changes detected even when the preview hash is unchanged;
  amount, space and speed settings revert correctly; favourite metadata remains
  separate from edited settings.
- Invalid/full-library creation leaves entries, selection, artwork and document
  undo/modified state intact.
- Ink, grain pencil and embedded image-tip presets created through application
  commands, serialized through the actual app preference-save method, written to
  temporary files, decoded and hydrated through the shared startup method.
  Brush packs are written/read and imported into a fresh library. Complete
  settings and rendered preview pixels match the originals.
- Actual egui hit testing at 960x700, 640x400 and 480x320 points: Revert, Save as
  new, section-chooser scrolling, Name and tags access, resize from a larger
  window, visible action/state text, inline invalid-name feedback and rejection
  of a disabled save.

The preference round trip uses memory-backed eframe storage plus temporary files
and the shared app hydration path. It intentionally avoids recovery/extension
startup and the user's real settings store. This is automated serialization/
hydration evidence, not a native process restart or crash/disk-durability proof.

## Native visual evidence and identity

Final OpenGL screenshots at application zoom 100/150/200% were inspected. They
show the live brush preview, changed stabilization marker, enabled Update/Revert,
Save as new and the complete Unsaved preset changes line. The compact 200%
layout uses its section chooser and retains the stabilizer controls. Logs record
observed viewports and pixel scales rather than assuming them from window args:

| Application zoom | Viewport points | Pixels per point |
| --- | --- | ---: |
| 100% | 1536x840.8 | 1.25 |
| 150% | 1024x560.5 | 1.875 |
| 200% | 768x420.4 | 2.5 |

- Final images/logs: `target/qa/brush-studio-final-{100,150,200}.{png,log}`.
  Earlier `brush-studio-{100,150,200}.png` captures are superseded.
- Test/lint/build logs: `target/qa/brush-studio-{tests,clippy,release}.log`.
  Focused UI log: `target/qa/brush-studio-ui-tests.log`.
- Brush-workflow checkpoint release and identical frozen executable SHA-256:
  `8FED0D4F7EC63B3A880E60A2C2604CC7C112E8C5550E5EB0F16C61155C2046A2`.
  Frozen binary: `target/qa/dudel-brush-studio.exe`.
- Reproduce with a new output path, for example
  `dudel.exe --capture-brush-studio NEW_IMAGE.png 1280 800 2`.

## Remaining work

The later `PREFERENCE_RECOVERY.md` checkpoint guards app-level preference decode
failures and provides verified-backup recovery, with 737 tests passing. Whole-
container read/write durability and isolated real-store restart remain open; the
app-level guard does not certify preservation after an underlying RON failure.

Physical XP-Pen brush ergonomics, native process restart against an isolated real
settings store, longer sessions/resource profiling, external clipboard, supported
CSP interchange and packaging remain open. Earlier mask performance measurements
remain their frozen checkpoint evidence; this build makes no new FPS or physical
latency claim. The professional drawing-application goal remains active.
