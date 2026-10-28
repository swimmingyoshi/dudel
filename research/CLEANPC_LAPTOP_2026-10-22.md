# Clean-PC test on the owner's laptop, 2026-10-22

Candidate `dudel-0.1.0-candidate-20261021-190309-093` (EXE `7C3090D3...F659D7`).
Kit: `Run-AutomatedChecks.ps1` plus the guided `AGENT_PROMPT.md`, run with an AI agent
(OpenCode) and the owner. Raw results (local, not committed):
`target/qa/cleanpc-laptop-20261022/`, including `RESULTS.md`, both automated runs, logs,
screenshots and recovery fixtures.

## Machine

- Windows 11 Pro 26200, a second Windows install from 2026-06-12. It is **not pristine**:
  CLIP STUDIO, Node.js, Git, VS Code, Discord, Brave and others were already installed.
- Intel Core i5-10310U, 15.7 GB RAM, Intel UHD Graphics (driver 31.0.101.2114).
- 1920x1080 display. Native diagnostics report 1.5 pixels per point, consistent with 150%
  Windows scaling (the owner has not confirmed this).
- Documents is redirected to OneDrive, so the library defaulted to
  `OneDrive\Documents\Dudel` and the app folder to `OneDrive\Documents\DudelApp`.

## Results

| Area | Result |
| --- | --- |
| **Visual C++ runtime** | **Missing. Dudel could not start.** Every launch and diagnostic exited with `-1073741515` (0xC0000135). The runtime registry key and System32 `vcruntime140.dll` were absent, even with that much software installed. The first run is preserved under `before-vc-runtime/`. Microsoft's signed `vc_redist.x64` v14.51.36247 was then installed (exit 0) |
| Zip checksum, extraction, EXE hash | Pass |
| Automated checks after the runtime install | **All pass.** The OpenGL and DX12 (Intel UHD) native sessions completed with save/reopen verified. UI captures at app scale 1 and 1.5 were produced. Crash recovery passed for v4 and v5 on the first attempt (restore 1.8 s and 2.0 s). No intermittent failure on this machine |
| Native session duration | OpenGL 120.5 s, DX12 98.1 s, versus about 33 s on the RTX 3060 desktop. These are diagnostic timings on a low-end iGPU, not drawing latency |
| SmartScreen | No warning, although `ZoneId=3` was confirmed on the zip and the extracted EXE. This PC's SmartScreen policy is unknown; it is not evidence that other PCs won't warn |
| Drawing, layers, Undo/Redo (incl. Ctrl+Y) | Pass (owner) |
| Save, reopen, PNG export, import as layers or canvas | Pass (owner). The two-layer reopen of the owner's own drawing was not confirmed separately; the older-version two-layer project reopen passed |
| Real crash recovery (Task Manager kill) | Pass (owner) |
| Older-version project (`…111249-426`) opened in the candidate | Pass, with both layers and the artwork. The Paper theme and brush size carried over; panel-layout migration was not checked separately |
| "Uninstall" (delete app folders) | Projects and exports were preserved with identical hashes. A recovery snapshot (15.8 KB) remains in `%LOCALAPPDATA%\Dudel\Dudel\data\recovery` |
| Unaided-user assessment | Not given |

## Follow-ups

Release blocker:
1. **Remove the Visual C++ runtime requirement.** Link the C runtime statically
   (`+crt-static` for `x86_64-pc-windows-msvc`), and apply it to both the packaged build
   and the source archive's build configuration. Verify with the PE import table.
   The alternative, requiring users to install `vc_redist` themselves, failed the first
   real attempt.

UI from the laptop, at 1080p and 150% (owner requests):
2. Tool/brush settings are closed on the laptop screen. Clicking a tool should open its
   settings.
3. Allow resizing of the brush/tool settings area, the brush and size controls, and the
   color wheel.
4. Project-tab close buttons look out of place (alignment, spacing, style).
5. Show a clear cue when a recovery backup has actually been written, distinct from saving
   the project.

Documentation:
6. Known issue or FAQ: a recovery snapshot may remain in `%LOCALAPPDATA%\Dudel` after the
   app folder is deleted. Note also that OneDrive-redirected Documents places the library
   under OneDrive.

## Round 2 (2026-10-22, candidate `…20261022-170440-814`, EXE `1E82CF1F...7493DF9`)

Raw results (local): `target/qa/cleanpc-laptop-round2-20261022/`.

- **Visual C++ fix proven.** The owner uninstalled the x64 runtime before the run, and the
  script recorded both the registry key and System32 `vcruntime140.dll` as absent. All
  automated checks passed:
  - `exe_imports` lists only Windows system DLLs.
  - Normal launch, both native sessions (OpenGL 80 s, DX12 79 s on Intel UHD), UI captures
    and crash recovery for v4 and v5 passed on the first attempt.
  - The owner also used the app manually with the runtime still absent.
  - The runtime was left uninstalled on the laptop, at the owner's choice.
- **Tool rail (fixed):** clicking Brush, Eraser or Fill opens their settings, and the B/E keys
  leave the panel unchanged. Confirmed by the owner.
- **Save progress (fixed):** the bottom bar keeps its height and the canvas stays still while
  saving (maximized window). Not separately confirmed: narrow window, Cancel placement.
- **Portable:** the app ran from the USB stick, saved to `Documents\Dudel` and reused the
  PC's settings. It also launched straight from inside the zip.
- **Upgrade:** round-1 projects (`pikachu-test`, `from-old`) opened with correct layers, and
  the theme carried over. Brush carry-over was not separately confirmed.
- **Recovery and uninstall:** pass.
- **Drag problem:** not encountered in this build, so it was not reproduced.
- **Kit error:** the prompt asked for a 4000x3000 canvas, which exceeds the 4,194,304-pixel
  project envelope (the UI correctly refused it). The test used a smaller canvas instead.

Owner requests:
- Support for larger paintings. This is outside the 0.1.0 envelope, recorded as follow-up
  work in RELEASE_STATUS.
- A configurable default project folder. This already exists in View > Settings > Storage &
  folders, so it is a discoverability note.
