# Dudel for Windows

Dudel is a pen-focused illustration app. [The release decision](RELEASE_DECISION.md)
records Windows 0.1 acceptance, evidence and known limits. `BUILD.json` identifies
this package; public availability is a separate publishing action.

Extract the complete archive and run `dudel.exe`. Start with the
[quick start](QUICK_START.md) for drawing, layers, saving and PNG export.
The [workflow checklist](CANDIDATE_TEST_CHECKLIST.md) is available for targeted
troubleshooting; it is not a request to repeat completed testing.
The C runtime is included; a separate Visual C++ redistributable is not required.
Keep `dudel_thumbnails.dll` beside the executable for optional Explorer previews.
Before deleting the app folder, use View > Settings > File Explorer previews >
Remove .dudel previews if you enabled them. Projects and preferences are stored
separately; see the quick start.

- Save editable artwork as `.dudel`; export PNG for sharing.
- Read [format support](FORMAT_SUPPORT.md) before importing CSP files or opening
  projects with an older Dudel build.
- Read [release notes](RELEASE_NOTES.md) for current limits and validation gaps.
- Use Help for keyboard shortcuts and View -> Tablet input for input diagnostics.
- When reporting a problem, include the version/hash from `BUILD.json`, Windows,
  GPU/tablet driver, display scaling and steps on a disposable canvas.

The sustained study passed, and the owner confirms working Artist 12 use and
Explorer previews. Existing laptop installation/runtime results are retained.
Remaining engineering/distribution details are recorded in
[the checklist](RELEASE_CHECKLIST.md); no blanket repeat study is scheduled.

To run the optional Windows DX12 study when leaving the PC idle, save your work
and close Dudel first. Install Python 3.11+ with `python` in PATH, extract the
package to a writable local folder with at least 8 GiB free, then run there:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\RunPerformanceStudy.ps1
```

The script checks the frozen executable and supplied study tools, records
CPU/RAM/GPU/driver information, and runs three standard sessions followed by a
maximum-layer session lasting at least two hours. Allow four hours or more and
keep a laptop plugged in with its lid open; do not suspend the PC. Reports stay in `diagnostics/target/qa/study-*`.
For release performance evidence, leave the PC idle: movie playback, games,
compilation and other foreground work can share CPU/GPU/memory and affect results.
A concurrent-use run is exploratory; record the activity and repeat the acceptance
study while idle. The synthetic study does not replace physical tablet acceptance.
`-CheckOnly` validates prerequisites without starting; `-Mode Standard` or
`-Mode Maximum` runs one part. The controller stops on failure, preserves reports
and restores its temporary awake request. Keep results from different PCs separate.

Dudel is GPL-3.0-only. Official executable downloads may be sold; you can build the
matching source yourself. [SOURCE.md](SOURCE.md) identifies the corresponding source
archive. [Distribution policy](DISTRIBUTION.md), [LICENSE](LICENSE) and
[third-party notices](THIRD_PARTY_NOTICES.md) describe the supplied sources and terms.
