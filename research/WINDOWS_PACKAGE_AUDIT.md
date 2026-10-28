# Windows executable audit, 2026-10-17

The current frozen engineering executable is
`5B97376259EFB878ED082EFB796E2F30B8C098238D91B516E62CD5F33C1E1D22`.
Its source inputs are recorded in `benchmarks/job-peak-build.json`; this is newer
than the currently packaged GPL candidate.

The release executable now uses the Windows GUI subsystem, avoiding an extra
console window during ordinary launch. Debug builds and test runners retain their
console. The release was compiled successfully in an isolated Cargo target because
the user's running application holds the original release executable open. The
running application was left undisturbed.

A native DX12 one-round/four-layer smoke run passed, including save/open, mask and
pixel checks and a visually inspected capture. Redirected diagnostics remain
available on the GUI executable. This smoke test is not the 20-layer or maximum
32-layer acceptance workload. State: `target/qa/job-peak-build-20261017/SMOKE_STATE.json`.

## Runtime dependency

Direct PE imports include `VCRUNTIME140.dll`. The reference PC already has the
Microsoft Visual C++ runtime installed, so successful launch here does not establish
clean-machine readiness. The quick start and release notes now link the official
[x64 redistributable](https://aka.ms/vc14/vc_redist.x64.exe), described by
[Microsoft's supported runtime documentation](https://learn.microsoft.com/cpp/windows/latest-supported-vc-redist).
No runtime installer was downloaded or run during this audit.

`benchmarks/windows-package-audit.json` records the PE subsystem, executable hash
and direct import names. This inventory does not include dynamically loaded DLLs
or establish graphics-driver availability. Clean non-developer Windows startup,
prerequisite handling, installation and upgrade remain G4 acceptance work. Windows
Sandbox was unavailable on this machine; no clean environment has been claimed.
