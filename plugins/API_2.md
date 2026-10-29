# Extension API 2

API 2 adds brush presets, tool presets, application commands and custom panels
to the existing sandboxed Rhai/WASM pixel filters. Install
`example-studio-kit.dudelplugin` through **Filter > Manage extensions** to try all
five contribution types. Brush and tool buttons appear in the tool rail. Open
contributed panels from the manager or the Filter menu.

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
| `tool` | `tool`: one of the tool definitions below | `tools` |
| `filter` | `source`: Rhai `filter(r,g,b,a)` | `pixels.filter` |
| `wasm_filter` | `wasm`: byte array using the existing WASM ABI | `pixels.filter` |
| `host` | `command`: supported application command | `commands` |

Host commands: `undo`, `redo`, `new_layer`, `deselect`, `transform`, `fit_canvas`,
`flip_view`. These call the application's normal action paths. Artwork edits
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

These are contributed configurations of native tool engines, not arbitrary
pointer-event programs. Brush selection does not duplicate entries in the user's
library. **Save as new preset** explicitly keeps a local editable copy.

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

```powershell
python scripts/plugin_tool.py plugins/sdk/studio-kit studio-kit.dudelplugin --executable target/debug/dudel.exe
target/debug/dudel.exe --check-plugin studio-kit.dudelplugin
```

Validation does not execute programs. Installation validates capability grants,
brush data, script/module shape, unique IDs and panel references before atomic
publication. Updates preserve enabled state. Packages cannot request filesystem,
network, process access, native DLL loading or arbitrary document mutation.

This is the first contribution API, not the complete long-term plugin platform.
Custom stroke algorithms, arbitrary UI layouts, importer/exporter hooks,
dependency resolution, plugin-specific settings and live document scripting
still need separate versioned contracts. They must preserve the existing edit
queue, undo, cancellation and document lifetime rules.
