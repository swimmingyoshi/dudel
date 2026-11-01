# Developing and sharing Dudel plugins

Plugins use versioned `.dudelplugin` JSON packages. API 1 covers filters;
[API 2](plugins/API_2.md) adds brushes, native tool presets, commands and panels.
Install, enable, disable, run and uninstall them from Filter > Manage extensions.
Filter execution runs in a background job, respects the active layer and
selection/mask, commits atomically and supports undo/cancel.
Installed packages persist in Dudel's per-user data directory.

## Make a plugin

Copy `plugins/sdk/warm-paper` to a new folder. Change the manifest's ID, name,
author, release and description, then edit `filter.rhai`. IDs contain lowercase
letters, digits and hyphens. Keep an ID stable when publishing an update.
For this filter-only example, API and package version are 1. To contribute
brushes, tools or panels, start with `plugins/sdk/studio-kit`, use version/API 2,
and follow its [contribution contract](plugins/API_2.md). Arbitrary document
scripting and native pointer-event tool implementations remain future work.

```powershell
cargo build --locked
python scripts/plugin_tool.py plugins/sdk/warm-paper warm-paper-1.0.0.dudelplugin --executable target/debug/dudel.exe
```

On Linux use `target/debug/dudel`. The tool validates using the actual Dudel
host before writing a new package; it refuses to overwrite an existing package.
The package embeds its program, so recipients need only the `.dudelplugin` file.
Test on disposable artwork, including alpha, selections, locked layers, undo and
cancellation. Share the package plus its editable source and license in a tagged
repository release. There is no hosted marketplace or automatic updater yet.

## Rhai API

Define `fn filter(r, g, b, a) { [r, g, b, a] }`. Channels are integers 0..255.
The host skips fully transparent pixels, preserves alpha when alpha-lock is on,
and mixes results through selection coverage. Scripts have no filesystem,
network, shell, UI or document access. Source is limited to 32 KiB, with runtime
operation, expression, call-depth, collection and string limits. A failure or
cancellation discards the whole pending result.

## WASM API

Set `entry` to a compiled `.wasm` file. The same packager embeds the binary.
Export `dudel_api_version() -> i32` returning 1 and
`dudel_filter_rgba(i32) -> i32`. Pixels use little-endian RGBA bytes packed into
an i32. Modules cannot import host functions or WASI or use a start function;
the host applies memory, stack and fuel limits. Maximum module size is 1 MiB,
and the complete JSON package must fit 5 MiB. See `plugins/example-invert.wat`
and `plugins/example-wasm-invert.dudelplugin` for the exact calling convention.

## Compatibility

The host rejects unsupported API versions and capabilities before installation.
Installing asks the user first and lists what each declared capability allows;
a replacement marks capabilities the installed release lacked and preserves its
enabled state. Keep
API 1 compatible; introduce a new negotiated version for future tool/document
capabilities rather than granting existing packages more access. Plugin authors
must specify their own distribution terms; Dudel's repository examples are GPLv3.
