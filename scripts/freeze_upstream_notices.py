"""Freeze verified upstream notice bytes from a packaged matching-source candidate.

Review the candidate's provenance first. This preserves its existing evidence;
it neither fetches new terms nor decides license compatibility.
"""
import argparse
import hashlib
import json
from pathlib import Path, PurePosixPath
import tomllib


def freeze(candidate, destination):
    source = Path(str(candidate) + "-source")
    index = json.loads((candidate / "third-party-licenses/INDEX.json").read_text())
    if index["manual_review"]:
        raise ValueError("Resolve missing candidate notices before freezing")
    destination.mkdir(exist_ok=False)
    blobs = destination / "blobs"
    blobs.mkdir()
    packages = {}
    for package in index["packages"]:
        provenance = package["upstream_provenance"]
        if not provenance:
            continue
        key = f'{package["package"]}-{package["version"]}'
        crate = source / "vendor" / key
        manifest = (crate / "Cargo.toml").read_bytes()
        info = tomllib.loads(manifest.decode())["package"]
        files = []
        for relative, origin in zip(package["files"][-len(provenance):], provenance, strict=True):
            path = PurePosixPath(relative)
            base = PurePosixPath("third-party-licenses") / key
            if not path.is_relative_to(base) or ".." in path.parts:
                raise ValueError("Notice escapes package")
            data = (candidate / Path(*path.parts)).read_bytes()
            digest = hashlib.sha256(data).hexdigest().upper()
            if digest != origin["sha256"]:
                raise ValueError(f"Notice provenance mismatch: {relative}")
            (blobs / digest).write_bytes(data)
            files.append({"relative": path.relative_to(base).as_posix(), **origin})
        packages[key] = {
            "repository": info.get("repository"),
            "declared_license": package["declared_license"],
            "crate_checksum": json.loads((crate / ".cargo-checksum.json").read_text())["package"],
            "manifest_sha256": hashlib.sha256(manifest).hexdigest().upper(),
            "files": files,
        }
    (destination / "INDEX.json").write_text(json.dumps({
        "version": 1,
        "origin_candidate": candidate.name,
        "origin_notice_index_sha256": hashlib.sha256(
            (candidate / "third-party-licenses/INDEX.json").read_bytes()).hexdigest().upper(),
        "packages": packages,
    }, indent=2) + "\n", encoding="utf-8")
    return {"packages": len(packages), "unique_blobs": len(list(blobs.iterdir()))}


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("candidate", type=Path)
    parser.add_argument("destination", type=Path)
    args = parser.parse_args()
    print(json.dumps(freeze(args.candidate, args.destination)))
