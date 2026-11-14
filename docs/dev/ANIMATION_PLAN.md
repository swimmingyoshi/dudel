# Animation

Status: built for 0.6 (2026-11-14) with **frames as layers**, chosen by the
owner over the earlier cels-and-tracks proposal, which needed a new project
format and its own save, undo and recovery paths.

## Design

- Each top-level layer or folder is a frame, bottom first (`src/animation.rs`).
  A folder is a frame of several layers. Hidden top-level entries are left
  out. The bottom and top entries can be a background and a foreground that
  show with every frame.
- A layer's `hold` is how many frame periods its frame shows for (1 to 240).
  The document's `animation` holds fps, Loop/Ping-pong/Once, background,
  foreground and onion skin settings. Both are saved in the project metadata
  only when used, are undoable edits (`Edit::Animation`, `Property::Hold`,
  `Edit::NewFrame`), and are compared by history.
- The layers never change visibility to show a frame. The canvas and the
  exports composite a copy of the document (tiles shared) in which only the
  frame, background, foreground and onion skin are visible
  (`animation::frame_view`), so saving, undo and recovery work as for any
  layered picture and older versions open an animation as layers.
- The onion skin tints earlier frames red and later ones green with a Color
  Overlay effect on the copy, fading with distance; it is never exported.
- Exports (`src/animation_export.rs`) run as background export jobs: GIF
  (exact palettes up to 255 colors, else NeuQuant; background disposal;
  holds as delays that keep time), numbered PNG frames, sprite sheets, and
  MP4 through FFmpeg. Ping-pong exports come back without repeating ends.

- Playback (`src/animation_cache.rs`) shows each frame as one picture
  rendered at its on-screen size (`Document::composite_reduced`) by a
  background job, in playing order from the playhead, within 512 MiB (the
  resolution drops for long animations). The clock only advances to ready
  frames. Redrawing tiles per frame took about 0.2 s at 1080p and 6 s for a
  painted 4K frame in the app; cached frames take 60-100 ms each to render
  once (`--benchmark-animation`). Drawing never uses the cache.
- Stepping between frames (with the onion skin) uses the tiles. On a frame
  change the display hurries (`CanvasTextures::hurry`): it waits for its
  worker within one update for up to 12 ms and gives it larger batches, so
  a 1080p step shows in about 2 screen refreshes instead of 14, a 4K one
  with a small character in 6 instead of 41, and a 4K frame painted edge
  to edge in 165 instead of 421 (still about 3 s).
- Keys: a Space tap plays or pauses (a held or dragged Space still pans,
  zooms or rotates); comma and period step frames; all are commands with
  editable shortcuts.

## Left to do

- The owner's pen pass.
- Reordering frames by dragging in the timeline (the layers panel does it
  today) and larger frame thumbnails.
- Later, if asked: linked frames, audio, tweening, camera moves.
