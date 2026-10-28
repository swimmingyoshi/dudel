//! Diagnostic: count native default-heap blocks of one size before and after
//! repeating a single wgpu operation on a headless DX12 device. Identifies which
//! operation leaves driver-owned blocks behind.
//!
//! cargo run --release --features allocation-profile --example dx12_upload_probe -- [SIZE]
#[cfg(windows)]
use eframe::wgpu;
#[cfg(windows)]
use std::future::Future;

#[cfg(windows)]
fn block_on<F: Future>(future: F) -> F::Output {
    let waker = std::task::Waker::noop();
    let mut context = std::task::Context::from_waker(waker);
    let mut future = std::pin::pin!(future);
    loop {
        if let std::task::Poll::Ready(output) = future.as_mut().poll(&mut context) {
            return output;
        }
        std::thread::yield_now();
    }
}

#[cfg(windows)]
fn count_blocks(size: usize) -> usize {
    use windows_sys::Win32::System::Memory::{
        GetProcessHeap, HeapLock, HeapUnlock, HeapWalk, PROCESS_HEAP_ENTRY,
    };
    let heap = unsafe { GetProcessHeap() };
    let mut count = 0;
    unsafe { HeapLock(heap) };
    let mut entry: PROCESS_HEAP_ENTRY = unsafe { std::mem::zeroed() };
    while unsafe { HeapWalk(heap, &mut entry) } != 0 {
        if entry.wFlags & 4 != 0 && entry.cbData as usize == size {
            count += 1;
        }
    }
    unsafe { HeapUnlock(heap) };
    count
}

#[cfg(windows)]
type Case<'a> = (&'static str, Box<dyn Fn() + 'a>);

#[cfg(windows)]
const SIDE: u32 = 320;
#[cfg(windows)]
const REPEATS: usize = 200;

#[cfg(windows)]
fn settle(device: &wgpu::Device) {
    let _ = device.poll(wgpu::PollType::wait_indefinitely());
}

#[cfg(not(windows))]
fn main() {
    eprintln!("dx12_upload_probe requires Windows (DX12 and the native Windows heap).");
    std::process::exit(1);
}

#[cfg(windows)]
fn main() {
    let size: usize = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(184);
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
        backends: wgpu::Backends::DX12,
        ..Default::default()
    });
    let adapter = block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
        .expect("DX12 adapter");
    println!("Adapter: {:?}", adapter.get_info().name);
    let (device, queue) =
        block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).expect("device");
    let extent = wgpu::Extent3d {
        width: SIDE,
        height: SIDE,
        depth_or_array_layers: 1,
    };
    let texture_descriptor = wgpu::TextureDescriptor {
        label: None,
        size: extent,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    };
    let target = device.create_texture(&texture_descriptor);
    let pixels = vec![127u8; (SIDE * SIDE * 4) as usize];
    let write = |submit_each: bool| {
        for _ in 0..REPEATS {
            queue.write_texture(
                target.as_image_copy(),
                &pixels,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(SIDE * 4),
                    rows_per_image: Some(SIDE),
                },
                extent,
            );
            if submit_each {
                queue.submit([]);
                let _ = device.poll(wgpu::PollType::Poll);
            }
        }
        queue.submit([]);
    };
    let cases: Vec<Case> = vec![
        (
            "empty submit",
            Box::new(|| {
                for _ in 0..REPEATS {
                    queue.submit([]);
                    let _ = device.poll(wgpu::PollType::Poll);
                }
            }),
        ),
        ("write_texture, submit each", Box::new(|| write(true))),
        ("write_texture, one submit", Box::new(|| write(false))),
        (
            "write_buffer 400 KiB, submit each",
            Box::new(|| {
                let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                    label: None,
                    size: pixels.len() as u64,
                    usage: wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                for _ in 0..REPEATS {
                    queue.write_buffer(&buffer, 0, &pixels);
                    queue.submit([]);
                    let _ = device.poll(wgpu::PollType::Poll);
                }
            }),
        ),
        (
            "create/destroy MAP_WRITE buffer",
            Box::new(|| {
                for _ in 0..REPEATS {
                    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                        label: None,
                        size: pixels.len() as u64,
                        usage: wgpu::BufferUsages::MAP_WRITE | wgpu::BufferUsages::COPY_SRC,
                        mapped_at_creation: true,
                    });
                    buffer.destroy();
                }
            }),
        ),
        (
            "64 writes + GPU copies per submit",
            Box::new(|| {
                let tiles: Vec<_> = (0..64)
                    .map(|_| {
                        device.create_texture(&wgpu::TextureDescriptor {
                            usage: texture_descriptor.usage | wgpu::TextureUsages::COPY_SRC,
                            ..texture_descriptor.clone()
                        })
                    })
                    .collect();
                let sink = device.create_texture(&texture_descriptor);
                for _ in 0..REPEATS / 20 {
                    let mut encoder = device.create_command_encoder(&Default::default());
                    for tile in &tiles {
                        queue.write_texture(
                            tile.as_image_copy(),
                            &pixels,
                            wgpu::TexelCopyBufferLayout {
                                offset: 0,
                                bytes_per_row: Some(SIDE * 4),
                                rows_per_image: Some(SIDE),
                            },
                            extent,
                        );
                        encoder.copy_texture_to_texture(
                            tile.as_image_copy(),
                            sink.as_image_copy(),
                            extent,
                        );
                    }
                    queue.submit([encoder.finish()]);
                    let _ = device.poll(wgpu::PollType::Poll);
                }
            }),
        ),
        (
            "spawn/join short-lived thread",
            Box::new(|| {
                for _ in 0..REPEATS {
                    std::thread::spawn(|| std::hint::black_box(1))
                        .join()
                        .unwrap();
                }
            }),
        ),
        (
            "create/destroy texture",
            Box::new(|| {
                for _ in 0..REPEATS {
                    device.create_texture(&texture_descriptor).destroy();
                    queue.submit([]);
                    let _ = device.poll(wgpu::PollType::Poll);
                }
            }),
        ),
    ];
    for (name, case) in &cases {
        settle(&device);
        let before = count_blocks(size);
        case();
        settle(&device);
        let after = count_blocks(size);
        println!(
            "{name:36} {REPEATS} repeats: {size}-byte blocks {before} -> {after} ({:+}, {:.2} per repeat)",
            after as isize - before as isize,
            (after as f64 - before as f64) / REPEATS as f64
        );
    }
}
