# Native artwork recovery and settings, 2026-10-17

Frozen engineering executable SHA-256:
`151CC53C708BF5681883E566C686BC355C9CCED3636D8324D948F8EE9037000F`.
[Source/build identity](../benchmarks/native-recovery-build.json).

The normal initialization now delegates recovery discovery to `init_recovery_in`.
Production still obtains the same per-user directory; an explicitly invoked
`--probe-artwork-recovery` supplies a new marked disposable directory instead.
It never loads the user's preferences or artwork. The normal drawing/stabilizer
behavior is unchanged.

## Interrupted native application loop

`scripts/check_native_recovery.py` freezes the executable and runs v4 and v5
separately. Each producer uses the native eframe window and full Dudel update loop:

1. Save an original 65 x 33 two-layer project containing hidden RGB, partial edge
   pixels, opacity and clipping; v5 also includes fractional mask coverage.
2. Change artwork and wait for the real 15-second recovery eligibility timer and
   normal idle repaints. Read/compare the completed snapshot, then repeat for a
   second generation. No backdated timer or accelerated writer repaints are used.
3. Preserve the second generation as the expected recovery state, make a third
   unsnapshotted edit, and publish a ready marker while remaining alive.
4. The controller verifies the child's PID, native container header, completed
   snapshot and file hashes, kills only its own live child and waits for exit.
5. A new native process discovers the interrupted snapshot through the shared
   startup path. The normal Recover artwork window is captured; synthetic pointer
   press/release events hit the visibly rendered Restore copy button.
6. The real load worker restores an unsaved copy without a project path. Compare
   full serialized editable state with generation two, save a separate project
   through the real save worker, reopen it and compare again. The original project
   and interrupted snapshot remain byte-identical across termination and restore.

Both native processes and both restore/save workflows pass. Four observed times
from edit to completed snapshot were **16.899, 15.784, 16.732 and 15.791 seconds**.
Startup and restored/saved screenshots were visually inspected. The inspector's
navigator/color area scrolls independently from its pinned layer panel.

Evidence: [results](../benchmarks/native-artwork-recovery-results.json), raw state,
logs and screenshots under `target/qa/native-artwork-recovery-20261017/`.

This measures small idle fixtures on the reference Windows machine. Larger files,
ongoing strokes, queued edits, active adjustments, cleanup and slow storage can
delay a completed snapshot; the quick start explains those limits. The wrapper
uses normal application UI/load/save behavior with isolated startup directory
injection. It does not test the installed application's per-user directory
discovery on a clean PC, OS mouse/tablet input or physical power loss. The sixteen
separate publication-boundary child terminations cover interrupted writes; this
native test kills after a completed checkpoint with newer edits still unsaved.

## Current-build settings lifecycle

The same frozen executable ran the existing native settings probe in a fresh
profile through `first`, `reopen`, `corrupt`, `repair`, `failure` and `retry`.
All six phases pass. The corruption screenshot visibly shows saving paused and
requires an original-settings backup before replacement. Failure preserves the
original; retry commits changed preferences. The failure fixture blocks a target
with a directory, rather than claiming actual disk exhaustion.

[Settings results](../benchmarks/native-settings-current-results.json); raw native
captures/logs: `target/qa/native-settings-current-20261017/`.

## Regression and remaining acceptance

783 default / 792 all-feature tests pass (one high-memory test ignored normally),
as do formatting, both warnings-denied Clippy configurations, 14 Python checks and
the release build. The dedicated maximum-project round trip already passed after
the last storage changes; the diagnostic/helper extraction does not change codecs.

G1 remains open for the complete OS dialog/error workflow, larger/busy recovery
cadence and device/storage failure matrix. Clean-machine acceptance, sustained
memory/performance, independent workflow and physical XP-Pen gates remain open.

## Optional larger native recovery workloads (2026-10-19)

`scripts/check_native_recovery.py` accepts `--workload large` and
`--workload large-adjustment`. Both use a 2048 x 2048 canvas (the supported maximum
area) with two deterministic noisy raster layers; v5 also has a fractional mask.
The ordinary small workload remains the default. Supply a new unique output
directory under `target/qa`, for example:

```powershell
python scripts/check_native_recovery.py --executable PATH_TO_ENGINEERING_EXE --output target/qa/large-recovery-UNIQUE --workload large
python scripts/check_native_recovery.py --executable PATH_TO_ENGINEERING_EXE --output target/qa/busy-recovery-UNIQUE --workload large-adjustment
```

The busy case opens a real Brightness/Contrast preview and holds it for at least
20 seconds. It requires a visibly changed ready preview, observations beyond the
normal 15-second recovery eligibility interval with no temporary snapshot, then
cancels the preview and verifies that normal checkpointing resumes. Both workloads
verify two generations, interrupt only their own producer, restore through the
startup button, and save/reopen with exact editable state. The recovery timer is
never backdated. Busy-phase diagnostic polling requests 100 ms repaints; the idle
workload uses the normal repaints. Completion timing is sampled before diagnostic
readback/equality checks. This is not a physical-stroke, queue-load or power-loss
test, and two layers do not exercise maximum layer count or storage saturation.

Full-state comparisons use serialized byte vectors rather than a JSON Value for
every pixel channel, and the expected JSON is streamed to disk. These changes
reduce diagnostic overhead; they are not application memory-performance results.

[Large and adjustment-busy results](../benchmarks/large-busy-native-recovery-20261019.json)
pass on engineering revision `57eead3`, executable
`4101A78E3D7E3E8BAF12471C6D85344722B2CDE63AD67A54444BF490551A3796`.
The only compiled-input difference from the packaged `8D2C768E...ABE9279C`
candidate is `src/recovery_probe.rs`. Ten focused recovery regressions, formatting,
default all-target Clippy and release compilation pass. No production storage or
recovery scheduling implementation changed.

| Workload | Container | Edit-to-observed-completion, generations 1 / 2 |
| --- | --- | --- |
| Large idle | v4 | 15.51 / 15.06 seconds |
| Large idle with mask | v5 | 16.40 / 14.01 seconds |
| Large, adjustment open for 20.08 s | v4 | 21.30 / 14.06 seconds |
| Large with mask, adjustment open for 20.04 s | v5 | 21.27 / 14.07 seconds |

Both busy cases observed 60 eligible frames with no checkpoint of the temporary
preview. Checkpoint completion followed cancellation by 1.22 / 1.24 seconds.
All four cases preserved the original project and interrupted snapshot, restored
generation two through the startup button, and saved/reopened with exact editable
state. The v5 startup/restored captures for both workloads were reviewed.

The roughly 14-second second-generation observations do not imply an accelerated
timer: production eligibility is measured from the previous checkpoint's start;
the next fixture edit occurs after that checkpoint completes and its verification
finishes. Current timing excludes diagnostic readback/equality, unlike the old
small-fixture observations above. These results close this particular large-area
and active-adjustment gap, not the complete G1 matrix or final-candidate acceptance.

## Optional Windows save sharing-lock failure

Add `--save-sharing-failure` to the controller command to exercise an actual
Windows filesystem sharing violation during the restored copy's save. The probe
creates a valid pre-existing destination only inside its disposable fixture and
opens it with sharing disabled. The real save worker must report failure while
preserving that file, the recovered document's editable state, its unsaved flag
and unset project path. The probe captures the failure UI, releases its own handle,
and retries the same destination through the ordinary save worker. Successful
publication and exact reopen are then checked as usual. Original project and
interrupted snapshot bytes must remain unchanged throughout. This is an OS file
sharing test, not a simulation of disk exhaustion or a failing storage device.
The option is rejected on non-Windows systems before creating fixtures.

[Native Windows sharing-failure results](../benchmarks/native-save-sharing-failure-20261019.json)
pass for both v4 and v5 on engineering revision `2afdaaa`, executable
`E1B5FAB1C66F4ABB5DE9C767DFD756CBBFD243365D5E6DB9B1455C6AF7477525`.
Both cases observed Windows error 32, preserved the destination and recovered
editable state, retained the unsaved/path state, then saved and reopened the same
destination successfully after releasing the lock. Failure and saved-copy captures
were reviewed. Formatting, default all-target Clippy and release compilation pass.
This closes that specific OS sharing-error path, not the full failure matrix.
