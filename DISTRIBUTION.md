# Dudel source and binary distribution

The chosen project license is **GPL-3.0-only**. See [LICENSE](LICENSE) and the
`license` field in Cargo.toml. Third-party dependencies/assets retain their
original license and notice files.

Dudel's source is available to build without paying for an executable. Official
prebuilt Windows packages may be sold for convenience and support. GPLv3 permits
charging for copies and retains recipients' redistribution rights; distributing
binaries requires the corresponding source under the license's terms.
[GNU license](https://www.gnu.org/licenses/gpl-3.0.html),
[GNU distribution guidance](https://www.gnu.org/philosophy/selling.en.html).

## Build from the repository

Install Rust and the Windows Visual Studio C++ build tools / Windows SDK. The
current reference compiler is Rust 1.97.1. From the repository root:

```powershell
cargo build --locked --release --bin dudel
.\target\release\dudel.exe
```

The core build requires Rust/platform tools; Python 3.11+ is used by the optional
verification and packaging scripts. `./scripts/Verify.ps1` runs the complete
ordinary check sequence. The high-memory test has a separate explicit command
in the release checklist.

## Matching source for packaged builds

`./scripts/Package.ps1` produces a normal executable ZIP, a matching source ZIP
and SHA-256 sidecars under `artifacts/releases/`. Each invocation:

- Records the revision, uncommitted state, compiler and exact input hashes, plus
  hashes and PE imports for both `dudel.exe` and `dudel_thumbnails.dll`.
- Builds with `--remap-path-prefix` so the binaries name `/dudel`, `/cargo` and
  `/rustc` instead of the build machine's folders, then refuses either binary or
  ZIP that still contains a user-profile or checkout path
  (`scripts/check_public_tree.py`).
- Copies current source, examples, embedded plugin assets and build scripts,
  including authorized changes not committed yet. User artwork/preferences,
  private research notes, raw traces and repository history are excluded.
- Vendors the locked dependency sources and creates relocatable Cargo source
  configuration. It checks offline dependency resolution for both workspaces from
  the copied folder and rejects thumbnail dependencies absent from the shared inventory.
- Preserves dependency license/copyright/notice files and indexes them in the
  binary package. Source archives retain the complete vendored crates as well.
- Adds `SOURCE.md` to the binary download, identifying the matching source archive
  and its checksum. Source `BUILDING.md` explains the offline build. Source input
  hashes match the binary's build identity; different build environments may
  produce different executable bytes.
- Bundles tracked Markdown/report dependencies so the supplied documentation can
  be followed offline. Missing, private/untracked or oversized references are
  labeled as repository-only; artwork and raw traces are not collected. Build
  input hashes are checked again after documentation preparation.

Distribute the matching source archive alongside each executable download.
Archive availability, bundled subcomponent notices and licensing review are part
of release acceptance, not inferred solely from a metadata SPDX expression.
Validate an actual source rebuild and archive checksums before publishing.
For the accepted Windows 0.1 inputs, `./scripts/Package.ps1 -Release` produces
`dudel-0.1.0-windows-x64.zip` and its matching source ZIP. It requires the version
and both built binary hashes to match `RELEASE_ACCEPTANCE.json`, and refuses to
overwrite an existing release folder. Acceptance and limits are recorded in
[RELEASE_DECISION.md](RELEASE_DECISION.md). Existing offline-rebuild evidence may
be reused only after comparing every compiled input, Cargo settings and both
deliverable hashes; prose-only changes do not require another full rebuild.
From the repository, `python scripts/check_source_archive.py PATH_TO_SOURCE.zip
--output target/qa/UNIQUE_REBUILD_FOLDER` checks the sidecar, extracts and CRC-checks
the archive, verifies recorded source hashes, and performs a fresh offline release
build of the application and, on Windows, the thumbnail DLL in an isolated target
directory. Inspect its `STATE.json` and `build.log`;
`passed` certifies that local rebuild only, not a clean non-developer Windows test.
`python scripts/check_release_notices.py CANDIDATE REVIEWED_CANDIDATE` checks both
archives' sidecars and CRCs, the GPL LICENSE and SOURCE.md link, every dependency's
declared license and notice files against `INDEX.json`, upstream provenance hashes,
and binary/source notice parity. It also reports whether the notices are byte-identical
to an earlier reviewed candidate, so subcomponent reviews only need repeating for
changed files.

## Release channel

The owner selected **our own website/store** on 2026-10-18. Official paid builds
will be distributed there; matching source remains available without buying an
executable. Keep the binary ZIP, matching source ZIP and both SHA-256 sidecars
together for each version. Show the version, Windows architecture, release status,
signing status and corresponding-source download on the product/download page.

The owner chose **unsigned initial distribution on 2026-10-21**, with signing to
follow if donations support it. Current packages are unsigned; release notes explain
the possible SmartScreen prompt. Do not imply a trusted publisher signature or an
accepted production release. Signing credentials and payment secrets do not belong in this repository
or downloadable source packages.

Ordinary hosted CI uploads remain development candidates. The `-Release` package
is prepared against the pinned acceptance record and documented support matrix.
The website/domain, payment integration and public upload are not configured by
this policy document. The publication handoff keeps those launch actions separate
from completed release preparation.
