# Bundled asset and MPL source review

Candidate `dudel-0.1.0-candidate-20261019-111249-426`, executable
`8D2C768E9179B61A9D05FD884E791C763465C6A3C8FB46E763399670ABE9279C`.
[Exact file evidence](../benchmarks/bundled-assets-review-20261019.json) records
hashes, executable offsets, font name tables and original crate checksums.
This review narrows the bundled-component work; it does not close G4.

## Fonts actually embedded

All four complete TTF payloads match their Cargo crate checksums and occur
byte-for-byte in the executable. The source archive retains them under
`vendor/epaint_default_fonts-0.33.3/fonts`. The binary ZIP preserves the listed
standalone terms under `third-party-licenses/epaint_default_fonts-0.33.3/fonts`.
No font was modified or renamed by Dudel.

| Font | Preserved terms | Attribution evidence |
| --- | --- | --- |
| Hack-Regular | Hack-Regular.txt, including MIT and Bitstream Vera terms | Source Foundry and Bitstream notices in both the font metadata and standalone terms; DejaVu public-domain statement retained |
| NotoEmoji-Regular | OFL.txt | Google copyright and OFL identification in the original font metadata |
| Ubuntu-Light | UFL.txt | Canonical copyright and UFL identification in the original font metadata |
| emoji-icon-font | emoji-icon-font-mit-license.txt | John Slegers copyright in the standalone MIT notice; this TTF has no copyright/license name-table fields |

The packaged Hack terms permit sale within a larger software package and retain
their naming/attribution conditions. The font itself is not a standalone Dudel
product. Canonical's [font licence](https://canonical.com/legal/font-licence)
requires preservation of the font's terms and attribution; embedding does not
relicense the font as application code. Its [FAQ](https://canonical.com/legal/font-licence/faq)
discusses that distinction. The exact unchanged payload and separate terms are
verified here. This is not a determination that every possible application/font
combination or future font modification satisfies all applicable terms.

## MPL dependencies

All original files listed in the Cargo checksums match for `option-ext 0.2.0`
(7 files) and `smartstring 1.0.1` (24 files). Their manifest declarations are
`MPL-2.0` and `MPL-2.0+`, respectively. The source files contain no Exhibit-B
incompatibility marker. The full MPL texts contain Exhibit B as part of the
licence itself; that occurrence is not an attached source-file marker.

[Mozilla's FAQ, Q14/Q25](https://www.mozilla.org/en-US/MPL/2.0/FAQ/) and its
[MPL/GPL combination guidelines](https://www.mozilla.org/en-US/MPL/2.0/combining-mpl-and-gpl/)
describe conditions for distributing a larger GPL work containing MPL files.
For unchanged files, the guidelines preserve their original notices and MPL
availability. Dudel's matching source archive includes these complete unchanged
crates, and the binary package names that archive in SOURCE.md. Their source
remains available under its original terms; Dudel's application licence does not
remove those terms. No blanket GPL-only replacement of third-party notices is
made. This audit checks the delivered source and declared terms; it does not
reconstruct the entire upstream development history.

## Remaining review

The four old manifest-only releases have a [separate recorded disposition](LEGACY_LICENSE_DECLARATIONS_2026-10-19.md):
their explicit pinned declarations, original source and preserved attribution
are accepted for this versioned notice bundle. Canonical references remain
clearly labelled; no upstream file or copyright year has been invented. Complete
native-subcomponent compatibility and distribution acceptance remain open. No
artifacts were changed, published, or rebuilt for these reviews.
