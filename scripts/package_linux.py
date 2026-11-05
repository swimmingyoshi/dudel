"""Linux counterpart of Package.ps1: an executable tarball, matching source ZIP and
SHA-256 sidecars under artifacts/releases/.

Linux stays experimental: nothing here reads RELEASE_ACCEPTANCE.json, which covers
the Windows deliverables only. Needs cargo-zigbuild and zig on PATH, so the binary
links against an old glibc instead of whatever the build machine has.

Usage: python scripts/package_linux.py [--glibc 2.31]
"""
import argparse
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tarfile

REPOSITORY = Path(__file__).resolve().parents[1]
TARGET = "x86_64-unknown-linux-gnu"
DOCUMENTS = ("QUICK_START.md", "RELEASE_NOTES.md", "LINUX_PORT.md", "DISTRIBUTION.md", "FORMAT_SUPPORT.md",
             "CREATOR_FEATURES.md", "PLUGIN_DEVELOPMENT.md", "TESTING.md")


def run(*command):
    # rstrip: the first `git status --porcelain` line can begin with a meaningful space.
    return subprocess.run(command, cwd=REPOSITORY, check=True, stdout=subprocess.PIPE, text=True).stdout.rstrip()


def sha(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest().upper()


def source_identity():
    """Hash actual build inputs, including uncommitted/untracked Rust modules."""
    names = ["Cargo.toml", "Cargo.lock", "LICENSE", "LICENSE-psd-tools", "shell/thumbnails/Cargo.toml", "shell/thumbnails/Cargo.lock"]
    for folder in ("src", "examples", "plugins", "shell/thumbnails/src"):
        names += [p.relative_to(REPOSITORY).as_posix() for p in (REPOSITORY / folder).rglob("*") if p.is_file()]
    return [{"path": name, "sha256": sha(REPOSITORY / name)} for name in sorted(names)]


def glibc_ceiling(executable):
    """Newest glibc symbol version the executable needs, as a tuple."""
    symbols = run("objdump", "-T", str(executable))
    return max(tuple(int(part) for part in version.split("."))
               for version in re.findall(r"GLIBC_(\d+(?:\.\d+)+)", symbols))


def needed_libraries(executable):
    return re.findall(r"NEEDED\s+(\S+)", run("objdump", "-p", str(executable)))


def owner_free(info):
    """Keep the build account's name out of the tarball."""
    info.uid = info.gid = 0
    info.uname = info.gname = ""
    return info


def package(glibc):
    if shutil.which("cargo-zigbuild") is None:
        raise SystemExit("Install cargo-zigbuild and zig first; a plain cargo build ties the binary to this machine's glibc")
    cargo_config_path = REPOSITORY / ".cargo/config.toml"
    cargo_config_before = sha(cargo_config_path)
    # Keep this machine's folders out of the shipped binary: panic locations would
    # otherwise name the user profile through the Cargo/rustup homes and the checkout.
    cargo_home = os.environ.get("CARGO_HOME") or str(Path.home() / ".cargo")
    remapping = {str(REPOSITORY): "/dudel", cargo_home: "/cargo", run("rustc", "--print", "sysroot"): "/rustc"}
    if any("'" in path for path in remapping):
        raise SystemExit("Build paths cannot contain apostrophes.")
    flags = ", ".join(f"'--remap-path-prefix={path}={target}'" for path, target in remapping.items())
    before = source_identity()
    subprocess.run(["cargo", "zigbuild", "--locked", "--release", "--bin", "dudel", "--target", f"{TARGET}.{glibc}",
                    "--config", f"target.{TARGET}.rustflags=[{flags}]"], cwd=REPOSITORY, check=True)
    after = source_identity()
    if before != after or sha(cargo_config_path) != cargo_config_before:
        raise SystemExit("Source changed during build; rerun against a stable workspace.")
    metadata_text = run("cargo", "metadata", "--locked", "--format-version", "1")
    metadata = json.loads(metadata_text)
    root = next(p for p in metadata["packages"]
                if p["name"] == "dudel" and Path(p["manifest_path"]) == REPOSITORY / "Cargo.toml")
    name = f'dudel-{root["version"]}-linux-x64'
    releases = REPOSITORY / "artifacts" / "releases"
    releases.mkdir(parents=True, exist_ok=True)
    stage = releases / name
    stage.mkdir()

    executable = stage / "dudel"
    shutil.copy2(Path(metadata["target_directory"]) / TARGET / "release" / "dudel", executable)
    subprocess.run(["strip", "--strip-unneeded", str(executable)], check=True)
    ceiling = glibc_ceiling(executable)
    if ceiling > tuple(int(part) for part in glibc.split(".")):
        raise SystemExit(f"Executable needs glibc {ceiling}, newer than the {glibc} requested")
    python = sys.executable
    subprocess.run([python, "scripts/check_public_tree.py", "--binary", str(executable)], cwd=REPOSITORY, check=True)

    linux = REPOSITORY / "scripts" / "linux"
    shutil.copy2(linux / "RELEASE_README.md", stage / "README.md")
    shutil.copy2(linux / "dudel.desktop", stage)
    shutil.copy2(linux / "install.sh", stage)
    for document in DOCUMENTS:
        shutil.copy2(REPOSITORY / document, stage)
    for license_file in REPOSITORY.glob("LICENSE*"):
        shutil.copy2(license_file, stage)

    build = {
        "version": root["version"],
        "platform": "linux",
        "revision": run("git", "rev-parse", "HEAD"),
        "working_tree": run("git", "status", "--porcelain").splitlines(),
        "built_utc": datetime.now(timezone.utc).isoformat(),
        "cargo": run("cargo", "--version"),
        "rustc": run("rustc", "--version"),
        "linker": run("cargo-zigbuild", "--version").splitlines()[0],
        "target": f"{TARGET}.{glibc}",
        "glibc_required": ".".join(map(str, ceiling)),
        "features": "default (profiling disabled)",
        # Targets only; the local folders they replace are deliberately not recorded.
        "path_remapping": list(remapping.values()),
        "executable_sha256": sha(executable),
        "artifacts": [{"path": "dudel", "sha256": sha(executable), "imports": needed_libraries(executable)}],
        "source_inputs": after,
        "cargo_config": {"path": ".cargo/config.toml", "sha256": cargo_config_before,
                         "text": cargo_config_path.read_bytes().decode("utf-8")},
        "acceptance": "Experimental Linux build; RELEASE_DECISION.md covers Windows only. See LINUX_PORT.md for open checks.",
    }
    (stage / "BUILD.json").write_text(json.dumps(build, indent=2) + "\n", encoding="utf-8")
    metadata_file = Path(metadata["target_directory"]) / f"package-metadata-{name}.json"
    metadata_file.write_text(metadata_text, encoding="utf-8")
    subprocess.run([python, "scripts/package_source.py", "--repository", str(REPOSITORY), "--destination", f"{stage}-source",
                    "--binary", str(stage), "--metadata", str(metadata_file)], cwd=REPOSITORY, check=True)
    # License expressions identify review work; they do not replace license texts/notices.
    dependencies = sorted(({key: p.get(key) for key in ("name", "version", "license", "source")} for p in metadata["packages"]),
                          key=lambda p: (p["name"], p["version"]))
    (stage / "DEPENDENCIES.json").write_text(json.dumps(dependencies, indent=2) + "\n", encoding="utf-8")

    archive = releases / f"{name}.tar.gz"
    with tarfile.open(archive, "x:gz", compresslevel=9) as tar:
        tar.add(stage, arcname=name, filter=owner_free)
    # Final gate on everything published: no personal paths or private research.
    subprocess.run([python, "scripts/check_public_tree.py", "--archive", str(archive), f"{stage}-source.zip"], cwd=REPOSITORY, check=True)
    archive.with_name(f"{archive.name}.sha256").write_text(f"{sha(archive)}  {archive.name}\n", encoding="ascii")
    print(f"Package: {archive}")
    print("Experimental Linux build; publication has not been performed.")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--glibc", default="2.31", help="oldest glibc the binary should run on")
    package(parser.parse_args().glibc)
