# Cancel ordinary project saves

2026-10-17. Closes the ordinary-save cancellation capability gap identified in
the artwork-safety matrix. Indexed saves retain their existing commit protocol.

## Behavior

Queued and active ordinary saves now expose **Cancel save**. Queued cancellation
retains and consumes the ordered render snapshot ticket but does not launch a
writer; the preceding artwork and undo action are preserved. Active cancellation
uses a shared atomic flag checked during native raster/mask/selection encoding
and again after `sync_all`, at the publication fence. Same-directory staging is
discarded when cancellation is observed before that fence.

Cancellation cannot interrupt the current codec operation or filesystem flush.
Once the publication fence has passed, replacement may complete: a successful
publication remains a successful save even if a cancellation request follows.
The owner trusts the confirmed worker result rather than inferring failure from
a flag set after success. A canceled save does not change the saved path/revision,
recent-file list or recovery data. Cancel also clears a waiting New/Open/Close
continuation so the current canvas remains open, including after late success.

Progress distinguishes waiting, saving and canceling. Controls wrap on narrow
windows; redundant cancellation is disabled while a request is pending. Native
v4/v5 container layouts and ordinary uncanceled pixels/metadata are unchanged.

## Evidence

- Actual egui pointer events activate the ordinary and queued Cancel save controls
  at 320 x 240 logical points, not just direct method calls.
- A gated ordinary Save as worker confirms existing bytes, original path, recovery
  data, artwork and undo remain intact after cancellation.
- A queued real stroke/snapshot cancellation preserves the stroke/undo, consumes
  its ticket and allows a subsequent successful save.
- A worker gated after actual publication confirms late cancellation reports
  Project saved while suppressing the waiting canvas replacement.
- A real panicking save worker leaves the current artwork/path/recovery/dirty
  state intact and does not run its pending canvas action.
- Controlled native encoding can stop during payload writes. Uncanceled controlled
  output matches ordinary output and reopens exactly. Fractional masks receive
  cancellation through their existing tile codec; selection writes are chunked.
- A cancellation injected during the flush boundary is observed before publication;
  a post-publication flag does not manufacture a failure. Prior bytes and staging
  cleanup are checked.

Full suites at this implementation checkpoint: 782 default / 791 all-feature
passed, one maximum-memory test ignored normally. Dedicated maximum project and
recovery round-trip passed again in 100.17 seconds. Default/all-feature all-target
warnings-denied Clippy, formatting, eleven Python analysis checks and optimized
build pass. The GPL/About metadata/UI work that follows requires its own final
candidate check record.

This does not by itself close G1: actual editor crash/recovery discovery, recovery
cadence and the complete OS dialog/clipboard/device matrix remain open.
