# Review of four manifest-only licence declarations

Reviewed candidate `dudel-0.1.0-candidate-20261019-111249-426`.
[Exact release/source/notice evidence](../benchmarks/legacy-declarations-review-20261019.json)
records the pinned revisions, manifest/source hashes, authors and packaged terms.

| Package | Explicit upstream terms | Upstream release manifest |
| --- | --- | --- |
| block 0.1.6 | MIT | [47178790](https://raw.githubusercontent.com/SSheldon/rust-block/47178790cfc9d4a8b092051d8b413b78bd31254a/Cargo.toml) |
| dispatch 0.2.0 | MIT | [82d6c7a5](https://raw.githubusercontent.com/SSheldon/rust-dispatch/82d6c7a5b75dc0c71c3f46f87bb6c16a476f7748/Cargo.toml) |
| malloc_buf 0.0.6 | MIT | [a7811e5f](https://raw.githubusercontent.com/SSheldon/malloc_buf/a7811e5f4c6f9685dd57ad9bfe34e9bf6b0ba4f9/Cargo.toml) |
| hexf-parse 0.2.1 | CC0-1.0 | [4225763d](https://raw.githubusercontent.com/lifthrasiir/hexf/4225763d744183d720f575ae96d04161b4d08ea0/parse/Cargo.toml) |

All four releases lack a standalone upstream licence file, but each explicitly
declares its terms in the release manifest. Cargo's [manifest documentation](https://doc.rust-lang.org/cargo/reference/manifest.html#the-license-and-license-file-fields)
defines `license` as the licence under which the package is released and describes
`license-file` as the separate text-file mechanism. The absence of the latter is
not evidence that the former is absent.

The review decision is to accept these four exact versions for the notice bundle
on their explicit upstream declarations, retaining their original sources and
attribution. This is a documented engineering interpretation of the evidence,
not a claim that upstream supplied a standalone file. The first three manifests
name Steven Sheldon; the fourth names Kang Seonghoon. The matching source archive
contains every original crate file: 4, 12, 2 and 4 files respectively all match
their registry checksums. Upstream `src/lib.rs` bytes also match the packaged
sources at the pinned release revisions.

The binary and matching source preserve byte-identical `UPSTREAM_Cargo.toml`,
`ATTRIBUTION.md` and clearly labelled `LICENSE_REFERENCE_MIT.txt` or
`LICENSE_REFERENCE_CC0-1.0.txt`. Their hashes match the frozen notice-provenance
index. References remain references: no copyright year, ownership statement or
upstream-authored licence file has been invented. Redistribution must retain
these files and the original source/attribution, and version/provenance changes
require a new review.

This resolves the four named manifest-only review items for this candidate's
bundle. It does not certify every native subcomponent, trademark, patent or
future dependency version, and it does not close G4 or approve public release.
The original candidate archives are unchanged. Historical checkpoints that list
these items as pending retain their historical meaning; this is the current
disposition.
