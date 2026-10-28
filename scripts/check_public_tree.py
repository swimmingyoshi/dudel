"""Refuse personal paths and private research in what the public repository publishes.

Usage: python scripts/check_public_tree.py                  # every tracked file
       python scripts/check_public_tree.py --binary PATH ...  # built exe/dll files
       python scripts/check_public_tree.py --archive ZIP ...  # release ZIPs, minus third-party trees

Built-in rules reject Windows user-profile paths and this checkout's absolute path.
Owner-specific terms (private note names and similar) come from a deny-list kept
outside the repository, one case-insensitive term per line, `#` for comments:
$DUDEL_PUBLIC_DENYLIST, else ../dudel-research/public-denylist.txt. Without one,
only the built-in rules run (as on CI).
"""
import os
import re
import subprocess
import sys
import zipfile
from pathlib import Path

# Upstream crate sources and license texts carry their own example profile paths.
THIRD_PARTY = ("vendor/", "third-party-licenses/")

# A profile folder under Users, JSON-escaped or with forward slashes; C:\Users\<name> passes.
USER_PROFILE = rb"[A-Za-z]:(?:\\|/)+Users(?:\\|/)+(?!Public\b|Default\b)[A-Za-z0-9_.-]"
USER_PROFILE_WIDE = rb"[A-Za-z]\x00:\x00(?:\\\x00|/\x00)+U\x00s\x00e\x00r\x00s\x00(?:\\\x00|/\x00)+[A-Za-z0-9_.-]\x00"


def deny_terms(repository):
    configured = os.environ.get("DUDEL_PUBLIC_DENYLIST")
    path = Path(configured) if configured else repository.parent / "dudel-research" / "public-denylist.txt"
    if not path.is_file():
        return None
    lines = path.read_text(encoding="utf-8-sig").splitlines()
    return [line.strip() for line in lines if line.strip() and not line.lstrip().startswith("#")]


def literal(name, text):
    """Case-insensitive matches for `text` as UTF-8 and as UTF-16LE (wide strings in binaries)."""
    raw = text.encode("utf-8")
    wide = b"".join(re.escape(bytes([c])) + rb"\x00" for c in raw)
    return [(name, re.compile(re.escape(raw), re.IGNORECASE)), (name, re.compile(wide, re.IGNORECASE))]


def rules(repository, terms):
    checks = [("user profile path", re.compile(USER_PROFILE)),
              ("user profile path", re.compile(USER_PROFILE_WIDE))]
    root = str(repository)
    for spelling in {root.replace("/", "\\"), root.replace("\\", "/"), root.replace("/", "\\").replace("\\", "\\\\")}:
        checks += literal("checkout path", spelling)
    for term in terms or []:
        checks += literal(f"deny-list term {term!r}", term)
    return checks


def scan(data, checks):
    """Sorted unique (line number, rule name) for every match in `data`."""
    return sorted({(data.count(b"\n", 0, m.start()) + 1, name)
                   for name, pattern in checks for m in pattern.finditer(data)})


def archive_members(path):
    """(label, bytes) for each first-party file in a release ZIP with one top-level folder."""
    with zipfile.ZipFile(path) as archive:
        for info in archive.infolist():
            inner = info.filename.split("/", 1)[-1]
            if not info.is_dir() and not inner.startswith(THIRD_PARTY):
                yield f"{Path(path).name}:{inner}", archive.read(info)


def targets(repository, arguments):
    if "--binary" in arguments:
        for name in arguments[arguments.index("--binary") + 1:]:
            yield name, Path(name).read_bytes()
    elif "--archive" in arguments:
        for name in arguments[arguments.index("--archive") + 1:]:
            yield from archive_members(name)
    else:
        names = subprocess.check_output(["git", "ls-files", "-z"], cwd=repository).decode("utf-8").split("\0")
        for name in names:
            if name and (repository / name).is_file():
                yield name, (repository / name).read_bytes()


def main():
    repository = Path(__file__).resolve().parents[1]
    terms = deny_terms(repository)
    checks = rules(repository, terms)
    checked = failures = 0
    for label, data in targets(repository, sys.argv[1:]):
        checked += 1
        for line, name in scan(data, checks):
            print(f"{label}:{line}: {name}")
            failures += 1
    scope = "built-in rules" if terms is None else f"built-in rules and {len(terms)} deny-list terms"
    print(f"Checked {checked} files with {scope}: {failures} problem(s).")
    sys.exit(1 if failures else 0)


if __name__ == "__main__":
    main()
