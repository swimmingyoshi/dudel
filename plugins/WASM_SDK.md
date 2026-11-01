# WASM pixel filter API 1

Build the included module with `cargo run --example build_wasm_filter`, then
choose Filter → Run filter from file → WebAssembly module and select
`plugins/example-wasm-invert.wasm`. Installed WASM packages, such as
`example-wasm-invert.dudelplugin`, appear in the Filter menu by name.

Binary modules must export:

```text
dudel_api_version() -> i32                 // must return 1
dudel_filter_rgba(rgba: i32) -> i32         // packed RGBA result
```

Each packed value uses R in bits 0–7, G in 8–15, B in 16–23 and A in 24–31.
The argument may appear negative because the packed bits are passed as an i32.
Treat it as four bytes, rather than clamping the whole integer.

The host invokes filters on nontransparent pixels of the active layer within the
selection, in row-major order. Alpha lock restores input alpha after each call;
locked layers reject the operation. One fresh instance is created per job, so
module state may persist between pixels of that job. Do not assume invocation
for fully transparent or unselected pixels.

Limits: binary size 1 MiB; no imports (including WASI); no start functions;
one memory capped at 16 MiB, one table capped at 1024 elements, one instance;
64-level recursion and 64 KiB value stack; 10,000 fuel units per API-version or
pixel invocation, with 100 million fuel units across filtered pixels. Fuel is an
interpreter work budget, not a wall-clock deadline. Excessive initial allocation,
growth or execution traps. Module compilation runs on the filter worker and has
no mid-compilation cancel hook. Pixel execution checks cancellation between calls.

Workers report progress and commit once only if the document revision is still
current. Failure, cancellation or stale results leave artwork intact. A committed
result supports one undo. Memory limits cover module linear memory and tables,
not the host's module compiler, layer snapshots or output buffers.

The example builder also creates `plugins/example-wasm-invert.dudelplugin`.
Install it through Filter → Manage extensions. WASM packages use the same metadata
as Rhai, with format `dudel-wasm-extension` and a `wasm` byte array replacing
`source`. Packages read at most 5 MiB; embedded modules remain capped at 1 MiB.
Installation checks binary syntax, imports and export signatures without executing
code. The returned API version and resource limits are checked on execution.
Command transactions, brush plugins and UI contributions remain pending.
