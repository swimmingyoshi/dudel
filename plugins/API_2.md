# Extension API 2

API 2 adds brush presets, tool presets, scripted tools, application commands and
custom panels to the existing sandboxed Rhai/WASM pixel filters. Install
`example-studio-kit.dudelplugin` through **Filter > Manage extensions** to try all
five contribution types, and `example-scripted-tools.dudelplugin` for scripted
tools. Brush and tool buttons appear in the tool rail. Open
contributed panels from the manager or the Filter menu. Filter commands are listed
in the Filter menu with the built-in filters.

## Package contract

Use `format: "dudel-extension"`, `version: 2`, and `api: 2`. Metadata and the
5 MiB package limit follow [API 1](SDK.md). API 1 packages continue to work.
API 2 places programs inside `contributes.commands`, leaving top-level `source`
and `wasm` empty. Unknown contribution fields are rejected to catch typos.

`contributes` has `commands` and `panels` arrays. Command and panel IDs are unique
within their own package and contain 1–80 lowercase ASCII letters, digits or
hyphens. The host namespaces all UI identities by package and contribution ID.
Maximums: 64 commands, 8 panels, 64 widgets per panel, 64 installed packages.

Each command has `id`, `label` and an `action` with one of these `type` values:

| Action type | Fields | Required capability |
| --- | --- | --- |
| `brush` | `brush`: Dudel brush object, including optional embedded tip/grain and pressure dynamics | `brushes` |
| `tool` | `tool`: one of the tool definitions below | `tools` (`tools.script` for `script` tools) |
| `filter` | `source`: Rhai `filter(r,g,b,a)` | `pixels.filter` |
| `wasm_filter` | `wasm`: byte array using the existing WASM ABI | `pixels.filter` |
| `host` | `command`: supported application command | `commands` |

Host commands: `undo`, `redo`, `new_layer`, `deselect`, `invert_selection`,
`transform`, `fit_canvas`, `flip_view`. These call the application's normal action paths. Artwork edits
participate in native undo and target permissions; view changes do not create
artwork history. Filters retain cancellation, stale-result rejection and atomic
publication from API 1.

Tool definitions use `kind`:

| Kind | Fields |
| --- | --- |
| `brush` | `brush`, `erase` (boolean) |
| `shape` | `options`: `shape` (`Freehand`, `Rectangle`, `Ellipse`), `erase`, `constrain`, `from_center` |
| `fill` | `tolerance` (0–255) |
| `selection` | `shape` (`rectangle`, `lasso`, `polygon`, `wand`) |
| `pointer`, `picker`, `move` | No additional fields |
| `script` | `source`: Rhai tool script (see below), `erase` (boolean, default false) |

Apart from `script`, these are contributed configurations of native tool engines. Brush selection does not duplicate entries in the user's
library. **Save as new preset** explicitly keeps a local editable copy.

## Scripted tools

A `script` tool decides where the brush goes. Its Rhai program receives the pen's
samples and returns points; Dudel draws them with the user's current brush, color
and eraser setting. The script never reads or changes the document: its points go
into one ordinary stroke, so undo (one step per stroke), masks, selections, layer
locks, symmetry and cancellation work exactly as for a hand-drawn line.

```rhai
fn begin(x, y, pressure) { this.count = 0; [] }   // optional, at pen-down
fn point(x, y, pressure) {                         // required, every sample
    this.count += 1;
    [[x, y - 10.0], [x, y + 10.0, pressure, 1]]    // two lines: tracks 0 and 1
}
fn end(x, y, pressure) { [] }                      // optional, at lift
```

- Coordinates are canvas pixels; pressure is 0–1.
- `this` is a map kept for the whole stroke. It starts with `canvas_width`,
  `canvas_height`, `brush_size` and `zoom`; scripts add their own fields.
- Each function returns nothing or an array of points. A point is
  `[x, y]`, `[x, y, pressure]`, `[x, y, pressure, track]`, or a map with `x`, `y`
  and optional `pressure`, `track` (0–1023) and `lift`.
- Every track is a separate line with its own spacing, stabilizer and pressure
  dynamics. `lift: true` ends that track's line at the point; its next point starts
  a new one. When the pen lifts, every open line ends where it last was.
- Limits: 32 KiB of source, 200,000 script operations and 4096 returned points per
  call, 64 tracks per stroke, coordinates within a million pixels. Native brush
  generation also has a **4096-dab allowance per sample, lift or stabilizer
  catch-up update**, shared across all tracks and hooks in that update. This is
  checked while generating dabs, before output arrays can grow beyond the limit;
  brush spacing and pressure can make one returned segment generate many dabs.
  An over-budget update contributes no dabs. Earlier painted updates stay as one
  undo step, the tool stops, and the status bar asks for shorter paths or wider
  spacing. Other script errors also stop the tool. `print` does nothing.

A scripted tool stays in use until another tool is chosen; changing brushes keeps
it. It is highlighted in the tool rail. Installation needs the `tools.script`
capability, which the install prompt describes as running code while drawing.
The example's source is in `plugins/sdk/scripted-tools/`: Parallel lines (three
tracks), Hatching (`lift`), Coil (state in `this`) and a hatching eraser.

## Panels

A panel has `id`, `title` and `widgets`. Declare `ui.panels`. Widgets have a `type`:

- `label` with `text`; `separator` with no fields.
- `button` with `command`, referencing a command in the same package.
- `brush_size`, `opacity`, `color`: native brush controls; require `brushes`.
  Color becomes mask coverage when a layer mask is targeted.
- `snapping`, `guide_lock`: native guide controls; require `tools`.

Panels are scrollable native windows with independent positions and stable IDs.
The host draws widgets; plugin code does not execute during layout. Commands
resolve against the currently installed, enabled package when clicked. Disabling
or uninstalling removes tools and closes panels. Already selected brush settings
remain usable; already running filters retain API 1 cancellation semantics.
Panel visibility is session-local. Native settings use their existing persistence.

## Build and validate

Start with `plugins/sdk/studio-kit/manifest.json`. Package without overwriting an
existing output:

Filter and scripted-tool commands may name a `.rhai` file beside the manifest
with `"source_file"` instead of `"source"`; packaging inlines it.

Brushes (in `brush` commands and `brush` tools) may name PNG files for their
textures: `"image_file"` for the tip (which also sets `"tip": "Image"`) and
`"grain_file"` for the paper grain. Packaging asks Dudel to convert them exactly as
**Brush settings > Import tip/grain** does: dark marks on white or transparent
become paint, tips are padded square, and both are scaled to at most 128 px. With
flow below 1 a grain shows through overlapping dabs; at full flow it only shows at
the stroke's edges. `example-textured-brushes.dudelplugin` (source in
`plugins/sdk/textured-brushes/`) has a stamped tip with scatter, a grainy pencil and
a chalk brush with both. Packages over 256 KiB are written without indentation.

```powershell
python scripts/plugin_tool.py plugins/sdk/studio-kit studio-kit.dudelplugin --executable target/debug/dudel.exe
target/debug/dudel.exe --check-plugin studio-kit.dudelplugin
```

Validation does not execute programs. Installation validates capability grants,
brush data, script/module shape, unique IDs and panel references before atomic
publication. Updates preserve enabled state. Packages cannot request filesystem,
network, process access, native DLL loading or arbitrary document mutation.

This is the first contribution API, not the complete long-term plugin platform.
Arbitrary UI layouts, importer/exporter hooks,
dependency resolution, plugin-specific settings and live document scripting
still need separate versioned contracts. They must preserve the existing edit
queue, undo, cancellation and document lifetime rules.
