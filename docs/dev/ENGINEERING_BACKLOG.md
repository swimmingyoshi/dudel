# Engineering backlog

Work on the code, the build and performance that is agreed to be worth doing
but not started. Features and releases are in [ROADMAP.md](ROADMAP.md). Written
2026-11-12, after 0.4.0; numbers are from that day unless a line says otherwise.

Tick an item when it is done and say where (commit or file). Add what you
find; delete what turns out not to matter, with a line saying why.

## 1. Open report: the wet brush lags on long strokes

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
- [ ] A trace of the owner's own stroke (`DUDEL_TRACE_STROKE`) would show
      how fast the pen really moves, which decides whether the fixes
      applied are enough. If "frame to frame"
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

### What is left

- [ ] **Confirm on the owner's pen**, with Low latency on and off. A
      `DUDEL_TRACE_STROKE` run shows the real pen speed and what a real
      frame costs beyond the CPU work measured here.
- [ ] **Is 30 to 45 frames a second while far behind acceptable?** If the
      pen cursor feels worse than the shorter queue feels better, lower the
      20 ms or raise the 16 dabs.
- [ ] **Paint off the interface thread**, so painting does not compete with
      showing frames at all. Larger change; only if the above is not
      enough.
- [ ] **Paper height once per pixel.** `Paper::hold` and `Paper::pool` each
      work out `height`, which takes the sine and cosine of the paper
      direction and two noise lookups, for every pixel of every dab. Work
      it out once per pixel per dab first (same result, bit for bit), then
      consider keeping heights per tile. Two thirds of the round 300 px
      wash's coverage time is the paper (1.66 ms with, 0.54 ms without).
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

- [ ] **Wet brush lag** (section 1: fixed in the scripted run on
      2026-11-13; to confirm on the owner's pen).
- [ ] **Planning a drying step** takes 34 to 100 ms on a 2-megapixel wash. It
      runs in the background now, so the window stays smooth, but the wash
      dries slowly. `wet::plan` allocates five buffers per 32 px tile.
- [ ] **Thick paint relief** is a `layer_mask::Mask` written one pixel at a
      time (`material::touch_relief`). A store that hands out row bands, as
      `Pixels::band_tiles` does, would bring the palette knife near plain
      smudge.
- [ ] **Pigment mixing** is the remaining cost on watercolor and thick paint
      layers (`pigment::mix`: 16 bands per mix, behind a 4096-slot cache).
- [ ] **`blend_brush::capture`** now copies in row bands across cores but
      still asks for each pixel; read rows from the layer's tiles instead.
- [ ] **Affine relief follow** still scans the whole canvas.
- [ ] **A recorded baseline.** Turn `painting_cost` and `drying_cost` into a
      script that writes a JSON file under `benchmarks/`, so a regression
      shows up as a changed number in a diff.

## 3. Code structure

In the order they pay off:

- [ ] **Split the single crate into a workspace.** Dudel is one binary crate
      of about 168,000 lines, so any edit recompiles all of it: about a
      minute for `cargo check`, six minutes for a release test build, and an
      incremental cache that grows by gigabytes. A core crate (document,
      pixels, blending, materials, file formats) with no UI code, and the
      app on top. Do it in stages, starting with modules that already have
      no egui dependency: `pigment`, `sha256`, `paper`, `brush_edge`,
      `pixels`, `layer_mask`, the `clip_*` and `psd_*` readers. The engine
      then also builds and benchmarks without a window.
- [ ] **Break up `Document::paint_dab_rows`** (`document.rs`). One function
      holds five painting paths (mask stripes, blend modes row by row, blend
      modes in bands, plain paint in bands, plain paint row by row) plus the
      coverage closure. Separate the coverage, and one function per path.
- [ ] **Screenshot modes in `main.rs`.** About 60 `--capture-*` flags are
      separate booleans in one 1,500-line `run()`. A table of name and
      fixture function makes adding one a single line.
- [ ] **The `Dudel` app struct** has several hundred fields. Group them by
      subsystem, as `setup` and `drying` are, so per-project state cannot be
      missed when tabs switch (`project_session!` in `tabs.rs`).
- [ ] **Line endings.** Sources are a mix of LF and CRLF, which makes
      scripted edits fragile. Settle it with `.gitattributes`.

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
- [ ] **Smaller debug symbols:** in `Cargo.toml`, line tables only for the
      dev profile and no symbols for dependencies. Roughly halves
      `debug/deps` and shortens linking. `Cargo.toml` is a recorded build
      input, so do it between releases.
- [ ] **`scripts/Clean.ps1`** that removes the incremental cache, stale
      verification folders and extracted candidate folders; run it after
      each release.
- [ ] The workspace split (section 3) shrinks the incremental cache at its
      source.

## 5. Development process

- [ ] **CI minutes:** find out whether the hosted runs cache dependencies
      and what that would save; today each run appears to build from
      scratch.
- [ ] **`Package.ps1`:** the `FILES.json` step was fixed after its only full
      run failed (a bad `Replace` call). The fix was run by hand on that
      candidate; the script has not been run end to end since. Do that
      before the next release.
- [ ] **A complete self-update** has never run for real: 0.4.0 cannot update
      itself, so the first test is possible once a release with `FILES.json`
      is published. Try it from an installed and from a portable copy.

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
