"""Build API 1 filter or API 2 contribution packages using Dudel's validator."""
import argparse
import json
import os
from pathlib import Path
import subprocess
import tempfile


def run(executable: Path, *args: str) -> subprocess.CompletedProcess:
    # An older Dudel that lacks a command would open its window instead of
    # answering, so give up rather than wait on it.
    try:
        return subprocess.run([str(executable.resolve(strict=True)), *args], capture_output=True, text=True, timeout=60)
    except subprocess.TimeoutExpired:
        raise ValueError(f"{executable} did not answer {args[0]}; is it older than this tool?") from None


def build(project: Path, output: Path, executable: Path) -> None:
    project = project.resolve(strict=True)
    manifest_path = project / "manifest.json"
    if manifest_path.stat().st_size > 5 * 1024 * 1024:
        raise ValueError("Manifest exceeds 5 MiB")
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    if output.suffix != ".dudelplugin":
        raise ValueError("Output must end in .dudelplugin")
    if manifest.get("api") == 2:
        if "entry" in manifest:
            raise ValueError("API 2 defines programs inside contributed commands")
        # Filter and scripted-tool programs may live in their own .rhai files
        # beside the manifest; "source_file" is replaced by their text.
        # Brushes may name PNG files for their tip ("image_file") and paper
        # grain ("grain_file"); Dudel converts them as its brush settings do.
        for command in manifest.get("contributes", {}).get("commands", []):
            action = command.get("action", {})
            tool = action.get("tool") if isinstance(action.get("tool"), dict) else {}
            for brush in (action.get("brush"), tool.get("brush")):
                if not isinstance(brush, dict):
                    continue
                for key, field, kind in (("image_file", "image", "tip"), ("grain_file", "grain", "grain")):
                    if key not in brush:
                        continue
                    image = (project / brush.pop(key)).resolve(strict=True)
                    if not image.is_relative_to(project) or image.suffix.lower() != ".png":
                        raise ValueError(f"{key} must be a .png file inside the plugin project")
                    result = run(executable, "--brush-texture", kind, str(image))
                    if result.returncode:
                        raise ValueError(result.stderr.strip() or f"Dudel rejected {image.name}")
                    brush[field] = json.loads(result.stdout)
                    if kind == "tip":
                        brush.setdefault("tip", "Image")
            for holder in (action, action.get("tool") if isinstance(action.get("tool"), dict) else None):
                if holder is None or "source_file" not in holder:
                    continue
                source = (project / holder.pop("source_file")).resolve(strict=True)
                if not source.is_relative_to(project) or source.suffix != ".rhai":
                    raise ValueError("source_file must be a .rhai file inside the plugin project")
                if source.stat().st_size > 32768:
                    raise ValueError(f"{source.name} exceeds 32 KiB")
                holder["source"] = source.read_text(encoding="utf-8")
    else:
        entry = (project / manifest.pop("entry")).resolve(strict=True)
        if not entry.is_relative_to(project):
            raise ValueError("Entry must stay inside the plugin project")
        if entry.suffix == ".rhai":
            if entry.stat().st_size > 32768:
                raise ValueError("Rhai source exceeds 32 KiB")
            manifest.update(format="dudel-rhai-extension", source=entry.read_text(encoding="utf-8"), wasm=[])
        elif entry.suffix == ".wasm":
            if entry.stat().st_size > 1024 * 1024:
                raise ValueError("WASM exceeds 1 MiB")
            manifest.update(format="dudel-wasm-extension", wasm=list(entry.read_bytes()), source="")
        else:
            raise ValueError("Entry must be a .rhai or compiled .wasm file")
    payload = json.dumps(manifest, ensure_ascii=False, indent=2).encode("utf-8")
    # Textures would put a byte per line; large packages are written compact.
    if len(payload) > 256 * 1024:
        payload = json.dumps(manifest, ensure_ascii=False, separators=(",", ":")).encode("utf-8")
    if len(payload) > 5 * 1024 * 1024:
        raise ValueError("Package exceeds 5 MiB")
    with tempfile.TemporaryDirectory(prefix="dudel-plugin-") as temporary:
        candidate = Path(temporary) / "candidate.dudelplugin"
        candidate.write_bytes(payload)
        result = run(executable, "--check-plugin", str(candidate))
        if result.returncode:
            raise ValueError(result.stderr.strip() or "Dudel rejected the plugin")
    # Never silently replace a previously published package.
    with output.open("xb") as stream:
        stream.write(payload)
        stream.flush()
        os.fsync(stream.fileno())


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("project", type=Path, help="Directory containing manifest.json and the entry program")
    parser.add_argument("output", type=Path)
    parser.add_argument("--executable", required=True, type=Path, help="Current Dudel executable (debug or release)")
    args = parser.parse_args()
    try:
        build(args.project, args.output, args.executable)
    except (OSError, ValueError, KeyError, TypeError) as error:
        parser.exit(1, f"Plugin packaging: {error}\n")
    print(f"Validated plugin package: {args.output}")


if __name__ == "__main__":
    main()
