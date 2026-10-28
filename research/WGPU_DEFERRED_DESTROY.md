# wgpu deferred-destroy retention

2026-10-17. Resolves the retained Rust component of the repeated-use DX12 growth
in `DX12_ALLOCATION_OWNERSHIP.md`. Generated native-window evidence on the
RTX 3060; not physical input or release certification.

## Cause

`egui_wgpu::Renderer::free_texture` calls `Texture::destroy`. In wgpu-core
27.0.3, dropping the resulting `DestroyedTexture` pushes its texture views and
bind groups onto `Device::deferred_destroy` (`resource.rs`, `DestroyedTexture::drop`).
That list is drained only by `Device::deferred_resource_destruction`, which is
called only from `Global::poll_single_device` (`device/global.rs`), i.e.
`Device::poll`. Queue submission performs maintenance but does not drain it,
and egui-wgpu 0.33.3 never polls. Every freed display texture therefore kept
its deferred entries, the weak-reference allocations they hold, and the raw
D3D12 views/bind groups those entries were meant to destroy.

## Evidence

The stack trace (`allocation-stack-profile`, >=64 KiB events) of the twelve-round
DX12 `--soak-native-drawing-session` run at 1280x800/100% attributed the only
growing large group to `RawVec<[usize; 4]>::grow_one` inside
`DestroyedTexture::drop` <- `Texture::destroy` <- `Renderer::free_texture`.
The 32-bin live-size histogram placed the remaining growth in 32-512 byte
requests (+24 MiB over rounds 4-12), consistent with per-texture retained
entries rather than artwork or history.

## Fix

`Dudel::update` now calls `device.poll(PollType::Poll)` once per frame when a
wgpu render state exists. This is nonblocking: it does not wait for the GPU.
Deferred views/bind groups from one frame's freed textures are released at the
start of the next frame.

## Result

Same workload and profiled build configuration, before (commit `0cb7fa8`,
`target/qa/allocation-stack-0cb7fa8/dudel.exe`, SHA-256 `c2d279c1...365238`)
and after (`target/qa/allocation-stack-devicepoll/dudel.exe`, SHA-256
`b44dd2df...33c777`). Both exited zero with all strokes, mask edits and
save/reopens verified.

| Matching idle stage, rounds 4-12 | Rust live drift before / after, MiB | Process-private drift before / after, MiB |
| --- | ---: | ---: |
| Reset fixture | +26.07 / 0.00 | +76.41 / +31.86 |
| History released, before save | +30.07 / +0.01 | +86.81 / +30.74 |
| Reopened, history released | +30.07 / 0.00 | +77.36 / +44.98 |

Final reopened idle live Rust requests held exactly 64.6 MiB in all twelve
rounds after the fix (68.1 rising to 108.7 MiB before). No large trace group
changed between rounds 4 and 12. Final private commitment fell from 794.5 to
728.9 MiB and resident from 519.7 to 453.8 MiB.

Process-private commitment still rises about 3-5 MiB per round outside Rust
requests. That remainder is unassigned (driver, heap backing or other native
owners) and is the next memory study; this is not a memory-gate pass.

Summaries and inspected captures: `benchmarks/allocation-stack-dx12-sustained-*`
(before) and `benchmarks/allocation-stack-devicepoll-dx12-sustained-*` (after).
Raw reports and stack sidecars stay local (gitignored). Normal and all-feature
suites (760/766 passed, one high-memory test ignored), warnings-denied Clippy,
formatting and the optimized build pass.
