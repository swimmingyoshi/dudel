# Smooth catch-up after stopping the pen

> **Superseded 2026-10-17.** The string-pull stabilizer and this separate
> resting-pen easing were replaced by one time-based smoothing filter
> (`stroke.rs`: `smoothing_seconds`, `smoothing_fraction`). The drawn point
> covers `1 - e^(-dt/tau)` of its distance to the pen per interval, with
> tau = 2 ms per stabilizer unit, along the pen's path while it moves and on
> the frame clock while it rests. `stabilizer.rs` and its pause/anchor/ease
> state were removed; replay `CatchUp` commands are unchanged. The text below
> records the previous design. The new feel needs the artist's retest.

2026-10-17. Implements the user's candidate feedback: drawing feels good, but
the stabilized line snaps to the pen too quickly when motion stops.

## Cause and resulting behavior

`canvas.rs` used a 65 ms exponential time constant after a 60 ms pause. It
immediately consumed a large portion of the remaining lag. More seriously,
resuming motion updated the resting timestamp but did not clear the last
catch-up timestamp. The next pause could therefore include the intervening
drawing time and consume almost the entire lag in one callback.

`stabilizer.rs::Resting` now owns the resting clock and eased progress. Real
movement restarts both. After the existing 60 ms pause, the line follows a
450 ms quintic ease-in/ease-out, with zero velocity and acceleration at each
end. Each callback advances at most 1/30 second, so a stalled callback cannot
produce a large jump by consuming several seconds of elapsed wall time.

Motion is measured from a retained screen-space anchor, rather than comparing
only adjacent input packets. Small movements accumulate, preventing a series of
subpixel steps from being mistaken for a stopped pen. The one-screen-pixel
jitter tolerance scales correctly with canvas zoom.

The brush engine continues receiving geometric catch-up fractions and supplying
pressure from its recorded trail. Recorded replay commands, dab spacing,
document/brush formats and pen-lift completion semantics are unchanged. The
stationary animation ends at the existing subpixel lag tolerance; lift still
finishes the remaining endpoint through the original engine path.

## Verification and candidate

- Four deterministic controller tests check gentle start/finish, bounded
  advancement after stalls, resumed movement, 30/60/120/240 Hz cadence and
  cumulative subpixel motion at several zoom levels.
- All sixteen stroke-engine tests pass, including pressure history, stationary
  catch-up, zoom invariance and lift endpoint/overshoot regressions.
- Formatting and all-feature all-target warnings-denied Clippy pass.
- Complete suites pass: 776 default and 785 all-feature tests, one high-memory
  test ignored normally. The user retested this exact candidate and reported
  **"Smooth now"**. This confirms the targeted catch-up feel, not the entire
  hardware/session checklist.

Normal candidate: `artifacts/releases/dudel-0.1.0-candidate-20261017-134927-038/`.
[Build/source/package identity](../benchmarks/smooth-catchup-build.json).
The user tested the same brush/stabilization settings against this candidate and
accepted its catch-up feel. Preserve this easing behavior through later work.

## Broader execution progress

The same source includes configurable diagnostic-only native workloads:
`DUDEL_SOAK_ROUNDS=2..240`, `DUDEL_SOAK_LAYERS=4..32` and
`DUDEL_SOAK_DENSITY=sparse|pattern|noise`. Defaults retain the previous twelve-round,
four-layer sparse fixture. Additional dense fixtures fill every additional layer;
the base layer retains the original dense mask fixture. Extended layer stacks
include clipping and Multiply blending. Frame/checkpoint reserves are fixed from
the selected options before measurement; short diagnostic mode retains its
original one-round/four-layer fixture.

The first unprofiled thirty-round, twenty-layer DX12 session was launched from a
frozen executable/PDB with matching input hashes. Its live process/session and
logs are recorded in `target/qa/standard20-soak-20261017/RUN_STATE.json`. Revalidate
that handle before continuing; a state file alone is not proof of a live run.
Do not restart a run merely because a polling call yields no new output.

The new [artwork-safety matrix](ARTWORK_SAFETY_MATRIX.md) records sixteen actual
child-process terminations around native/recovery publication plus a Windows
read-only destination check. It distinguishes shared storage-primitive verification
from complete editor-process recovery and physical power-loss certification.
