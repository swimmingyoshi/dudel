# App preference recovery and overwrite protection

2026-10-16. Follow-up to `BRUSH_CREATION_WORKFLOW.md`. This protects Dudel's
preference value after application-level decoding/validation fails. Framework
container corruption and interrupted settings-file writes were separate work at
this checkpoint. The follow-up is now implemented and verified in
`SETTINGS_STORE_DURABILITY.md`; the audit below documents the original boundary.

## Problem and resulting behavior

Startup previously reported a preference decode error but continued with defaults.
The app's normal save callback could subsequently replace the stored preference
value with that fallback state. A malformed JSON value, unsupported version or
invalid brush entry could therefore lead to unintended replacement.

A failed restore now installs a recovery guard. Normal preference save callbacks
return before serializing or replacing `dudel.preferences.v1`; egui memory
persistence is paused too. Closing the recovery dialog keeps that guard active.
A persistent Settings saving is paused banner reopens the dialog. Artwork/project
saving continues independently.

The nonmodal recovery window explains the state and exposes:

1. Export original settings: write the original preference string as exact UTF-8
   bytes to a new file using same-directory staging, file flush/sync and
   non-overwriting publication. Existing destinations are preserved. The written
   file is read back and compared before recording a successful backup.
2. Use current settings: re-read the current protected preference value and
   compare it byte-for-byte with the recorded backup. Validate the same current
   snapshot the save callback will encode, then lift the guard. The following
   normal save can write this session's settings.
3. Restore settings file: read a bounded candidate, fully decode/validate it,
   verify the original backup still matches, then apply it. Failed candidate or
   backup checks leave the session/recovery state intact. Existing input gestures
   close before settings replacement; brush filters/recents and test-pad contact
   reset so the restored library is discoverable.
4. Keep original settings: dismiss the window while keeping writes paused.

Backup export alone never resumes writes. Missing, changed, deleted or outdated
backups cannot authorize replacement. If the protected value changes after a
backup, another matching backup is required. No original blob is retained by the
recovery state itself: it holds bounded error text, a path and UI state, reading
the storage value only for requested recovery actions. Error feedback is capped
at 512 characters plus an ellipsis. Backup verification streams fixed-size chunks.

The guard does not introduce a new preference schema. The shared snapshot encoder
retains version 1 and the existing valid-current-brush fallback behavior. It also
does not salvage partial settings from invalid input or repair unsupported
versions automatically; the original exported data remains available for repair.

## Automated evidence

Full suite: **737 passed, zero failed, one existing high-memory test ignored**.
Warnings-denied all-target Clippy, formatting, whitespace and optimized build pass.
Five added regression groups cover:

- Malformed JSON, unsupported versions, invalid long brush names and a large
  unknown enum value. Repeated actual app save callbacks and dismissed dialogs
  leave the original stored value and unrelated keys unchanged; error text is
  bounded and egui memory persistence stays paused.
- Exact multi-chunk UTF-8 backup, rejection of an existing sentinel destination,
  missing storage, same-length backup mutation, deleted backup, changed original
  value and invalid current session state. Only a fresh matching backup and valid
  session can enable preference saving.
- Failed/valid settings-file restoration, original backup requirements, unchanged
  artwork/history on an idle restore and subsequent valid app serialization.
- A real asynchronous project save through the application while the preference
  guard is active, reopening its file to compare artwork bytes, followed by a
  settings save callback that still preserves the protected preference value.
- Actual egui hit testing: Keep original, persistent banner Review, verified-backup
  Use current and the resulting valid settings save.

These tests use memory-backed eframe storage and owned temporary files. They do
not replace or export real user settings. Native file-picker interaction, a
physical process restart against the real settings directory, disk/device faults
and power-loss behavior are not established by these tests.

## Native visual verification and artifacts

The generated `--capture-settings-recovery NEW_IMAGE.png WIDTH HEIGHT SCALE`
fixture uses an invalid synthetic preference value and a fixture storage object.
It does not apply the real app preference value or use it as backup data. App and
egui memory persistence remain disabled in capture mode.

Native OpenGL captures were inspected at application zoom 100/150/200%. The
banner, nonmodal explanation, Details, Export original, disabled replacement
choices before backup, Keep original and window close control fit. Logged actual
viewports were 1536x840.8, 1024x560.5 and 768x420.4 points, with 1.25/1.875/2.5
pixels per point respectively. These establish layout, not tablet input or I/O
latency. Large backup/verification actions currently perform synchronous I/O
following the file choice; large-library latency has not been measured.

- Images/logs: `target/qa/preferences-recovery-{100,150,200}.{png,log}`.
- Test/lint/build logs: `target/qa/preferences-recovery-{tests,clippy,release}.log`.
  Focused regressions: `target/qa/preferences-recovery-targeted-tests.log`.
- Current release and identical frozen recovery executable SHA-256:
  `B4A7EA30EA640B4CFFE0A99570B9A44C18D54CBB3ED551C5BD1690E876C015B9`.
  Frozen binary: `target/qa/dudel-preferences-recovery.exe`.
- Reproduce layout with a new output path:
  `dudel.exe --capture-settings-recovery NEW_IMAGE.png 1280 800 2`.

## Settings-file durability boundary at this checkpoint

The installed, locked eframe 0.33.3 source was inspected at
`src/native/file_storage.rs`. Its RON container loader falls back to an empty map
after a read/parse failure, while its writer uses direct file creation and
serialization. That whole-container failure can appear to Dudel as a missing key,
so this application-level decode guard cannot detect it. The framework can also
persist native window metadata independently of Dudel's app preference callback.
Keeping the app key unchanged does not certify crash-safe preservation of the
entire framework container.

The follow-up in `SETTINGS_STORE_DURABILITY.md` implements and verifies durable
settings-file preservation/recovery, including
whole-container read failures, interrupted writes, failed writes/retry and an
isolated real-store restart. Preserve existing preference/library compatibility
and keep actual user stores out of diagnostic tests. The backup helper's sync and
exact readback are not a claim of filesystem/device power-loss certification.

Physical XP-Pen, sustained-session resource/GPU interaction, external clipboard,
supported CSP interchange and packaging gates remain open. The full professional
drawing-application goal remains active.
