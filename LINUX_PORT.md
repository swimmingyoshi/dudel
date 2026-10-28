# Linux port

Tracking the Linux port (Wayland first). The `master` and `linux` branches now
share the Windows and Linux implementation. Windows x64 remains the release
validation target; Linux acceptance items below remain open.

## Current automation (2026-10-18)

The Linux workflow uses the pinned Rust toolchain on Ubuntu 24.04 and runs
`bash scripts/verify.sh full`: formatting, default/all-feature suites and Clippy,
Python checks and release compilation. It then checks an isolated 1280 x 800
native startup capture with software OpenGL/X11 under Xvfb and uploads build/capture
hashes. After fixing the Windows-only DX12 example's platform gates, hosted run
39177554445 on `aca86c0` passes 849 default / 853 all-feature tests, both Clippy
configurations, 32 Python checks and release compilation. The native startup
artifact checksum/CRC and capture hash match; its 1280 x 800 screenshot was
visually reviewed. [Exact evidence](benchmarks/linux-hosted-baseline-20261018.json).
The later tab-recovery change has separate hosted checks pending.
This does not validate Wayland tablet
input, Hyprland workspace switching, real GPU performance or multi-monitor use.

Pushes to `linux` run Linux once, while synchronized `master` pushes run Windows
once; PRs and manual dispatch can run either workflow. Documentation-only reviewed
commits may use the skip policy in TESTING.md. The physical checklist below stays
open until build-identified hardware results exist.

On Ubuntu, install the development libraries and headless-test dependencies listed
in `.github/workflows/linux.yml`, then use `cargo build --locked --release` and run
`./target/release/dudel`. On an actual Wayland desktop let winit select Wayland;
the X11/software overrides in CI are limited to the headless startup check.

## Baseline (2026-10-17, Arch/Omarchy, Hyprland, Intel GPU)

- `cargo build --release` succeeds with no source changes; the window opens
  natively on Wayland (wgpu/Vulkan).
- With the pinned toolchain (1.97.1): fmt, clippy `-D warnings` and all
  tests pass (791 after the Wayland pen backend).

## Done

- [x] Test harness maps Ctrl to the command modifier on every non-macOS
      platform, matching eframe (fixed 3 workflow-test failures on Linux).
- [x] Window sets `app_id`/`WM_CLASS` to `dudel` so compositors, window rules
      and launchers can identify it.

## To do

### 1. Pen pressure (blocker for tablet use)

Windows reads `WM_POINTER` messages in `native_pen.rs`. On Linux the egui
fallback in `input.rs` only sees pressure through `Event::Touch { force }`,
and winit does not forward Wayland tablet events, so a pen paints at a fixed
pressure of 1.0.

- [x] Wayland backend (`src/wayland_pen.rs`): binds `zwp_tablet_manager_v2`
      on eframe's `wl_display` with its own event queue and reader thread, and
      turns tool frames on our `wl_surface` into `Packet`s with pressure,
      tilt, rotation, eraser, barrel and compositor timestamps. Same API as
      `native_pen::Backend`; on Linux `crate::native_pen` aliases it.
- [x] Widget input: binding the tablet seat ends the compositor's pointer
      emulation, so `Input::raw_input_hook` feeds egui pointer events rebuilt
      from the tool (tip = primary, BTN_STYLUS = secondary, BTN_STYLUS2 =
      middle). Painting suppresses them while the pen is in range.
- [ ] Hardware check on Hyprland: pressure, tilt, eraser end, barrel
      buttons, menus/sliders with the pen, cursor visibility while hovering,
      fractional scaling and UI zoom.
- [ ] X11/XWayland fallback via XInput2 valuators (lower priority).
- [x] View > Tablet input shows the Wayland status and a "Use Wayland
      tablet input" toggle (off = pen acts as a mouse, no pressure).
- [ ] Tool cursor: the client never calls `zwp_tablet_tool_v2.set_cursor`;
      set one if the compositor shows none while the pen hovers.
- [ ] Validate with the XP-Pen Artist 12 (see `XP_PEN_VALIDATION.md`).

### 2. Image paste shortcut

`clipboard_shortcuts.rs` (Windows-only) catches Ctrl+V before egui-winit's
text-only paste. On Linux `take_paste` is hard-wired to `false`.

- [ ] Confirm Ctrl+V with an image on the clipboard; if it is swallowed, detect
      the chord from egui events on non-Windows platforms.

### 3. Platform details

- [x] Hidden-window freeze: with vsync, Mesa's present (EGL swap or Vulkan
      FIFO) waits for frame callbacks that Hyprland never sends to windows on
      a workspace you are not viewing. The UI thread blocked in
      `eglSwapBuffers`, stopped answering compositor pings and was killed as
      unresponsive (SIGKILL, no core dump). On Wayland Dudel now presents
      without waiting for vsync (compositors never tear); Windows and X11
      are unchanged.

- [ ] Verify file dialogs (`rfd`, xdg portal) and recovery/settings paths
      (`directories`) under `~/.local/share` / `~/.config`.
- [ ] Verify settings-file locking (`try_lock`) with two running instances.
- [ ] Memory diagnostics (`benchmark::process_memory`, `region_census`) return
      nothing on Linux; optionally read `/proc/self/status` and `/proc/self/smaps`.

### 4. Packaging and CI

- [ ] `.desktop` file and icon (`StartupWMClass=dudel`).
- [ ] Linux release archive (tarball first; AppImage or Flatpak later).
- [x] `scripts/verify.sh` equivalent of `Verify.ps1`; hosted baseline passes on `aca86c0`.
- [x] Ubuntu 24.04 workflow (fmt, test, clippy, build and X11 startup); hosted baseline passes.
- [x] Linux build/run instructions above; distro/hardware acceptance remains open.
