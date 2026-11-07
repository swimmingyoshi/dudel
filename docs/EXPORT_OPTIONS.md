# Export options

Current 0.1 behavior is documented below. The following are proposed 0.2.0
workflow goals, tracked by [UPGRADE_PLAN.md](../UPGRADE_PLAN.md):

- [ ] Provide one discoverable export flow with format-specific options.
- [ ] Let artists choose JPEG quality and matte/background color; keep PNG and
      lossless WebP transparency behavior clear. Explain GIF's palette and
      transparency limits and BMP/JPEG flattening before writing.
- [ ] Preview the conversion (including flattening/transparency loss) before
      the file is written, with a clear cancel path.
- [ ] Add **Export again** using the previous format, destination policy and
      settings. Prompt before overwriting and protect unrelated files.
- [ ] Keep exports tied to a captured document snapshot. Cancellation,
      encoder failure and disk errors leave the document and existing output
      intact; test that repeated export does not mark the project saved/dirty.
- [ ] Decide which settings persist globally and which are per project or
      per export. Avoid placing export preferences in artwork document data.

These items are release goals, not implemented behavior. The current options
and conversion behavior are:

Choose **Export image** or **Export image with transparency**, then select a file type in the save dialog or enter its extension.

| Format | Behavior |
| --- | --- |
| PNG | Lossless RGBA; preserves project DPI |
| WebP | Lossless RGBA; preserves transparency |
| JPEG / JPG | Quality 92; flattened onto white canvas paper |
| BMP | Flattened onto white canvas paper |
| GIF | Single image; limited palette and binary transparency |

Only PNG currently writes project DPI metadata. JPEG and BMP cannot retain transparency. PSD, SVG and project package commands remain separate. Exports capture a document snapshot, run in the background and leave project save state unchanged.

In **View > Timelapse**, choose speed and output size, then **Export GIF** or **Export MP4**. Both hold the final artwork for two seconds. MP4 is silent H.264; odd pixel dimensions receive white padding to make them even. Captured artwork is at most 1280 pixels on its longest side.

MP4 requires FFmpeg with the `libx264` encoder on `PATH`. Restart Dudel after changing PATH. No encoder is downloaded or bundled automatically. Missing encoders report an error without replacing the destination. GIF and image exports require no external encoder.

Video streams into a temporary file and is published after successful encoding. Cancellation terminates the encoder and removes staged output. Existing files of a different detected format are protected from replacement.

Container flags follow the [official FFmpeg documentation](https://ffmpeg.org/ffmpeg-formats.html). See the [animation timeline plan](ANIMATION_PLAN.md) for editable animation work.
