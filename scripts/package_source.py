"""Create matching source and vendored dependencies in a new candidate directory."""
import argparse
import hashlib
import json
from pathlib import Path
import re
import shutil
import subprocess
import tomllib
import zipfile
from upstream_notices import collect as upstream_notices, notice_path
from package_docs import bundle_docs


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest().upper()


def check_thumbnail_dependencies(metadata, lock_path):
    """Refuse an archive whose app-only vendor/notices tree omits DLL dependencies."""
    def key(package):
        return package["name"], package["version"], package.get("source")
    covered = {key(p) for p in metadata["packages"] if p.get("source")}
    with lock_path.open("rb") as stream:
        locked = tomllib.load(stream)
    required = {key(p) for p in locked["package"] if p.get("source")}
    missing = sorted(required - covered)
    if missing:
        raise ValueError(f"Thumbnail dependencies absent from vendored app inventory: {missing}")


def bundle(repository, destination, binary, metadata_path):
    repository, destination, binary = (p.resolve() for p in (repository, destination, binary))
    if not destination.is_relative_to(repository / "artifacts" / "releases"):
        raise ValueError("Source staging must be inside this repository's artifacts/releases")
    destination.mkdir(exist_ok=False)
    metadata = json.loads(metadata_path.read_text(encoding="utf-8-sig"))
    check_thumbnail_dependencies(metadata, repository / "shell/thumbnails/Cargo.lock")
    build = json.loads((binary / "BUILD.json").read_text(encoding="utf-8-sig"))
    paths = set()
    for name in ("Cargo.toml", "Cargo.lock", ".gitattributes", "LICENSE", "README.md", "QUICK_START.md",
                 "RELEASE_NOTES.md", "RELEASE_CHECKLIST.md", "RELEASE_STATUS.md", "RELEASE_DECISION.md", "RELEASE_ACCEPTANCE.json", "DELIVERY_PLAN.md", "DISTRIBUTION.md",
                 "XP_PEN_VALIDATION.md", "FORMAT_SUPPORT.md", "PROJECT_FORMAT_V4.md", "LINUX_PORT.md", "PLUGIN_DEVELOPMENT.md", "CREATOR_FEATURES.md", "TESTING.md", "build.rs", "rust-toolchain.toml"):
        if (repository / name).is_file():
            paths.add(Path(name))
    for folder in ("src", "examples", "plugins", "scripts", ".github", "research/fixtures/replay"):
        paths.update(p.relative_to(repository) for p in (repository / folder).rglob("*") if p.is_file())
    paths.update(p.relative_to(repository) for p in (repository / "research").glob("*.py"))
    # The subset check above keeps the shared vendor/notices tree complete.
    thumbnails = repository / "shell" / "thumbnails"
    paths.update(p.relative_to(repository) for p in (thumbnails / "Cargo.toml", thumbnails / "Cargo.lock"))
    paths.update(p.relative_to(repository) for p in (thumbnails / "src").rglob("*") if p.is_file())
    paths = {p for p in paths if "__pycache__" not in p.parts}
    for relative in sorted(paths):
        original = repository / relative
        if original.is_symlink() or not original.resolve().is_relative_to(repository):
            raise ValueError(f"Source file escapes repository: {relative}")
        target = destination / relative
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(original, target)
    for item in build["source_inputs"]:
        relative = Path(item["path"])
        if relative.is_absolute() or ".." in relative.parts:
            raise ValueError("Invalid build input path")
        copied = destination / relative
        if not copied.is_file() or digest(copied) != item["sha256"]:
            raise ValueError(f"Copied input no longer matches binary build: {relative}")
    cargo_config = destination / ".cargo" / "config.toml"
    cargo_config.parent.mkdir()
    log_path = destination / "VENDOR.log"
    with cargo_config.open("w", encoding="utf-8") as config, log_path.open("w", encoding="utf-8") as log:
        subprocess.run(["cargo", "vendor", "--locked", "--versioned-dirs", "--no-delete",
                        str(destination / "vendor")], cwd=repository, stdout=config, stderr=log, check=True)
    config_text = cargo_config.read_text(encoding="utf-8")
    config_text, replacements = re.subn(r'(?m)^directory\s*=.*$', 'directory = "vendor"', config_text)
    if replacements < 1:
        raise ValueError("Cargo did not produce vendored source configuration")
    # Keep the repository's build settings (static CRT) so source rebuilds match the binary.
    recorded = build.get("cargo_config")
    if recorded:
        settings = (repository / recorded["path"]).read_bytes().decode("utf-8")
        if settings != recorded["text"]:
            raise ValueError("Cargo build settings changed after the binary build")
        config_text = settings.rstrip() + "\n\n" + config_text
    # Bytes, not text: write_text would translate newlines and alter the recorded settings.
    cargo_config.write_bytes(config_text.encode("utf-8"))
    # Resolve from the copied folder; --manifest-path from another cwd would
    # ignore the copied .cargo/config.toml and fail to verify relocatability.
    with log_path.open("a", encoding="utf-8") as log:
        subprocess.run(["cargo", "metadata", "--locked", "--offline", "--format-version", "1"],
                       cwd=destination, stdout=subprocess.DEVNULL, stderr=log, check=True)
        subprocess.run(["cargo", "metadata", "--locked", "--offline", "--format-version", "1",
                        "--manifest-path", "shell/thumbnails/Cargo.toml"],
                       cwd=destination, stdout=subprocess.DEVNULL, stderr=log, check=True)
    notices = binary / "third-party-licenses"
    notices.mkdir(exist_ok=False)
    entries, missing = [], []
    for package in sorted(metadata["packages"], key=lambda p: (p["name"], p["version"])):
        if package.get("source") is None:
            continue
        key = f'{package["name"]}-{package["version"]}'
        root = destination / "vendor" / key
        if not root.is_dir():
            raise ValueError(f"Missing vendored dependency: {key}")
        license_files = [p for p in root.rglob("*") if p.is_file() and notice_path(p.relative_to(root))]
        copied_files = []
        for path in sorted(license_files):
            if path.is_symlink() or not path.resolve().is_relative_to(root):
                raise ValueError(f"License file escapes dependency: {path}")
            relative = path.relative_to(root)
            target = notices / key / relative
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(path, target)
            copied_files.append(target.relative_to(binary).as_posix())
        upstream = []
        # Asset terms alone do not replace the crate/workspace's own license.
        if not any(p.parent == root for p in license_files):
            upstream = upstream_notices(package, root, notices / key, repository / "artifacts" / "license-cache")
            copied_files.extend(item["path"].relative_to(binary).as_posix() for item in upstream)
        if not copied_files or not package.get("license"):
            missing.append(key)
        entries.append({"package": package["name"], "version": package["version"],
                        "declared_license": package.get("license"), "files": copied_files,
                        "upstream_provenance": [{k: v for k, v in item.items() if k != "path"} for item in upstream]})
    (notices / "INDEX.json").write_text(json.dumps({"packages": entries, "manual_review": missing}, indent=2) + "\n", encoding="utf-8")
    shutil.copytree(notices, destination / "third-party-licenses")
    (binary / "THIRD_PARTY_NOTICES.md").write_text(
        "# Third-party notices\n\nDependency license/copyright/notice files are preserved under "
        "`third-party-licenses/`; `INDEX.json` maps packages to their declared licenses and actual files. "
        "The matching source archive preserves complete vendored crates, including bundled assets and their notices. "
        "Third-party files retain their original licensing terms. This inventory includes development and "
        "other-platform dependencies as well as runtime dependencies.\n\n"
        + ("Manual license-file review still required for: " + ", ".join(missing) + ".\n" if missing else
           "Every resolved registry package has a declared license and at least one collected notice/license file. "
           "This does not replace the release review of bundled subcomponents.\n"), encoding="utf-8")
    (destination / "SOURCE_BUILD.json").write_text(json.dumps(build, indent=2) + "\n", encoding="utf-8")
    (destination / "BUILDING.md").write_text(
        "# Build Dudel from matching source\n\n"
        "Dudel is GPL-3.0-only; see LICENSE. Official prebuilt downloads may be sold. "
        "This source is available to build without paying for a prebuilt executable.\n\n"
        f'Tested compiler: `{build["rustc"]}`. Install Rust and the Windows Visual Studio C++ '
        "build tools/Windows SDK. Extract the complete archive, including `.cargo` and `vendor`, "
        "then run from this folder:\n\n```powershell\ncargo build --release --locked --offline --bin dudel\n"
        "cargo build --release --locked --offline --manifest-path shell/thumbnails/Cargo.toml\n"
        "Copy-Item shell/thumbnails/target/release/dudel_thumbnails.dll target/release/\n"
        ".\\target\\release\\dudel.exe\n```\n\n"
        "Dependencies are vendored with their original sources/licenses, so Cargo does not need the network "
        "after the compiler/platform build tools are installed. `.cargo/config.toml` links the C runtime statically "
        "so the executable runs without the Visual C++ redistributable; keep it when building. "
        "`SOURCE_BUILD.json` records matching binary/source "
        "identities. Different toolchains, SDKs and build environments can change binary bytes.\n\n"
        "Run these commands from the extracted root so both workspaces use its vendored configuration. "
        "With a custom CARGO_TARGET_DIR, both artifacts instead appear in that target's release folder.\n\n"
        "Development checks: `cargo test --locked --offline`; full Windows/Python checks, including the DLL: `./scripts/Verify.ps1 -Mode Full`. "
        "The approximately 1 GiB maximum-project test is a separate ignored test; see RELEASE_CHECKLIST.md.\n", encoding="utf-8")
    # Keep public report links usable offline without shipping ignored/private
    # research, raw artwork, captures or repository history.
    bundle_docs(repository, destination)
    for item in build["source_inputs"]:
        if digest(destination / item["path"]) != item["sha256"]:
            raise ValueError(f'Documentation packaging changed a build input: {item["path"]}')
    archive = Path(str(destination) + ".zip")
    if archive.exists():
        raise FileExistsError(archive)
    with zipfile.ZipFile(archive, "x", compression=zipfile.ZIP_DEFLATED, compresslevel=6, strict_timestamps=False) as zipped:
        for path in sorted(destination.rglob("*")):
            if path.is_file() and path.name != "VENDOR.log" and "__pycache__" not in path.parts:
                zipped.write(path, arcname=f"{destination.name}/{path.relative_to(destination).as_posix()}")
    checksum = digest(archive)
    archive.with_suffix(".zip.sha256").write_text(f"{checksum}  {archive.name}\n", encoding="ascii")
    (binary / "SOURCE.md").write_text(
        f"# Matching source\n\nThis executable is licensed under GPL-3.0-only (LICENSE).\n\n"
        f"Matching source/dependencies/build instructions: **{archive.name}**.\n\nSHA-256: `{checksum}`.\n\n"
        "Distribute this matching source archive alongside the executable download. The archive includes "
        "BUILDING.md, the exact Cargo.lock, local source modifications and vendored dependency sources.\n",
        encoding="utf-8")
    bundle_docs(repository, binary)
    return {"archive": str(archive), "sha256": checksum, "dependencies": len(entries), "manual_review": missing}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("repository", "destination", "binary", "metadata"):
        parser.add_argument(f"--{name}", required=True, type=Path)
    args = parser.parse_args()
    print(json.dumps(bundle(args.repository, args.destination, args.binary, args.metadata)))


if __name__ == "__main__":
    main()
