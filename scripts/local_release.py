"""Make a release that exists only on this PC, to click through an update.

    cargo build --profile fast
    python scripts/local_release.py

It writes a copy of the development build to update (`app`) and a folder with
a release list and package (`releases`), then prints how to start it. The
"newer" version is the same program under another number, so it still calls
itself the old one after the restart. Nothing is published or downloaded.
"""
import argparse
import hashlib
import json
import re
import shutil
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent


def manifest_value(name):
    text = (ROOT / "Cargo.toml").read_text(encoding="utf-8")
    return re.search(rf'^{name} = "(.*)"', text, re.MULTILINE).group(1)


def package_files(executable, version):
    files = {"dudel.exe": executable.read_bytes(), "RELEASE_NOTES.md": (ROOT / "RELEASE_NOTES.md").read_bytes(),
             "BUILD.json": json.dumps({"version": version, "revision": "local"}).encode()}
    files["FILES.json"] = json.dumps({"files": sorted([*files, "FILES.json"])}).encode()
    return files


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    parser.add_argument("--executable", type=Path, default=ROOT / "target/fast/dudel.exe")
    parser.add_argument("--version", default="9.9.9", help="the version offered")
    parser.add_argument("--output", type=Path, default=ROOT / "artifacts/local-update")
    parser.add_argument("--wrong-checksum", action="store_true", help="publish a checksum that does not match")
    arguments = parser.parse_args()
    if not re.fullmatch(r"\d+\.\d+\.\d+", arguments.version):
        parser.error("--version must look like 1.2.3")
    if not arguments.executable.is_file():
        parser.error(f"build first: {arguments.executable} does not exist")
    output = arguments.output.resolve()
    app, releases = output / "app", output / "releases"
    # Only what an earlier run of this script made is replaced.
    for folder in (app, releases):
        if folder.exists():
            shutil.rmtree(folder)
        folder.mkdir(parents=True)

    for name, data in package_files(arguments.executable, manifest_value("version")).items():
        (app / name).write_bytes(data)

    version, name = arguments.version, f"dudel-{arguments.version}-windows-x64.zip"
    with zipfile.ZipFile(releases / name, "w", zipfile.ZIP_DEFLATED) as archive:
        for path, data in package_files(arguments.executable, version).items():
            archive.writestr(f"dudel-{version}-windows-x64/{path}", data)
    package = releases / name
    digest = hashlib.sha256(package.read_bytes()).hexdigest()
    if arguments.wrong_checksum:
        digest = "0" * 64
    (releases / f"{name}.sha256").write_text(f"{digest}  {name}\n", encoding="ascii")
    repository = manifest_value("repository")
    major, minor, patch = (int(part) for part in version.split("."))
    skipped = f"{major}.{minor}.{patch - 1}" if patch else None

    def release(number, assets):
        return {"tag_name": f"v{number}", "draft": False, "prerelease": True,
                "html_url": f"{repository}/releases/tag/v{number}",
                "body": f"A local test release.\n\n## What's new\n\n- **Number {number}:** from this PC only.\n\n"
                        "## Install\n\nNot shown.",
                "assets": [{"name": asset, "size": (releases / asset).stat().st_size,
                            "browser_download_url": f"{repository}/releases/download/v{number}/{asset}"}
                           for asset in assets]}

    listing = [release(version, [name, f"{name}.sha256"])]
    if skipped:
        listing.append(release(skipped, []))
    (releases / "releases.json").write_text(json.dumps(listing, indent=2), encoding="utf-8")
    print("In PowerShell:\n")
    print(f'$env:DUDEL_RELEASES = "{releases}"')
    print(f'& "{app / "dudel.exe"}"')
    print("\nThen Help > Check for updates. For the What's new window, also set")
    print("$env:DUDEL_UPDATED_FROM = '0.6.0' before starting.")


if __name__ == "__main__":
    main()
