# Durable application settings

Checkpoint: 2026-10-16. Extends the app-key recovery work described in
`PREFERENCE_RECOVERY.md` to the entire eframe RON container.

## Behavior

Normal startup resolves the same eframe application settings path and reads a
stable original snapshot. Valid legacy RON maps, unknown keys, and the existing
`dudel.preferences.v1` JSON value remain compatible. eframe receives a private
temporary `app.ron` path for its direct writes. Capture modes also receive private
paths and cannot write to the normal framework settings file.

A coalescing worker accepts the latest requested app-preference snapshot. It
waits for a complete, validated private RON file containing that exact value,
then writes and syncs same-directory staging files and atomically replaces the
public store. An unchanged container does not rotate away the previous distinct
snapshot. A valid old primary becomes `app.ron.previous`; invalid primary data
never replaces a valid previous version. Unknown or invalid previous data is
archived to a new `dudel-settings-preserved-*.ron` file before rotation. Unreadable
backups stop publication unless their absence is positively established.

The worker holds a cooperating-writer lock from its first publish until shutdown.
It compares the public file against its stable baseline before publishing and
again immediately before replacement. Another Dudel window cannot publish while
that lock is held. A session whose public settings changed externally must restart
and reload rather than overwrite newer data. Uncooperative external programs do
not honor this lock; their writes during the final comparison/replacement window
cannot be excluded.

Failed publication leaves a persistent settings-save banner with the failure
reason and Retry. Retrying forces a fresh framework flush even when the app key
has not changed. Normal close drains the requested final snapshot after eframe
has joined its private-file writer. A final save failure is shown in a native
dialog. Project saving operates independently.

An invalid or unsupported public container opens the previous valid preferences
when available, while settings replacement remains paused. Recovery requires an
exact, newly exported original container backup, including arbitrary non-UTF-8
bytes. The backup is verified again before enabling current or restored settings.
Recovery accepts validated app JSON or compatible RON containing app preferences.
Initial read-access failures remain guarded and require restored access and a
restart before publication can proceed.

## Verification

Eight focused regression groups cover compatible migration/reopen, distinct
previous snapshots, partial private writes, damaged-primary fallback and exact
backup gating, external changes, writer contention and retry, blocked backup
writes, preservation of damaged previous versions, and invalid/future settings.

The native probe uses a new explicitly marked fixture directory. Each phase is a
separate executable process and uses the actual eframe storage and App::save
callbacks. Run these phases in order with one new folder:

```powershell
dudel.exe --probe-settings-store NEW_FIXTURE_FOLDER first
dudel.exe --probe-settings-store NEW_FIXTURE_FOLDER reopen
dudel.exe --probe-settings-store NEW_FIXTURE_FOLDER corrupt
dudel.exe --probe-settings-store NEW_FIXTURE_FOLDER repair
dudel.exe --probe-settings-store NEW_FIXTURE_FOLDER failure
dudel.exe --probe-settings-store NEW_FIXTURE_FOLDER retry
```

The first launch saves brush/color settings; reopen verifies them and saves a
change. Corrupt deliberately truncates only the marked fixture's public file,
opens previous settings, and verifies the damaged file stays untouched. Repair
exports and verifies that exact original before enabling publication. Failure
blocks the previous-backup path with a fixture directory, verifies publication
fails and the primary remains exact, and captures the failure banner. Retry
removes that empty fixture directory and verifies changed settings can be saved.
Every phase writes a new screenshot and JSON report. Existing outputs are refused.

Verified checkpoint:

- Full suite: **745 passed, zero failed, one existing high-memory test ignored**.
  All-target Clippy with warnings denied, `cargo fmt --check`, normal Git whitespace
  check and optimized offline build pass. Logs:
  `target/qa/settings-store-{tests,clippy,release,diff-check}.log`.
- Release and identical frozen executable SHA-256:
  `BE0952F180A9621EB29038856E81062920B71671C00BB27C3D45A3F0995ECB8C`.
  Frozen executable: `target/qa/dudel-settings-store.exe`.
- All six separate-process native phases passed in
  `target/qa/settings-native-store-01/`. Each phase has a JSON result and PNG.
  Process logs: `target/qa/settings-native-PHASE.{stdout,stderr}.log`.
- Viewed `corrupt.png` and `failure.png` at 1280x800. The persistent banner,
  recovery export and guarded choices, dismissal, and failure Retry control fit.
  The native retry phase verifies a new launch after removing the blocked path;
  it does not simulate clicking Retry in the same open native window. Same-session
  repeated request/retry and status clearing are covered by the worker regressions.
- The long fixture brush name exposes a separate top-toolbar overflow: at this
  width the rightmost Snap control is partly outside the viewport. Record and fix
  that layout issue next; these screenshots certify settings controls only.
  The follow-up is now fixed and verified in `RESPONSIVE_BRUSH_TOOLBAR.md`.

## Limits and release gates

This is staged/synced atomic replacement and previous-version recovery, not a
device power-loss certification. Directory metadata durability and arbitrary
non-cooperating external writers are outside the guarantees above. The worker's
five-second private-snapshot deadline reports incomplete writes as failures;
pending requests are coalesced rather than queued without a bound.

The native probe verifies app preferences through real framework persistence.
It disables persisted egui memory/window geometry and does not certify desktop
window restoration or every legacy user settings file. It uses generated data
and does not read or modify the user's actual app store.

Physical XP-Pen, sustained native memory/GPU interaction, external clipboard,
supported CSP interchange and packaging gates remain open. The professional
application goal remains active.
