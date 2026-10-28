# GPL distribution checkpoint, 2026-10-17

The owner chose open source with paid prebuilt executables and free self-builds,
then explicitly selected GPLv3. Cargo metadata and the root LICENSE now identify
Dudel as GPL-3.0-only. Help > About Dudel shows that identity and source/license
links. Its interaction regression passes without changing artwork.

## Frozen candidate

`artifacts/releases/dudel-0.1.0-candidate-20261017-144000-843.zip`

- Executable SHA-256: `2402AE958CFBBDFA178809186FCD34E501C31C4761D619E677E7317E7D494703`.
- Binary ZIP SHA-256: `59EA667FA8686138D6C274D4036F3C45608673340685C867A37C474AF7C9915C`.
- Adjacent matching `-source.zip` SHA-256:
  `4A7A55A610FAC51F487A98ECFEDD315F11759127133036C5A3300AED3CCEC8FE`.
- Both archives have SHA-256 sidecars. Binary ZIP checksum, CRC and required
  documents pass. See `benchmarks/gpl-package-verification.json`.
- Extracted binary launch passed with an isolated DX12 capture profile;
  `target/qa/gpl-binary-archive-startup-20261017/RESULT.json` records the result.
  The 1280 x 800 capture was visually inspected: canvas, toolbar and panels render.

The executable is the same frozen build used by the current three-run memory
controller. Later documentation and verification-script additions do not change
that executable identity. Historical failed packaging attempts remain separate;
they are not accepted distribution packages.

## Source and notices

Package.ps1 builds with locked default features and checks input hashes before
and after compilation. It records the Git revision, dirty state, compiler and
actual source/embedded-asset hashes. The matching source archive includes those
exact inputs, examples, build scripts, locked vendored dependencies, relocatable
`.cargo/config.toml`, LICENSE and offline BUILDING.md. Source input hashes are
checked again after copying. Packaging verifies offline dependency resolution.
The binary's SOURCE.md names the matching source archive and checksum.

The inventory covers 452 resolved registry packages, including development and
other-platform dependencies. Original license, notice, copyright and AUTHORS
files are preserved. Missing workspace files are retrieved only from the crate's
recorded Git revision, with URL/revision/content hashes retained. Vendored crates
are not patched, so Cargo's original checksums remain valid.

Four older releases have no standalone license file in either their crate or
their verified release revision: block 0.1.6, dispatch 0.2.0, malloc_buf 0.0.6 and
hexf-parse 0.2.1. The bundle preserves their upstream manifest declarations and
authors, compares lib.rs bytes with that fixed release, and supplies clearly
identified canonical SPDX reference text. These references do not invent an
upstream copyright notice. They remain explicit release-review items. The index's
empty `manual_review` list means no package lacks collected files/declared terms;
it does not certify complete licensing review or bundled-asset compliance.

## Verification and remaining gates

- Current application: 783 default / 792 all-feature tests pass, one high-memory
  test ignored normally. Formatting, both warnings-denied Clippy configurations
  and default release build pass. The dedicated maximum-project test passed
  separately after the storage changes (100.17 seconds).
- Fourteen Python checks pass, including regressions for font notice names, pre-1980 Cargo timestamps,
  preservation of hidden Cargo configuration and refusing to replace an archive.
- The corrected source ZIP contains 24,591 files; CRC and input hashes pass.
  The independently extracted earlier archive passed a fresh offline release
  build in 11m 29s, then its rebuilt executable passed an isolated DX12 startup
  capture and visual inspection. The corrected source archive differs only in
  documentation/packaging/notice files: every compiled input, Cargo configuration
  and all 23,547 vendored files were byte-compared with that successfully rebuilt
  source archive. No unchanged compilation was repeated. The authoritative records
  are `target/qa/gpl-source-archive-rebuild-20261017/STATE.json` and the package
  verification JSON. A different executable hash from the extracted path is
  expected; byte-reproducible binaries across build paths are not claimed.
- Font notice audit found names outside the original filename matcher. The final
  binary and source now include OFL.txt, UFL.txt, Hack-Regular.txt, the emoji font
  MIT notice and upstream crate MIT/Apache terms; exact bundled copies are checked.
  The earlier 142937 package is superseded because its binary notice bundle lacked
  those font terms. Its complete source already contained the original font terms.

`scripts/check_source_archive.py` performs that independent archive rebuild from
an extracted directory with a fresh target directory. Its build log and state
are the authoritative execution record. This local test uses installed developer
tools; it does not substitute for clean non-developer Windows acceptance.

Hosted CI, clean-machine operation, signing/channel decisions, notice/subcomponent
review and the remaining artwork, performance and physical-workflow gates are
open. Nothing has been published or pushed as part of this checkpoint.

The current memory repeat ran concurrently with compilation/packaging for part of
its duration. Its correctness and per-process memory results do not establish
isolated frame, queue or job timing acceptance.
