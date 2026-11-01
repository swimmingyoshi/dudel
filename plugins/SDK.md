# Rhai extension API 1

For brush/tool contributions, commands and panels, see [Extension API 2](API_2.md).

Open Filter → Manage extensions → Install and choose
`example-invert.dudelplugin`. It appears in the Filter menu under its name while
enabled; choose it (or Run filter in the manager) to apply it to the active layer. Undo reverses the committed result once. An active rectangle
selection constrains the pixels supplied to the filter.

A `.dudelplugin` is a UTF-8 JSON object containing the example's fields. `version`
is the package schema; `api` is the host contract. Both must be 1. `release` is
display metadata. IDs use lowercase ASCII letters, digits and hyphens (1–80 bytes).
Name/author support 100 bytes, release 40, description 1000. Metadata must be
nonempty and contain no control characters. Packages read at most 5 MiB; source
supports at most 32 KiB. Only `pixels.filter` is accepted in capabilities.

Define `filter(r, g, b, a)` returning four integer channels. Inputs are straight
RGBA bytes; output channels clamp to 0–255. Fully transparent pixels are skipped.
Alpha-locked layers retain their original alpha. Locked layers reject filters.
Do not rely on invocation for excluded pixels. Use no state between calls.

The host registers no filesystem, network, UI or document functions. Scripts run
with the standard Rhai package, 2000 operations per invocation, expression depths
32/16, call depth 8, arrays 64, maps 32 and strings 4096. Module resolution is not
provided. Failure or cancellation discards the pending result. Editing artwork
while a worker runs makes its result stale, which the host discards.

Install validates metadata, syntax and a four-parameter filter function without
executing the script. Return type/errors are checked during execution. The manager
stores packages under the platform's Dudel local-data `extensions` directory as
`.dudelinstalled` files. Installation/update and enabled-state changes use atomic
replacement. Updates preserve enabled state and request confirmation for an
existing ID. Disabling prevents new manager runs; it does not cancel a worker
already running. Uninstall cancels pending filter work. Maximum 64 installed filters.

Raw `.rhai` files remain supported by Filter → Run filter from file. This API
covers Rhai pixel filters. WASM pixel packages use the parallel contract in WASM_SDK.md; brush/tool presets and UI contributions use API_2.md.
