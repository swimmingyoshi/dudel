# The ui branch

Merged into master on 2026-11-18, with Classic as the default and the other
styles described as a preview in the user docs. The open questions below
are still open. These notes record what
the branch holds, what was decided along the way, what is still open, and a
suggested way forward. How the styles and palettes behave is in
[UI_STYLES.md](UI_STYLES.md); the crate layout is in [CRATES.md](CRATES.md).

## Why

The owner asked for a workspace that can be arranged like Clip Studio Paint
and Photoshop (palettes that can be moved, grouped as tabs, floated and
closed, arranged for different workflows), for settings that are easier to
find, and for the code to be split into crates. This revisits the 2026-11-10
decision to keep the fixed panel arrangement ([ROADMAP](ROADMAP.md), 0.3):
the fixed arrangement stays as Classic, and the new styles are opt-in.

## What is on the branch

| Commit | What it does |
|---|---|
| `07d04bb` | Interface styles: Slate, Float and Atelier beside Classic, with their own icon art (Line, Duotone, Solid), density and colors; Settings › Interface to switch and customize |
| `950d2c3` | Workspace split: `dudel-core` (the engine, no window toolkit), `dudel-ui` (styles, icons, widgets) and the app |
| `655ee65` | Palettes: in Slate, Float and Atelier every panel is a dockable, movable, closable palette; Window menu, lock and reset; workspaces save the arrangement |
| `5a4adb4` | Settings window: a category sidebar beside the page, search over every setting, Reset per page |
| `78ff482` | Palettes don't repeat each other (Tool property without a second brush list when Brushes is open; Layers without its controls when Layer property is open) |
| `f96d74a` | Backlog notes: the split's pixel proof and rebuild times |
| `3e72e33` | Slate puts Tools, Brushes and Tool property down the left; every palette gets a margin and a scroll bar that stays in view; Tool property has one scroll area |
| `cfb9287` | These notes |
| `d944045` | Dialogs and menus fit the window at any interface size: New canvas and Canvas size measure their title and buttons and scroll the rest, with a visible scroll bar; New canvas is wider with touch-sized buttons and its labels line up; a menu cut off at the window's edge shows its scroll bar |
| `44b8e69` | Slate's tools become a vertical bar at the left edge, as in Classic, with brushes and tool property beside it; Slate's columns start at fixed widths in points |
| `f52ed21` | Pen fixes and a comfier Float: rail variants open with a tap again, a hold or the side button; floating palettes have a grip to move them, and in Float one dropped on the canvas floats there; Float's color popover from its rail and island |
| `3fefb4a`, `00610df` | Floating palettes come back where they were left, at their size, after a restart or a workspace; one saved before that is placed like a new palette instead of sitting squeezed to its tab in the top-left corner. Master merged in (the lighter pen quick menu, sharp pixel-art timelapse) |
| `6f00d70` | In Float, dragging a palette's tab moves its window (no grip there); the Navigator palette gets zoom and rotation sliders |
| after `6f00d70` | View › Theme sorted into Style and Colors, marking only what is on screen; Dudel Dark, Paper, Graphite and saved themes now color Slate, Float and Atelier too, through a palette made from the theme |

Size: 158 files changed against master, most of it files moved into
`crates/dudel-core`.

## Trying it

The branch was merged and deleted on 2026-11-18; everything below is on
master. Build and run it with `cargo run --profile fast`. Then use View › Theme › Style, or
Settings › Interface, to pick a style. Classic is the default, and existing
preferences open in Classic.

## Decided so far

- **Styles are starting layouts** (2026-11-16): each style begins from its
  own palette arrangement, and the user's changes to it are remembered per
  style.
- **Floating palettes stay inside Dudel's window** (2026-11-16), not as
  separate operating-system windows.
- **Classic stays exactly as it was.** Every change is checked against the
  previous build's Classic screenshots (see Checking below). Classic has no
  palettes.
- **Brush Studio stays the full brush editor**, in the role of CSP's sub
  tool detail; Tool property shows the everyday settings.
- **Slate is the CSP-like dark layout** (2026-11-16): the tools as a
  vertical bar at the left edge, as in Classic, with brushes and tool
  property beside it; color, navigator and layers on the right.
- **Keep all four styles** (2026-11-18); they are refined later. The themes
  work well enough for now and get a closer review later.
- **New installs choose their look** (2026-11-18): a first-time setup
  (`src/first_run.rs`) asks for the Dudel folder, the style and colors, and
  the control size, then opens the welcome window. Classic stays selected
  unless the artist picks another. This settles open questions 2 and 3.
- **One commit per step** on the branch. Nothing is pushed until the owner
  asks.

## Open questions

None of these is decided. They are listed so that the branch can be judged
as a whole.

1. **Ship it, and when.** Merge it into a release (0.7?) as an opt-in
   preview, keep it on the branch longer, or take only parts of it.
2. **Which styles.** Keep all four, or fewer. Slate has had the most work.
   Float (touch, floating palettes) and Atelier (light colors, the
   Draw/Paint/Animate modes) have had less, and neither has had a pen review
   on the tablet.
3. **The default for new installs.** Classic for everyone, or Slate for new
   installs only.
4. **Palettes in Classic.** Classic could become one more arrangement of the
   same palettes. That would retire `workspace::classic_panels`, but Classic
   would no longer match its screenshots exactly.
5. **Tool property and Brush Studio.** Whether the options bar, Tool property
   and Brush Studio should become one palette with pages, as in CSP, or stay
   as three levels of detail.
6. **Palettes outside the window**, for second monitors. egui can open extra
   native windows, but each one costs a renderer and adds focus problems with
   a pen; this was turned down for now (see Decided so far).
7. **More for Float.** The owner asked for a comfier Float (2026-11-16).
   The color popover is the first step, and floating palettes now come
   back where they were left; candidates next are larger rail sliders and
   a brush preview on the rail.
8. **Tablet use.** Whether Lock palettes should be on by default for pen-only
   setups, and whether Float's touch density suits the Artist 12.

## Before it can be merged

- **License notices.** Done (2026-11-18). The palettes bring four new
  packages: `egui_dock`, plus `duplicate`, `heck` and
  `proc-macro2-diagnostics`, which are used only while building. All are
  MIT or Apache-2.0, and each ships its own license files, which packaging
  copies from the vendored crate; nothing needed freezing in
  `scripts/upstream-notices`. `check_release_notices.py` already skips
  Dudel's own crates.
- **One CI run.** Before the merge (2026-11-18) the branch passed
  `Verify.ps1 -Mode Full -SkipReleaseBuild` locally (1,499 tests, Clippy
  with and without features, the thumbnail DLL, the Python checks) and
  Clippy for Linux through `cargo-zigbuild clippy`. That run found the
  thumbnail DLL still reading `src/file_preview_header.rs`, which the split
  had moved; fixed. Packaging has not run on it. CI runs on pull requests and on
  master, so one pull request run (or the run when it reaches master)
  covers both.
- **Keeping up with master.** The split moves about a hundred files, so
  every change on master to an engine file has to be carried over into
  `crates/dudel-core`. The longer the branch waits, the more of that there
  is. Merge master into the branch whenever master moves, rather than all
  at once at the end.
- **User docs.** Done (2026-11-18): QUICK_START has "Arrange the
  workspace", TESTER_GUIDE an optional layout step, and the release notes a
  "Workspace styles (preview)" block.
- **Engine rebuild time.** After an edit in the engine, the test build is
  slower than before the split (101 s against 57 s), because the app's tests
  are rebuilt too. `cargo test -p dudel-core` avoids this while working on
  the engine. An edit in the app rebuilds faster (34 s against 53 s). Both
  measurements are in the [engineering backlog](ENGINEERING_BACKLOG.md),
  section 3.

## Suggested way forward

A recommendation, not a decision:

1. **The owner tries Slate on the tablet** for real drawing: whether the
   left side holds what is needed, whether tabs are easy to hit with the
   pen, and what Lock palettes should do by default.
2. **Settle questions 1 to 3** (ship it, which styles, the default). They
   decide how much more work Float and Atelier get.
3. **Do the merge work above**: notices, user docs, master merged in, one
   CI run.
4. **Merge with Classic as the default and the other styles marked as a
   preview.** Classic matches its old screenshots, so existing users see no
   change unless they choose a style. The crate split cannot easily land on
   its own: it moved the style code that `07d04bb` added into `dudel-ui`.
5. **Afterwards**, depending on testers: palettes in Classic (question 4) and
   one combined tool property (question 5).

## Checking changes on the branch

- **Classic must not change.** Every capture mode (`--capture-*`, listed in
  `src/capture_fixtures.rs`) is shot with the previous build and the new
  one, and the two are compared pixel for pixel. A mode that differs is shot
  again on both sides, and the old build is also compared with itself, to
  separate real changes from run-to-run noise. `project-memory` always
  differs, because it shows live numbers. Captures run in a hidden window.
- **Styles.** `DUDEL_CAPTURE_STYLE=Slate` (or Float, Atelier), plus
  `DUDEL_CAPTURE_PALETTES=Layers,Color` to open palettes. Check at
  1440×900 and at 1280×800.
- **Engine refactors.** The 32 profiled strokes (`--profile-stroke` with
  the `fast` build; see [Testing](TESTING.md#build-profiles)) must paint
  byte-identical canvases at the same speed.
- **A layout change only for palettes.** `dock::in_palette(ui)` says
  whether a row is drawn inside a palette. Even a change that looks harmless,
  such as `horizontal` to `horizontal_wrapped`, moves labels by a pixel, so
  palette-only layout goes through a check like `choice_row` in
  `src/tool_panel.rs`.
- **Screen sizes.** Check dialogs and menus at 1366×768, at 1920×1080
  with 150% scaling (the capture's fifth argument, `1.5`), and at
  1280×800, in every style. Float's touch sizing is the tallest. A dialog
  that scrolls takes its height from `dialogs::dialog_body_room` and
  reports itself with `measure_dialog`, rather than taking a fixed amount
  off the window's height.
- **Changing a starting arrangement.** Raise `PRESETS` in `src/dock.rs`, so
  that arrangements saved under the old presets give way to the new ones.
  Column widths in points go in `columns`; the test
  `starting_columns_get_their_widths_in_points_once_laid_out` checks they are
  reached at 1280 and 1920 points wide.
