"""Preserve all files in a new ZIP; clamp Cargo's pre-1980 archive timestamps."""
import argparse
from pathlib import Path
import zipfile


def archive(directory, output):
    with zipfile.ZipFile(output, "x", compression=zipfile.ZIP_DEFLATED,
                         compresslevel=6, strict_timestamps=False) as zipped:
        for path in sorted(directory.rglob("*")):
            if path.is_file() and "__pycache__" not in path.parts and path.name != "VENDOR.log":
                zipped.write(path, arcname=f"{directory.name}/{path.relative_to(directory).as_posix()}")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("directory", type=Path)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    archive(args.directory, args.output)
