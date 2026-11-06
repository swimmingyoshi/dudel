# Text tool plan (0.2)

Status: design, 2026-11-05. No code is written yet. The owner accepted all four
decisions below on 2026-11-05. Tick stages off here as they land.

## Goal

Artists can add editable text to a drawing: type it, choose font, size, color and
alignment, reopen the project and change the words later, and move, rotate and
scale it without it turning into pixels. Dudel can draw CSP text layers today, but
it cannot create or edit text.

## Approach

Copy what vector layers do. `Layer` gains `text: Option<TextLayer>` beside `vector`.
The layer's pixels stay an ordinary raster that holds the rendered text, so
compositing, effects, masks, clipping, PNG and PSD export need nothing new. The
record is what can be edited, and pixel edits are refused until the layer is
rasterized.

Saved tiles hold the rendered text, so opening a project never redraws it. A
project looks the same on a computer without its font.

```rust
pub(crate) struct TextLayer {
    string: String,
    font: String,        // full name, as `clip_text::find_font` takes it
    size: f64,           // em size in canvas pixels; the UI also shows points
    color: [u8; 3],
    align: Align,        // left, right, center
    vertical: bool,
    wrap: Option<f64>,   // None: point text; Some: box width to wrap at
    line_spacing: f64,   // multiple of the font's line height
    start: [f64; 2],     // where the first line starts, in canvas pixels
    along: [f64; 2],     // unit vector along the lines
    down: [f64; 2],      // unit vector across them
    csp: Option<[f64; 2]>, // CSP's span and thickness, until the first edit
}
```

Size is stored in pixels, not points, so that changing the document's DPI doesn't
resize text and scaling only has to multiply one number.

`along`, `down` and `start` are the shape `clip_text::Text` already uses, so one
renderer serves both. Skew and perspective can't be expressed by them, which is
the point: those transforms rasterize (see stage 3).

## Decisions

1. **Where is the text typed?** Decided: a small floating editor beside the
   text box, with the real result updating live on the canvas. egui's `TextEdit`
   then provides IME, selection and clipboard for free. Typing directly on the
   canvas feels nicer but needs a custom caret and selection that follow zoom and
   rotation, plus our own IME handling. It can replace the floating editor later
   without changing the file format.
2. **Is kerning needed in the first release?** Decided: yes, by adding
   `rustybuzz` in stage 5, before 0.2 ships. `ab_glyph` has no shaping, and as far
   as we know it reads only the old `kern` table, which many modern fonts leave out
   in favour of GPOS. Headings would look badly spaced ("AV"), and Arabic and
   ligatures wouldn't work. Confirm the kerning gap against real fonts in stage 0.
   The new dependency needs the usual notice review.
3. **One style per layer?** Decided: yes for 0.2. Mixed styles inside one layer
   need rich-text editing and a run-based record. Artists can use one layer per
   style in the meantime. The importer also accepts only single-style CSP layers.
4. **Should new vertical text be in 0.2?** Decided: yes if stage 5 has room,
   because it matters for manga. Imported vertical text works either way. Correct
   Japanese needs vertical glyph forms (the `vert` feature), so it depends on
   decision 2.

## Stages

### Stage 0: make the renderer ready for live use

`clip_text::render` was written for one-off imports. Move the shared parts to
`src/text_render.rs` and fix these first:

- [ ] `find_font` reads every installed font file on every call. Build one index of
      names to file and face on an existing long-lived worker (`parallel::run`, and
      never a short-lived thread, because of the NVIDIA thread leak), and load fonts
      on demand with a small cache.
- [ ] `render` allocates a full-canvas `f32` buffer, which is 400 MB at 100 MP. Size
      coverage to the text's bounds and write through the sparse `Pixels` writer.
- [ ] `render` maps each glyph pixel to one canvas pixel, which is only correct at
      right angles. Any other rotation leaves holes. Transform the outlines and
      rasterize them instead (`ab_glyph_rasterizer` is already in `Cargo.lock`).
- [ ] Add kerning behind one layout function, so that shaping can replace it. CSP
      text stays unkerned, because CSP doesn't kern.
- [ ] Report characters the font doesn't have, so that the UI can name them. Fallback
      fonts can come later.
- [ ] Compare all CSP text samples before and after. Results must be unchanged.

### Stage 1: record, file format and history

- [ ] `src/text_layer.rs`: the record, `validate()` and `same()`. Validation limits
      string length and font name length, needs finite numbers and unit vectors, and
      must return an error on damaged data rather than panic.
- [ ] `Layer.text` in `src/document.rs`, with validation beside the vector check. A
      layer can't be both vector and text. `paint_blocker_for` gets a text message,
      and comparison at the `vector_layer::same` call includes text.
- [ ] **Format v8.** Add `TEXT_MAGIC = b"DUDL\x08\r\n"`, chosen in `tile_project.rs`
      ahead of `VECTOR_MAGIC`, or a v7 build would open the file and drop the text.
      Update `storage.rs` (both reader sites), `file_preview_header.rs` (the Explorer
      thumbnail DLL shares it) and `PROJECT_FORMAT_V4.md`.
- [ ] `document_edits.rs`: `NewText` and `SetText`, and reuse the rasterize edit.
      Merging down clears `text` as it clears `vector` (`transform.rs`).
- [ ] Rendering runs on the existing edit queue and is cancelled by newer edits.

### Stage 2: the tool

- [ ] `Tool::Text` on `T`, which is free (`Ctrl+T` stays Free transform). `Tool::ALL`
      grows to nine, and the rail and `tool_shift` size themselves from it. Add it to
      the shortcut editor and to command search with a `layer.new_text` action.
- [ ] Click for point text, or drag for a wrapping box. Clicking an existing text
      layer's box edits it.
- [ ] The editor opens with focus. `wants_keyboard_input` already keeps canvas
      shortcuts out of a focused text field; check tool keys, Space, `R` and `F`.
- [ ] Options bar: font (searchable, with a preview), size in px and pt, color (the
      foreground chip), alignment and line spacing.
- [ ] A whole editing session is **one undo step**. Commit on clicking away or
      `Ctrl+Enter`, and restore on Escape. A new layer left empty is removed.
- [ ] Session state joins `project_session!` in `tabs.rs`, or it leaks between tabs.
- [ ] Layers panel: a "Text" row label and type filter (`layers_panel.rs`,
      `layer_navigation.rs`), and an editing-strip action to rasterize.

### Stage 3: moving and transforming

- [ ] Pointer (`O`) picks a text box and moves it, in line with other objects, and
      double-clicking opens the editor. Drawing tools never move it.
- [ ] Move, rotate, flip and uniform scale keep text editable, through a `follow`
      equivalent in the `document_edits.rs` transform paths. Canvas resize, crop and
      rotate do too.
- [ ] Non-uniform scale, skew, perspective and mesh ask first, then rasterize.
- [ ] **Missing font.** Pixels stay, and the editor says which font is missing and
      asks for a replacement. A whole-pixel move just shifts pixels. A transform that
      needs a redraw asks to choose a font or rasterize.

### Stage 4: editable CSP text

- [ ] `clip_metadata.rs` keeps the parsed text as a `TextLayer`, with `csp` holding
      CSP's layout so the first render still matches its preview. Layers CSP's
      importer already skips stay skipped.
- [ ] The first edit clears `csp` and switches to Dudel's own layout.

### Stage 5: shaping and vertical text

- [ ] `rustybuzz` behind the stage 0 layout function.
- [ ] New vertical text, if there is room in 0.2.

### Not in 0.2

- Editable text in PSD. Export stays rendered pixels with a note in the report, and
  import stays pixels. Reading `TySh` is the natural next step.
- Text in SVG export, text on a path, mixed styles and fallback fonts.
- Text in the extension API.

## Testing

- Unit: record validation with damaged values, layout, wrap, alignment, kerning on
  and off, and rotated renders without holes.
- Round trip: save, reopen and compare; a v7 reader refuses v8; a project without
  text still saves as v7 or lower.
- History: a session undoes in one step; Escape changes nothing; an empty new layer
  disappears; two tabs keep separate sessions.
- Workflow tests through real egui input, at desktop and compact sizes, including
  tool keys while typing.
- Memory: a large text at the 100 MP limit stays near the size of its bounds.
- Owner's pass: Japanese IME, the Windows touch keyboard for pen-only use (a pen
  display may have no keyboard in reach), a missing font, and a CSP text file.
