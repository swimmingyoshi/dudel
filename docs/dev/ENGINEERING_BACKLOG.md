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

Measured the same day with the tools below. No engine code was changed. The
cause is narrowed down but not confirmed on the owner's own strokes: the next
step is a trace from a real pen session.

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

### What is not known yet

The scripted run leaves out the GPU, the pen driver and the side panels, and
it guesses the brush size, canvas size and pen speed. In a real session each
frame is longer, so painting gets fewer turns a second and falls behind
sooner.

- [x] Stage timers behind a switch.
- [x] A repeatable stroke (scripted; `--replay-input` was not needed).
- [x] Which stage costs most, and which grows with the stroke (none does).
- [ ] **A trace of the owner's own slow stroke** (`DUDEL_TRACE_STROKE`),
      with the brush, its size and the canvas size noted. If "frame to
      frame" is well above "CPU per frame", the display or GPU is holding
      frames back; if "rest of the interface" is large, the side panels are.

### Fixes, in the order they should pay off (none started)

- [ ] **Paper height once per pixel.** `Paper::hold` and `Paper::pool` each
      work out `height`, which takes the sine and cosine of the paper
      direction and two noise lookups, for every pixel of every dab.
      Overlapping dabs repeat the same pixels about twelve times. Work it out
      once per pixel per dab first (same result, bit for bit), then consider
      keeping heights per tile.
- [ ] **Let painting use the frame's spare time.** The 8 ms limit leaves
      half of a 60 Hz frame unused while dabs wait. Raise it while trailing,
      or move painting off the frame.
- [ ] **The copy under the dab** (`blend_brush::capture`) reads pixel by
      pixel through a closure on one thread: read rows from the layer's
      tiles, in bands.
- [ ] **Blur less.** Each dab blurs its whole area twice although nearly
      all of it was blurred for the previous dab.
- [ ] **Small wet dabs:** make the pixel-by-pixel path cheaper (coverage a
      row at a time, writes a tile span at a time) or paint several small
      dabs across cores.
- [ ] **Paper view tiles on a background worker.**

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

- [ ] **Wet brush lag** (section 1, which has the measured costs per stage
      and the fixes in order).
- [ ] **Planning a drying step** takes 34 to 100 ms on a 2-megapixel wash. It
      runs in the background now, so the window stays smooth, but the wash
      dries slowly. `wet::plan` allocates five buffers per 32 px tile.
- [ ] **Thick paint relief** is a `layer_mask::Mask` written one pixel at a
      time (`material::touch_relief`). A store that hands out row bands, as
      `Pixels::band_tiles` does, would bring the palette knife near plain
      smudge.
- [ ] **Pigment mixing** is the remaining cost on watercolor and thick paint
      layers (`pigment::mix`: 16 bands per mix, behind a 4096-slot cache).
- [ ] **`blend_brush::capture`** reads its area pixel by pixel; read rows
      from the layer's tiles instead.
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
