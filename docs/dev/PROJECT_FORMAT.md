# Dudel project container v4

The project and recovery writer selects v4, v5, v6, v7 or v8 according to the features
present, as described below. Document schema is 1, or 2 with masks.
v1 JSON, v2 gzip JSON and v3 dense binary projects remain readable and are
converted on the next save. Old applications do not understand the v4 header.
Current limits are 1,000 raster layers, 2,000 entries, ten folder levels and a
shared 2 GiB raster/mask pixel budget; canvases are at most 16,384 pixels per side
and 100 megapixels in total.

## Layout

All binary integers are unsigned little-endian u32. The raw file begins with
seven bytes `DUDL\x04\r\n`, followed by exactly one gzip member. Projects with layer
masks use `DUDL\x05\r\n` (schema version 2), and projects where any layer has layer
effects (even switched off), an adjustment layer or non-default fill opacity use `DUDL\x06\r\n` (schema
version 1 or 2). Vector layers require `DUDL\x07\r\n`, taking precedence over
those headers. They retain stroke recordings as metadata and rendered raster
tiles for display. Text layers require `DUDL\x08\r\n`, taking precedence over all
the others. They retain their words and layout as metadata and rendered raster tiles
for display, so opening a project never needs its fonts. The layout after the header is the same. Builds that predate a
header refuse the file instead of opening it without data they do not know, so a
resave there cannot silently drop masks, effects, adjustments, fill opacity,
vector recordings or text. Its payload is:

1. Metadata byte length (1 to 16,777,216), then that many UTF-8 JSON bytes.
2. For each raster layer in metadata stack order: tile count, then that many tile
   records. Folder entries have no count or records.
3. Each tile record: tile index followed by packed straight RGBA8 pixels. There
   is no pixel-length field; geometry determines the exact byte count.

Metadata matches document schema 1 or 2 with every layer's `pixels` equal to `[]`.
It retains dimensions, active layer, selection and layer names/properties/folder
structure. A layer may carry `effects`, its layer effects bottom to top (each
`{enabled, effect}`; settings are range-checked on load, at most 32 per layer,
on raster layers and folders). `fill` is a finite number from 0 to 1, defaulting to 1;
it fades source pixels without fading their effects. The UI exposes fill on raster
layers. A raster layer may instead be an adjustment layer: `adjustment` holds
its settings (one of the documented kinds, range-checked on load); its pixels are
unused. Bevels carry a `pair` number shared by their highlight and shadow entries
(0 in files saved before pairing). It must validate before tile data is allocated. An optional `guides`
object holds drawing guides: shared symmetry (mode, copies, center, rotation),
per-layer symmetry as `[stack index, symmetry]` pairs, grid, guide lines, guide
lock and ruler visibility. Files without it open with default guides, and readers
that predate it ignore it. Untrusted guide values are repaired or dropped on load
rather than failing the document; guides are never part of undo history.

Tile side is fixed at 256. Columns = ceil(width / 256), rows = ceil(height / 256).
Index = tile_y * columns + tile_x. Tile count cannot exceed columns * rows;
indexes must be strictly increasing and inside that grid. The valid rectangle is
`[tile_x*256, tile_y*256, min((tile_x+1)*256,width), min((tile_y+1)*256,height)]`.
Pixels are row-major in this rectangle, with no edge padding. Payload length is
rectangle width * rectangle height * 4 bytes, at most 262,144 bytes.

All-byte-zero tiles are omitted. Alpha-zero pixels with nonzero RGB are data and
must be retained. The reader rejects explicitly empty records, duplicate or
unordered/out-of-range indexes, excessive counts, truncated records, bad metadata,
gzip checksum/footer damage, extra uncompressed payload and trailing compressed
bytes or members. Decoding allocates sparse tile grids and one bounded tile
payload buffer at a time; it does not create whole-layer compatibility buffers.
Compression is for the whole stream; independent tile compression, random access
and incremental save updates are not provided by this format.

## File-browser preview

Saved projects (not recovery snapshots) set the gzip header's FEXTRA flag and
carry one extra subfield with id `DP`: a PNG, or JPEG when the PNG would not
fit, of the composite on white paper, at most 256 px on its longer side and
never larger than the canvas. The whole extra field is capped by gzip at
65,535 bytes, so a reader finds the preview within the first 65,554 bytes
without inflating anything. Gzip decoders skip extra fields, so this needs no
container version change and older Dudel builds open these files unchanged.
The preview is advisory: loading ignores it, and a project without one is
valid. `src/file_preview_header.rs` is the reader, shared by the Explorer
thumbnail provider in `shell/thumbnails`.

## Storage and recovery

Projects use the existing same-directory staged write, file sync and atomic
replacement. Failure leaves the prior destination intact. This does not certify
power-loss durability or directory-sync behavior. The project file cap remains
4 GiB, 1,000 raster layers (32 before the layer-limit change), a 2 GiB stored-pixel budget and 16,777,216 pixels per layer (4,194,304 before 2026-10-22); all existing folder,
selection, dimension and metadata bounds still apply.

Recovery wrapper `DRCV\x01\r\n` retains its existing path length/JSON fields,
followed by a complete v4 project. Reading also accepts a nested v3 project and
legacy gzip-JSON recovery. Recovery files retain the 551 MiB cap. Background save
and load jobs share these storage entry points and their existing revision guards.

## Verification

Unit fixtures cover sparse partial edge tiles, hidden RGB, flat input adaptation,
blank layers, layer metadata/selection, v3 project migration and v3 recovery
wrappers, malformed records, checksums and trailing data. Existing storage tests
exercise v1/v2 migration, atomic replacement and v4 project/recovery round trips.
The maximum-size project/recovery test remains explicitly invoked separately.
Release reports compare v3 and v4 on identical supported sparse 2048-square,
20-layer artwork; they do not certify dense-document, UI or filesystem latency.

Guide persistence checkpoint (2026-10-14, headless-tested only): guide-only edits
now advance the persistence/recovery revision without adding artwork undo entries.
Save completion publishes only the captured guide baseline; failed saves and
newer guide edits retain unsaved/recovery state. The optional guides metadata and
v4 wire format do not change. The developer-only --experimental-indexed-storage
flag uses separate .didx files; normal project and recovery formats remain v4.

Reference layers (updated 2026-10-18): optional `references` metadata lists up
to eight reference boards. The existing primary image fields remain unchanged
(name, center, scale, rotation, opacity, visibility, lock, above/below, flip).
Optional `images` lists up to eleven additional placements, with the same
image/transform fields but no nested boards or stack positions. Optional
`position: {gap, depth}` places the board in the artwork stack. Gap is a saved
artwork index (0 is bottom); live positions bind to layer/folder identities and
are resolved back to indices on save and recovery. Session IDs are not persisted.

Legacy single-image, above/below references retain `REFS\x01`. Boards with
additional images or explicit stack positions use `REFS\x02`, so older builds
reject them instead of silently discarding order. Each trailer has: a u32 count equal
to the sum of all primary and additional placements, then a u32 length and PNG
per placement, board by board with each primary first. Each PNG has at most
4096 x 4096 pixels; total decoded reference RGBA is capped at 512 MiB to retain
the earlier eight-image maximum compatibility. New imports are reduced to
1536 pixels on the longest side and held to a separate 128 MiB project import
budget. Counts, nesting, metadata, dimensions and aggregate decoded bytes are
validated before publication. The trailer follows the optional selection trailer;
files without references keep their earlier bytes. Legacy single-image reference
files retain their metadata and image order. Older builds reject the new trailer tag before reading any image payload. Legacy v3 and experimental
indexed storage refuse reference boards. View-only sampled layers are injected
only into display snapshots and are explicitly rejected by artwork validation.

Layer symmetry persistence checkpoint (2026-10-14, headless-tested only): optional guides.layer_symmetry indices may refer to raster layers or folder headers. Both are sanitized for bounds and finite values. Duplicate entries receive independent copies of explicit settings; entries without overrides retain global inheritance. Indices are derived from the captured saved document stack, while live and queued editing uses session layer identities. No v4 wire/schema change.
