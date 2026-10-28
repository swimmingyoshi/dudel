# Maximum-document idle memory: heap retention, not live ownership

2026-10-20. Follow-up to the frozen-candidate study
`artifacts/releases/dudel-0.1.0-candidate-20261019-111249-426/diagnostics/target/qa/study-20261020-173514-181`.
Its three standard 20-layer runs passed. The 32-layer noise, mixed-brush, two-hour run
(62 rounds) stopped with `memory-targets-not-met`:

| Stage | Net, MiB (limit 32) | Slope, MiB/round (limit 1) | Result |
| --- | ---: | ---: | --- |
| Initial idle | 3.9 | 7.46 | fail |
| History released, before save | 7.2 | 0.11 | pass |
| Final idle | 54.0 | 0.84 | fail |

Tracked ownership (registered snapshots, display, history) was constant in every stage.

## Pattern

- **Initial idle.** From round 23, private commitment alternates between about 985 MiB
  and 1,370–1,510 MiB. Resident memory moves with it. The high readings rise about
  5 MiB per round. Tracked live memory stays at 512 MiB, the baseline only.
- **Final idle.** Private commitment rises from 1,501 to about 1,560 MiB by round 36,
  then flattens.

## Allocation-profiled repeat

Setup:
- Temporary worktree at the candidate commit `fda875f` (removed afterwards; reports moved into this repo's local `target/qa`), built with
  `cargo build --offline --release --features allocation-profile`.
- Same workload: DX12, 32 layers, noise density, mixed brushes, 40 rounds, 1280x800 at 100%.
- Exited 0 after 5,828 s. Report: `target/qa/alloc-max32-20261020/report.json`
  (local, not committed).

It reproduced: initial idle switched to the high mode at round 25, reading
1,447–1,503 MiB, with low readings in rounds 29, 32 and 38.

| Stage | Live Rust requests, MiB | Private commitment, MiB |
| --- | --- | --- |
| Initial idle | **551 in all 40 rounds** | 999–1,019 (low), 1,447–1,503 (high) |
| History released, before save | constant | 1,037–1,070 |
| Final idle | **1,062 in all 40 rounds** | 1,534 → 1,594, flat after about round 25 |

Final-idle resident memory stayed between 1,272 and 1,283 MiB while commitment grew.

## Conclusion

No Rust owner retains the replaced document. Live requested bytes are identical on
low and high rounds. The extra ~450 MiB at initial idle is freed memory that the
process heap keeps committed and resident.

That fits how pixels are stored:
- Each tile is a `Vec<u8>` of 256 x 256 x 4 bytes (256 KiB), which is below the size
  at which the Windows NT heap hands large blocks to `VirtualAlloc`.
- A reopened 32-layer document is about 2,048 such blocks inside heap segments.
- Whether those segments are decommitted after the drop depends on heap layout, so
  the outcome varies from round to round.

The final-idle growth is also outside live Rust requests. It is bounded and settles.
Commitment grows without matching resident growth, which points to heap backing or
driver commitment, not touched data.

## Trial: compact the process heap after a document replace

The change was first tried in the temporary worktree:
- `apply_action` sets `heap_compact_pending`.
- `compact_heap_when_settled` runs once from `update` and calls
  `HeapCompact(GetProcessHeap(), 0)`.
- It waits until the display, render queue, load and save are idle and the thumbnail
  matches the current revision.
- About 20 lines across `document_actions.rs` and `main.rs`.

Same profiled 40-round workload. Exited 0 after 4,981 s. Report:
`target/qa/alloc-max32-heapcompact-20261020/report.json`.
Values are the per-stage `assess_soak_memory.py` metrics (five warm-up rounds).

| Stage | Net / slope before (MiB, MiB/round) | Net / slope with compaction |
| --- | ---: | ---: |
| Initial idle | 492.7 / 15.50 (fail) | **1.2 / 0.05 (pass)** |
| History released, before save | 2.4 / 0.11 | −7.4 / −0.20 |
| Final idle | 49.5 / 1.34 (fail) | **−1.6 / 0.10 (pass)** |

- **Private commitment.** Initial idle stayed at 1,012–1,018 MiB in all 40 rounds.
  Final idle stayed at 1,534–1,546 MiB.
- **Live Rust requests.** Unchanged: 551 MiB at initial idle, 1,062 MiB at final idle.
- **Callback intervals.** p50 13.4 ms (unchanged), p99 34.0 ms (was 63.5), p99.9 102.7 ms
  (was 113.5), max 1,092 ms (was 1,392). There was no sign of a compaction hitch. These
  are instrumented timings, not a responsiveness baseline.

The overall `per_run_memory_targets_pass` is still false. That is only because the
assessment correctly rejects allocation-profiled builds (`configuration_target_pass`).
Acceptance needs an uninstrumented candidate that contains the change, followed by the
full study.

Ported to the main working tree on 2026-10-20. Closing a tab also sets the flag. The
flag is process-wide on purpose: the heap is shared, so it is not part of
`project_session!`. Regression test: `replaced_document_compacts_heap_once_after_thumbnail_settles`.
Checks:
- Default tests: 886 passed, 3 ignored.
- All-feature tests: 895 passed, 3 ignored.
- Warnings-denied Clippy passed in both configurations, and `cargo fmt --check` passed.

Candidate with the change: `artifacts/releases/dudel-0.1.0-candidate-20261021-190309-093`
(revision `2227f46` plus 26 working-tree entries, default features, executable SHA-256
`7C3090D3…92F659D7`). `RunPerformanceStudy.ps1 -CheckOnly` passed.

Full study on that candidate: **passed** (2026-10-21 16:38–19:31 local, exit 0). Reports:
`…190309-093/diagnostics/target/qa/study-20261021-223831-433`.
`STUDY.json` has `status: passed` and `release_accepted: false`; the other release gates
are still open.

| Session | Rounds | Initial idle net / slope | Before-save net / slope | Final idle net / slope |
| --- | ---: | ---: | ---: | ---: |
| standard20 run 1 | 30 | 2.1 / 0.04 | 5.6 / 0.07 | 3.8 / 0.09 |
| standard20 run 2 | 30 | 1.7 / 0.09 | −0.1 / −0.02 | 4.4 / 0.11 |
| standard20 run 3 | 30 | −1.4 / 0.03 | 4.8 / −0.05 | 3.8 / 0.08 |
| maximum32, two hours | 59 (7,247 s) | 4.8 / 0.07 | 1.9 / −0.01 | 7.8 / 0.10 |

The maximum32 session had previously failed at 3.9 / **7.46** and **54.0** / 0.84.
All job-peak targets passed. Continuity for maximum32: the largest unexplained callback
gap was 86.4 ms, with 0.002 s of estimated suspension.

## Status

**Resolved (2026-10-21)** by option 2 below. Heap compaction is committed, and the full
study passed on the uninstrumented candidate. Release criteria were not changed.

Options that were considered:
1. Change pixel-tile allocation, for example a dedicated private heap or a tile pool
   that decommits or reuses. Not needed now; revisit only if the retention returns in
   other workflows.
2. **Chosen:** explicitly reclaim heap memory once a document replace or tab close has
   settled.
3. Revise the gate so it separates committed heap backing from live ownership. Not
   pursued.
