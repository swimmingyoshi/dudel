# Interface styles (ui branch)

Dudel can show its workspace in four styles. Classic is the interface as it
was; Slate, Float and Atelier rearrange the same panels, so every feature is
reachable in every style.

| Style   | Layout                                                                 | Icons   | Density | Colors              |
|---------|------------------------------------------------------------------------|---------|---------|---------------------|
| Classic | Tool rail, tool settings left, color and layers right                  | Classic | Regular | The chosen theme    |
| Slate   | Grouped rail, context bar, one dock of tabbed groups, status bar       | Line    | Compact | Graphite and amber  |
| Float   | Canvas fills the window; tool islands, slider rail, popovers           | Duotone | Touch   | Smoked glass, teal  |
| Atelier | Brush drawer, color tray and layers, timeline below, Draw/Paint/Animate | Solid   | Regular | Stone, ultramarine  |

## Switching and customizing

- **View › Theme › Interface style** switches between the four.
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

Rail buttons with a corner mark have variants: right-click or press and hold
for brush presets, fill or gradient, lasso fill or erase, and tool options.
