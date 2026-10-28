# Indexed storage evaluation: commit primitive

This is an experimental journal protocol, not the next public Dudel project
format. Normal saves/recovery still use the existing formats and atomic staging.
Raster CSP import has a validated flat subset; folder/clipping rendering and
physical hardware gates remain open. This work evaluates the subsequent storage
direction independently and does not claim that those gates have passed.

## Prototype layout

All integers are little endian. Header length is 12,288 bytes: eight-byte private
magic `DIDX\x02\r\n\0` and two 64-byte root slots at offsets 4,096 and 8,192.
All other header bytes are reserved zero. Root updates occupy separate 4 KiB
pages, isolated from payload appends. Separation assumes write units no larger
than 4 KiB; larger units and real device behavior require validation before use.
No
existing project or CSP file is accepted as a journal. Creation uses create-new.

Each root contains `DIXROOT1`, generation u64, manifest offset u64, manifest size
u64, committed file length u64, manifest CRC32 u32, root CRC32 u32, and 16 reserved
zero bytes. Root CRC covers bytes before/after its checksum field. Generation N
uses slot N modulo two. The other slot retains the preceding generation.

Payloads append after the committed end. The full replacement manifest follows
them: count u32 then sorted unique 24-byte entries (key u64, payload offset u64,
length u32, CRC32 u32). An omitted key is deleted; unchanged keys retain their
previous offsets. The manifest end must equal the committed length. Referenced
payloads must be nonempty, bounded, and entirely before the current manifest.
CRC32 detects accidental damage; it is not authentication of untrusted files.

Prototype bounds: 1 MiB per record, 8,192 keys, 600 MiB aggregate live referenced
bytes, 2 GiB physical journal. These are journal budgets, not increases to canvas
or layer limits. There is no tile codec or document schema in this primitive.

## Commit order

1. Hold a nonblocking exclusive OS file lock. Validate every requested key, size,
   aggregate and offset before writing. Duplicate changes are errors.
2. Trim only an uncommitted tail from the independently confirmed base end.
   Retain the data and manifests reachable from both existing roots.
3. Append changed payloads and the complete new manifest.
4. Call `File::sync_all` and require success before publishing the new root.
5. Replace the alternating root slot, then call `sync_all` again.
6. Only after both sync calls succeed advance the in-memory confirmed generation.

An I/O error or injected interruption makes the handle unwritable. It must close
and reopen before any retry: a reported failure after root publication can still
have persisted the new generation. Reusing stale pointers could otherwise truncate
an already published commit. Shared read locks and exclusive write locks are
nonblocking; no UI thread is meant to wait for a lock.

## Open, corruption, and recovery

Read at most the bounded header and two manifests to choose the latest valid
generation. Validate root checksums, offsets, lengths, generation chain, manifest
checksum, sorted keys, reference extents and live-byte budget before allocation.
Payload reads are random access, bounded, and checksum-verified individually.
A future document reader must validate every required object before publication.

An invalid nonzero root slot with another valid slot exposes recovery state.
The prototype permits reading the remaining generation but refuses to append;
repair must be an explicit future operation preserving the damaged source.
Corrupt referenced payloads return an error instead of silently rolling artwork
back to an older generation. No generation is inferred from uncommitted tail bytes.
No valid root means no confirmed generation, including interrupted first creation.

The tests inject failures around write/sync boundaries and damage pointers and
payloads on temporary files. They test protocol behavior, not real power cuts or
storage-device flush compliance. Directory-entry durability for new creation and
atomic compaction replacement requires separate platform validation. Rust API
contracts: [file locks and sync_all](https://doc.rust-lang.org/std/fs/struct.File.html).

## Required next layers and release gates

- Specify a versioned document manifest containing persistent layer identities,
  metadata, tile keys, selection pages and a bounded thumbnail. Preserve folders,
  CSP byte opacity, hidden RGB, selection coverage and all current project fields.
- Encode tiles independently with exact decoded-size, checksum, codec and edge
  geometry checks. Opening a file must not require a flat layer allocation.
- Define changed-tile detection independent of transient Arc identities. Snapshot
  sources, worker lifetimes and queued edit/save/replacement fences must remain
  ordered and accounted for.
- Compact into a new same-directory file from one confirmed generation; reopen and
  verify it before atomic replacement. Retain the original on every failure.
  Bound transient memory and disk headroom; never compact by deleting the source.
- Test index/reference corruption, torn writes at byte boundaries, partial payload
  and manifest writes, sync failures, competing processes and uncertain retries.
  Add subprocess termination and actual filesystem/device experiments separately.
- Benchmark supported sparse and dense documents: unchanged/one-tile/full saves,
  index open, selected tile read, thumbnail load, recovery and compaction. Include
  compression/index overhead, physical growth, IO and memory costs.
- Keep v3/v4/JSON/recovery readers. Do not switch normal saves until complete
  document round trips, durability, compaction and drawing responsiveness pass.

Synthetic runner: `--study-indexed-commits [NEW_REPORT.json]`. It creates only a
temporary generated journal and an exclusive-create numeric report; it never
opens user projects. This is evidence for a commit building block, not incremental
project saving, format compatibility or power-loss certification.

## Initial evidence (2026-10-14)

`benchmarks/indexed-commit-release-1.json` reports two synthetic generations,
536,670 bytes after the first commit and 262,220 appended bytes for one changed
256-square RGBA record, including the replacement index. The other record's
offset is reused and both payloads reopen exactly. Appended byte count excludes
the 64-byte root overwrite and is not a write-time or throughput measurement.
The complete suite passes 475 tests with one pre-existing large-memory test
ignored; Clippy with denied warnings and formatting/whitespace checks pass.

Seven primitive tests cover reuse/deletion, fault checkpoints surrounding both
syncs, every root-prefix length, every truncated append length of a small fixture,
damaged payload rejection, checksummed malicious manifest references, competing
writer exclusion and invalid transaction preservation. A damaged root leaves the
older valid generation readable but blocks append; uncertain handles cannot retry.
These temporary-file tests do not emulate drive/controller behavior or a full
process/power failure. All document format and integration gates above remain open.

## Independent tile codec evaluation

`src/indexed_tile.rs` adds an experimental, independently decoded straight RGBA
tile record. It is not used by v3/v4 or normal saves. Tile geometry is 1..256 on
each axis, including cropped edge tiles. The parent document schema must supply
the expected geometry; matching byte count alone is insufficient.

The 32-byte little-endian header contains eight-byte `DITILE01`, width u16,
height u16, channels u8 (4), codec u8 (0 raw, 1 zlib), reserved zero u16, decoded
length u32, payload length u32, decoded CRC32 u32, and header CRC32 u32. Header CRC
covers the preceding 28 bytes. There is no implicit color/profile conversion.

Encode omits only tiles whose every RGBA byte is zero. Hidden nonzero RGB under
zero alpha is meaningful source data and is retained. Zlib at fast compression
is used only when smaller than raw data; otherwise the raw record prevents
compression expansion. Absence maps to deletion of the corresponding journal key.

Decode bounds the complete record to 32 + 262,144 bytes and verifies version,
geometry, channels, codec, reserved bits, lengths and header checksum before
allocating decoded pixels. Zlib output has a tile-size bound plus one byte to
detect overflow. End-of-stream, exact output length and exact input consumption
are mandatory, rejecting truncation, concatenated streams and trailing bytes.
Decoded CRC is checked for both codecs; explicit all-zero records are rejected.
Only a complete valid tile is returned. No full layer allocation is needed.

`--study-indexed-tiles [NEW_REPORT.json]` evaluates solid/noise/partial-edge/
hidden-RGB/empty fixtures, then commits encoded tiles through the journal and
reopens changed and retained records. A subsequent all-zero edit deletes its key
and reopens the remaining tile exactly. It uses only generated temporary data and
numeric reports. This is not a project writer, latency benchmark or device crash
test. Metadata, selection pages, stable layer keys, thumbnails and compaction are
still required before any editor integration or format rollout.

`benchmarks/indexed-tile-release-1.json` records exact results for all five
fixtures: solid 262,144-byte RGBA uses 1,255 record bytes; deterministic noise uses
262,176 (raw plus 32-byte header); 3x17 edge uses 53; hidden RGB uses 36; all-zero
tile is omitted. The encoded changed record plus the two-key manifest appends
1,307 bytes. The unchanged offset is reused, both tiles reopen exactly, and the
subsequent empty-tile deletion is verified. Counts exclude the root overwrite and
are not representative-artwork compression ratios or latency guarantees.

Four codec tests cover exact edge/channel/alpha bytes, both codecs, empty omission,
all truncations of a compressed fixture, each header byte, resealed malformed
headers, corrupted checksums, over/under-sized decoded streams, concatenated
streams and explicit empty records. A Python zlib/CRC32 vector independently
checks decoder interoperability. Full suite: 479 passed, one existing large-memory
test ignored; warning-denied Clippy and formatting/whitespace checks pass.

## Experimental document object schema

`src/indexed_document.rs` introduces a private schema independently of editor
save/open. Object zero is a bounded (4 KiB) JSON manifest with schema version 1,
ordered persistent layer keys, a next-key counter, and an explicit selection-mask
presence flag. Unknown manifest fields and versions are rejected. Object one is
the existing bounded native metadata JSON, including all current layer properties,
folder hierarchy, active index and rectangular selection bounds, with no pixels
or mask payload embedded. Its 1 MiB limit remains unchanged.

Layer keys are positive unique u32 values, independent of process-local LayerId.
Current layers retain keys across reorder; newly introduced identities consume a
monotonic counter. Removed keys are never recycled. A deleted and later restored
runtime identity may receive a fresh storage key; no inactive history is retained
in this mapping. Reload mints fresh runtime LayerIds and binds them to persisted
storage keys, preserving subsequent object addressing without serializing runtime
identity. The caller advances key state only after successful commit.

Raster keys use top-two-bit tag 01, layer key in bits 16..47, and row-major tile
index in bits 0..15. Reserved bits, unknown owners, folder-owned pixels and tile
indices beyond the validated canvas grid are errors. Payloads are independently
encoded `DITILE01` records. All-zero tiles are absent. No CRC-only equality or
transient allocation identity is used to infer that data is unchanged.

Selection-page keys use tag 10 and a low-16-bit grid index, with all other bits
zero. Payloads are currently raw single-channel coverage, not expanded RGBA:
`DIMASK01`, width u16, height u16, payload CRC32 u32, then exactly width*height
bytes. Geometry must match the cropped 256-pixel grid cell. All-zero pages are
omitted, but manifest presence distinguishes an empty selection from no selection.
After reconstruction, mask bounds must equal native metadata selection bounds.
Coverage page compression is not implemented at this checkpoint.

The encoder visits one owned encoded object at a time and does not retain the
document's complete encoded tile set. Invalid source geometry is rejected before
emitting objects. The reader validates the entire bounded sorted object inventory
and ownership before allocating artwork, decodes into sparse Pixels, checks every
required payload, then validates the complete document before returning it.
Cancellation and late output/decode errors return no document/key-state result.
The record provider must enforce its allocation bound before returning bytes;
Journal.read does so. Selection reconstruction allocates at most the current
four-megapixel mask. Metadata parsing is bounded and temporary raw metadata is
released before pixel reconstruction. No complete layer is flattened.

`--study-indexed-documents [NEW_REPORT.json]` composes this schema and the journal
using a small generated document. It intentionally retains complete encoded maps
in its driver for exact comparison; this is not the eventual bounded streaming
save worker. It tests initial reopen, fresh runtime-ID mapping, reorder/rename/
one-tile edit, exact changed-object detection and a second reopened document.
Thumbnail, streaming commit integration, compaction, failure-injected document
publication and full lifecycle/hardware gates remain open. Normal saves are still
v4 and existing legacy/recovery readers remain unchanged.

`benchmarks/indexed-document-release-1.json` verifies a generated 257x259 document
with two raster leaves and one folder, fractional CSP leaf opacity, native folder
opacity, blend/clipping/lock/visibility/collapse properties, hidden RGB, cropped
tiles and feathered selection coverage. Eight objects occupy 80,374 bytes after
the initial commit. Reorder, rename and one-tile edit change three objects and
reuse five, appending 953 bytes excluding root overwrite. Initial and edited
documents reopen exactly, including mask bytes, and keys remain stable after
runtime IDs are regenerated. These sizes describe this fixture, not dense-world
compression, memory or throughput performance.

Three schema tests cover full property/mask round trips and native composite
equality; empty-versus-absent mask; fresh runtime identity; key preservation under
reorder and monotonic allocation after deletion; invalid versions/counters/owners/
indices/folder tiles; corrupted payloads; missing required records; mask-bounds
inconsistency; preflight/late cancellation; late visitor failure; and source tile
geometry mismatch before emission. Full suite: 482 passed, one existing memory
test ignored; warning-denied Clippy, formatting and whitespace checks pass.

## Streaming full-inventory transaction

`Journal.replace_stream` now connects an object producer directly to journal
appends. `replace_document` connects the document visitor without retaining full
encoded maps. A new bounded reference map starts empty: every emitted key is part
of the new inventory, and old keys absent from it are removed on commit. Duplicate
keys, excessive counts, empty/oversized records and live/file budgets are checked
incrementally. A failure may leave tail bytes but cannot publish that partial
inventory. Both pointers' referenced data remain intact until normal publication.

For an existing key with equal length and CRC, the sink reads and verifies the old
record and compares the complete encoded bytes. Only byte equality reuses its
offset; CRC alone is never sufficient. Each comparison record is dropped before
the next emission. Old and new reference maps are bounded by 8,192 entries. Source
objects are emitted one at a time; no encoded layer/document map is retained.

The sink accepts at most 1 MiB of record data with a 2 MiB incoming allocation
capacity. Its old-record comparison allocation is at most 1 MiB. Thus sink-owned
record allocations have a 3 MiB cap, separate from encoder raw/compression scratch,
metadata/index allocations and source document memory. Reported peak capacity
counts incoming and old-comparison record allocations only; it is not a measured
process-memory or complete worker bound. The codec's tile and metadata limits
remain independent. Future worker registration/global snapshot accounting is
still required before editor integration.

Manifest publication is shared with the original change-list primitive. It seeks
explicitly to the append position after any reuse reads, writes the index, syncs
payload/index, writes the alternating root, then syncs again. Both write paths
mark their handles unwritable before I/O or callbacks, so Result failures and
panic unwinding cannot leave a retryable stale generation. Only confirmed success
restores writability and publishes in-memory index/key state. Close/reopen is
required after every uncertain failure. Cancellation is checked by the document
producer and at the pre-publication checkpoints. Once root writing starts, an
error can still mean a complete new commit persisted; reopening resolves it.

`--study-indexed-stream [NEW_REPORT.json]` runs generated initial/unchanged/edited/
removal saves and reopens documents with the streaming path. It preserves the
older complete-map study as a separate baseline. These commands do not change
normal saves or open user projects. Thumbnail, compaction and full lifecycle/
durability/worker integration gates remain open.

Four added tests exercise exact reuse against an independent Python-generated
CRC32 collision; arbitrary emission order and unchanged-only publication cursor;
late producer/cancellation/duplicate/capacity failures followed by exact old
document reopen and safe subsequent save; document failures at all four sync/root
boundaries; cancellation before publication; and panic poisoning in both write
paths. Existing raw-prefix, pointer, payload and malformed-index tests also pass.
The full suite passes 486 tests with one existing memory test ignored; Clippy with
denied warnings and formatting/whitespace checks pass. These remain temporary-file
protocol tests, not actual power-cut certification.

`benchmarks/indexed-stream-release-1.json` verifies initial, unchanged, edited and
removal generations without complete encoded maps. The small fixture initially
occupies 80,374 bytes. An unchanged inventory reuses all eight records and appends
196 index bytes; reorder/rename/tile edit changes three and reuses five, appending
954 bytes. Layer/selection removal deletes four objects. All reopened documents
match exactly. Observed sink-owned record allocation peaks are 65,552 bytes for
the initial commit and 131,104 for reuse/edit; these exclude encoder scratch,
index maps, source documents and allocator overhead. They are not process memory
measurements. Appended counts exclude the root overwrite.

The visitor still scans and encodes all present tiles to establish exact equality;
this does not prove a dirty-tile CPU fast path or dense-document save latency.
The save worker/global memory registry, thumbnails, compaction, realistic dense/
sparse benchmarks and process/device failure validation remain rollout gates.

## Bounded committed thumbnail (private schema v2)

The experimental document writer now emits manifest version 2 with `thumbnail:
true` and required object key 2. Version-1 documents without thumbnails remain
readable; unknown versions and inconsistent flag/key combinations are errors.
The normal v3/v4/JSON/recovery formats and save paths are unchanged.

Thumbnail geometry preserves aspect ratio with integer rounding, never upscales,
and limits the longest axis to 256 pixels (at least one pixel on each axis).
Each output pixel samples the nearest source pixel center through the existing
native white-canvas compositor. That includes visibility, opacity, imported byte
opacity, clipping, folders and blend modes; it excludes selection/UI overlays.
Output is opaque RGBA. This is a representative nearest-center preview, not an
antialiased reconstruction, transparency-preserving export or CSP render proof.

Generation performs at most 256-squared sampled compositions and allocates at
most 262,144 raw output bytes, independent of source canvas area. It never flattens
a complete source layer or composites a full-size canvas. Source metadata/pixel
geometry is validated first; cancellation is checked before, per output row and
before encoding. The independent tile codec bounds encoded and decoded records
and verifies checksums, geometry and opaque output. Its maximum record size is
262,176 bytes. Raw/compression scratch and source/index memory remain separate
from this raw output bound.

The thumbnail is emitted in the same full inventory and root publication as the
document. Metadata-only edits may reuse its exact record; sampled pixel edits
update it. The thumbnail reader fetches only objects 0, 1 and 2 from the confirmed
journal generation. It does not touch raster/selection payloads, so reading a
preview is not a health check of the full project. The full v2 document reader
also validates the required thumbnail before returning artwork. Cache-damage
recovery policy remains a rollout concern; no source is silently repaired.

Three thumbnail tests compare every sampled pixel with native full composition
on a generated scene with overlapping clipping/byte opacity/folder opacity, test
blank and thin/large aspect-ratio bounds, and reject cancellation, truncation,
wrong geometry and transparent records. A schema test covers preview-only record
access, v1 compatibility, missing/corrupt v2 thumbnails, and the distinction
between readable previews and corrupted raster payloads.

`--study-indexed-thumbnails [NEW_REPORT.json]` verifies committed preview reads,
exact native pixels, reuse after a metadata edit and changed pixels/record after
a sampled artwork edit. It uses only generated temporary data and numeric reports.
UI preview integration, compaction, worker lifecycle/global memory accounting,
dense/sparse workloads and real device/process failure tests remain open.

Release evidence: `benchmarks/indexed-thumbnail-release-1.json` verifies a 254x256
preview, 1,261-byte edited record, exact initial/edited pixels, metadata-only
offset reuse and changed record after a sampled pixel edit. Read keys are exactly
0/1/2. `benchmarks/indexed-stream-thumbnail-release-1.json` verifies v2 initial/
unchanged/edited/removal saves: nine initial objects, all nine reused on unchanged
save (220 index bytes appended), three changed/six reused on edit (995 appended),
four removed on layer/selection deletion, exact document reopen throughout.
These generated fixture counts exclude root overwrites and are not memory,
latency, visual quality or dense-document throughput guarantees. The full suite
passes 490 tests with one existing memory test ignored; warning-denied Clippy and
formatting/whitespace checks pass. No normal save format has been switched.

## Verified compaction and pathname coordination

`compact_document` copies only the current confirmed generation to a same-directory
temporary file. It preserves all live encoded object bytes, persistent keys and
schema. The compacted generation is old+1; previous unreachable generations are
retired. It never deletes the original before replacement or compacts a damaged/
unconfirmed generation. A subsequent append must use the new physical index.

Source validation now shares the full document parser in validation-only mode:
all codecs, thumbnail, owner/grid references, selection-page checksums and aggregate
mask bounds are checked without constructing raster layers or a canvas-sized mask.
Encoded records stream individually, with existing record/live/file budgets.
The staged on-disk index is independently parsed, every record is compared exactly
with the source, and the staged document is validated again before publication.
Basic filesystem permissions are copied; arbitrary ACL/security metadata
preservation remains a platform rollout concern.

Windows tests rejected overwriting an open destination even when its byte lock
was absent. Compaction therefore closes that old data handle only while holding
a separate exclusive pathname lease. The staged/replacement data handle remains
locked throughout the rename. All current writers acquire the same lease before
opening data; readers acquire a shared lease when one exists. Read-only copies
without a lease remain readable using the shared data-file lock. A first writer
creates a lease and still must acquire the data lock, so existing readers block it.

The persistent empty sibling `.<filename>.didx-lock` is deliberately not removed
at close. Removing/recreating a lock file could split coordination between inode
generations. Windows opens it without delete sharing so its pathname cannot be
replaced while held. Nonempty/symlink/nonregular lease files are refused without
altering their contents. Lease support is part of private container v2; older
experimental v1 containers are rejected by this writer, and older v1 writers
reject v2 magic instead of bypassing the new protocol. These were temporary-only
prototypes, separate from public v3/v4/JSON/recovery formats.

Before replacement, compaction verifies that the destination still identifies
the source file and is a regular, singly linked file. Final symlinks, hard-link
aliases, missing/unusable identities and changed destinations are refused. Windows
identity uses volume serial/file index/link count from
[GetFileInformationByHandle](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-getfileinformationbyhandle).
Canonical pathname/handle checks protect cooperating operations and detect tested
external path swaps; they do not certify hostile namespace races or all network
filesystems. Replacement uses `TempPath.persist` in the source directory.

The source and compacted copy are byte-for-byte equivalent in live objects before
the old handle closes. On a failed replacement, the original stays intact and the
temporary path is retained until its open data handle closes, then cleaned up.
On a failure after replacement, a complete new generation may already be present.
Handles stay unwritable after all errors or panic unwinding; close/reopen resolves
the namespace outcome. No callback/fallible operation occurs between the old data
handle handoff and the persistence attempt except that persistence operation.
After success, the new data handle/index are retained and the pathname lease stays
held. Data sync is requested again; directory-entry/device power-cut durability
still needs separate platform experiments.

Four compaction tests cover reclamation and following-save correctness, competing
reader/writer exclusion at the handoff, all six copy/sync/verify/replace failure
checkpoints, cancellation, real Windows replacement failure from an external open
handle, delayed-stage cleanup, source path swaps, hard links, CRC-valid invalid
tile codecs, pre/post-replacement panics, persistent lease protection, copied
read-only files and foreign lease-data preservation. Validation-only parsing also
passes the schema corruption scenarios used by full document loading.

`--study-indexed-compaction [NEW_REPORT.json]` performs generated multi-revision
compaction, exact reopen and a subsequent save/reopen. Normal save/recovery and
painting paths remain unchanged. Full worker/global memory integration, larger
sparse/dense lifecycles, process termination, filesystem metadata, and real
device/power-cut validation remain required before format rollout.

Release evidence: `benchmarks/indexed-compaction-release-1.json` reduces a generated
seven-generation, nine-live-record journal from 101,761 to 81,682 bytes and
publishes generation 8. Exact document reopening and a following save/reopen both
pass. The full suite passes 494 tests with one existing large-memory test ignored;
warning-denied Clippy, formatting and whitespace checks pass. These are small local
Windows filesystem fixtures, not dense-workload, hostile-alias, directory-sync,
cross-platform ACL or physical power-loss certification. The accepted painting
executable and normal project formats remain unchanged.

## Supported workload lifecycle harness

`--benchmark-indexed-lifecycle [NEW_REPORT.json]` generates three supported
2048-square workloads: sparse 20-layer, dense patterned 20-layer and dense
deterministic-noise 32-layer. The last fixture reaches the current maximum logical
RGBA payload (512 MiB); it does not increase canvas/layer limits. Hidden RGB,
fractional/byte opacity and selection coverage are retained. Source construction
uses individual tile writes after validating supported geometry/layer counts.

Each case measures current-v4 save/load and experimental initial save/load,
thumbnail-only load, unchanged save, sampled one-tile/metadata edit save/load,
verified compaction and compacted load. Every loaded document is compared for
complete metadata, exact RGBA and exact selection coverage. The small edge-case
test covers both storage formats and the entire harness. Real benchmark artifacts
are numeric; all generated project/journal files live in temporary directories.

Load timings end before whole-document equality comparisons. All timings include
ordinary filesystem effects: caches are affected by the preceding saves/reads and
are not flushed. These are single-run storage measurements, not cold-disk, GUI,
tablet, GPU or power-cut tests. Source pixel capacity and snapshot-registry bytes
are component accounting, not RSS/peak working set. Sink record allocation excludes
encoder/index/source memory. The harness checks that source pixel capacity does
not grow across storage operations; it does not infer process memory from that.
Append byte counts exclude root overwrites and zero-byte pathname lease files.

`benchmarks/indexed-lifecycle-release-1.json` passed all three supported cases,
including the 512 MiB logical raster maximum. Source pixel capacities remain
unchanged at 10/320/512 MiB; registry live allocation accounting including masks
is about 14.0/324.1/516.1 MiB. Every v4/indexed initial, edited and compacted load
matches complete native metadata, RGBA and mask bytes. No larger limit is enabled.

Single-run timings in milliseconds (not cold-cache or UI latency):

| Fixture | v4 save | Indexed first save | Indexed unchanged | Indexed one-tile edit | Thumbnail load |
| --- | ---: | ---: | ---: | ---: | ---: |
| Sparse 20-layer | 10.9 | 67.0 | 63.7 | 62.8 | 1.1 |
| Dense patterned 20-layer | 4699.0 | 4695.5 | 4648.3 | 5177.2 | 2.7 |
| Dense noise 32-layer | 6182.1 | 7106.6 | 6271.6 | 6281.7 | 2.6 |

Unchanged saves reuse 45/1,285/2,053 records and append only 1,084/30,844/49,276
index bytes. Edits change three records in each case and append 6,911/365,984/
510,808 bytes. That is less rewriting, but re-encoding all present tiles keeps
dense save CPU cost near a full v4 save. The format is not ready to replace v4.
Sparse files are also larger (199,599 versus 68,003 bytes), principally because
current selection pages are raw while v4 compresses their zero coverage globally.
Dense file sizes are comparable: about 288.6 versus 288.9 MB and 476.7 versus
476.3 MB. Verified compaction takes 25.7/3903.3/6918.3 ms in this run and preserves
exact reopens. This is evidence against a premature default-format switch.

The next performance work is confirmed-source dirty-tile reuse across real save
worker lifetimes, with strict file/generation binding, mutation invalidation,
bounded non-owning descriptors and validated fallback on cache misses/reopen.
Persistent addressing must remain independent of allocation identities. A warm
cache-only benchmark must not substitute for reopen behavior or permit CRC-only
source equality. Selection-page compression and sparse thumbnail overhead are
secondary measured opportunities. Full worker/process-memory, hardware and
durability gates remain open. Full tests: 495 passed, one existing memory test
ignored; warning-denied Clippy and formatting/whitespace checks pass.

## Reopen-safe decoded raster comparison

The first response to the dense-save measurement is a cache-independent path.
The document producer can now emit bounded raw RGBA tile objects. For each
existing raster key, the journal reads and validates its committed encoded
record, decodes it under the tile bounds, and compares both geometry and every
raw byte with the new source. An exact match retains its encoded record and
offset without compression. New/changed/resized tiles are encoded; all-zero
source tiles are omitted. Corrupt committed records cause an error, not automatic
healing. Duplicate emitted keys remain errors even when an earlier tile is empty.

This uses the currently locked journal's confirmed index, so no allocation-ID
cache or carried file/generation hint is trusted. It works after reopening with
freshly decoded pixel allocations and across separate source-worker lifetimes.
Persistent addressing remains unchanged. A same-length/CRC source collision must
still compare unequal bytes and be written. Metadata, thumbnail and selection
records continue through the encoded exact-byte comparison path. There is no
disk-format change. The original encoded visitor remains available as a baseline.

Counters distinguish raster encoding from reuse before encoding. Sink capacity
accounting now includes the incoming raw, old encoded and old decoded buffers
when all coexist, in addition to the prior encoded-record path; tile bounds keep
these within the existing 3 MiB sink budget. Encoder/index/source allocations and
process memory remain separate. The path still reads and decodes all present
committed raster tiles, and thumbnails still regenerate; no sublinear or zero-IO
save latency is claimed.

Three new tests cover fresh allocations, independent gated SourceWorker saves,
snapshot isolation under concurrent live edits and source-registry retained-pixel
accounting; raw CRC collision/geometry change/empty deletion/duplicate omission;
and rejection of an outer-checksummed damaged tile codec without publication or
source-file changes. Lifecycle assertions require zero raster encodes for an
unchanged save and exactly one for the one-tile edit. Performance must be measured
with the same supported workload harness, including reopen, before claiming gain.

## Compressed selection coverage (private document schema v3)

The experimental writer now emits manifest version 3, with a required thumbnail
as in v2. Raster encoding, stable keys, journal container and public project
formats are unchanged. Readers accept v1/v2 raw DIMASK01 coverage pages and v3
DIMASK02 pages; mixing a page layout with the wrong manifest version is an error.
Older experimental readers reject the new manifest version before artwork loads.

DIMASK02 uses a 32-byte header: magic[8], width/height u16, channels u8 (=1),
codec u8 (0 raw, 1 zlib), reserved u16 (=0), raw length u32, encoded length u32,
raw CRC32 u32 and header CRC32 u32 (first 28 bytes). Width/height are 1..256,
must match the cropped grid cell and bound raw output to 65536 bytes. Only a
strictly smaller compressed payload is emitted/accepted; otherwise raw coverage
is stored. All-zero pages remain absent. Empty-versus-absent selection presence,
feathered u8 values and metadata bounds preserve their existing semantics.

The decoder uses an output-length-plus-one sentinel and verifies exact stream
termination, consumed input, output length and checksums. Trailing/concatenated
streams, truncated payloads, geometry mismatches, unknown codecs/reserved fields
and encoded empty pages are rejected. No canvas-sized decompression buffer is
used; the full materializing reader still separately allocates its bounded canvas
mask, while validate-only compaction uses per-page buffers and scalar bounds.
Source raw, compressor scratch, encoded page and decoded page are distinct from
the journal sink accounting; this does not certify whole-process memory.

Schema3 validation: 500 tests pass (one existing memory test ignored), all-target
Clippy with warnings denied, fmt and whitespace checks pass. The supported release
lifecycle report is benchmarks/indexed-lifecycle-compressed-mask-release-1.json;
exact initial/edited/compacted loads and thumbnail/source-capacity checks pass
for sparse20, dense-pattern20 and dense-noise32. Sparse initial size199599 ->69205
bytes versus v4's68003; all three files shrink130394 bytes for the same two-point
selection. Sink peaks remain525550/749563/756973 bytes (codec scratch excluded).
Unchanged/edited raster encoding counts remain0/1. These are ordinary cached
single-run generated measurements, not general sparse-size parity or isolated
mask-compression latency improvements. Small raw pages have32-byte headers rather
than the legacy16 bytes. Normal saves and the accepted drawing build are unchanged.

## Locked-session raster proofs

The next optimization is deliberately scoped to the live locked Journal handle.
After successful document replacement it retains, for each present source raster,
the confirmed record reference, source canvas/cropped grid region, and a weak
Pixels RegionStamp. Its generation must match the Journal's current confirmed
index and its record must still equal that key's current reference. Only then
can matching source storage/tile identities skip source copying and old-record
read/decode/compression. These are immutable copy-on-write identities: writes
disassociate weak stamps or replace the affected tile. Equal checksums and reused
raw addresses without a live weak control block are never evidence.

Proofs are private members of the owning Journal; they cannot be exported into
another file or attached to a reopened handle. Moving that handle between
background workers preserves its locks and proof binding. Reopening starts with
no proofs and uses validated decoded byte comparison to rebuild them. Fresh pixel
allocations, geometry changes, replacements and edited tiles fall back too.
Compaction and generic low-level writes discard the inventory. A replacement
takes the previous inventory before any callback/I/O and publishes the new one
only after confirmed commit; errors or unwinding leave it empty and the writer
poisoned. No proof authorizes skipping validation during full document open or
compaction, nor certifies disk health against latent corruption during a locked
session. The optimization assumes the existing exclusive handle/lease protocol;
it does not support uncoordinated memory-mapped/external file mutation.

There are at most 8192 entries, with a checked 2 MiB descriptor-capacity budget
per published inventory; old and pending inventories can coexist during saving.
Their vectors and RegionStamp tile descriptors are counted separately from the
record sink. Weak Arc control blocks can remain until proofs retire, but artwork
Vec payloads are not strongly retained by proofs. These component counts are
not whole-process memory or allocator/control-block accounting.

Locked-session validation: 504 tests pass, one existing ignored, Clippy/fmt/
whitespace pass. benchmarks/indexed-lifecycle-locked-session-release-1.json keeps
reopened save timings and adds independently labeled same-handle background
workers. Sparse20/pattern20/noise32 warm unchanged43.1/66.5/136.1ms; one-tile
edit41.0/69.2/143.0ms. Reopened unchanged52.0/1754.9/2830.3ms. Warm unchanged
raster reads/encodes0/0, warm edits1/1, with40/1280/2048 and39/1279/2047
respectively skipped. Proof descriptor capacities10752/344064/393216 bytes;
record sink peaks are separate. All exact reopening/compaction/thumbnail and
source-capacity assertions pass. Tests include concurrent live edits with worker
handoff, undo/flat replacement, geometry/generation checks, low-level writes,
compaction, publication faults and source payload lifetime. The cache is not
integrated into normal saves; no UI/tablet, cold disk, process peak or device
power-loss certification follows from these generated single-run measurements.
Verification scope clarification: lifecycle thumbnail pixel equality is checked
against the initial source snapshot. Later full document loads validate cached
thumbnail codec/geometry and exact document metadata/RGBA/masks; the benchmark
does not independently recompare each later cached thumbnail to its compositor.
## Opt-in editor session (headless-tested only)

The developer launch flag --experimental-indexed-storage enables separate .didx
open/save actions. Default/public .dudel and recovery writers remain v4. The
same FIFO snapshot/SaveJob fence handles indexed saves; an epoch rejects completion
from a retired canvas, while newer same-canvas revisions keep modified/recovery.
A live Session owns Journal/Keys/canonical path and moves through SourceWorker;
its channel returns only a confirmed session, retaining no artwork source after
the worker ends. Descriptor diagnostics are separate from source registry bytes.

New files are staged in the same directory under the target's exclusive pathname
lease, committed and fully validate-only checked before persist_noclobber. The
stage TempPath is owned after the data handle in Journal drop order. Failure or
cancellation before final publication drops only that owned stage. Existing
sessions use in-place root publication; cancellation after its commit point is
not a rollback. Reopening after errors validates complete payloads before writing
and starts with no proof inventory. Runtime-ID mappings are not guessed: fresh
keys start above the committed monotonic counter. Save as changes target without
modifying its previous source. Foreign formats, symlink/multiple-link targets and
normal writes over indexed magic are refused by their respective guards. Existing
namespace/device/filesystem metadata limitations remain; this is not production
rollout or power-loss certification.

Guide metadata is captured from app state for either format. Its saved baseline
advances on confirmed success; revisions now include settled guide changes even
when artwork was already dirty, enabling ordinary v4 recovery to capture guides.
No guide edit enters artwork undo. Workspace test compilation still has the
handoff-protected mutable-binding error in indexed_session.rs; headless scratch
validation repairs that binding only (530 pass, one ignored, Clippy clean).
The actual workspace release candidate builds under target/measure separately.