# Fixed upstream notices

`INDEX.json` maps exact published crate checksums and manifest hashes to original
upstream notice bytes in `blobs/`. Blob names are their SHA-256 hashes; Git must
preserve their bytes without newline conversion. Each entry retains its upstream
URL, fixed revision and any reference-only classification.

These bytes were preserved from the identified, mechanically audited candidate
in the index. Packaging checks the source binding and hashes before using them,
avoiding GitHub availability and anonymous API rate limits for this dependency
set. New package versions fall back to the existing fixed-revision collector.

To refresh, first inspect the new candidate's provenance and unresolved entries,
then run `python scripts/freeze_upstream_notices.py <candidate-directory>
<new-output-directory>` and review its changes. Do not replace the index with
unreviewed downloads or substitute a moving upstream branch.

The blobs retain their original terms. This is a reproducible collection of
notices, not a license compatibility decision. Four legacy crates still carry
explicit canonical-reference classifications rather than invented upstream
copyright notices; bundled subcomponents still require distribution review.
