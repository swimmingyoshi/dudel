"""List a Windows executable's imported DLLs and fail if it needs the Visual C++
runtime (VCRUNTIME/MSVCP/UCRT api-ms-win-crt), which clean Windows installs lack.

Usage: python scripts/check_pe_imports.py PATH_TO.exe [--json]
"""
import json
import struct
import sys
from pathlib import Path

REDISTRIBUTABLE = ("vcruntime", "msvcp", "concrt", "vccorlib", "api-ms-win-crt-", "ucrtbase")


def imports(path):
    data = Path(path).read_bytes()
    pe = struct.unpack_from("<I", data, 0x3C)[0]
    if data[pe:pe + 4] != b"PE\0\0":
        raise ValueError("Not a PE file")
    sections_count = struct.unpack_from("<H", data, pe + 6)[0]
    optional_size = struct.unpack_from("<H", data, pe + 20)[0]
    optional = pe + 24
    magic = struct.unpack_from("<H", data, optional)[0]
    directories = optional + (112 if magic == 0x20B else 96)
    sections = []
    for i in range(sections_count):
        base = optional + optional_size + 40 * i
        size, address, raw_size, raw_pointer = struct.unpack_from("<IIII", data, base + 8)
        sections.append((address, max(size, raw_size), raw_pointer))

    def offset(rva):
        for address, size, raw in sections:
            if address <= rva < address + size:
                return rva - address + raw
        raise ValueError(f"RVA {rva:#x} outside sections")

    def name_at(rva):
        start = offset(rva)
        return data[start:data.index(b"\0", start)].decode("ascii")

    found = []
    for index, record in ((1, 20), (13, 32)):  # import table, delay-load import table
        rva, size = struct.unpack_from("<II", data, directories + 8 * index)
        if not rva:
            continue
        cursor = offset(rva)
        while True:
            fields = struct.unpack_from("<5I" if record == 20 else "<8I", data, cursor)
            if not any(fields):
                break
            found.append(name_at(fields[3] if record == 20 else fields[1]))
            cursor += record
    return sorted(set(found), key=str.lower)


def main():
    path = sys.argv[1]
    dlls = imports(path)
    runtime = [d for d in dlls if d.lower().startswith(REDISTRIBUTABLE)]
    result = {"executable": str(path), "imports": dlls, "visual_cpp_runtime_imports": runtime,
              "passed": not runtime}
    if "--json" in sys.argv:
        print(json.dumps(result, indent=2))
    else:
        print("\n".join(dlls))
        print("Visual C++ runtime imports:", ", ".join(runtime) if runtime else "none")
    sys.exit(0 if not runtime else 1)


if __name__ == "__main__":
    main()
