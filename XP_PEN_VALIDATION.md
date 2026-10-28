# XP-Pen Artist 12 hands-on validation

## Owner confirmation, 2026-10-28

The owner confirms: "artist 12 works, already been tested multiple times during
the development." This is accepted qualitative evidence of working Artist 12
use. Do not turn the historical diagnostic rows below into another blanket
hands-on test request. No new tablet session is requested or scheduled.

Exact pen-to-display latency, driver/scaling metadata and a formal timed session
were not supplied with this confirmation; do not invent them or treat their
absence as a report that the tablet does not work. A further test needs a specific
unresolved issue that the existing owner feedback and automated evidence cannot
answer. See [the evidence review](benchmarks/release-evidence-reuse-20261028.json).

## Owner feedback on line lag, 2026-10-28

Smoke-testing the clean candidate `dudel-0.1.0-candidate-20261028-205750-323`
(default OpenGL renderer), the owner saw the line trail the pen slightly with
stabilizer 0, opacity 100 % and size 30. Enabling **View > Settings > Display >
Low latency** and restarting fixed it: "that fixed it, and i think thats what i
was visualizing." A regression test shows that stabilizer 0 places dabs exactly
at the pen in both Smooth and Rope modes, so the delay was frame presentation
(vsync with up to two queued frames), not stroke smoothing. This is qualitative
feedback; pen-to-display latency was not measured. On the owner's decision, Low
latency is now on by default; a saved choice to turn it off is kept. See the
quick start's [lag section](QUICK_START.md#if-the-line-lags-behind-the-pen).

## Historical diagnostic detail

Pressure was confirmed by the user: light strokes are thin and pressing harder
mid-stroke makes them thicker. The user also confirmed general drawing feel and
the revised stationary catch-up; build-specific evidence is recorded below.
Automated raster tests and screenshots do not establish tablet usability.

Record the build commit, Windows display scaling, driver version, canvas size,
brush preset and approximate session duration with each new result.

| Check | Physical evidence | Status |
|---|---|---|
| Pressure changes width during a stroke | User confirmed light-to-heavy strokes | Confirmed on earlier prototype |
| Stabilizer feel | Replaced 2026-10-17 by one time-based smoothing filter at the artist's request; earlier "Smooth now" applied to the removed catch-up design | Retest pending |
| 1–2 px hard brush: slow diagonals and fast curves | None yet | Pending |
| Pen-down and pen-lift responsiveness | Synthetic lifecycle timings only | Pending |
| Navigator drag and zoom on the pen display | None yet | Pending |
| Symmetry center drag and mirrored strokes | Automated pixel tests only | Pending |
| Move layers into and out of folders | Automated structure tests only | Pending |
| Resize library/settings divider | Screenshots only | Pending |
| Pen/mouse switching, focus loss, pen leaving canvas | Ordered-event fixtures only | Pending |
| Save/open/recovery while drawing | Worker/commit tests only | Pending |
| Long session, memory stability and repeated undo/redo | None yet | Pending |

Use a fresh canvas for the brush checks: one horizontal, diagonal and curved
stroke at each size, first gently, then with pressure changing along the stroke.
Repeat with a fast curve and compare the visible mark with the pen movement.
Report whether a delay occurs at initial contact, during movement or after lift.
This log remains open; do not infer a pass from lack of reported problems.

User follow-up: 'it feels smooth and works fine' in response to the thin hard-brush and tablet-workflow question. Confirms subjective smoothness/usability of the tested build; exact commit, timings and individual UI interactions were not supplied. The newer stroke-buffer change was still under test and is not covered by that feedback.

## Candidate feedback, 2026-10-17

For the identified display-pool candidate (normal executable SHA-256
22547ECEEF04CB0A4CB653653A6927F1147FAC004A22DFA38C52A44F4BCACD24), the user
reports that the pen and drawing feel good, but the line catches up to the pen
too quickly when movement stops. Treat this as a drawing-quality defect, not a
complete checklist pass. Driver version, display scaling, session duration and
individual pan/zoom/save results were not supplied.

Implementation follow-up replaces the stale resting timer and abrupt exponential
catch-up with restartable eased progress. The retest below supplies targeted
physical acceptance of that behavior.
User retest on the smoothing candidate (normal executable SHA-256
995C5A449999E4C30D98430922104DEF52A71F8613BC3162DEFEDFF9CB323E7F):
"Smooth now". This confirms the revised stationary catch-up feel for the same
brush/stabilization settings. Preserve that behavior. It does not independently
complete the driver/scaling, two-hour session, clipboard or file-dialog checks.
## Owner feedback, 2026-10-21

The user reports drawing several times on the Artist 12 with recent builds and that
"everything worked great". This is positive subjective evidence of general pen,
pressure and workflow usability on recent builds. The exact build, driver/scaling,
session length and the individual rows above were not identified. A short targeted
pass on the study-passed candidate `…20261021-190309-093` (EXE `7C3090D3...F659D7`)
can close the remaining pending rows: one 20–30 minute session that records Windows
scaling and notes any problem per row.

## Local device metadata, 2026-10-17

Windows reports an active UGD monitor with EDID friendly name "Artist 12" and an
installed Pentablet package version 3.4.13.231129. See
benchmarks/tablet-installed-metadata.json. The user explicitly confirmed
"its the xp pen artist 12" on 2026-10-17, resolving the earlier plan
assumption of an XP-Pen 13. The target device is the XP-Pen Artist 12.
Installed-package metadata does not prove
which pen driver/Windows Ink path was active during the accepted drawing session.
Display scaling and uninterrupted hands-on session duration remain unconfirmed.
