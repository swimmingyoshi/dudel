# Restore automatic panel layout

2026-10-17. The View menu allowed persistent manual sidebar overrides, but offered
no action to return to responsive choices even though the README described one.

**View > Automatic panel layout** now clears both manual sidebar overrides and
shows panels again. It sits near the top beside Show all panels, making it
reachable on a short workspace. Automatic behavior then uses the current tool
and available width: compact Brush/Eraser workspaces keep more canvas space,
while larger workspaces expose the inspector and tool properties. Existing
sidebar widths remain available; this action restores visibility choices.

The action does not modify artwork, document revision, modified state or history.
Its resulting preferences encode/restore through the existing preference path.
The menu remains available when Tab has hidden the panels.

## Evidence

The real egui interaction test opens View and clicks the visible action at
480x320, 960x700, 1280x800 and 2560x1000. It starts with hidden panels and opposite
manual overrides, verifies responsive visibility, encodes/restores preferences,
checks identical document/revision/modified state, and confirms the prior opacity
edit remains undoable. This is preference roundtrip evidence; it is not a new
physical tablet or filesystem durability result.

Native View-menu captures were inspected at 1280x800/100%, 1280x800/200% and
480x320/100%. The new action is readable and fully visible near the top at each
size. Tall menus scroll; the screenshots do not imply every menu item fits at
once. Capture CLI: `--capture-view-menu NEW.png WIDTH HEIGHT SCALE` uses the
existing private capture session and does not load/write user preferences or
artwork. Images: `benchmarks/automatic-layout-menu-{100,200,small}.png`; logs:
`target/qa/automatic-layout-menu-{100,200,small}.{stdout,stderr}.log`.

Added `QUICK_START.md`, linked from README, with the current New canvas, brush,
layer, save/reopen, recovery and export flow, plus panel recovery, tablet input
and display settings. Corrected stale README inspector-menu and rename-Undo
wording. Guide entry labels were checked against their UI source; no complete
physical first-use walkthrough or all-file-dialog acceptance is claimed.

Full suites: **760 normal / 763 allocation-profile passed**, zero failures,
one existing high-memory test ignored in each. Default/all-feature all-target
warnings-denied Clippy, formatting, Git whitespace and optimized build pass.
Logs: `target/qa/automatic-layout-{tests,profile-tests,normal-clippy,clippy,release}.log`.
Release/frozen `target/qa/dudel-automatic-layout.exe` SHA-256:
`0DB81976F4DBCDE44579B37A3408978A2B08B73707BA2BB205F09D8DD53432D8`.

The concurrent frozen-baseline sustained DX12 check passed document correctness
but exposed process-memory growth; see `SUSTAINED_NATIVE_DX12.md`. That finding,
physical pen validation, actual clipboard exchange, CSP fixtures and packaging
remain open in the overall professional application goal.
