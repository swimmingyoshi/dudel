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

Not yet investigated. What is known and what is only suspected:

- Known: a 200 px wet dab costs 3.7 ms (7.3 ms on a watercolor layer) against
  0.8 ms for plain paint, measured by `material::look::painting_cost`. A fast
  stroke with tight spacing can ask for more dabs per frame than that allows.
- Known: that test paints at full pressure with a fixed size. It does not
  cover pressure changing the size, which is the reported case.
- Suspected, in the order to check:
  1. Each wet dab copies and blurs the area under it (`blend_brush::capture`).
     The copy reads one pixel at a time through a closure, on one thread.
  2. The lag grows with stroke length, which points at something that grows
     with the stroke: the stroke buffer, the water field (`wet::Field`, a hash
     map of 32 px tiles read and written row by row per dab), or the display
     refresh of an ever larger dirty area.
  3. Paper adds `paper.hold` and `paper.pool` to every covered pixel.
  4. Drying is paused while a stroke is down, so it should not be the cause;
     confirm that the background drying step is really idle during a stroke.

### The measurement to run first

Do this before changing any code, so the fix aims at the real cost.

- [ ] **Stage timers behind a switch.** Add named timers around the stages
      of one frame of a stroke and write them to a log when Dudel is started
      with a flag (for example `--trace-stroke FILE`), off otherwise:
      input samples to dabs (`stroke.rs`), per dab: coverage, capture and
      blur, blending, relief, water read and write; then stroke buffer
      upkeep, display refresh (composite and texture upload), and the whole
      frame. One line per frame: frame time, dabs painted, dabs still queued,
      and milliseconds per stage.
- [ ] **A repeatable stroke.** Record the slow stroke once with the existing
      `--replay-input` support (or script one: 4000 px long, pressure rising
      and falling, 150 to 300 px wet brush, Cold-pressed paper, on a plain
      and on a watercolor layer) so before and after are the same input.
- [ ] **Read the log for two things:** which stage takes the most time per
      dab, and which stage's time grows as the stroke gets longer. The second
      answers "why long strokes".
- [ ] Check the queue: if "dabs still queued" climbs during the stroke, the
      engine is behind the pen; if it stays at zero and the frame time is
      high, the display is.

Existing tools that help: **View > Settings > Performance monitor** (frame
times in the app), `--benchmark` and its JSON report, the native session and
frame logs (`native_session.rs`, `native_frame_log.rs`), and the
`allocation-profile` feature.

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

- [ ] **Wet brush lag** (section 1).
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
