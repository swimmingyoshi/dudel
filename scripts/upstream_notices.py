"""Recover omitted workspace license files at a crate's recorded Git revision.

Only GitHub repository metadata and fixed commit hashes are accepted. Unresolved
legacy crates stay explicitly pending; never substitute a moving default branch.
"""
import hashlib
import json
from pathlib import Path, PurePosixPath
import re
import tomllib
from urllib.error import HTTPError, URLError
from urllib.parse import urlparse, quote
from urllib.request import Request, urlopen

LICENSE_NAME = re.compile(r"(?i)^(licen[sc]e|copying|notice|unlicense|copyright)")


def notice_path(path):
    """Include differently named font terms as well as conventional notices."""
    return (any(LICENSE_NAME.match(part) or re.search(r"(?i)licen[sc]e", part)
                or re.fullmatch(r"(?i)(authors.*|ofl\..*|ufl\..*)", part) for part in path.parts)
            or ("fonts" in path.parts and path.suffix.lower() in (".txt", ".md")))
# Release tags inspected against their published crate's manifest and lib.rs.
# These old releases predate cargo_vcs_info and have no standalone license file.
LEGACY = {
    ("block", "0.1.6"): ("SSheldon", "rust-block", "47178790cfc9d4a8b092051d8b413b78bd31254a", "", "MIT"),
    ("dispatch", "0.2.0"): ("SSheldon", "rust-dispatch", "82d6c7a5b75dc0c71c3f46f87bb6c16a476f7748", "", "MIT"),
    ("malloc_buf", "0.0.6"): ("SSheldon", "malloc_buf", "a7811e5f4c6f9685dd57ad9bfe34e9bf6b0ba4f9", "", "MIT"),
    ("hexf-parse", "0.2.1"): ("lifthrasiir", "hexf", "4225763d744183d720f575ae96d04161b4d08ea0", "parse/", "CC0-1.0"),
}


def fetch(url, cache):
    key = hashlib.sha256(url.encode()).hexdigest()
    path = cache / key
    if path.is_file():
        return path.read_bytes()
    request = Request(url, headers={"User-Agent": "Dudel-source-packager", "Accept": "application/vnd.github+json"})
    with urlopen(request, timeout=20) as response:
        data = response.read(8 * 1024 * 1024 + 1)
    if len(data) > 8 * 1024 * 1024:
        raise ValueError("Upstream notice response exceeds bound")
    path.write_bytes(data)
    return data


def bundled(package, crate, target, bundle=None):
    """Use reviewed fixed bytes before any network lookup; reject stale bindings."""
    bundle = bundle or Path(__file__).parent / "upstream-notices"
    index = json.loads((bundle / "INDEX.json").read_text(encoding="utf-8"))
    if index["version"] != 1:
        raise ValueError("Unsupported bundled notice index")
    key = f'{package["name"]}-{package["version"]}'
    record = index["packages"].get(key)
    if record is None:
        return None
    checksum = json.loads((crate / ".cargo-checksum.json").read_text())["package"]
    manifest = hashlib.sha256((crate / "Cargo.toml").read_bytes()).hexdigest().upper()
    if (checksum != record["crate_checksum"] or manifest != record["manifest_sha256"]
            or package.get("repository") != record["repository"]
            or package.get("license") != record["declared_license"]):
        raise ValueError(f"Bundled notice source binding mismatch: {key}")
    files = []
    # Check every byte before publishing any notices for this package.
    for entry in record["files"]:
        relative = PurePosixPath(entry["relative"])
        digest = entry["sha256"]
        if (relative.is_absolute() or not relative.parts or ".." in relative.parts
                or any(":" in p or "\\" in p for p in relative.parts)
                or not re.fullmatch(r"[A-F0-9]{64}", digest)):
            raise ValueError(f"Invalid bundled notice path or digest: {key}")
        data = (bundle / "blobs" / digest).read_bytes()
        if hashlib.sha256(data).hexdigest().upper() != digest:
            raise ValueError(f"Bundled notice content mismatch: {key}")
        files.append((entry, data, target / Path(*relative.parts)))
    result = []
    for entry, data, output in files:
        output.parent.mkdir(parents=True, exist_ok=True)
        output.write_bytes(data)
        result.append({"path": output, **{k: v for k, v in entry.items() if k != "relative"}})
    return result


def collect(package, crate, target, cache):
    fixed = bundled(package, crate, target)
    if fixed is not None:
        return fixed
    legacy = LEGACY.get((package["name"], package["version"]))
    if legacy:
        return legacy_reference(package, crate, target, cache, legacy)
    repository = urlparse(package.get("repository") or "")
    if repository.hostname != "github.com":
        return []
    pieces = repository.path.strip("/").split("/")
    if len(pieces) < 2:
        return []
    owner, repo = pieces[0], pieces[1].removesuffix(".git")
    if not all(re.fullmatch(r"[A-Za-z0-9_.-]+", p) for p in (owner, repo)):
        return []
    vcs_path = crate / ".cargo_vcs_info.json"
    if not vcs_path.is_file():
        return []
    vcs = json.loads(vcs_path.read_text(encoding="utf-8"))
    revision = vcs.get("git", {}).get("sha1", "")
    if not re.fullmatch(r"[a-fA-F0-9]{40}", revision):
        return []
    prefix = PurePosixPath(vcs.get("path_in_vcs") or ".")
    if prefix.is_absolute() or ".." in prefix.parts or any(":" in p or "\\" in p for p in prefix.parts):
        return []
    cache.mkdir(parents=True, exist_ok=True)
    try:
        tree_url = f"https://api.github.com/repos/{owner}/{repo}/git/trees/{revision}?recursive=1"
        tree = json.loads(fetch(tree_url, cache))
        if tree.get("truncated"):
            return []
        ancestors = {prefix, *prefix.parents}
        files = []
        for entry in tree.get("tree", []):
            path = PurePosixPath(entry["path"])
            if entry["type"] != "blob" or path.is_absolute() or ".." in path.parts or any(":" in p or "\\" in p for p in path.parts):
                continue
            if not notice_path(path):
                continue
            ancestor_license_folder = any(LICENSE_NAME.match(part) and PurePosixPath(*path.parts[:index]) in ancestors
                                          for index, part in enumerate(path.parts))
            # Workspace-root/ancestor terms plus this package's asset notices.
            if path.parent not in ancestors and not path.is_relative_to(prefix) and not ancestor_license_folder:
                continue
            if entry.get("size", 0) > 1024 * 1024:
                continue
            url = f"https://raw.githubusercontent.com/{owner}/{repo}/{revision}/{quote(path.as_posix())}"
            data = fetch(url, cache)
            output = target / "upstream" / Path(*path.parts)
            output.parent.mkdir(parents=True, exist_ok=True)
            output.write_bytes(data)
            files.append({"path": output, "url": url, "revision": revision,
                          "sha256": hashlib.sha256(data).hexdigest().upper()})
        return files
    except (HTTPError, URLError, TimeoutError, ValueError, KeyError) as error:
        (target / "upstream-review.txt").parent.mkdir(parents=True, exist_ok=True)
        (target / "upstream-review.txt").write_text(str(error), encoding="utf-8")
        return []


def legacy_reference(package, crate, target, cache, legacy):
    owner, repo, revision, prefix, license_id = legacy
    cache.mkdir(parents=True, exist_ok=True)
    base = f"https://raw.githubusercontent.com/{owner}/{repo}/{revision}/{prefix}"
    manifest = fetch(base + "Cargo.toml", cache)
    info = tomllib.loads(manifest.decode())["package"]
    if (info["name"], info["version"], info["license"]) != (package["name"], package["version"], license_id):
        raise ValueError("Legacy release manifest does not match the vendored crate")
    upstream = fetch(base + "src/lib.rs", cache)
    if hashlib.sha256(upstream).digest() != hashlib.sha256((crate / "src/lib.rs").read_bytes()).digest():
        raise ValueError("Legacy release source does not match the vendored crate")
    target.mkdir(parents=True, exist_ok=True)
    manifest_file = target / "UPSTREAM_Cargo.toml"
    manifest_file.write_bytes(manifest)
    url = f"https://spdx.org/licenses/{license_id}.txt"
    data = fetch(url, cache)
    reference = target / f"LICENSE_REFERENCE_{license_id}.txt"
    reference.write_bytes(data)
    (target / "ATTRIBUTION.md").write_text(
        f'# {package["name"]} {package["version"]}\n\n'
        f'Authors listed by the matching upstream release: {", ".join(info.get("authors", []))}.\n\n'
        f'That release declares `{license_id}` in Cargo.toml but provides no separate license file. '
        'The original source and manifest are preserved; LICENSE_REFERENCE is the canonical SPDX text, '
        'not an invented upstream copyright notice. Source lib.rs was byte-compared against the fixed release revision.\n\n'
        f'Upstream: {base}\nLicense reference: {url}\n', encoding="utf-8")
    return [{"path": path, "url": origin, "revision": revision,
             "kind": "verified-upstream-manifest" if path == manifest_file else "canonical-license-reference",
             "sha256": hashlib.sha256(path.read_bytes()).hexdigest().upper()}
            for path, origin in [(manifest_file, base + "Cargo.toml"), (reference, url), (target / "ATTRIBUTION.md", base)]]
