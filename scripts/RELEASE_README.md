# Dudel for Windows

Dudel is a drawing app made for pen tablets. Thanks for trying it.

## Start

1. Keep this whole folder together: `dudel.exe` needs nothing else installed.
   Keep `dudel_thumbnails.dll` beside it for optional `.dudel` previews in File
   Explorer.
2. Run `dudel.exe`. The builds aren't code-signed yet, so Windows SmartScreen may
   say "Windows protected your PC": choose **More info > Run anyway**.
3. The welcome window offers a new canvas and lists the keys worth learning
   first. The [quick start](docs/QUICK_START.md) covers drawing, layers,
   saving, recovery and export.

Your artwork is saved in `Documents\Dudel`; settings and recovery snapshots stay
in `%LOCALAPPDATA%\Dudel`. To remove Dudel, first choose **View > Settings >
File Explorer previews > Remove .dudel previews** if you turned previews on, then
delete this folder. Your projects stay where they are.

## Good to know

- Save editable work as `.dudel`; export PNG, JPEG, WebP, BMP, GIF or PSD to
  share it. [Format support](docs/FORMAT_SUPPORT.md) lists what Clip Studio
  Paint and Photoshop imports keep.
- [Release notes](RELEASE_NOTES.md) describe this version and its limits.
- **View > Tablet input** shows the pressure Dudel receives from your pen.

## Problems and feedback

Use **Help > Report a problem…**, and include the version from
**Help > About Dudel**, your Windows version, graphics and tablet drivers and
the steps that lead to the problem. If Dudel closed unexpectedly, attach the
newest file from **Help > Problem reports folder**. `BUILD.json` in this folder
identifies the exact build.

## License and source

Dudel is free software under GPL-3.0-only ([LICENSE](LICENSE)). Official
downloads may be sold; you can always build the matching source yourself.
[SOURCE.md](SOURCE.md) names the matching source archive, and
[third-party notices](THIRD_PARTY_NOTICES.md) list the components Dudel uses.
See also the [distribution policy](docs/DISTRIBUTION.md).

## Optional: performance study

`RunPerformanceStudy.ps1` runs an unattended engineering study of several hours
for developers who ask for one; it is not needed to use Dudel. It requires
Python 3.11+ and 8 GiB of free space. Close Dudel first, then run
`powershell -NoProfile -ExecutionPolicy Bypass -File .\RunPerformanceStudy.ps1`
in this folder (`-CheckOnly` checks the prerequisites without starting).
Reports stay in `diagnostics\target\qa\study-*`.
