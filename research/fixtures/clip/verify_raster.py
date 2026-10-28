"""Independent read-only raster oracle; compares numeric reports, never exports art.

Usage: python verify_raster.py FILE.clip RUST_REPORT.json NEW_COMPARISON.json
Requires NumPy for bounded array conversion. Supports only the observed five-byte
tile layout, pixel-unit canvases and transparent raster defaults used by samples.
Not an importer or general-purpose .clip validator.
"""
import argparse
import contextlib
import json
import sqlite3
import struct
import tempfile
from pathlib import Path

import numpy as np


def read_exact(reader, size):
    data = reader.read(size)
    if len(data) != size:
        raise ValueError("truncated sample")
    return data


def fnv(data):
    value = 0xCBF29CE484222325
    for byte in data:
        value = ((value ^ byte) * 0x100000001B3) & 0xFFFFFFFFFFFFFFFF
    return f"{value:016x}"


def compare(source, rust_report, single_layer_document=None):
    import zlib

    results = []
    with source.open("rb") as reader, tempfile.TemporaryDirectory() as directory:
        if source.stat().st_size > 1024**3:
            raise ValueError("source cap")
        assert read_exact(reader, 8) == b"CSFCHUNK"
        reader.seek(48)
        sql_offset = int.from_bytes(read_exact(reader, 8), "big")
        reader.seek(sql_offset)
        assert read_exact(reader, 8) == b"CHNKSQLi"
        size = int.from_bytes(read_exact(reader, 8), "big")
        assert 0 < size <= 32 * 1024**2
        path = Path(directory) / "metadata.db"
        path.write_bytes(read_exact(reader, size))
        with contextlib.closing(sqlite3.connect(path.as_uri() + "?mode=ro&immutable=1", uri=True)) as db:
            db.execute("PRAGMA trusted_schema=OFF")
            db.set_progress_handler(lambda: 1, 1_000_000)
            canvas = db.execute("SELECT CanvasWidth,CanvasHeight,CanvasUnit FROM Canvas LIMIT 2").fetchall()
            assert len(canvas) == 1 and canvas[0][2] == 0
            width, height = map(int, canvas[0][:2])
            assert tuple(canvas[0][:2]) == (width, height)
            assert 0 < width <= 4096 and 0 < height <= 4096 and width * height <= 4_194_304
            assert rust_report["canvas"] == [width, height]
            rows = db.execute("""SELECT l.MainId,l.LayerOffsetX,l.LayerOffsetY,
                l.LayerRenderOffscrOffsetX,l.LayerRenderOffscrOffsetY,o.Attribute,o.BlockData
                FROM Layer l JOIN Mipmap m ON m.MainId=l.LayerRenderMipmap
                JOIN MipmapInfo i ON i.MainId=m.BaseMipmapInfo
                JOIN Offscreen o ON o.MainId=i.Offscreen
                WHERE l.LayerType=1 AND l.LayerFolder=0
                AND length(o.Attribute)<=1234 AND length(o.BlockData)<=1024 LIMIT 33""").fetchall()
            assert len(rows) <= 32
            if single_layer_document is not None:
                assert len(rows) == 1
                opacity, visibility, blend, clip = db.execute("SELECT LayerOpacity,LayerVisibility,LayerComposite,LayerClip FROM Layer WHERE MainId=?", [rows[0][0]]).fetchone()
                assert opacity == 256 and visibility & 1 and blend == 0 and clip == 0
                assert single_layer_document["canvas"] == [width, height] and single_layer_document["entries"] == 1
            indexed = {}
            for layer, x, y, rx, ry, attr, external_id in rows:
                assert len(attr) <= 1234 and len(external_id) <= 1024
                if isinstance(external_id, bytes):
                    external_id = external_id.decode("ascii")
                indexed[layer] = db.execute("SELECT Offset FROM ExternalChunk WHERE CAST(ExternalID AS TEXT)=? LIMIT 2", [external_id]).fetchall()
                assert len(indexed[layer]) == 1
        assert {row[0] for row in rows} == {item["layer"] for item in rust_report["rasters"]}
        for layer, x, y, rx, ry, attr, _ in rows:
            bw, bh, columns, grid_rows = struct.unpack_from(">4I", attr, 38)
            assert 0 < bw <= 4096 and 0 < bh <= 4096
            assert columns == (bw + 255) // 256 and grid_rows == (bh + 255) // 256
            assert struct.unpack_from(">5I", attr, 140) == (20, 0, 0, 0, 4)
            output = np.zeros((height, width, 4), dtype=np.uint8)
            offset = indexed[layer][0][0]
            reader.seek(offset)
            assert read_exact(reader, 8) == b"CHNKExta"
            payload = int.from_bytes(read_exact(reader, 8), "big")
            length = int.from_bytes(read_exact(reader, 8), "big")
            assert 0 < length <= 1024 and length <= payload - 8
            read_exact(reader, length)
            body_bytes = int.from_bytes(read_exact(reader, 8), "big")
            assert body_bytes == payload - length - 16
            for index in range(columns * grid_rows):
                start = reader.tell()
                record_size = int.from_bytes(read_exact(reader, 4), "big")
                assert 104 <= record_size <= 112 + 1024**2
                assert int.from_bytes(read_exact(reader, 4), "big") == 19
                assert read_exact(reader, 38).decode("utf-16-be") == "BlockDataBeginChunk"
                stored_index, unpacked, tw, th, present = struct.unpack(">5I", read_exact(reader, 20))
                assert (stored_index, unpacked, tw, th) == (index, 327680, 256, 256)
                if present == 1:
                    outer = int.from_bytes(read_exact(reader, 4), "big")
                    compressed = int.from_bytes(read_exact(reader, 4), "little")
                    assert outer == compressed + 4 and record_size == compressed + 112
                    assert 0 < compressed <= 1024**2
                    decoder = zlib.decompressobj()
                    raw = decoder.decompress(read_exact(reader, compressed), 327681)
                    assert decoder.eof and not decoder.unused_data and not decoder.unconsumed_tail and len(raw) == 327680
                    values = np.frombuffer(raw, dtype=np.uint8)
                    tile = np.empty((256, 256, 4), dtype=np.uint8)
                    colors = values[65536:].reshape(256, 256, 4)
                    tile[:, :, :3] = colors[:, :, [2, 1, 0]]
                    tile[:, :, 3] = values[:65536].reshape(256, 256)
                    sx, sy = (index % columns) * 256, (index // columns) * 256
                    dx, dy = sx + x + rx, sy + y + ry
                    left, top = max(0, -dx), max(0, -dy)
                    right = min(256, bw - sx, width - dx)
                    bottom = min(256, bh - sy, height - dy)
                    if left < right and top < bottom:
                        output[dy + top:dy + bottom, dx + left:dx + right] = tile[top:bottom, left:right]
                else:
                    assert present == 0 and record_size == 104
                assert int.from_bytes(read_exact(reader, 4), "big") == 17
                assert read_exact(reader, 34).decode("utf-16-be") == "BlockDataEndChunk"
                assert reader.tell() == start + record_size
            expected = {
                "rgba_fnv1a": fnv(output.tobytes()),
                "nonzero_pixels": int(np.any(output != 0, axis=2).sum()),
                "partial_alpha_pixels": int(((output[:, :, 3] > 0) & (output[:, :, 3] < 255)).sum()),
                "hidden_rgb_pixels": int(((output[:, :, 3] == 0) & np.any(output[:, :, :3] != 0, axis=2)).sum()),
            }
            actual = next(item["statistics"] for item in rust_report["rasters"] if item["layer"] == layer)
            matched = all(actual[key] == value for key, value in expected.items())
            results.append({"layer": layer, "hash_and_counts_match": matched, "expected": expected})
            if not matched:
                raise ValueError(f"raster mismatch on layer {layer}")
            if single_layer_document is not None:
                # A neutral, unmasked single-layer transparent composite retains
                # source RGBA at nonzero alpha and has zero RGB at zero alpha.
                # The Rust preflight separately checks other source features.
                output[output[:, :, 3] == 0, :3] = 0
                composite = {"rgba_fnv1a": fnv(output.tobytes()),
                             "nonzero_pixels": int(np.any(output != 0, axis=2).sum()),
                             "partial_alpha_pixels": int(((output[:, :, 3] > 0) & (output[:, :, 3] < 255)).sum()),
                             "hidden_rgb_pixels": 0}
                if not all(single_layer_document["composite"][key] == value for key, value in composite.items()):
                    raise ValueError("single-layer native composite mismatch")
                results[-1]["single_layer_composite_hash_and_counts_match"] = True
    return {"scope": "Independent raster-leaf RGBA hash/count comparison; no whole-document rendering/import validation.", "comparisons": results}


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("source", type=Path)
    parser.add_argument("rust_report", type=Path)
    parser.add_argument("new_output", type=Path)
    parser.add_argument("--single-layer-document", type=Path)
    args = parser.parse_args()
    document = json.loads(args.single_layer_document.read_text()) if args.single_layer_document else None
    result = compare(args.source, json.loads(args.rust_report.read_text()), document)
    with args.new_output.open("x", encoding="utf-8") as writer:
        json.dump(result, writer, indent=2)
    print(f"Matched {len(result['comparisons'])} raster leaf hashes and counts.")
