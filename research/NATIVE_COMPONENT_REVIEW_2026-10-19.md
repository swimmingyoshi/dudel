# Native component inventory and selected-feature review

Candidate `dudel-0.1.0-candidate-20261019-111249-426`.
[Exact payload and feature evidence](../benchmarks/native-component-review-20261019.json)
records original-file hashes, paths, selected features and notices. The locked
dependency graph matches the candidate's Cargo.lock. This is a source/notice and
feature review; it does not replace final distribution or clean-machine acceptance.

## Runtime C implementation

The default Windows graph enables `libsqlite3-sys 0.38.2`'s `bundled` feature and
does not enable SQLCipher. Its build script compiles `sqlite3/sqlite3.c`. That
amalgamation identifies SQLite **3.53.2**, source ID
`2026-06-03 19:12:13 d6e03d8c777cfa2d35e3b60d8ec3e0187f3e9f99d8e2ee9cac695fd6fcdf1a24`.
The same ID occurs in the frozen executable. The amalgamation's copyright
disclaimer is intact, and its source ZIP bytes match the registry checksum.

SQLite's own [copyright statement](https://sqlite.org/copyright.html) places its
deliverable engine code in the public domain and permits commercial distribution.
This is distinct from the Rust wrapper's MIT terms and the separately preserved
SQLCipher BSD terms. SQLCipher sources travel in the complete vendored archive,
but the reviewed default feature selection does not compile them. Their original
source and accompanying licence remain present. No source was relicensed or edited.

The locked Linux graph does not enable `wayland-backend`'s optional `log` feature,
so its build-script condition for the two C logging shims is false. This finding
does not establish physical Wayland behavior or a Linux distribution package.

## Other native source included in the source archive

All **47** C/C++/header/assembly-named files across twelve crates match their
original registry checksums in the source ZIP. Every involved package's collected
notice files match byte-for-byte between binary and source packages.

| Crate / files | Reviewed role |
| --- | --- |
| android-activity / 13 | Android games SDK glue, with original Apache headers; another-platform source |
| cc / 2 | Compiler-family/C++ capability probes used by build tooling |
| libsqlite3-sys / 9 | SQLite/SQLCipher sources, headers and optional WASI shim; selected default engine discussed above |
| ndk-sys / 1 | Android binding wrapper header |
| nix / 1 | Kernel-module test fixture; explicit GPL-2.0+ **or MIT** header and Dual MIT/GPL module tag retained |
| objc-sys / 2 | Optional ObjFW compatibility import headers |
| ttf-parser / 12 | Qt/Freetype/HarfBuzz font-view testing tool source |
| walkdir / 1 | Comparison utility source |
| wayland-backend / 2 | Optional logging shims, disabled in the reviewed Linux graph |
| wayland-protocols-plasma / 1 | Protocol-build test source |
| webbrowser / 1 | iOS test-app bridge header |
| wit-bindgen / 2 | WASI runtime helper C source |

These source files are preserved as upstream crate material. Their presence in
the complete source archive does not imply that they are linked into the Windows
application or that Dudel ships an Android app, kernel module or Qt font viewer.

## Prebuilt-named payloads

The inventory also verifies sixteen Windows target import libraries, two
`libloading` test DLLs and four `wit-bindgen` WASI/example helper payloads against
their original checksums and source ZIP copies. The 341 DLL/library-named vcpkg
fixtures are all zero-byte placeholders, not bundled Boost/OpenSSL runtime code.
`COPYING.LIB` is a licence text and is excluded from binary-payload classification.
Original package terms are preserved for all these files.

This closes the native-source/notice inventory gap and identifies the selected
SQLite implementation. Payload hashes alone do not establish the preferred-source
form or whole-program compatibility of every prebuilt component. The final G4
review must retain that distinction, the previously recorded font/MPL review and
the manifest-only dispositions. System/driver prerequisites and actual clean
Windows behavior remain separate runtime acceptance work. No build, dependency,
package bytes or product behavior changed for this review.
