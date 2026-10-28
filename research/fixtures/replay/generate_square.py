"""Generate an independent hard-square tap oracle, not a tablet recording."""
import gzip
import json
import struct
from pathlib import Path


def chunk(data):
    return struct.pack("<I", len(data)) + data


def project(pixels):
    metadata = {
        "version": 1, "width": 8, "height": 8, "active": 0,
        "selection": None,
        "layers": [{"name": "Square oracle", "visible": True, "opacity": 1.0,
                    "blend": "Normal", "clipped": False, "locked": False,
                    "alpha_locked": False, "pixels": []}],
    }
    data = json.dumps(metadata, separators=(",", ":")).encode()
    # One nonzero edge tile, index zero, tightly packed 8x8 RGBA.
    payload = chunk(data) + struct.pack("<II", 1, 0) + pixels
    return b"DUDL\x04\r\n" + gzip.compress(payload, compresslevel=1, mtime=0)


sample = {"x": 4.0, "y": 4.0, "pressure": 0.0,
          "pressure_known": True, "time": 1.0, "tilt": [-12.0, 20.0],
          "twist": 36.0}
mapping = {"origin": [4.0, 4.0], "zoom": 1.0, "rotation": 0.0,
           "mirror": False, "half": [4.0, 4.0]}
trace = {
    "version": 1,
    "settings": {
        "brush": {"name": "Independent square oracle", "size": 4.0,
                  "opacity": 1.0, "flow": 1.0, "hardness": 1.0,
                  "spacing": 0.15, "pressure_size": False,
                  "pressure_opacity": False, "tip": "Square"},
        "color": [17, 83, 201], "erase": False, "layer": 0,
        "symmetry": {"mode": "Off", "segments": 6, "center": None},
    },
    "commands": [
        {"Point": {"screen": sample, "mapping": mapping, "lift": False}},
        {"Point": {"screen": dict(sample, time=1.001),
                   "mapping": mapping, "lift": True}},
    ],
    "cancelled": False,
}
base = bytearray(8 * 8 * 4)
base[:4] = bytes([9, 8, 7, 0])  # Invisible RGB must survive replay.
expected = base.copy()
for y in range(2, 6):
    for x in range(2, 6):
        expected[(y * 8 + x) * 4:(y * 8 + x + 1) * 4] = bytes([17, 83, 201, 255])

result = b"DRPL\x01\r\n" + chunk(json.dumps(trace, separators=(",", ":")).encode())
result += chunk(project(base)) + chunk(project(expected))
Path(__file__).with_name("hard-square.dudelreplay").write_bytes(result)
