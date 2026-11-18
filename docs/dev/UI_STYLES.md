# Interface styles (ui branch)

Dudel can show its workspace in four styles. Classic is the interface as it
was; Slate, Float and Atelier rearrange the same panels, so every feature is
reachable in every style.

| Style   | Layout                                                                 | Icons   | Density | Colors              |
|---------|------------------------------------------------------------------------|---------|---------|---------------------|
| Classic | Tool rail, tool settings left, color and layers right                  | Classic | Regular | The chosen theme    |
| Slate   | Tool bar at the left edge, brushes and tool property beside it; color and layers right; status bar | Line | Compact | Graphite and amber  |
| Float   | Canvas fills the window; tool islands, slider rail, popovers           | Duotone | Touch   | Smoked glass, teal  |
| Atelier | Tool bar at the left edge, brush drawer above tool property; navigator, color and layers right; Draw/Paint/Animate | Solid   | Regular | Stone, ultramarine  |

## Switching and customizing

- **View › Theme** has two groups. **Style** switches between the four.
  **Colors** offers the style's own colors (Slate colors, Float colors,
  Atelier colors; Custom colors once edited) and the themes: Dudel Dark,
  Paper, Graphite and any in the Themes folder. Exactly one of each group
  is marked, the one on screen. Below them: Edit theme… (or Edit style
  colors… while the style's own are used), Import theme…, Export theme…
  and Customize style….
- A theme works in every style. Slate, Float and Atelier make a whole
  palette from its five colors (`Palette::from_theme`: title bar, wells and
  lines derived; the accent darkened or lightened until it reads on the
  panels, as Paper's pale blue and Graphite's grey would not), so their
  raised islands and accent marks keep working. Theme colors chosen in one
  of them carry over to another; coming from Classic, a style brings its
  own colors. Classic draws a theme exactly as before.
- **Settings › Interface** shows a preview of each, and lets every part be
  changed on its own: layout, icon look, density, corner radius, and colors
  (the theme's, or the style's own ten colors). Float can put its slider rail
  on the right for left-handed drawing.
- The choice is saved with the preferences (`interface`). Older versions
  ignore the field and show Classic.

## Palettes

In Slate, Float and Atelier every panel is a palette, as in Clip Studio Paint
and Photoshop: Tools, Tool property, Brushes, Color, Navigator, Layers,
Layer property and Timeline.

- Drag a palette's tab onto another group to join it, or onto an edge of a
  group or of the canvas to split that area. Drag it off to let it float.
- Right-click a tab for Close and Float. A group or a floating palette has an
  X that closes it.
- **Window** (between View and Help) lists every palette: check one to open
  it, uncheck it to close it. A palette opens in the group it started in, or
  floats when that group is gone. Window also holds **Lock palettes** (no
  dragging, closing or floating, for a pen on a tablet) and **Reset to the
  … arrangement**.
- Each style starts from its own arrangement and remembers your changes to
  it. Float keeps the canvas whole: its palettes always float, opened from the
  right island. Atelier's Draw, Paint and Animate buttons bring their palettes
  forward.
- **View › Workspaces** saves the style and its arrangement under a name.
- Tab still hides every panel; the palettes come back as they were.
- When a style's starting arrangement changes (`PRESETS` in `src/dock.rs`),
  arrangements saved before give way to it once; named workspaces keep theirs.
- A starting arrangement's columns get widths in points (`columns` in
  `src/dock.rs`: Slate's tool bar 72, brushes 280, color and layers 300)
  once the window's size holds still; the canvas takes the rest. Widths
  dragged afterwards are kept. A palette group can be made as narrow as
  56 points, so the Tools palette can be a one-column bar (under 110 points
  it shows the grouped rail).

Tabs are as wide as their names (in Float, where a tab moves its floating
palette, they fill the bar). The canvas's tab names the project and what a
stroke edits ("Apple • › Artwork · Layer 1"); the strip above the canvas
appears only when it has something to act on: a locked layer, a selection,
a mask or reference to switch back from. The Navigator palette's preview
fills the room above its controls; a short palette leaves out the zoom and
rotation sliders, which the status bar also has.

Each palette keeps a margin around its contents, and its scroll bar stays in
view beside the contents whenever there is more to scroll to. Tool property
has one scroll area: brush, selection and canvas settings keep their name,
preview and page tabs in place and scroll the settings below them; other
tools scroll as a whole. Rows of choices wrap in a narrow palette.

Palettes don't repeat each other: with Layer property on screen, the Layers
palette leaves the layer controls to it, and with Brushes on screen, Tool
property shows the current tool's settings without a second brush list (as
in Atelier). Brush Studio (the Studio button) stays the full editor, like
CSP's sub tool detail.

## Where the code is

- The `dudel-ui` crate (`crates/dudel-ui`, egui only; see `CRATES.md`):
  - `style.rs`: `Interface`, the presets, palettes, applying colors and
    spacing, and `Active` (the style in use, kept in the egui context).
  - `icon_art.rs`: every icon as parts on a 24-unit grid in SVG path data,
    drawn as Line, Duotone or Solid through `ab_glyph_rasterizer` and cached
    as textures at the exact pixel size.
  - `icons.rs`: the `Icon` names, icon buttons, and the Classic painter icons.
  - `widgets.rs`: scrub field, tab strip, segmented control.
- `src/interface.rs`: applying a style on top of the theme, the settings
  page and the menu.
- `src/dock.rs`: palettes and their arrangements, built on `egui_dock`
  (0.18, the release for egui 0.33). Arrangements are saved as RON text in
  the preferences (`palettes`), because an area not laid out yet has an
  infinite rectangle that JSON cannot hold; one that no longer reads falls
  back to the style's own.
- `src/layouts.rs`: the bars of the Slate, Float and Atelier styles and
  Float's islands. `workspace::classic_panels` is the Classic layout,
  unchanged.

`DUDEL_CAPTURE_PALETTES=Layers,Color` (with `DUDEL_CAPTURE_STYLE`) opens
those palettes in a capture.

## Checking a style

`DUDEL_CAPTURE_STYLE=Slate` (or Float, Atelier) makes any `--capture-*` mode
render in that style; with `--capture-settings` it opens the Interface page.
Without the variable every capture is Classic and matches master's pixels,
except the Settings window, which has the new Interface tab.

Rail buttons with a corner mark have variants: brush presets, fill or
gradient, lasso fill or erase, and tool options. They open with a tap on the
button already chosen, a press and hold (`HOLD` in `src/layouts.rs`), the
pen's side button pressed over the button, or a right-click. egui's own
context menu needed a clean right-click and closed on any left click, so
with a pen it opened and closed at once.

In Float the color sits at the top of the side rail and at the end of the
right island; either opens a popover with a large wheel, both colors and
hex, recent colors, swatches and color ideas, and **Open the Color palette**
to keep a wheel on screen.

A floating palette's tab fills its bar, so a grip along its top moves the
window (egui_dock 0.18 never records a window's rect; `window_rect` reads
it from egui). Float has nothing to dock into, so there its tabs don't
undock: dragging a tab moves its window (`tab_drags`, which reads
egui_dock's tab ids), and there is no grip.

The Navigator palette has zoom and rotation sliders above its view buttons
(Classic keeps them in the status bar), so Float, which has no status bar,
has a zoom slider too. In Float a palette dropped on the canvas floats where it was
dropped rather than snapping back.

egui doesn't keep window positions between sessions, so the dock does:
each floating palette's corner and inside size are saved with the
arrangements (`windows` in the palette preferences) and put back once the
window's size holds still, at least `SMALLEST_WINDOW` and inside the
window. A palette with no saved place is placed like a newly opened one.
