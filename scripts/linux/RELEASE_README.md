# Dudel for Linux (experimental)

Dudel is a pen-focused illustration app. Windows x64 is the supported platform;
this Linux build is experimental and is not covered by the Windows release
decision. `BUILD.json` identifies this package, and [the Linux port](LINUX_PORT.md)
lists what has and has not been checked.

Extract the archive, then run `./dudel` from the folder, or run `./install.sh` to
add it to your launcher. `./install.sh --uninstall` removes it again and keeps your
projects and preferences. Start with the [quick start](QUICK_START.md) for drawing,
layers, saving and PNG export.

## Requirements

- x86-64 Linux with glibc 2.31 or newer (`BUILD.json` records the exact version).
- Wayland or X11, with `libxkbcommon` and working Vulkan or OpenGL drivers. These
  are loaded at startup; Wayland sessions also need `libwayland-client`.
- `xdg-desktop-portal` and a portal backend for the open and save dialogs.

Pen pressure, tilt and the eraser end use the Wayland tablet protocol. Under X11 a
pen acts as a mouse with fixed pressure. See View > Tablet input for diagnostics.

## Known gaps

- Physical tablet use, multi-monitor setups and most distributions remain untested.
- There is no Dudel icon yet, so the launcher shows a generic graphics icon.
- `.dudel` files are not associated with Dudel; open them from inside the app.
- File manager previews of `.dudel` files are Windows-only.

Save editable artwork as `.dudel` and export PNG for sharing. Read
[format support](FORMAT_SUPPORT.md) before importing CSP files, and the
[release notes](RELEASE_NOTES.md) for general limits. When reporting a problem,
include the version and hash from `BUILD.json`, your distribution, compositor, GPU
and tablet, and the steps on a disposable canvas.

Dudel is GPL-3.0-only. Official executable downloads may be sold; you can build the
matching source yourself. [SOURCE.md](SOURCE.md) identifies the corresponding source
archive. [Distribution policy](DISTRIBUTION.md), [LICENSE](LICENSE) and
[third-party notices](THIRD_PARTY_NOTICES.md) describe the supplied sources and terms.
