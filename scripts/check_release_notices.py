"""Final notices check for a packaged candidate: archive checksums/CRCs, required
license/source documents, notice index against the dependency inventory, upstream
provenance hashes, binary/source notice parity and carry-over from a reviewed candidate.

Usage: python scripts/check_release_notices.py CANDIDATE_NAME REVIEWED_CANDIDATE_NAME
"""
import hashlib, json, sys, zipfile
from pathlib import Path

REPOSITORY = Path(__file__).resolve().parents[1]
releases = REPOSITORY / "artifacts" / "releases"
name, reviewed = sys.argv[1], sys.argv[2]
sha = lambda b: hashlib.sha256(b).hexdigest().upper()
problems, facts = [], {}

def zip_of(n, suffix=""):
    p = releases / f"{n}{suffix}.zip"
    side = (releases / f"{n}{suffix}.zip.sha256").read_text().split()[0].upper()
    digest = sha(p.read_bytes())
    if digest != side: problems.append(f"{p.name}: sidecar mismatch")
    z = zipfile.ZipFile(p)
    bad = z.testzip()
    if bad: problems.append(f"{p.name}: CRC failure at {bad}")
    return z, digest

binz, bin_sha = zip_of(name)
srcz, src_sha = zip_of(name, "-source")
facts["binary_zip"] = {"sha256": bin_sha, "members": len(binz.namelist())}
facts["source_zip"] = {"sha256": src_sha, "members": len(srcz.namelist())}

def files(z, prefix):
    return {n[len(prefix):]: z.read(n) for n in z.namelist() if n.startswith(prefix) and not n.endswith("/")}

b = files(binz, f"{name}/")
s = files(srcz, f"{name}-source/")

# Required documents
repo_license = (REPOSITORY / "LICENSE").read_bytes()
for doc in ("LICENSE", "SOURCE.md", "THIRD_PARTY_NOTICES.md", "BUILD.json", "DEPENDENCIES.json", "third-party-licenses/INDEX.json"):
    if doc not in b: problems.append(f"binary missing {doc}")
if b.get("LICENSE") != repo_license: problems.append("binary LICENSE differs from repository LICENSE")
if b"GNU GENERAL PUBLIC LICENSE" not in b.get("LICENSE", b"") or b"Version 3, 29 June 2007" not in b.get("LICENSE", b""): problems.append("LICENSE is not GPLv3 text")
if s.get("LICENSE") != repo_license: problems.append("source LICENSE differs from repository LICENSE")
source_md = b.get("SOURCE.md", b"").decode("utf-8", "replace")
if f"{name}-source.zip" not in source_md or src_sha not in source_md: problems.append("SOURCE.md does not name the matching source zip and its SHA-256")
for doc in ("BUILDING.md", "Cargo.lock", ".cargo/config.toml"):
    if doc not in s: problems.append(f"source missing {doc}")

# Both Windows deliverables must match their recorded packaged identities.
build = json.loads(b["BUILD.json"].decode("utf-8-sig"))
recorded_artifacts = build.get("artifacts", [{"path": "dudel.exe", "sha256": build["executable_sha256"]}])
if "dudel_thumbnails.dll" in b and {a["path"] for a in recorded_artifacts} != {"dudel.exe", "dudel_thumbnails.dll"}:
    problems.append("BUILD.json does not identify both packaged Windows artifacts")
for artifact in recorded_artifacts:
    path = artifact["path"]
    if path not in b or sha(b[path]) != artifact["sha256"].upper():
        problems.append(f"packaged artifact hash mismatch: {path}")
facts["artifacts_checked"] = [a["path"] for a in recorded_artifacts]
if "SOURCE_BUILD.json" not in s or json.loads(s["SOURCE_BUILD.json"].decode("utf-8-sig")) != build:
    problems.append("binary/source build identity mismatch")

# Notice index vs dependency inventory
index = json.loads(b["third-party-licenses/INDEX.json"].decode("utf-8-sig"))
deps = json.loads(b["DEPENDENCIES.json"].decode("utf-8-sig"))
dep_map = {(d["name"], d["version"]): d.get("license") for d in deps if d["name"] != "dudel"}
idx_map = {(p["package"], p["version"]): p for p in index["packages"]}
if set(dep_map) != set(idx_map):
    problems.append(f"index/dependency mismatch: missing {sorted(set(dep_map) - set(idx_map))[:5]}, extra {sorted(set(idx_map) - set(dep_map))[:5]}")
listed = set()
provenance_checked = 0
for key, p in idx_map.items():
    if not p["files"]: problems.append(f"{key}: no notice files")
    if not p.get("declared_license"): problems.append(f"{key}: no declared license")
    elif key in dep_map and p["declared_license"] != dep_map[key]: problems.append(f"{key}: license {p['declared_license']} != {dep_map[key]}")
    for f in p["files"]:
        listed.add(f)
        if f not in b: problems.append(f"{key}: listed file missing in binary: {f}")
    # Provenance records the hash of a preserved file; legacy references are renamed, so match by content.
    package_hashes = {sha(b[f]) for f in p["files"] if f in b}
    for prov in p.get("upstream_provenance", []):
        if prov["sha256"].upper() not in package_hashes:
            problems.append(f"{key}: no preserved file matches upstream provenance {prov['url']}")
        provenance_checked += 1
notice_files = {k for k in b if k.startswith("third-party-licenses/") and k != "third-party-licenses/INDEX.json"}
orphans = notice_files - listed
if orphans: problems.append(f"{len(orphans)} notice files not in INDEX, e.g. {sorted(orphans)[:3]}")
facts["manual_review"] = index.get("manual_review")
if index.get("manual_review"): problems.append(f"INDEX manual_review not empty: {index['manual_review']}")
facts["packages"] = len(idx_map)
facts["notice_files"] = len(notice_files)
facts["upstream_provenance_hashes_checked"] = provenance_checked

# Binary/source parity
src_notices = {k: v for k, v in s.items() if k.startswith("third-party-licenses/")}
diff = [k for k in set(src_notices) | set(b) if k.startswith("third-party-licenses/") and src_notices.get(k) != b.get(k)]
if diff: problems.append(f"{len(diff)} notice files differ between binary and source, e.g. {sorted(diff)[:3]}")
facts["binary_source_notice_parity"] = not diff

# Embedded font terms present
for f in ("OFL.txt", "UFL.txt", "Hack-Regular.txt", "emoji-icon-font-mit-license.txt"):
    if f"third-party-licenses/epaint_default_fonts-0.33.3/fonts/{f}" not in b: problems.append(f"font terms missing: {f}")

# Carry-over from the reviewed candidate
rz = zipfile.ZipFile(releases / f"{reviewed}.zip")
r = files(rz, f"{reviewed}/")
r_notices = {k: sha(v) for k, v in r.items() if k.startswith("third-party-licenses/")}
c_notices = {k: sha(v) for k, v in b.items() if k.startswith("third-party-licenses/")}
changed = sorted(k for k in set(r_notices) | set(c_notices) if r_notices.get(k) != c_notices.get(k))
facts["identical_to_reviewed_candidate"] = {"reviewed": reviewed, "notice_files_identical": not changed,
    "dependencies_identical": json.loads(r["DEPENDENCIES.json"].decode("utf-8-sig")) == deps,
    "third_party_notices_md_identical": r.get("THIRD_PARTY_NOTICES.md") == b.get("THIRD_PARTY_NOTICES.md"),
    "changed_files": changed[:20]}

print(json.dumps({"candidate": name, "passed": not problems, "problems": problems, **facts}, indent=2))
sys.exit(1 if problems else 0)
