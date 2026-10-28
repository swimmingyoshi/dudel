# Windows dependency inventory review, 2026-10-17

The default `x86_64-pc-windows-msvc` Cargo normal-edge graph contains 214 distinct
dependency versions. Every one maps to actual notice files in the locally verified
GPL package. `Cargo.toml` and `Cargo.lock` were byte-hash checked against that
package before reusing its notice inventory. The command and per-package mapping
are in `benchmarks/windows-dependency-review.json`.

This is a target dependency graph, not an assertion that all 214 crates contribute
linked runtime code. It can include compiler plugins. It excludes build/development
edges and does not enumerate dynamically loaded system or graphics components.
The complete source and notice bundle still covers all 452 resolved packages;
nothing was removed based on this narrower inventory.

Of the four legacy releases without standalone upstream license files, `block`,
`dispatch` and `malloc_buf` are absent from this Windows normal-edge graph.
`hexf-parse 0.2.1` is present. All four remain included in the complete vendored
source/notice package and their previously recorded release review remains open.
The inventory does not turn absence from this graph into distribution clearance.

The graph also identifies the bundled default fonts and the MPL-declared
`option-ext 0.2.0` and `smartstring 1.0.1`. Their original sources and collected
terms are present in the matching source package; font notice inclusion has
separate exact-file verification in `benchmarks/gpl-package-verification.json`.
These facts support the existing bundled-subcomponent review; they do not close
the release's licensing, clean-machine or final-candidate acceptance gates.

No dependency versions or application behavior changed during this review.
