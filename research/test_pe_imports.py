"""Regression for the Visual C++ runtime import check used by packaging."""
from pathlib import Path
import struct
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "scripts"))
from check_pe_imports import imports, REDISTRIBUTABLE


def minimal_pe(dlls):
    """A PE32+ image with one section holding an import table naming `dlls`."""
    section_rva, raw = 0x1000, 0x200
    descriptors = 20 * (len(dlls) + 1)
    names, blob = [], b""
    for dll in dlls:
        names.append(section_rva + descriptors + len(blob))
        blob += dll.encode("ascii") + b"\0"
    body = b"".join(struct.pack("<5I", 0, 0, 0, name, 0) for name in names) + b"\0" * 20 + blob
    body += b"\0" * (-len(body) % 0x200)
    header = bytearray(raw)
    header[0:2] = b"MZ"
    struct.pack_into("<I", header, 0x3C, 0x40)
    header[0x40:0x44] = b"PE\0\0"
    struct.pack_into("<HHIIIHH", header, 0x44, 0x8664, 1, 0, 0, 0, 240, 0x22)
    optional = 0x58
    struct.pack_into("<H", header, optional, 0x20B)
    struct.pack_into("<I", header, optional + 108, 16)
    struct.pack_into("<II", header, optional + 112 + 8, section_rva, len(body))
    section = optional + 240
    header[section:section + 8] = b".idata\0\0"
    struct.pack_into("<IIII", header, section + 8, len(body), section_rva, len(body), raw)
    return bytes(header) + body


class PeImportTests(unittest.TestCase):
    def check(self, dlls):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "app.exe"
            path.write_bytes(minimal_pe(dlls))
            return imports(path)

    def test_lists_imports_and_flags_visual_cpp_runtime(self):
        found = self.check(["KERNEL32.dll", "VCRUNTIME140.dll", "api-ms-win-crt-heap-l1-1-0.dll"])
        self.assertEqual(found, ["api-ms-win-crt-heap-l1-1-0.dll", "KERNEL32.dll", "VCRUNTIME140.dll"])
        runtime = [d for d in found if d.lower().startswith(REDISTRIBUTABLE)]
        self.assertEqual(runtime, ["api-ms-win-crt-heap-l1-1-0.dll", "VCRUNTIME140.dll"])

    def test_system_only_imports_pass(self):
        found = self.check(["KERNEL32.dll", "user32.dll", "api-ms-win-core-synch-l1-2-0.dll"])
        self.assertFalse([d for d in found if d.lower().startswith(REDISTRIBUTABLE)])


if __name__ == "__main__":
    unittest.main()
