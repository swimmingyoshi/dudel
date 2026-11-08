# Export options

0.2.0 workflow goals, tracked by [UPGRADE_PLAN.md](../UPGRADE_PLAN.md):

- [x] One export flow with format-specific options: **File > Export > Export
      image…** (Ctrl+Shift+E) opens a window.
- [x] JPEG quality and a background color for flattening; transparency kept
      where the format can. The window explains GIF's palette and transparency
      limits and BMP/JPEG flattening before anything is written.
- [x] A preview of the conversion (flattening, JPEG quality, GIF palette) before
      the file is chosen; Cancel or closing the window writes nothing.
- [x] **Export again** (Ctrl+Shift+R) repeats the project's last image export to
      the same file with the same settings. The save dialog asks before
      overwriting another file; Export again overwrites only its own export and
      still refuses a destination that has become another kind of file.
- [x] Exports stay tied to a captured document snapshot. Cancellation, encoder
      failure and disk errors leave the document and existing output intact;
      exporting, repeated or not, doesn't mark the project saved or changed.
- [x] Export settings are preferences, shared by all projects and never stored
      in the artwork. The last export (file and settings) is remembered per open
      project for the session, so each tab exports again to its own file.

## Export image

Choose a format, set its options, check the preview, then **Export…** and pick
where to write. The window remembers its settings.

| Format | Behavior |
| --- | --- |
| PNG | Lossless RGBA; keeps the document's DPI. Transparency optional |
| WebP | Lossless; transparency optional |
| JPEG / JPG | Lossy, quality 1-100 (92 by default); always flattened |
| BMP | Uncompressed; always flattened |
| GIF | Single image, at most 256 colors; a pixel is either fully see-through or not. Transparency optional |

When transparency isn't kept (always for JPEG and BMP), see-through areas
become white, or the chosen **Background** color. The preview runs the reduced
picture through the same flattening and, for JPEG and GIF, the same encoder, so
quality loss and color banding show before writing. The preview is reduced to
at most 256 pixels on its longest side, so fine detail differs from the file.

Only PNG writes project DPI metadata. A file name typed with another of these
extensions exports in that format, with the window's other settings. PSD, SVG
and project package commands remain separate. Exports capture a document
snapshot, run in the background and leave project save state unchanged.

For visual QA, `dudel --capture-export OUT.png [WIDTH HEIGHT SCALE]` screenshots the
window; `DUDEL_CAPTURE_EXPORT_FORMAT=jpeg` or `gif` picks the format shown.

## Timelapse

In **View > Timelapse**, choose speed and output size, then **Export GIF** or **Export MP4**. Both hold the final artwork for two seconds. MP4 is silent H.264; odd pixel dimensions receive white padding to make them even. Captured artwork is at most 1280 pixels on its longest side.

MP4 requires FFmpeg with the `libx264` encoder on `PATH`. Restart Dudel after changing PATH. No encoder is downloaded or bundled automatically. Missing encoders report an error without replacing the destination. GIF and image exports require no external encoder.

Video streams into a temporary file and is published after successful encoding. Cancellation terminates the encoder and removes staged output. Existing files of a different detected format are protected from replacement.

Container flags follow the [official FFmpeg documentation](https://ffmpeg.org/ffmpeg-formats.html). See the [animation timeline plan](ANIMATION_PLAN.md) for editable animation work.
