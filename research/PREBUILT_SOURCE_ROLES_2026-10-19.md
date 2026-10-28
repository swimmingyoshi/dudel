# Prebuilt payload and provided-source roles

Reviewed candidate `dudel-0.1.0-candidate-20261019-111249-426`.
[Role/source evidence](../benchmarks/prebuilt-role-review-20261019.json) extends
the [native inventory](NATIVE_COMPONENT_REVIEW_2026-10-19.md). Original payloads
and package terms remain unchanged. The binary ZIP's sole native program is
`dudel.exe`; it does not ship the upstream fixture DLLs or native import libraries
as additional executable files. Its optional diagnostic Python scripts are source.

| Payload | Role and source accounting |
| --- | --- |
| Sixteen Windows architecture/version import libraries | Original Microsoft packages describe these as Windows import libraries under MIT OR Apache-2.0. Each includes its original manifest, platform build script and Rust crate source. `windows-targets` and `windows-link` source provide external-import macros and raw-dylib paths. These are upstream platform link inputs, not unidentified Dudel implementation binaries. |
| Two libloading DLLs | Ordinal-loading test fixtures. The preserved `tests/windows.rs` describes their single exported test function and credits WindowsBunny/retep998. Their filenames are absent from the production library source. |
| Four wit-bindgen payloads | WASI helper archive/objects and an example interface module. Helper C sources and the target-conditional build script are preserved. The script returns before linking the helper archive on non-Wasm targets. The locked Windows normal/build dependency tree does not select wit-bindgen. |
| 341 vcpkg library/DLL-named fixtures | All are zero-byte placeholders, not executable Boost/OpenSSL libraries. |

The documented Windows source build selects `--bin dudel`. Cargo's
[target-selection documentation](https://doc.rust-lang.org/cargo/commands/cargo-build.html#target-selection)
distinguishes binaries, tests and examples. The presence of upstream dependency
tests/examples in the complete source archive is not evidence that their fixture
programs are linked into Dudel. Original third-party terms continue to apply to
those separately preserved materials.

The engineering disposition is to retain these unchanged upstream platform inputs
and fixtures, with their original terms, manifests, build scripts and provided
interface/helper sources. The complete source package has already passed the
[fresh offline Windows build](../benchmarks/adjustment-offline-rebuild-native-safety-20261019.json)
using the documented compiler/SDK prerequisites. No missing Dudel-authored source
payload was identified in this review.

This accounts for the prebuilt inventory and the sources/interfaces that accompany
it. It does not claim that all import libraries or test DLLs were independently
regenerated, that the alternate raw-dylib configuration was built, or that every
possible target/redistribution arrangement has been accepted. The GPL's
[corresponding-source and system-library definitions](../LICENSE) must remain
distinct from a file checksum or a successful build. The final G4 review stays
open alongside clean Windows, channel/signing and final candidate acceptance.
No dependency, build setting, executable or archive was changed for this review.
