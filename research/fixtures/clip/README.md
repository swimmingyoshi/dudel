# Read-only raster comparison

No user artwork or decoded images are committed here. The independent Python
oracle queries bounded temporary read-only metadata and converts one raster leaf
at a time in memory. It compares full row-major RGBA FNV-1a hashes plus nonzero,
partial-alpha and hidden-RGB counts. It writes only a new numeric JSON report.
Hash/count agreement is evidence for leaf conversion, not whole-document import
or rendering. Rust's reader separately validates framing and checksums.

Generate a report using the separate developer executable, then compare it:

```powershell
target/release/dudel-clip-raster.exe --verify-clip-raster FILE.clip NEW_RUST_REPORT.json
python research/fixtures/clip/verify_raster.py FILE.clip NEW_RUST_REPORT.json NEW_COMPARISON.json
```

The oracle needs NumPy and should run normally with assertions enabled. Its
supported sample subset is pixel-unit canvases within Dudel's limits, the observed
five-byte 256-square layout, resolved external references, and transparent raster
defaults. Canvas and bitmap geometry, partial source tiles and both signed offset
sources are handled separately. Non-raster objects, masks/effects, profiles,
opacity/blending, paper and animation are outside this comparison.

`borb`, `doodles`, and `vpet_001` produced 3, 6 and 6 matching raster leaf reports.
The ignored local artifacts use `csp-clip-raster-{sample}-1.json` and
`csp-clip-raster-oracle-{sample}-1.json` under `artifacts/`. Source and accepted
drawing-build SHA-256 hashes matched before/after. Synthetic Rust tests also
compare exact channel bytes, alpha/hidden RGB, offsets, edge clipping, grid stride,
sparse empty storage and atomic rejection/cancellation.

## Single-layer document oracle

A neutral visible single raster at full opacity and Normal/unclipped state can
also be checked as a complete transparent composite. Rust's preflight handles
other source features; this oracle zeros hidden RGB at zero alpha and retains
source RGBA at nonzero alpha. It does not generalize to paper, groups or clipping.

```powershell
python research/fixtures/clip/verify_raster.py FILE.clip RASTER_REPORT.json NEW_COMPARISON.json --single-layer-document DOCUMENT_REPORT.json
```

The authorized guy.clip sample matches leaf and composite hashes/counts across
its full 2048-square canvas. Its stored preview is reduced, so this comparison
is against source raster semantics, not a direct full-size CSP preview.

## Folder/clipping fixtures still needed

For exact CSP rendering comparisons, use a small pixel-unit canvas (for example
64 x 64) and export a full-size RGBA PNG after saving the matching .clip. Keep one
fixture per effect: overlapping colored semitransparent rasters in a Normal folder
at partial opacity; then a clipped colored raster over a base with opaque, partial
and zero alpha. A third fixture with nested folders helps distinguish isolation
and repeated opacity rounding. Avoid guides, masks, effects and color profiles so
unsupported features do not confound these comparisons. Retain files locally;
reports use numeric measurements and never copy source artwork to the repository.

Ordinary folders use a different type/flag pair from the root. The research CLI
--compare-clip-folder-caches studies saved full-scale caches where resolved; it
never treats a missing cache as blank. lighter/sketches2 lack these caches and have
reduced previews, so they do not establish exact folder rendering. Study handling
of their LayerMasking=32 is explicitly an unverified omission. Strict import stays
gated.

Full-size external reference command (numeric reports only):
  target/release/dudel-clip-export-check.exe --compare-clip-export FILE.clip REFERENCE.png NEW_REPORT.json
The reference must match canvas dimensions and is never resized. Confirm that the
PNG was exported from the same saved CSP state before using its metrics as format
evidence. Existing source/reference/report paths cannot be overwritten. Study
omissions remain explicit, import_ready remains false, and a match does not prove
unsupported features. Small controlled folder/clipping fixtures are still needed.
