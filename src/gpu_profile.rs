//! Optional diagnostic summary of wgpu's supported GPU suballocator report.
//! These counts are not total VRAM, Windows commitment, or driver allocation data.
#[derive(serde::Serialize)]
pub(crate) struct Snapshot {
    allocated_bytes: u64,
    reserved_bytes: u64,
    allocation_count: usize,
    block_count: usize,
}

pub(crate) fn snapshot(device: &eframe::wgpu::Device) -> Option<Snapshot> {
    let report = device.generate_allocator_report()?;
    Some(Snapshot {
        allocated_bytes: report.total_allocated_bytes,
        reserved_bytes: report.total_reserved_bytes,
        allocation_count: report.allocations.len(),
        block_count: report.blocks.len(),
    })
    // The report's vectors/names drop before the caller samples Rust requests.
}
