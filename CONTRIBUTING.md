# Contributing to Dudel

Thanks for helping. Bug reports, testing on other tablets and code are all
welcome.

## Reporting bugs and testing

- Use the issue templates: **bug report** for something broken, **tester
  feedback** for the [30-minute test](docs/TESTER_GUIDE.md).
- In the app, **Help > Report a problem…** opens the same form. If Dudel closed
  unexpectedly, attach the newest file from **Help > Problem reports folder**.
- Say which tablet, driver and Windows version you used. Testing on hardware
  other than the XP-Pen Artist 12 is especially useful.

## Building

Install [Rust](https://rustup.rs) (the toolchain is pinned in
`rust-toolchain.toml`) and, on Windows, the Visual Studio C++ build tools with
the Windows SDK. Then:

```powershell
cargo build --locked --release --bin dudel
.\target\release\dudel.exe
```

While you work on a change, `cargo run --profile fast` is the quicker way to
try it: the same optimization, but a rebuild after an edit takes 15 to 45
seconds, not three minutes.

On Linux, the system packages are the ones `.github/workflows/linux.yml` installs;
see also the [Linux port notes](docs/dev/LINUX_PORT.md).

## Before you open a pull request

```powershell
.\scripts\Verify.ps1          # formatting, tests, Clippy and the Python checks
```

On Linux run `bash scripts/verify.sh quick`. [Testing](docs/dev/TESTING.md)
explains quick and full verification and test filters.

- Keep changes focused, and add a test for a new behavior or a fixed bug.
- Format with `cargo fmt`; the pre-push hook refuses unformatted code.
- Never commit personal paths or private notes; `scripts/check_public_tree.py`
  checks for them.
- Documentation-only commits may use `[skip ci]`.

## Where things are

| Path | Contents |
|---|---|
| `src/` | The application (Rust, egui/eframe) |
| `shell/thumbnails/` | The File Explorer thumbnail DLL |
| `plugins/`, `examples/` | Plugin SDKs and example packages |
| `scripts/` | Verification, packaging and release tooling |
| `research/` | Diagnostic study tools and test fixtures |
| `docs/` | User guides; `docs/dev/` for developers, `docs/release/` for release records |

The plan for upcoming work is in the [roadmap](docs/dev/ROADMAP.md), and the
release process in [releasing](docs/dev/RELEASING.md).

## License

Dudel is GPL-3.0-only. By contributing you agree that your contribution is
licensed under the same terms.
