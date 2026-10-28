# Archived delivery checkpoints

Text is preserved; relative links are adjusted for this archive folder.
These are historical snapshots moved from DELIVERY_PLAN.md on 2026-10-18.
Words such as current/latest refer to their original checkpoint, not the present
build. See DELIVERY_PLAN.md and RELEASE_CHECKLIST.md for current scope and gates.

## Current assessment

Branch/progress review: `master` (the default branch) and `linux` are
synchronized from `5bd1db5`, which already contains every earlier `master` commit.
See [the current progress review](../research/PROGRESS_REVIEW_2026-10-17.md) for
the newer feature inventory and remaining priorities. The earlier build hashes,
test totals and measurements below apply to their identified historical builds.
The time-based stabilizer replaced the 450 ms catch-up implementation; its
current physical acceptance remains pending. Do not treat the historical
instruction to retain that implementation as a requirement on current code.

Current engineering build: `7F765F63EEA25BCDC2B625FF2A666ECC7F4B3E09DB674D3DD0B5A5C623B654D8`.
791 default / 800 all-feature tests, 28 Python checks, formatting, both Clippy
configurations and the Windows GUI release build pass. A native mixed-brush probe
extended two requested rounds to five, with 181.854 observed seconds, 80 mask edits
and five save/open pairs. Continuity and power-state restoration pass; resource
acceptance was deliberately not evaluated for this four-layer probe.
[Duration protocol](../research/NATIVE_DURATION_RUNS.md) and
[probe evidence](../benchmarks/duration-mixed-probe-results.json).

The earlier maximum/noisy run stopped at a diagnostic bound after 24 complete
rounds and produced no full report. Its old error text conflated time and frame
capacity; external lifetime points to the 300,000-frame limit. No memory/job pass
can be inferred from that incomplete run. The new diagnostic separates bounds,
reserves enough fixed storage and preserves partial errors. Full three-run standard
and measured two-hour maximum/mixed studies remain next, after matching-source
packaging/rebuild verification. Hardware/workflow and clean-machine gates stay open.

### Earlier checkpoints


Latest checkpoint: the artist confirmed the revised stationary catch-up is
"Smooth now". Keep the restartable 450 ms easing. Ordinary native saves now expose
Cancel with a checked pre-publication fence and accurate late-cancel outcomes.
Default/all-feature suites pass 783/792 tests; formatting, both warnings-denied
Clippy configurations, eleven Python checks and release build pass. The dedicated
maximum-project test passed separately after the storage changes (100.17 seconds).
Sixteen child terminations verify native/recovery publication integrity.

The first unprofiled thirty-round twenty-layer DX12 run passed its per-run memory
subtargets: final matching-idle net growth 1.15 MiB and fitted slope 0.102 MiB/round
after five warm-up rounds. All three current-candidate repeats completed (30 rounds,
20 layers each) and meet matching-idle private-memory and tracked-ownership targets.
Final-idle net growth is 3.15-5.57 MiB and fitted slope 0.106-0.208 MiB/round across
the three runs. [Aggregate](../benchmarks/gpl-standard20-three-run-summary.json).
Dense/32-layer, full-job/peak, two-hour, additional brush presets and physical
latency gates remain open.
The chosen license is GPL-3.0-only, with paid prebuilt binaries and freely
buildable source. Matching-source packaging, vendoring, archive integrity and font notice checks
pass. A fresh offline release build and rebuilt-executable launch pass; the final
archive's compiled inputs and vendored files match that build byte-for-byte.
Hosted CI, four legacy notice-review items and clean-machine acceptance remain open.
See [packaging evidence](../research/GPL_DISTRIBUTION_PACKAGING.md).
Details: [smoothing](../research/SMOOTH_STABILIZER_CATCHUP.md),
[cancellation](../research/CANCELABLE_NATIVE_SAVES.md),
[safety](../research/ARTWORK_SAFETY_MATRIX.md), and [distribution](../DISTRIBUTION.md).

Native safety follow-up: real v4/v5 native application-loop interruption/restart,
Restore copy, complete state comparison and separate save/reopen pass. Four small
idle-fixture snapshots completed 15.8-16.9 seconds after edits; six native settings
lifecycle phases pass. See [recovery evidence](../research/NATIVE_ARTWORK_RECOVERY.md).
A discovered 32-layer dense fixture exceeded the existing combined payload cap;
the corrected maximum masked/noisy fixture now validates at exactly 512 MiB.
[Fixture identity and scope](../research/NATIVE_SOAK_FIXTURES.md) are recorded, peak/job
budgets are explicit below, and the new large fixture test runs separately in CI.
Latest suites: 785/794 pass and 18 Python checks pass; two large-memory tests are
ignored normally, with separate passing evidence at their relevant source changes.
The frozen GUI engineering build records lifetime peaks and completed warm-cache
save/open jobs. The first maximum masked/noisy session stopped incompletely after
24 completed rounds at a diagnostic resource limit; it is not an acceptance pass. Source identity:
benchmarks/job-peak-build.json. See RELEASE_CHECKLIST.md for the isolated release
build and native smoke result. FORMAT_SUPPORT.md publishes the strict import matrix.

Execution checkpoint: the first implementation package adds bounded persistent
display workers, reducing the matching 184-byte thread-record proxy increase from
about 1,451 to 43 blocks/round. Twelve-round DX12 correctness passes; process-private
plateau remains open. Windows CI and candidate packaging are implemented locally.
See [the execution report](../research/DISPLAY_WORKER_REUSE.md) for exact identities,
767/776-test results, the passing dedicated high-memory test and remaining gates.

Core illustration tools, background jobs, recovery, responsive workspace, brush
studio and durable preferences are implemented. The release gaps are sustained
native resource stability, current-build physical tablet acceptance, complete
OS/file workflows and reproducible packaging. Earlier captures and tests support
their specific builds, not blanket release acceptance.

At the planning-review checkpoint, the DX12 fix eliminated matching-idle Rust allocation growth, but
process-private commitment still grows about 45 MiB over rounds 4-12. Its remaining
owner/plateau was unresolved. Region-census/band-worker work is now committed in
`dcb5a73`; the execution report above supersedes those measurements for current
priorities. See [the earlier evidence](../research/WGPU_DEFERRED_DESTROY.md).

## Fresh review verification

On the current working tree: `cargo test --locked` passed with 760 tests, zero
failures and one ignored high-memory test; `cargo fmt --all -- --check` and
`cargo clippy --locked --all-targets -- -D warnings` passed. No fresh all-feature,
release-build, native GUI or physical tablet acceptance was performed in this
planning task. Candidate-specific evidence belongs in [RELEASE_CHECKLIST.md](../RELEASE_CHECKLIST.md).
The product remains in development until G1-G4 pass. Completion of this assessment
and plan does not mean the application is release-ready.

