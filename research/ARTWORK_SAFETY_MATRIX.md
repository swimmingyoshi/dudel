# Artwork-safety acceptance matrix

2026-10-17 execution checkpoint. These are source-linked automated checks, not
full GUI, hardware, physical power-loss or filesystem certification. The current
completion plan's G1 remains open until the uncovered cases below are accepted.

## New process interruption evidence

`storage.rs::atomic_stream` delegates to the same staged write -> file sync ->
atomic persist sequence through a private observer-capable helper. Normal use
supplies a no-op observer. It does not add environment-controlled fault behavior
to a production executable.

`storage_interrupt_tests.rs` launches the current test executable as a child,
using an explicitly marked temporary fixture directory. The parent waits for an
exact phase marker, terminates only its own child handle and waits for process
exit. Cleanup guards prevent orphan children. Child phases are:

1. Half of the replacement payload written to same-directory staging.
2. Complete replacement written, before `sync_all`.
3. `sync_all` returned, before pathname publication.
4. Atomic persist returned, before normal operation completion.

Each phase is exercised for ordinary v4, mask-aware v5, v4 recovery and v5
recovery payloads: **16 actual child terminations**. Payloads come from the real
native/recovery encoders. Before publication the destination bytes remain exactly
the original; afterward they exactly match the replacement. The normal readers
reopen and compare complete serialized document state, including hidden RGB,
partial edge tiles, opacity/clipping and fractional mask coverage. This tests the
shared publication primitive with native payloads, not a whole editor process
crash during a user interaction. Interrupted staging leftovers are contained in
the disposable fixture and are not interpreted as recovery snapshots.

Additional tests inject a writer failure and errors at pre-publication boundaries,
checking original bytes/readability and staging cleanup. A Windows-only test uses
an actual read-only destination attribute and confirms rejection without replacing
the old project; original permissions are restored before fixture cleanup.

## Coverage and remaining acceptance

| Requirement | Authoritative automated evidence | Remaining work |
| --- | --- | --- |
| Native save/open with editable metadata and masks | `storage::tests`, `tile_project::tests`, `workflow_tests::normal_mask_workflow_creates_paints_selects_duplicates_transforms_saves_reopens_recovers_and_exports` | Candidate-specific manual dialog/restart workflow |
| Large supported project/recovery | Dedicated `binary_project::tests::maximum_supported_binary_project_roundtrip` passed at previous engineering checkpoint | Passed again after cancellable storage changes (100.17 seconds); dense/noisy v5 peak-memory/timing budget pending |
| Interruption before/after publication | New `storage::interruption_tests::killed_writers_preserve_complete_native_and_recovery_generations` | Native v4/v5 application-loop interruption, startup discovery and Restore copy now pass; see NATIVE_ARTWORK_RECOVERY.md. Power-loss durability remains distinct |
| Write failure / read-only destination | New `failed_writer_and_sync_boundary_do_not_replace_existing_artwork`, `windows_read_only_destination_rejects_replacement_and_preserves_artwork` | Kernel storage-full/device removal matrix; injected write failure is not actual ENOSPC |
| Save snapshot excludes later edits | `save_jobs::tests::queued_save_captures_only_preceding_strokes_and_preserves_newer_artwork`, `save_completion_cannot_replace_accepted_but_unrendered_strokes` | Candidate manual exercise |
| Save-as / pending replacement / unsaved state | `background_write_contains_snapshot_and_pending_save_blocks_replacement`, `only_successful_current_revision_clears_unsaved_state`, `deferred_actions_run_only_after_successful_current_save` | OS Save/Discard/Cancel dialog walkthrough |
| Worker error/panic | `snapshot_memory::tests::failed_and_panicked_source_workers_stop_accounting_without_changing_artwork` and application error branches | Application-level panicking save-worker regression now passes; manual error presentation pending |
| Cancel/stale load protects artwork | `load_jobs::tests::canceled_stale_and_failed_loads_preserve_document_and_path` | OS I/O cancellation limits and manual errors |
| Native/PNG export snapshot and cancellation | `export_jobs::tests`, including queued taper, later edits, early/late cancel, wrong destinations and close waiting | Manual opaque/transparent export and clipboard exchange |
| Recovery preserves a copy | `load_jobs::tests::recovery_worker_opens_unsaved_copy_and_keeps_source_snapshot`, `recovery::tests` retirement/close fences | Native startup restore/save passes; four small-fixture completions measured 15.8-16.9 s. Large/busy-workload cadence and OS manual workflow remain open |
| Ordinary save cancellation | Native and indexed saves expose Cancel; queued/worker cancellation, synced pre-publication fence and late success tests pass; see CANCELABLE_NATIVE_SAVES.md | Manual OS workflow and codec cancellation limits |
| Compatibility | Existing legacy JSON/v2/v3/v4 and masked v5 readers/tests | Publish explicit writer/reader matrix and inspect packaged candidate behavior |

Targeted new storage interruption tests passed, including real termination and
Windows read-only protection. Final suite/build identities and subsequent results
belong in the release checklist; this matrix does not turn partial coverage into
G1 acceptance.

## Native application-loop checkpoint

[The native recovery report](NATIVE_ARTWORK_RECOVERY.md) adds two abrupt native
producer terminations and restarts, on v4/v5 fixtures. It exercises the real Dudel
update loop, normal idle repaint/timer, shared startup discovery, visible Restore
copy hit testing, background load and separate save/reopen. Full editable state,
original files and retained snapshot bytes are checked. Six settings persistence,
restart, corruption, repair, failure and retry phases also pass on that frozen
engineering executable. This complements the sixteen publication-boundary child
terminations; it does not certify power loss or large/busy recovery cadence.
