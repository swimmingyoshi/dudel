# Engineering backlog

Work on the code, the build and performance that is agreed to be worth doing
but not started. Features and releases are in [ROADMAP.md](ROADMAP.md). Written
2026-11-12, after 0.4.0; numbers are from that day unless a line says otherwise.

Tick an item when it is done and say where (commit or file). Add what you
find; delete what turns out not to matter, with a line saying why.

## 1. Report closed: the wet brush lagged on long strokes

**Reported by the owner, 2026-11-12, after the wet-brush speed-up (8ec6ca4).**
With a paper set on the canvas, a wet brush is "kind of ok", but on long
strokes with pen pressure the line starts to fall behind the pen.

Measured the same day with the tools below, reproduced with the owner's own
brush, and fixed in the scripted run on 2026-11-13 (see "Fixes applied"):
the fast stroke that ended 853 dabs behind the pen now ends 11 behind. Still
to confirm on the owner's own pen.

### The tools

- **`dudel --profile-stroke REPORT.json [CASE NAME FILTER]`** paints scripted
  strokes through the real canvas frame (pen packets, painting queue, display
  compositing) without a window, in real time, and prints a summary per case.
  Every frame is in the report. Code: `src/stroke_profile.rs`. It does not
  measure the GPU, the pen driver or the side panels.
- **`DUDEL_TRACE_STROKE=FILE`** makes a normal session write one line per
  frame of every stroke to FILE. Summarize it with
  `python scripts/stroke_trace_summary.py FILE`. In PowerShell:

  ```powershell
  $env:DUDEL_TRACE_STROKE = "$env:TEMP\dudel-stroke.jsonl"
  targetelease\dudel.exe
  ```

- Both use the stage timers in `src/stage_timer.rs`. They cost one atomic
  load per timed section while off. To time something new, add a `Stage` and
  a `stage_timer::scope(...)` around it.

### What the scripted run shows

3000 x 2000 canvas at about half zoom, Cold-pressed paper, a 7,300 px stroke
in three overlapping sweeps at 1,800 px a second, pressure rising and falling,
200 pen reports a second, frames at 60 a second, 12 threads. Report:
`benchmarks/stroke-profile/baseline-20261112.json` (local, not in git).

| Case | ms per dab | Dabs asked per second | Frame p50 / p95 ms | Furthest behind the pen |
|---|---|---|---|---|
| Plain brush 60 px, paper | 0.11 | 446 | 1.5 / 3.2 | 0 px |
| Watercolor wash 60 px (as built in) | 0.79 | 375 | 5.9 / 8.3 | 4 px |
| The same without paper | 0.57 | 375 | 4.8 / 7.3 | 0 px |
| Watercolor round 18 px (as built in) | 0.04 | 2745 | 2.2 / 3.3 | 0 px |
| Water brush 44 px (as built in) | 0.28 | 681 | 4.0 / 5.9 | 0 px |
| Watercolor wash 150 px | 2.64 | 150 | 8.6 / 13.1 | 24 px |
| Watercolor wash 150 px, size by pressure | 0.94 | 335 | 6.5 / 12.5 | 24 px |
| Watercolor wash 300 px | 8.07 | 75 | 12.2 / 19.4 | 192 px |
| The same without paper | 6.79 | 75 | 10.0 / 16.9 | 48 px |
| Watercolor wash 300 px, size by pressure | 2.74 | 168 | 9.5 / 14.9 | 72 px |
| Watercolor wash 60 px, watercolor layer | 0.94 | 375 | 7.1 / 10.4 | 52 px |
| Watercolor wash 150 px over a wet wash | 2.88 | 150 | 9.1 / 15.1 | 48 px |

1. **Nothing in the painter gets slower as the stroke gets longer.** Time
   per dab is the same in the first, middle and last third of every stroke.
   Water reads and writes are 2 to 6% of a dab and do not grow, so the water
   field is not the cause. Display compositing per frame grows over the
   stroke, up to about double, but stays near 1 to 2 ms.
2. **The lag is a queue.** Painting gets 2 ms of each frame, or 8 ms once
   three dabs are waiting (`pump_render_queue`); a dab that takes longer
   still runs whole, one per frame. When the pen asks for more dabs a second
   than that pays for, the rest wait, and the backlog keeps growing for as
   long as the stroke goes on. That reads as "lags on long strokes".
3. **Cost grows with the square of the brush size**: 0.79 ms at 60 px,
   2.64 ms at 150 px, 8.07 ms at 300 px. The built-in sizes keep up here; at
   300 px the painter is at its limit (27 of 250 frames over 17 ms, 8 dabs
   behind). The same brush on a canvas twice as large, zoomed out to look the
   same, costs four times as much at the same pen speed.
4. **Pressure:** with size by pressure the dabs are closer together (twice
   as many in these strokes) and the full-pressure stretches cost the
   full-size price, so the backlog builds while pressing hard.
5. **Where a 300 px wet dab's 8.1 ms goes:** coverage 2.6 ms, half of it
   paper (1.3 ms without paper); blur 2.0 ms; blending 1.7 ms; the copy
   under the dab 1.3 ms; water 0.45 ms.
6. **Dabs under about 90 px are painted pixel by pixel on one thread**
   (`dab_bands::worthwhile`). A 60 px wet dab costs 0.79 ms, seven times a
   plain dab of the same size; 0.23 ms of it is paper.
7. A watercolor layer adds about 20% (pigment mixing); painting over a wash
   that is still wet adds about 10%.
8. Drying is idle while a stroke is down (`dry_tick` drops its job when the
   canvas is busy). Confirmed in the code.

Also found: the paper surface shown under the canvas is made on the UI
thread, six tiles a frame (`paper_view::prepare`). The first 11 frames after
a paper canvas opened each took about 20 ms longer than the frames after
them. It is not stroke lag, but it would stutter the same way whenever new
tiles come into view.

### The owner's case, reproduced (2026-11-12)

The owner's setup: **Chisel marker switched to Wet, water 50%, 300 px, on a
2048 px canvas.** Run it with
`dudel --profile-stroke REPORT.json chisel` (2048 x 2048, Cold-pressed paper,
a 5,270 px stroke in 2.9 s).

| Case | ms per dab | Dabs asked per second | Behind at pen lift | Finished after lift |
|---|---|---|---|---|
| Chisel marker 300 px, plain paint | 0.89 | 399 | 0 | 0.03 s |
| Chisel marker 300 px, wet 50% | 5.82 | 399 | 836 of 1,172 dabs (3,760 px) | 7.2 s |
| The same without paper | 5.86 | 399 | 828 dabs | 7.2 s |
| The same on a watercolor layer | 6.08 | 399 | 832 dabs | 7.4 s |
| The same with size by pressure | 1.86 | 890 | 1,630 of 2,613 dabs | 5.7 s |

- **The dabs are 4.5 px apart.** Spacing is a share of the tip's short side
  (`stroke::spacing`: size x aspect x spacing = 300 x 0.3 x 0.05). The pen
  asks for 400 wet dabs a second, at 5.8 ms each: 2.3 s of painting for
  every second of stroke. Painting gets at most 8 ms of a frame, so the
  line falls further behind for as long as the pen moves. This is the lag.
- **Every dab works on the full 300 px square.** `DabRaster::new` makes the
  box from the long radius whatever the aspect and angle. The tip covers
  under a quarter of it (a 300 x 90 px ellipse), yet the copy and the blur
  cost what they cost a round 300 px dab: blur 1.90 ms, coverage 1.34 ms,
  capture 1.16 ms, blend 0.98 ms, water 0.30 ms.
- **Each dab blurs 24 px around itself** (`blend_brush::blur_radius`) and
  the next dab is 4.5 px on, so consecutive dabs blur nearly the same area.
- Paper does not matter here (the marker has no granulation), nor does the
  layer kind. Cost per dab is again flat from the first third to the last.
- As plain paint the same brush keeps up: 0.89 ms a dab.

### What is not known yet

The scripted run leaves out the GPU, the pen driver and the side panels, and
it guesses the pen speed. In a real session each frame is longer, so
painting gets fewer turns a second and falls behind sooner. The owner's case
above is far enough over the limit that these cannot change the conclusion.

- [x] Stage timers behind a switch.
- [x] A repeatable stroke (scripted; `--replay-input` was not needed).
- [x] Which stage costs most, and which grows with the stroke (none does).
- [x] The owner's brush, size and canvas (2026-11-12), reproduced above.
- [x] Traces of the owner's own strokes (2026-11-13, below). For reading
      later ones: if "frame to frame"
      is well above "CPU per frame", the display or GPU is holding frames
      back; if "rest of the interface" is large, the side panels are.

### Fixes applied (2026-11-13)

Same scripted strokes as above, same PC. "Before" is 1baebd8. The profile
now shows frames on a 60 Hz display the way a blocking display does (a
frame that takes longer waits for the refresh after), which is a little
stricter than when "before" was measured.

| Case | ms per dab, before | now | Behind at pen lift, before | now | Finished after lift, before | now |
|---|---|---|---|---|---|---|
| Chisel marker 300 px, wet 50%, pen at 1,800 px/s | 5.82 | 2.01 | 853 dabs | 11 dabs | 8.1 s | 0.10 s |
| The same, frames back to back (Low latency on) | not run | 2.30 | not run | 0 dabs | not run | 0.01 s |
| The same, pen at 600 px/s | not run | 2.07 | not run | 0 dabs | not run | 0.05 s |
| The same with size by pressure | 1.86 | 0.88 | 1,837 dabs | 0 dabs | 6.2 s | 0.07 s |
| The same on a watercolor layer | 6.08 | 2.24 | 833 dabs | 43 dabs | 7.6 s | 0.23 s |
| Watercolor wash 300 px | 8.07 | 4.56 | 2 dabs | 1 dab | 0.12 s | 0.10 s |
| Watercolor wash 150 px | 2.64 | 1.81 | 0 dabs | 0 dabs | 0.03 s | 0.03 s |

During the stroke the fast cases are at most 11 to 49 dabs behind (66 to
294 px) and recover; before, the queue only grew.

Cheaper dabs (76b96b9 and the commit after it):

- [x] **A tight box for narrow and turned tips** (`document::tip_extent`):
      the turned ellipse's real reach instead of the square around its long
      side. Plain paint is the same bit for bit (tested over 360 tips); a
      wet dab's blur sees a smaller area and differs by at most 4 levels.
- [x] **The copy under the dab across cores** (`blend_brush::capture`), in
      row bands. Same pixels.
- [x] **The blur across cores without copies**: rows are blurred into a
      second buffer, columns back from it in strips of columns. Each row
      and column keeps its sums in the same order, so the result is the
      same bit for bit (the line-by-line test covers it).
- [x] **Coverage worked out once** (`dab_bands::Drawn`). It was made in
      bands, flattened on one thread, then copied band by band a second
      time before blending. Run serially the coverage stage took only two
      to four times as long as across twelve threads: the fan-out and the
      flattening cost more than the arithmetic saved.
- [x] **A spacing floor for wet and blur dabs**: 2% of the tip's long side
      (`brush::SOFT_DAB_SPACING`), with each held dab delivering more
      (`DabRaster::held`, applied after the paper so the tooth still
      shows). The owner's brush goes from a dab every 4.5 px to every 6 px.
      No built-in brush is affected.

More painting per second:

- [x] **Far behind the pen, painting takes most of the frame**
      (`render_controller`, `PAINT_FAR_BEHIND_DABS`): 20 ms instead of 8 ms
      once 16 dabs wait. The rest of a frame costs the same however long
      painting had. The price: frames come 30 to 45 times a second while
      the queue is that long (41 of 143 frames in the fast case above).

Round wet brushes and plain paint come out pixel-identical to before in the
scripted cases. The wet Chisel marker differs by 0.13 levels on average
(at most 29, on 0.04% of the canvas) and looks the same at 200%; all of
that difference is from the tight box and the 2% floor.

**What was tried and dropped, so nobody repeats it:**

- **A wider floor.** At 4% of the long side (12 px) the stroke's edge is
  plainly ribbed, one rib per dab, and at 3% faintly. Comparison images:
  `benchmarks/stroke-profile/look-comparison.png` (local only).
- **Sweeping each held dab's coverage back to the previous dab**, as if the
  tip were dragged. The ribs stayed. They do not come from the outline:
  each wet dab mixes the paint under it toward a blurred copy and then
  lays paint on top, so what a pixel ends up with depends on its last few
  dabs, and with few dabs that depends on where the pixel sits between
  them. Only many small steps average this out.
- **Keeping the worker threads spinning longer between stages**
  (`parallel::LINGER` at 200 to 2,000 microseconds instead of 50). Slower
  at every value: the spinning threads take time from the one doing the
  work in between.

### The owner's trace (2026-11-13, build 9191d9c)

Three strokes with the Chisel marker at about 300 px on a 2048 px canvas,
no paper, Low latency on. **The dabs were plain paint, not wet** (only the
`paint` stage ran), so this trace does not test the wet brush.

| Stroke | Pen down | Dabs a second | ms per dab | Furthest behind | At pen lift | Frame to frame p50 / p95 |
|---|---|---|---|---|---|---|
| 1 | 1.7 s | 648 | 0.49 | 0 dabs | 0 | 6.1 / 11.0 ms |
| 2 | 1.2 s | 849 | 0.47 | 40 dabs | 2 | 7.0 / 14.4 ms |
| 3 | 1.1 s | 1,100 | 0.49 | 22 dabs | 0 | 7.4 / 23.8 ms |

What it does show:

- **The pen is much faster than the script assumed**: 2,900 to 5,000
  canvas px/s against 1,800.
- **Frames follow one another at once** (135 to 165 a second), as Low
  latency intends. A frame costs 2.1 ms more than its CPU work, and the
  rest of the interface 1.0 ms. The profile's back-to-back cases now add
  that 2.1 ms.
- Plain paint keeps up at that speed, with the longer painting turn
  stepping in near the end of the fastest stroke (11 frames over 20 ms).

The same strokes scripted with the brush set to Wet, as traced otherwise
(`--profile-stroke REPORT.json "as traced"`):

| Case | ms per dab | Dabs a second | Behind at pen lift | Finished after lift |
|---|---|---|---|---|
| Plain paint, 4,000 px/s | 0.52 | 889 | 0 dabs | 0.02 s |
| Wet 50%, 2,900 px/s | 1.83 | 483 | 71 dabs (430 px) | 0.20 s |
| Wet 50%, 4,000 px/s | 1.82 | 667 | 292 dabs (1,750 px) | 0.68 s |

So at the owner's speed a wet stroke with this tip still trails, by a
fraction of a second where it was many seconds. Far behind the pen painting
already has about 83% of the time; the rest has to come from cheaper dabs
(blur 0.63 ms, blend 0.42 ms, coverage 0.32 ms, copy 0.23 ms of 1.82 ms).

The trace now starts every stroke with a line naming the tool, brush, mode,
size, water, canvas, paper and layer, so the next one says which it was.

### The owner's wet stroke (2026-11-13, build 6f10d17)

One stroke, traced as Brush: Chisel marker, mode Wet, 300 px, water 50%,
2048 x 2048, no paper, plain layer, zoom 32%, Low latency on.

- 2.0 s pen down, 784 dabs: 392 a second on average, about 460 a second
  (2,800 px/s) in its two fast stretches.
- 1.69 ms per dab (blur 0.61, blend 0.37, coverage 0.31, copy 0.22, water
  0.16), the same from the first third to the last.
- **Furthest behind: 44 dabs (about 260 px, less than the tip is long), in
  the fast stretches; back under 10 dabs within a quarter second, and 0
  at pen lift.** Before the fixes the same stroke was about six seconds
  of painting.
- The longer painting turn ran in those stretches: 40 of 127 frames came
  more than 20 ms after the one before (about 40 frames a second there,
  70 to 120 elsewhere).

**Owner's verdict: it still felt laggy and heavy.** Read with the trace,
"heavy" is the frame rate dropping to about 40 a second while the line
trailed, as much as the trail itself.

### Second round (2026-11-13, after the owner's wet stroke)

- [x] **Wide blurs on a coarser grid** (`blend_brush::coarse`): a blur of
      16 px or more is worked out on averages of 4 x 4 pixels (2 x 2 from
      8 px) and read back between them. Cells sit on the canvas's own grid,
      so they do not shift from dab to dab, and the two box passes are
      chosen to blur exactly as wide as before (`pass_radii`). Against the
      blur of every pixel it is within 0.15 levels on a smooth ramp, 1.2
      at a hard edge and 3.5 on 10 px black and white stripes (tested).
      Finished canvases differ from the previous build by at most 3 levels.
      The blur went from 0.63 ms to 0.09 ms of the owner's dab.
- [x] **A gentler step before the long painting turn**: 12 ms from 16
      dabs behind, 20 ms only from 64. The 40-frames-a-second stretches in
      the owner's trace started at 16.

| Case | ms per dab, owner's trace build | now | Behind at pen lift, then | now | Frames over 17 ms, then | now |
|---|---|---|---|---|---|---|
| Wet 50%, as traced, 2,900 px/s | 1.83 | 1.35 | 71 dabs | 7 dabs | 71 of 90 | 0 of 162 |
| Wet 50%, as traced, 4,000 px/s | 1.82 | 1.40 | 292 dabs | 137 dabs | 75 of 85 | 48 of 80 |
| Wet 50%, 1,800 px/s, display paced | 2.01 | 1.48 | 11 dabs | 0 dabs | 41 of 143 | 0 of 181 |
| Watercolor wash 300 px | 4.56 | 3.53 | 1 dab | 0 dabs | 1 | 1 |

At the speed of the owner's fast stretches (about 2,800 px/s) the wet
Chisel marker now keeps up at full frame rate in the scripted run. At
4,000 px/s it still trails and drops frames.

### The owner's verdict on the second round (2026-11-13, build 322e0db)

**"It feels much better now."** Low latency was on throughout. The trace of
that session: ten strokes with the same wet Chisel marker (2048 px canvas,
no paper), the longest 3.8 s and 1,586 dabs.

- 1.24 ms per dab over the session (1.69 ms on the build before).
- **Every stroke was fully painted at pen lift.** At most 66 dabs behind,
  for a moment in a stroke asking 443 dabs a second; the long stroke at
  most 27.
- Frames 115 to 180 a second (median per stroke). 22 of 1,617 frames came
  more than 20 ms after the one before, and only one frame had 64 or more
  dabs waiting, so the longest painting turn almost never ran.

### After closing: the paper (2026-11-13)

The owner's first report was with a paper set; the confirming traces had
none. The next fix on the list, done afterwards:

- [x] **Paper height once per pixel** (`paper::Surface`). The paper's
      direction is worked out once per dab instead of at every pixel, and
      the height once per pixel for both the tooth and granulation instead
      of once for each. Every canvas the profile paints is byte for byte
      the same.

| Case (with Cold-pressed paper) | Coverage per dab, before | after | ms per dab, before | after |
|---|---|---|---|---|
| Watercolor wash 300 px | 1.50 ms | 1.01 ms | 3.52 | 3.08 |
| Watercolor wash 150 px | 0.72 ms | 0.44 ms | 1.55 | 1.27 |
| Watercolor wash 60 px | 0.63 ms | 0.53 ms | 0.77 | 0.68 |
| Chisel marker 300 px, wet 50% | 0.53 ms | 0.50 ms | 1.48 | 1.44 |

The Chisel marker gains little: it has no granulation, so it asked for the
height once already; what is left of its paper cost (about 0.2 ms) is the
noise itself.

### If it comes back

- [ ] At faster strokes (4,000 px/s scripted, 137 dabs behind at lift),
      or with a paper on the canvas: **coverage and blend** are now most of
      a wet dab (0.53 and 0.45 ms of 1.48 ms with paper; the paper is
      0.22 ms of the coverage), then the copy (0.21 ms).
- [ ] **Is 30 to 45 frames a second while far behind acceptable?** If the
      pen cursor feels worse than the shorter queue feels better, lower the
      20 ms or raise the 16 dabs.
- [ ] **Paint off the interface thread**, so painting does not compete with
      showing frames at all. Larger change; only if the above is not
      enough.
- [x] **Paper height once per pixel** (done after closing, above). Next
      would be keeping heights per tile, so overlapping dabs stop asking
      for the same noise.
- [ ] **Blur less.** Each dab still blurs its whole area twice although
      nearly all of it was blurred for the previous dab. Any shortcut here
      changes the result and needs the same look comparison.
- [ ] **Small wet dabs:** make the pixel-by-pixel path cheaper (coverage a
      row at a time, writes a tile span at a time) or paint several small
      dabs across cores. Now the largest stage of every wet brush under
      about 90 px.
- [ ] **Paper view tiles on a background worker.**
- [ ] `render_region::dab_bounds` still reports the square around the long
      side times 1.41 as painted, so the display refreshes more than a
      narrow dab touched.

### Big brushes on the owner's layers (2026-11-13, TEST-PAINTLAYERS-3)

The owner's next report: watercolor layers and wet brushes are fine for
most brushes, but the wet Chisel marker (50% water, 76% pigment, 300 px)
is still the slow one, and the palette knife on thick paint feels laggy
and starts each stroke flat before turning. Their document: 2048 px, no
paper, a plain, a Watercolor and a Thick paint layer. Profile cases for
it: `--profile-stroke REPORT.json "thick paint layer"` and `"pigment 76"`.

What it showed, at the owner's pen speed with frames back to back (before
the changes below):

| Case | Painting per dab | Relief per dab | Behind at pen lift |
|---|---|---|---|
| Chisel 300 px, wet 50%, pigment 76%, plain layer | 1.43 ms | none | 7 dabs |
| same, Watercolor layer | 1.48 ms | none | 6 dabs |
| same, Thick paint layer | 2.75 ms | 1.23 ms | 357 dabs (1.2 s) |
| Palette knife 300 px, Thick paint layer | 1.33 ms | 0.81 ms | 1,569 dabs (2.6 s) |

So the Thick paint layer is the slow one, and the knife also lays far too
many dabs. Auto-drying is not involved: it waits while a stroke is painted.

- [x] **Relief in bands across cores** (`material::touch_relief`). The
      relief under a dab is copied out a tile row at a time
      (`Mask::read_region`), worked out in 32-row bands across cores and
      written back a tile at a time (`Writer::write_region`); the bristle
      noise keeps its hashes in a table per dab and the blur's 7 x 7 mean
      is summed a row at a time (whole numbers, so exact). The old code is
      kept in the tests as `touch_relief_by_pixel`, and the new one gives
      the same bytes in every case tested; the profile's canvases are
      identical. Side by side on a 300 px dab: building 16.1 ms to 1.6 ms,
      a knife-shaped smudge 1.7 ms to 0.9 ms (`material::relief_timing`).
- [x] **Tips that turn with the stroke are spaced along it**
      (`Brush::dab_spacing`, `reach_along_stroke`). The knife's blade lies
      along the stroke, but spacing was a share of its narrow side: a
      300 px knife laid a dab every 2 px. Spacing is now a share of how far
      the tip reaches along its direction of travel (9 px for that knife),
      through the existing floor, so each dab delivers more and flow builds
      up as before. Built-in brushes it touches: Palette knife (4.5x fewer
      dabs), Flat bristle (2.5x), Dry gouache (1.4x), Impasto (1.3x). The
      300 px knife: 0 dabs behind at lift (was 1,528 to 2,414), frames 6
      to 8 ms (were 22). Look: straight and gently curved stretches match;
      at the profile's sharp 90 degree corners the turning blade fans out
      in fewer, more visible steps. The other three brushes differ by more
      than 2 levels on at most 2% of their pixels. Owner on the pen,
      2026-11-13: "everything works much better now". Node brushes that
      keep the tip at one angle to the stroke (Direction into Rotation over
      180 degrees) are spaced the same way: `Compiled::turn_with_stroke`
      tries the graph once over headings, pressures, speeds, tilts and
      random values and finds that angle.
- [x] **No more flat start** (`Stroke::scatter`, `heading_reach`). A tip
      that turns with the stroke used angle 0 until a second dab gave the
      stroke a direction, so every stroke began with a flat bar. Its first
      dabs now wait until the stroke has gone a fifth of the brush size
      (2 to 24 px) and are then drawn at that direction; a tap still paints,
      flat if the pen did not move.
- [x] **Black vanished in pigment mixing** (`pigment::compute`). Mixes are
      weighted by luminance to the power 0.35, and pure black's is zero, so
      wet black over wet paint on Watercolor and Thick paint layers left the
      color under it almost unchanged. The model now takes anything darker
      than byte 10 as byte 10: 10% black over yellow gives a dark olive
      instead of yellow; colors without a channel near 0 mix exactly as
      before, pure primaries shift by at most 15 levels.
- [x] **A flicker in tile-shaped areas while drawing** (owner's recording:
      300 px Soft pastel, Wet, on a 2048 px canvas at 32%). On canvases of
      2048 x 2048 and up, tiles are kept reduced when zoomed out. A brush
      over 128 x 128 px of damage a frame takes the sliced display path,
      which recomposited each touched tile *quick* first (one pixel per
      shown texel) and exact after. On a grainy brush the two sample the
      paint differently, so the tiles around the brush flipped between
      them every frame, and settled once the stroke ended. Tiles that a
      bounded stroke changes, already shown at their level, are now
      composited exactly at once (`Tile::exact`); quick tiles stay for new
      tiles, new resolutions and whole-canvas changes such as hiding a
      layer. Test: `a_large_stroke_zoomed_out_never_shows_quick_tiles`
      (fails without the change). Cost: an exact composite of a whole tile
      instead of a sixteenth of one, on the display worker.
- [x] **Two undos for one wet stroke.** Drying after a stroke was an undo
      step of its own, so the first undo took back only the drying. Now the
      first drying after a stroke reopens that stroke's undo step
      (`History::reopen_last`), when the stroke painted the layer that is
      wet and nothing waits to be redone: one undo takes back the stroke
      and its drying, redo brings it back dried. Drying is still a step of
      its own when something else was done after the stroke. Test:
      `wet::a_wet_stroke_and_its_drying_undo_together`.
- [ ] **The wet Chisel at 300 px** still costs 1.4 ms a dab on plain and
      Watercolor layers, mostly blend (0.5 ms), coverage (0.35 ms) and the
      copy (0.25 ms); see "If it comes back".
- Direction (owner, 2026-11-13): make the CPU painter fast first, and keep
  GPU acceleration possible as an option for those who have a GPU. Keep
  per-pixel work in plain functions over buffers, so a GPU path could be
  checked against them.

## 2. Performance still on the table

Measured with a 200 px brush on a 3000 x 2000 canvas, release build
(`cargo test --release --bin dudel -- material::look --ignored --nocapture`).

| Case | Before 8ec6ca4 | Now |
|---|---|---|
| Plain paint, per dab | 1.4 ms | 0.8 ms |
| Wet brush | 16.3 ms | 3.7 ms |
| Wet brush on a watercolor layer | 19.9 ms | 7.3 ms |
| Palette knife (smudge on thick paint) | 13.8 ms | 5.2 ms |
| Smudge on a plain layer | 4.4 ms | 2.1 ms |
| Thick paint | 4.9 ms | 3.5 ms |
| Drying step, 2-megapixel wash, time on the UI thread | 57 to 107 ms | 6 to 7 ms |

- [x] **Wet brush lag** (section 1): the owner confirmed it feels much
      better on 2026-11-13.
- [ ] **Planning a drying step** takes 34 to 100 ms on a 2-megapixel wash. It
      runs in the background now, so the window stays smooth, but the wash
      dries slowly. `wet::plan` allocates five buffers per 32 px tile.
- [x] **Thick paint relief** now works in row bands across cores and
      reads and writes a tile at a time (section 1, big brushes).
- [ ] **Pigment mixing** is the remaining cost on watercolor and thick paint
      layers (`pigment::mix`: 16 bands per mix, behind a 4096-slot cache).
- [ ] **`blend_brush::capture`** now copies in row bands across cores but
      still asks for each pixel; read rows from the layer's tiles instead.
- [ ] **Affine relief follow** still scans the whole canvas.
- [ ] **A recorded baseline.** Turn `painting_cost` and `drying_cost` into a
      script that writes a JSON file under `benchmarks/`, so a regression
      shows up as a changed number in a diff.

## 3. Code structure

Worked through on 2026-11-13, measuring first.

- [x] **Break up `Document::paint_dab_rows`.** It is now `dab_rows.rs`:
      `Coverage::at` (what a dab delivers at a pixel) and one function per
      way of painting (`prefix_coverage_rows`, `blend_rows`, `paint_rows`).
      The arithmetic was moved, not rewritten: the canvases of all 32
      profiled strokes are byte-identical and paint at the same speed.
- [x] **`main.rs`.** `run()` was 1,640 lines. The windowless commands are
      `cli::run` (the eleven one-report benchmarks in a table) and the
      screenshot fixtures are `capture_fixtures::prepare`, whose `MODES`
      list replaces the 60-line `||` chain. All 65 screenshot modes give the
      same pixels as before. Each fixture is still a block in one function,
      because several build on one another in order (a mask fixture starts
      from the layer mask one); a new mode is a name and a block.
- [x] **Per-project state cannot be missed.** `tabs.rs` has a test that
      reads the app struct from the source: each of its 259 fields must be
      in `project_session!` (parked with its tab, 106 of them) or in the
      test's list of fields all tabs share (153). A new field in neither
      fails the suite. This is the guarantee the regrouping was wanted for,
      without touching thousands of call sites.
- [x] **Line endings.** `.gitattributes` gives every checkout LF. The index
      already held LF; 111 working files were CRLF.
- [ ] **Split the single crate into a workspace.** Not started, and the
      case for it is weaker than this list first said. Measured on an idle
      PC, one edit costs 5 s for `cargo check`, 25 s for the test program
      and 15 to 45 s for an optimized build with the new `fast` profile
      (the "minute per check" was measured while other builds ran). What
      the split would still buy: an engine that tests and benchmarks without
      a window, engine tests compiled with optimization (six pixel-heavy
      tests take 20 to 35 s each unoptimized and set the suite's 100 s),
      and a smaller incremental cache.

      What stands in the way: 110 of the 205 modules (120,000 lines) form
      one cycle, so no engine crate can be cut out yet. Only 22 modules
      (8,900 lines, 5%) are free of both the window toolkit and that cycle.
      The cycle has two causes:

      1. Engine files carry their own panels and dialogs. `fill.rs`,
         `material.rs`, `paper.rs`, `guides.rs`, `canvas_size.rs` and
         `gradient_tool.rs` each hold both the pixel work and the egui
         controls for it.
      2. Engine files end in an `impl crate::Dudel { ... }` block with the
         app's side of the feature (`blend_brush`, `vector_layer`,
         `document_edits`, `stage_timer`, `import_report`).

      So the first stage is not a workspace but a rule applied file by
      file: split each mixed file into the engine part and a `*_ui.rs` or
      app part, until `document` and what it uses name neither egui nor
      `Dudel`. (`document.rs` named `Dudel` once, for `takes_material`;
      that is now `Layer::takes_material`.) The workspace follows when the
      cycle no longer contains a panel. To measure progress, list for each
      module the modules it names with `crate::` (leaving out its tests)
      and find the groups that reach one another.
- [ ] **Group the app struct's fields by subsystem**, as `setup` and
      `drying` are. Still worth doing a subsystem at a time when one is
      being worked on anyway; the test above removed the urgency.

## 4. Build folder size

The build output had grown to about 77 GB:

| Folder | Size | What it is |
|---|---|---|
| `target/debug/incremental` | 36.1 GB | incremental cache for debug builds |
| `artifacts` | 14.0 GB | release packages and candidates, 446,000 files |
| `target/debug/deps` | 10.5 GB | debug libraries, test programs, 5.8 GB of debug symbols |
| `target/qa` | 4.8 GB | offline rebuild checks of source archives |
| `target/release` | 3.8 GB | release build |
| `target/review`, `target/wt` | 1.9 GB each | leftover verification builds |
| `target/x86_64-unknown-linux-gnu` | 1.1 GB | Linux cross-build |
| `shell/thumbnails/target` | 1.0 GB | thumbnail library build |

- [x] **Clean up once.** Done 2026-11-12 with the owner's approval:
      `target/debug/incremental`, `target/qa`, `target/review`, `target/wt`
      and the 17 extracted `-source` folders under `artifacts/releases`, each
      after its `-source.zip` matched its recorded SHA-256. Freed 50 GB;
      `target` is now 16 GB and `artifacts` 2.9 GB. Every ZIP and `tar.gz`
      stays, and so do the extracted program folders of old candidates
      (small). The table above is the state before the cleanup.
- [x] **Smaller debug symbols** (2026-11-13): line tables only for the dev
      profile and none for dependencies. The test program's symbol file
      went from 690 MB to 254 MB and its link from 8.8 s to 5.8 s.
- [x] **`scripts/Clean.ps1`** (2026-11-13) lists the incremental caches,
      measurement and verification folders, old package metadata and
      extracted source folders whose ZIP matches its checksum, and removes
      them with `-Apply`. Run it after each release. A day of measuring had
      put 31 GB back.
- [ ] The workspace split (section 3) shrinks the incremental cache at its
      source.

## 5. Development process

- [x] **CI minutes** (2026-11-13). The hosted runs already cache
      dependencies (`Swatinem/rust-cache`, restored in about a minute). A
      Windows run took 25 minutes: 5 for the build and the suite, 5 more to
      build and run the whole suite again with the diagnostic features, 5
      for lint and a release build, 1 for the largest-project test and 5.5
      for packaging, which builds the release again with the shipping flags.
      Now the diagnostic features' own tests run alone (the suite is not
      repeated) and Windows skips the release build that packaging repeats.
      The 0.5.0 run took 17.6 minutes on Windows, with the dependency
      cache rebuilt for the new debug settings; Linux took 8.8, the same as
      before, for the same reason.
- [x] **Optimized builds while working** (2026-11-13). A release build
      after one edit takes 3 minutes. `cargo run --profile fast` is the
      release profile with an incremental cache: 15 to 45 s, and the
      profiled strokes paint within 3% of a release either way. Packages
      still use `--release`. Details and what else was tried are in
      [TESTING.md](TESTING.md).
- [ ] **The suite's 100 seconds** are six tests of compositing and display
      tiles on 2048 px canvases (20 to 35 s each alone, unoptimized). An
      optimized test build is not the answer (one edit then takes 4.5
      minutes to rebuild). Smaller canvases in those tests, where the size
      is not the point, would be.
- [x] **`Package.ps1`** ran end to end for the 0.5.0 candidate and release
      (2026-11-13), `FILES.json` step included: the list matches the
      package's 931 files exactly.
- [ ] **A complete self-update** has never run for real: 0.4.0 cannot update
      itself. 0.5.0 is the first release with `FILES.json`, so the first
      test is updating a published 0.5.0 to the release after it. Try it
      from an installed and from a portable copy.

## 6. Product gaps found on the way

Tracked in [ROADMAP.md](ROADMAP.md) where they are features; listed here so
they are not lost:

- An application icon: shortcuts and `.dudel` files show the generic one.
- Signed update records.
- In-place updates on Linux.
- Nobody has yet used the 0.4 painting features with a pen in a planned
  test; the wet brush report above is the first feedback from real use.
- Whether the zoom readout shows a wrong value ("5%") while the Brush Studio
  tab is in front, or only in screenshot mode. Not checked in a real session.
