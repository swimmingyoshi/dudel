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

## Where the code is

- `src/ui/` names only egui, so it can become the `dudel-ui` crate:
  - `style.rs`: `Interface`, the presets, palettes, applying colors and
    spacing, and `Active` (the style in use, kept in the egui context).
  - `icon_art.rs`: every icon as parts on a 24-unit grid in SVG path data,
    drawn as Line, Duotone or Solid through `ab_glyph_rasterizer` and cached
    as textures at the exact pixel size.
  - `widgets.rs`: scrub field, tab strip, segmented control.
- `src/interface.rs`: applying a style on top of the theme, the settings
  page and the menu.
- `src/layouts.rs`: the Slate, Float and Atelier layouts.
  `workspace::classic_panels` is the Classic layout, unchanged.

## Checking a style

`DUDEL_CAPTURE_STYLE=Slate` (or Float, Atelier) makes any `--capture-*` mode
render in that style; with `--capture-settings` it opens the Interface page.
Without the variable every capture is Classic and matches master's pixels,
except the Settings window, which has the new Interface tab.

Rail buttons with a corner mark have variants: right-click or press and hold
for brush presets, fill or gradient, lasso fill or erase, and tool options.
