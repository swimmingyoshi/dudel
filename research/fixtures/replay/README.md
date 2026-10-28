# Stroke replay fixtures

`hard-square.dudelreplay` is a synthetic, independent rendering oracle, not a
physical tablet capture. `generate_square.py` deterministically rebuilds it:

```powershell
python research/fixtures/replay/generate_square.py
target/release/dudel.exe --replay-input research/fixtures/replay/hard-square.dudelreplay --batch-size 1 --report replay.json
target/release/dudel.exe --replay-input research/fixtures/replay/hard-square.dudelreplay --batch-size 64 --work-budget 64 --report sliced.json
```

On an 8x8 canvas, a stationary 4px hard square with pressure effects disabled paints
exactly sixteen `[17,83,201,255]` pixels at x/y 2..5. An existing `[9,8,7,0]` pixel at
the origin remains byte-identical. Expected pixels are constructed directly in
Python. The known pressure is zero, tilt/twist and monotonic timing are present.
Expected full RGBA FNV-1a checksum: `5075632377dbe46b`.

The v1 fixture starts with `DRPL\x01\r\n`, then three little-endian u32 length-prefixed
chunks: JSON trace metadata, starting v4 project, expected v4 project. Metadata is
limited to 32MiB, each compressed project to 20MiB, and trailing data is rejected.
The trace stores captured settings and ordered point/seed/reset calls plus a
cancellation flag. Point calls retain raw screen samples and view transforms;
seeds are document-space Shift-line anchors. Base/result have one active raster
layer with matching dimensions, selection and alpha-lock state. Each pixel payload
must fit 16MiB; traces must contain 1..65,536 calls. Both v4 chunks validate their
gzip checksums, lengths and sparse tile ordering through the ordinary project reader.

The replay uses the production sampler and buffered painter, finishes or restores
the stroke, and compares every output pixel against the oracle before writing
anything. Packet batch boundaries introduce no timing or additional samples.
Canvas integration tests separately cover captures of richer live canvas strokes.
Release timing of this tiny fixture does not establish tablet responsiveness.
`--work-budget` tests rendering slice boundaries independently of packet batches.
The allocation peak remains nonzero when a prepared raster is created and freed
within a single call. A maximum-step measurement excludes sampling, cancellation,
reclamation and history; total CPU rendering includes the applicable operations.
