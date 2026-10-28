"""Verify a candidate source ZIP and rebuild its extracted sources offline."""
import argparse
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import stat
import subprocess
import zipfile

REPOSITORY = Path(__file__).resolve().parents[1]


def sha(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest().upper()


def check(archive, output):
    archive, output = archive.resolve(), output.resolve()
    if not output.is_relative_to(REPOSITORY / "target" / "qa"):
        raise ValueError("Rebuild evidence must be inside this repository's target/qa")
    output.mkdir(parents=True, exist_ok=False)
    state = {"archive": str(archive), "started_utc": datetime.now(timezone.utc).isoformat()}

    def record(status, **details):
        state.update(status=status, **details)
        temporary = output / "STATE.tmp"
        temporary.write_text(json.dumps(state, indent=2) + "\n", encoding="utf-8")
        temporary.replace(output / "STATE.json")
        print(status, flush=True)

    try:
        record("verifying")
        checksum = archive.with_suffix(".zip.sha256").read_text(encoding="ascii").split()[0]
        actual = sha(archive)
        if actual != checksum.upper():
            raise ValueError("Source archive checksum mismatch")
        record("extracting", archive_sha256=actual)
        extracted = output / "extracted"
        with zipfile.ZipFile(archive) as zipped:
            entries = zipped.infolist()
            if len(entries) > 100_000 or sum(item.file_size for item in entries) > 8 * 1024**3:
                raise ValueError("Source archive exceeds extraction bounds")
            roots = set()
            for item in entries:
                path = PurePosixPath(item.filename)
                if (path.is_absolute() or ".." in path.parts or not path.parts
                        or any(":" in part or "\\" in part for part in path.parts)
                        or stat.S_ISLNK(item.external_attr >> 16)):
                    raise ValueError(f"Unsafe archive member: {item.filename}")
                roots.add(path.parts[0])
            if len(roots) != 1:
                raise ValueError("Expected one source archive root")
            # Reading each member during extraction checks its CRC.
            zipped.extractall(extracted)
        source = extracted / roots.pop()
        build = json.loads((source / "SOURCE_BUILD.json").read_text(encoding="utf-8"))
        for item in build["source_inputs"]:
            path = PurePosixPath(item["path"])
            if (path.is_absolute() or ".." in path.parts or not path.parts
                    or any(":" in part or "\\" in part for part in path.parts)):
                raise ValueError("Invalid recorded source path")
            if sha(source / Path(*path.parts)) != item["sha256"]:
                raise ValueError(f"Extracted source identity mismatch: {path}")
        settings = build.get("cargo_config")
        if settings:
            # Git may check the settings out with CRLF; compare content, not line endings.
            def lines(value):
                return value.replace("\r\n", "\n").rstrip()
            text = lines((source / ".cargo" / "config.toml").read_bytes().decode("utf-8"))
            if not text.startswith(lines(settings["text"])):
                raise ValueError("Source archive does not carry the recorded Cargo build settings")
        record("building", members=len(entries), source=str(source),
               expected_candidate_executable_sha256=build["executable_sha256"])
        environment = dict(os.environ, CARGO_TARGET_DIR=str(output / "build"))
        command = ["cargo", "build", "--locked", "--offline", "--release", "--bin", "dudel"]
        with (output / "build.log").open("wb") as log:
            subprocess.run(command, cwd=source, env=environment, stdout=log, stderr=log, check=True)
            # Older source archives have no shell workspace. New Windows packages
            # must reproduce both deliverables from the same offline vendor tree.
            thumbnail_manifest = source / "shell/thumbnails/Cargo.toml"
            has_thumbnails = thumbnail_manifest.is_file() and os.name == "nt"
            if has_thumbnails:
                subprocess.run(["cargo", "build", "--locked", "--offline", "--release",
                                "--manifest-path", "shell/thumbnails/Cargo.toml"],
                               cwd=source, env=environment, stdout=log, stderr=log, check=True)
        executable = output / "build" / "release" / ("dudel.exe" if os.name == "nt" else "dudel")
        runtime_imports = None
        rebuilt = [{"path": executable.name, "sha256": sha(executable)}]
        if os.name == "nt":
            from check_pe_imports import imports, REDISTRIBUTABLE
            runtime_imports = [d for d in imports(executable) if d.lower().startswith(REDISTRIBUTABLE)]
            if runtime_imports:
                raise ValueError(f"Rebuilt executable needs the Visual C++ runtime: {runtime_imports}")
            if has_thumbnails:
                dll = output / "build/release/dudel_thumbnails.dll"
                dll_imports = imports(dll)
                if any(d.lower().startswith(REDISTRIBUTABLE) for d in dll_imports):
                    raise ValueError("Rebuilt thumbnail DLL needs the Visual C++ runtime")
                rebuilt.append({"path": dll.name, "sha256": sha(dll), "imports": dll_imports})
            expected = {item["path"] for item in build.get("artifacts", [])}
            if expected and expected != {item["path"] for item in rebuilt}:
                raise ValueError("Rebuilt Windows deliverables differ from recorded artifacts")
        record("passed", executable=str(executable), executable_sha256=sha(executable),
               rebuilt_artifacts=rebuilt, expected_candidate_artifacts=build.get("artifacts"),
               visual_cpp_runtime_imports=runtime_imports,
               completed_utc=datetime.now(timezone.utc).isoformat(),
               scope="Archive checksum/CRC, recorded source hashes and fresh offline release compilation; not clean-machine acceptance")
    except BaseException as error:
        record("failed", error=str(error))
        raise


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("archive", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    check(args.archive, args.output)
