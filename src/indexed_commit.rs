//! Experimental commit primitive, used only by a synthetic storage study.
//! Not a project format or an editor save path. Records are opaque byte payloads;
//! the independent tile codec is used only by the separate tile study.
use std::{
    collections::BTreeMap,
    fs::{File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};
#[path = "indexed_lifecycle.rs"]
mod lifecycle;
#[path = "indexed_session.rs"]
mod session;
pub(crate) use session::{Session, load_document};
pub(crate) fn benchmark_lifecycle(output: Option<&Path>) -> Result<()> {
    lifecycle::run(output)
}

// Private container v2 requires the pathname-lease writer protocol. Older
// experimental writers reject it instead of bypassing compaction coordination.
const MAGIC: &[u8; 8] = b"DIDX\x02\r\n\0";
const ROOT_MAGIC: &[u8; 8] = b"DIXROOT1";
// Keep root updates on separate 4 KiB pages, away from payload appends.
// Actual device atomicity/flush behavior still requires platform experiments.
const ROOT_OFFSETS: [u64; 2] = [4096, 8192];
const HEADER: u64 = 12288;
const MAX_RECORD: usize = 1024 * 1024;
const MAX_KEYS: usize = 8192;
const MAX_LIVE: u64 = 600 * 1024 * 1024;
const MAX_FILE: u64 = 2 * 1024 * 1024 * 1024;
const MAX_INDEX: usize = 4 + MAX_KEYS * 24;
const MAX_PROOF_DESCRIPTORS: usize = 2 * 1024 * 1024;
type Result<T> = std::result::Result<T, String>;

fn io<T>(value: std::io::Result<T>) -> Result<T> {
    value.map_err(|e| e.to_string())
}
fn crc(bytes: &[u8]) -> u32 {
    let mut crc = flate2::Crc::new();
    crc.update(bytes);
    crc.sum()
}
fn u64_at(bytes: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap())
}
fn u32_at(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap())
}

#[derive(Clone, Debug, PartialEq)]
struct Record {
    offset: u64,
    length: u32,
    checksum: u32,
}
#[derive(Clone)]
struct Index {
    generation: u64,
    end: u64,
    entries: BTreeMap<u64, Record>,
    damaged_slot: bool,
}

fn root_crc(root: &[u8; 64]) -> u32 {
    let mut crc = flate2::Crc::new();
    crc.update(&root[..44]);
    crc.update(&root[48..]);
    crc.sum()
}
fn encode_index(entries: &BTreeMap<u64, Record>) -> Vec<u8> {
    let mut bytes = (entries.len() as u32).to_le_bytes().to_vec();
    for (key, record) in entries {
        bytes.extend(key.to_le_bytes());
        bytes.extend(record.offset.to_le_bytes());
        bytes.extend(record.length.to_le_bytes());
        bytes.extend(record.checksum.to_le_bytes());
    }
    bytes
}
fn parse_root(file: &mut File, root: &[u8; 64], file_length: u64) -> Result<Index> {
    if &root[..8] != ROOT_MAGIC
        || root[48..].iter().any(|b| *b != 0)
        || u32_at(root, 44) != root_crc(root)
    {
        return Err("Invalid indexed commit pointer".into());
    }
    let generation = u64_at(root, 8);
    let offset = u64_at(root, 16);
    let length = u64_at(root, 24);
    let end = u64_at(root, 32);
    if generation == 0
        || offset < HEADER
        || !(4..=MAX_INDEX as u64).contains(&length)
        || offset.checked_add(length) != Some(end)
        || end > file_length
    {
        return Err("Indexed commit pointer exceeds bounds".into());
    }
    io(file.seek(SeekFrom::Start(offset)))?;
    let mut bytes = vec![0; length as usize];
    io(file.read_exact(&mut bytes))?;
    if crc(&bytes) != u32_at(root, 40) {
        return Err("Damaged indexed manifest checksum".into());
    }
    let count = u32_at(&bytes, 0) as usize;
    if count > MAX_KEYS || bytes.len() != 4 + count * 24 {
        return Err("Invalid indexed manifest length".into());
    }
    let mut entries = BTreeMap::new();
    let mut previous = None;
    let mut live = 0u64;
    for row in bytes[4..].chunks_exact(24) {
        let key = u64_at(row, 0);
        let record = Record {
            offset: u64_at(row, 8),
            length: u32_at(row, 16),
            checksum: u32_at(row, 20),
        };
        if previous.is_some_and(|old| key <= old)
            || record.offset < HEADER
            || record.length == 0
            || record.length as usize > MAX_RECORD
            || record
                .offset
                .checked_add(record.length as u64)
                .is_none_or(|end| end > offset)
        {
            return Err("Invalid indexed record reference".into());
        }
        previous = Some(key);
        live += record.length as u64;
        if live > MAX_LIVE {
            return Err("Indexed live payload exceeds budget".into());
        }
        entries.insert(key, record);
    }
    Ok(Index {
        generation,
        end,
        entries,
        damaged_slot: false,
    })
}
fn read_index(file: &mut File) -> Result<Index> {
    let length = io(file.metadata())?.len();
    if !(HEADER..=MAX_FILE).contains(&length) {
        return Err("Indexed file size exceeds bounds".into());
    }
    io(file.seek(SeekFrom::Start(0)))?;
    let mut header = [0; HEADER as usize];
    io(file.read_exact(&mut header))?;
    if &header[..8] != MAGIC
        || header[8..4096].iter().any(|b| *b != 0)
        || header[4160..8192].iter().any(|b| *b != 0)
        || header[8256..].iter().any(|b| *b != 0)
    {
        return Err("Not an experimental indexed journal".into());
    }
    let mut valid = Vec::new();
    let mut damaged = false;
    for (slot_index, offset) in ROOT_OFFSETS.into_iter().enumerate() {
        let slot = &header[offset as usize..offset as usize + 64];
        if slot.iter().all(|b| *b == 0) {
            continue;
        }
        match parse_root(file, slot.try_into().unwrap(), length) {
            Ok(root) if root.generation % 2 == slot_index as u64 => valid.push(root),
            Ok(_) => damaged = true,
            Err(_) => damaged = true,
        }
    }
    valid.sort_by_key(|root| root.generation);
    if valid.len() == 2 && valid[0].generation.checked_add(1) != Some(valid[1].generation) {
        return Err("Indexed generation chain is inconsistent".into());
    }
    let mut root = valid.pop().ok_or("No confirmed indexed generation")?;
    root.damaged_slot = damaged;
    Ok(root)
}

struct RasterProof {
    record: Record,
    canvas: [usize; 2],
    region: [usize; 4],
    stamp: crate::pixels::RegionStamp,
}
#[derive(Default)]
struct RasterProofs {
    generation: u64,
    entries: Vec<(u64, RasterProof)>,
}
impl RasterProofs {
    fn descriptor_bytes(&self) -> usize {
        self.entries.capacity() * std::mem::size_of::<(u64, RasterProof)>()
            + self
                .entries
                .iter()
                .map(|(_, proof)| proof.stamp.heap_bytes())
                .sum::<usize>()
    }
}
struct Journal {
    file: File,
    // Declaration order closes data before deleting a failed stage and releases
    // the stable pathname lease last. Lease files are intentionally persistent.
    failed_stage: Option<tempfile::TempPath>,
    path_lease: Option<File>,
    index: Index,
    writable: bool,
    raster_proofs: RasterProofs,
}
fn lease_path(path: &Path) -> Result<PathBuf> {
    let resolved = if path.exists() {
        io(std::fs::canonicalize(path))?
    } else {
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        io(std::fs::canonicalize(parent))?.join(path.file_name().ok_or("Missing journal filename")?)
    };
    let mut name = std::ffi::OsString::from(".");
    name.push(resolved.file_name().ok_or("Missing journal filename")?);
    name.push(".didx-lock");
    Ok(resolved.with_file_name(name))
}
fn acquire_lease(path: &Path, writable: bool) -> Result<Option<File>> {
    let path = lease_path(path)?;
    if let Ok(metadata) = std::fs::symlink_metadata(&path)
        && (!metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() != 0)
    {
        return Err("Indexed pathname lease is not a regular empty lock file".into());
    }
    let mut options = OpenOptions::new();
    options
        .read(true)
        .write(writable)
        .create(writable)
        .truncate(false);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        use windows_sys::Win32::Storage::FileSystem::{FILE_SHARE_READ, FILE_SHARE_WRITE};
        options.share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE);
    }
    let lease = match options.open(&path) {
        Ok(file) => file,
        Err(error) if !writable && error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.to_string()),
    };
    if io(lease.metadata())?.len() != 0 {
        return Err("Indexed pathname lease contains foreign data".into());
    }
    if writable {
        lease.try_lock()
    } else {
        lease.try_lock_shared()
    }
    .map_err(|e| e.to_string())?;
    Ok(Some(lease))
}
#[derive(Clone, Copy, PartialEq, Debug)]
enum Phase {
    PayloadWritten,
    PayloadSynced,
    RootWritten,
    RootSynced,
}
#[derive(Default, serde::Serialize)]
struct StreamStats {
    objects: usize,
    changed: usize,
    reused: usize,
    removed: usize,
    sink_owned_record_bytes_peak: usize,
    raster_records_encoded: usize,
    raster_records_reused_without_encoding: usize,
    raster_records_reused_without_read: usize,
    raster_records_read_for_comparison: usize,
    raster_proof_descriptor_bytes: usize,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct FileIdentity {
    volume: u64,
    index: u64,
    links: u64,
}
#[cfg(windows)]
fn identity(file: &File) -> Result<FileIdentity> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        BY_HANDLE_FILE_INFORMATION, GetFileInformationByHandle,
    };
    let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
    // Borrowed live File handle and a correctly sized initialized output struct.
    if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) } == 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    let id = FileIdentity {
        volume: info.dwVolumeSerialNumber as u64,
        index: ((info.nFileIndexHigh as u64) << 32) | info.nFileIndexLow as u64,
        links: info.nNumberOfLinks as u64,
    };
    if id.volume == 0 || id.index == 0 {
        return Err("Filesystem does not expose a usable compaction identity".into());
    }
    Ok(id)
}
#[cfg(unix)]
fn identity(file: &File) -> Result<FileIdentity> {
    use std::os::unix::fs::MetadataExt;
    let m = io(file.metadata())?;
    Ok(FileIdentity {
        volume: m.dev(),
        index: m.ino(),
        links: m.nlink(),
    })
}
#[cfg(not(any(windows, unix)))]
fn identity(_: &File) -> Result<FileIdentity> {
    Err("Compaction file identity unsupported on this platform".into())
}
fn same_destination(path: &Path, expected: FileIdentity) -> Result<()> {
    let metadata = io(std::fs::symlink_metadata(path))?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err("Compaction requires a regular file path".into());
    }
    let current = io(File::open(path))?;
    let found = identity(&current)?;
    if found != expected || found.links != 1 {
        return Err("Compaction destination changed or has multiple hard links".into());
    }
    Ok(())
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CompactPhase {
    RecordCopied,
    StagedSynced,
    Verified,
    BeforeReplace,
    Replaced,
    FinalSynced,
}
#[derive(serde::Serialize)]
struct CompactStats {
    before_bytes: u64,
    after_bytes: u64,
    records: usize,
    old_generation: u64,
    new_generation: u64,
}
impl Journal {
    fn validate_document(&mut self, cancel: &impl Fn() -> bool) -> Result<()> {
        let objects = self.index.entries.keys().copied().collect::<Vec<_>>();
        crate::indexed_document::validate_records(
            &objects,
            &mut |key| self.read(key)?.ok_or("Missing compaction object".into()),
            cancel,
        )
    }
    /// Same-directory, verified replacement of one confirmed document generation.
    /// A stable pathname lease guards the old-handle close required by Windows;
    /// the staged/new data handle stays locked across rename.
    fn compact_document(
        &mut self,
        path: &Path,
        cancel: &impl Fn() -> bool,
        mut checkpoint: impl FnMut(CompactPhase) -> Result<()>,
    ) -> Result<CompactStats> {
        if !self.writable
            || self.path_lease.is_none()
            || self.index.damaged_slot
            || self.index.generation == 0
        {
            return Err("Compaction requires a confirmed writable generation".into());
        }
        let old_generation = self.index.generation;
        let before = io(self.file.metadata())?.len();
        self.writable = false;
        self.raster_proofs = RasterProofs::default();
        let result = (|| {
            if cancel() {
                return Err("Compaction canceled".into());
            }
            let expected = identity(&self.file)?;
            same_destination(path, expected)?;
            let destination = io(std::fs::canonicalize(path))?;
            let parent = destination.parent().ok_or("Missing compaction parent")?;
            self.validate_document(cancel)?;
            let temporary = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
            let (file, temporary_path) = temporary.into_parts();
            let mut staged = Self::initialize(file)?;
            staged.index.generation = old_generation;
            let keys = self.index.entries.keys().copied().collect::<Vec<_>>();
            staged.replace_stream(
                |emit| {
                    for key in &keys {
                        if cancel() {
                            return Err("Compaction canceled during copy".into());
                        }
                        emit(
                            *key,
                            self.read(*key)?.ok_or("Missing source compaction record")?,
                        )?;
                        checkpoint(CompactPhase::RecordCopied)?;
                    }
                    Ok(())
                },
                |_| Ok(()),
            )?;
            checkpoint(CompactPhase::StagedSynced)?;
            let on_disk = read_index(&mut staged.file)?;
            if on_disk.generation != staged.index.generation
                || on_disk.entries != staged.index.entries
                || on_disk.damaged_slot
            {
                return Err("Compacted index verification differs".into());
            }
            for key in &keys {
                if cancel() {
                    return Err("Compaction canceled during verification".into());
                }
                let a = self
                    .read(*key)?
                    .ok_or("Missing source verification record")?;
                let b = staged
                    .read(*key)?
                    .ok_or("Missing compacted verification record")?;
                if a != b {
                    return Err("Compacted record differs from source bytes".into());
                }
            }
            staged.validate_document(cancel)?;
            io(staged
                .file
                .set_permissions(io(self.file.metadata())?.permissions()))?;
            io(staged.file.sync_all())?;
            checkpoint(CompactPhase::Verified)?;
            if cancel() {
                return Err("Compaction canceled before replacement".into());
            }
            checkpoint(CompactPhase::BeforeReplace)?;
            if cancel() {
                return Err("Compaction canceled before replacement".into());
            }
            same_destination(&destination, expected)?;
            // Windows requires the old destination handle to close. The stable
            // pathname lease remains exclusive throughout this handoff; the
            // staged/replacement data file retains its own lock as well.
            self.file = staged.file;
            self.index = staged.index;
            if let Err(error) = temporary_path.persist(&destination) {
                self.failed_stage = Some(error.path);
                return Err(error.error.to_string());
            }
            checkpoint(CompactPhase::Replaced)?;
            io(self.file.sync_all())?;
            checkpoint(CompactPhase::FinalSynced)?;
            Ok(CompactStats {
                before_bytes: before,
                after_bytes: self.index.end,
                records: keys.len(),
                old_generation,
                new_generation: self.index.generation,
            })
        })();
        if result.is_ok() {
            self.writable = true;
        }
        result
    }
    fn replace_document(
        &mut self,
        doc: &crate::document::Document,
        keys: &crate::indexed_document::Keys,
        cancel: &impl Fn() -> bool,
        mut checkpoint: impl FnMut(Phase) -> Result<()>,
    ) -> Result<(crate::indexed_document::Keys, StreamStats)> {
        self.replace_objects(
            |emit| {
                crate::indexed_document::visit_objects(doc, keys, cancel, &mut |key, bytes| {
                    emit(key, bytes)
                })
            },
            |phase| {
                if matches!(phase, Phase::PayloadWritten | Phase::PayloadSynced) && cancel() {
                    return Err("Indexed document canceled before commit publication".into());
                }
                checkpoint(phase)
            },
        )
    }
    fn publish(
        &mut self,
        generation: u64,
        position: u64,
        manifest: &[u8],
        checkpoint: &mut impl FnMut(Phase) -> Result<()>,
    ) -> Result<u64> {
        let end = position
            .checked_add(manifest.len() as u64)
            .ok_or("Indexed length overflow")?;
        if end > MAX_FILE {
            return Err("Indexed journal requires compaction".into());
        }
        // Reuse comparison reads can leave the file cursor in an older record.
        io(self.file.seek(SeekFrom::Start(position)))?;
        io(self.file.write_all(manifest))?;
        checkpoint(Phase::PayloadWritten)?;
        io(self.file.sync_all())?;
        checkpoint(Phase::PayloadSynced)?;
        let mut root = [0; 64];
        root[..8].copy_from_slice(ROOT_MAGIC);
        root[8..16].copy_from_slice(&generation.to_le_bytes());
        root[16..24].copy_from_slice(&position.to_le_bytes());
        root[24..32].copy_from_slice(&(manifest.len() as u64).to_le_bytes());
        root[32..40].copy_from_slice(&end.to_le_bytes());
        root[40..44].copy_from_slice(&crc(manifest).to_le_bytes());
        let checksum = root_crc(&root);
        root[44..48].copy_from_slice(&checksum.to_le_bytes());
        io(self
            .file
            .seek(SeekFrom::Start(ROOT_OFFSETS[(generation % 2) as usize])))?;
        io(self.file.write_all(&root))?;
        checkpoint(Phase::RootWritten)?;
        io(self.file.sync_all())?;
        checkpoint(Phase::RootSynced)?;
        Ok(end)
    }
    /// Full replacement inventory emitted one object at a time. Missing keys are
    /// deleted. Only bounded record references and one comparison record survive
    /// between emissions; exact bytes, not CRC alone, decide reuse.
    fn replace_stream<T>(
        &mut self,
        produce: impl FnOnce(&mut dyn FnMut(u64, Vec<u8>) -> Result<()>) -> Result<T>,
        checkpoint: impl FnMut(Phase) -> Result<()>,
    ) -> Result<(T, StreamStats)> {
        self.replace_objects(
            |emit| {
                produce(&mut |key, bytes| {
                    emit(key, crate::indexed_document::Object::Encoded(bytes))
                })
            },
            checkpoint,
        )
    }
    fn replace_objects<T>(
        &mut self,
        produce: impl FnOnce(
            &mut dyn FnMut(u64, crate::indexed_document::Object) -> Result<()>,
        ) -> Result<T>,
        mut checkpoint: impl FnMut(Phase) -> Result<()>,
    ) -> Result<(T, StreamStats)> {
        if !self.writable || self.index.damaged_slot {
            return Err("Indexed journal requires explicit recovery before writing".into());
        }
        let generation = self
            .index
            .generation
            .checked_add(1)
            .ok_or("Indexed generation overflow")?;
        let mut entries = BTreeMap::new();
        let mut seen = std::collections::BTreeSet::new();
        let mut position = self.index.end;
        let mut live = 0u64;
        let mut stats = StreamStats::default();
        let previous_proofs = std::mem::take(&mut self.raster_proofs);
        let mut next_proofs = RasterProofs {
            generation,
            entries: Vec::new(),
        };
        // Poison before callbacks/I/O as unwinding may bypass Result handling.
        self.writable = false;
        let result = (|| {
            io(self.file.set_len(self.index.end))?;
            let value = produce(&mut |key, data| {
                if !seen.insert(key) || seen.len() > MAX_KEYS {
                    return Err("Duplicate or excessive streamed object key".into());
                }
                let (data, source_stamp) = match data {
                    crate::indexed_document::Object::Source {
                        pixels,
                        canvas,
                        region,
                    } => {
                        let [width, height] = canvas;
                        if width == 0
                            || height == 0
                            || width > 4096
                            || height > 4096
                            || width
                                .checked_mul(height)
                                .is_none_or(|n| n > crate::document::MAX_PIXELS)
                            || !pixels.has_geometry(width, height)
                            || region[0] >= region[2]
                            || region[1] >= region[3]
                            || region[2] > width
                            || region[3] > height
                            || region[0] % 256 != 0
                            || region[1] % 256 != 0
                            || region[2] != (region[0] + 256).min(width)
                            || region[3] != (region[1] + 256).min(height)
                            || key >> 62 != 1
                            || (key & 0xffff) as usize
                                != region[1] / 256 * width.div_ceil(256) + region[0] / 256
                        {
                            return Err("Invalid indexed source tile geometry".into());
                        }
                        let stamp = pixels.region_stamp(region);
                        if previous_proofs.generation == self.index.generation
                            && let Ok(at) = previous_proofs
                                .entries
                                .binary_search_by_key(&key, |(key, _)| *key)
                        {
                            let proof = &previous_proofs.entries[at].1;
                            if proof.canvas == canvas
                                && proof.region == region
                                && self.index.entries.get(&key) == Some(&proof.record)
                                && proof.stamp.matches(&pixels)
                            {
                                live = live
                                    .checked_add(proof.record.length as u64)
                                    .ok_or("Indexed live payload overflow")?;
                                if live > MAX_LIVE {
                                    return Err("Indexed live payload exceeds budget".into());
                                }
                                entries.insert(key, proof.record.clone());
                                next_proofs.entries.push((
                                    key,
                                    RasterProof {
                                        record: proof.record.clone(),
                                        canvas,
                                        region,
                                        stamp,
                                    },
                                ));
                                stats.reused += 1;
                                stats.raster_records_reused_without_encoding += 1;
                                stats.raster_records_reused_without_read += 1;
                                return Ok(());
                            }
                        }
                        let raw = pixels.region(width, region);
                        (
                            crate::indexed_document::Object::Rgba {
                                size: [
                                    (region[2] - region[0]) as u16,
                                    (region[3] - region[1]) as u16,
                                ],
                                bytes: raw,
                            },
                            Some((canvas, region, stamp)),
                        )
                    }
                    other => (other, None),
                };
                let (bytes, verified) = match data {
                    crate::indexed_document::Object::Source { .. } => unreachable!(),
                    crate::indexed_document::Object::Encoded(bytes) => (bytes, None),
                    crate::indexed_document::Object::Rgba { size, bytes: raw } => {
                        if key >> 62 != 1 {
                            return Err("Raw raster object has a non-raster key".into());
                        }
                        if raw.capacity() > MAX_RECORD * 2 {
                            return Err("Streamed raster allocation exceeds budget".into());
                        }
                        crate::indexed_tile::validate_rgba(size, &raw)?;
                        if raw.iter().all(|byte| *byte == 0) {
                            return Ok(());
                        }
                        let mut retained = None;
                        if let Some(old) = self.index.entries.get(&key).cloned() {
                            stats.raster_records_read_for_comparison += 1;
                            let encoded = self.read(key)?.ok_or("Missing committed raster")?;
                            let (old_size, decoded) = crate::indexed_tile::decode_record(&encoded)?;
                            stats.sink_owned_record_bytes_peak = stats
                                .sink_owned_record_bytes_peak
                                .max(raw.capacity() + encoded.capacity() + decoded.capacity());
                            if old_size == size && decoded == raw {
                                retained = Some((encoded, old));
                            }
                        }
                        if let Some((encoded, old)) = retained {
                            stats.raster_records_reused_without_encoding += 1;
                            (encoded, Some(old))
                        } else {
                            let encoded = crate::indexed_tile::encode(size, &raw)?
                                .ok_or("Nonempty raster unexpectedly omitted")?;
                            stats.sink_owned_record_bytes_peak = stats
                                .sink_owned_record_bytes_peak
                                .max(raw.capacity() + encoded.capacity());
                            stats.raster_records_encoded += 1;
                            (encoded, None)
                        }
                    }
                };
                if bytes.is_empty() || bytes.len() > MAX_RECORD {
                    return Err("Streamed indexed record exceeds budget".into());
                }
                if bytes.capacity() > MAX_RECORD * 2 {
                    return Err("Streamed record allocation exceeds budget".into());
                }
                live = live
                    .checked_add(bytes.len() as u64)
                    .ok_or("Indexed live payload overflow")?;
                if live > MAX_LIVE {
                    return Err("Indexed live payload exceeds budget".into());
                }
                let checksum = crc(&bytes);
                let mut record = verified;
                if record.is_some() {
                    stats.reused += 1;
                }
                stats.sink_owned_record_bytes_peak =
                    stats.sink_owned_record_bytes_peak.max(bytes.capacity());
                if record.is_none()
                    && let Some(old) = self.index.entries.get(&key).cloned()
                    && old.length as usize == bytes.len()
                    && old.checksum == checksum
                {
                    let previous = self.read(key)?.ok_or("Missing reuse candidate")?;
                    stats.sink_owned_record_bytes_peak = stats
                        .sink_owned_record_bytes_peak
                        .max(bytes.capacity() + previous.capacity());
                    if previous == bytes {
                        record = Some(old);
                        stats.reused += 1;
                    }
                }
                let record = if let Some(record) = record {
                    record
                } else {
                    let next = position
                        .checked_add(bytes.len() as u64)
                        .ok_or("Indexed offset overflow")?;
                    let manifest_bytes = 4 + (entries.len() as u64 + 1) * 24;
                    if next
                        .checked_add(manifest_bytes)
                        .is_none_or(|end| end > MAX_FILE)
                    {
                        return Err("Indexed journal requires compaction".into());
                    }
                    io(self.file.seek(SeekFrom::Start(position)))?;
                    io(self.file.write_all(&bytes))?;
                    let record = Record {
                        offset: position,
                        length: bytes.len() as u32,
                        checksum,
                    };
                    position = next;
                    stats.changed += 1;
                    record
                };
                if let Some((canvas, region, stamp)) = source_stamp {
                    next_proofs.entries.push((
                        key,
                        RasterProof {
                            record: record.clone(),
                            canvas,
                            region,
                            stamp,
                        },
                    ));
                }
                entries.insert(key, record);
                Ok(())
            })?;
            next_proofs.entries.sort_by_key(|(key, _)| *key);
            stats.raster_proof_descriptor_bytes = next_proofs.descriptor_bytes();
            if stats.raster_proof_descriptor_bytes > MAX_PROOF_DESCRIPTORS {
                return Err("Indexed raster proof descriptors exceed budget".into());
            }
            stats.objects = entries.len();
            stats.removed = self
                .index
                .entries
                .keys()
                .filter(|key| !entries.contains_key(key))
                .count();
            let manifest = encode_index(&entries);
            let end = self.publish(generation, position, &manifest, &mut checkpoint)?;
            Ok((value, end))
        })();
        match result {
            Ok((value, end)) => {
                self.index = Index {
                    generation,
                    end,
                    entries,
                    damaged_slot: false,
                };
                self.raster_proofs = next_proofs;
                self.writable = true;
                Ok((value, stats))
            }
            Err(error) => {
                self.writable = false;
                Err(error)
            }
        }
    }
    fn create(path: &Path) -> Result<Self> {
        if path.exists() {
            return Err("Indexed destination already exists".into());
        }
        let lease = acquire_lease(path, true)?;
        let file = io(OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(path))?;
        let mut journal = Self::initialize(file)?;
        journal.path_lease = lease;
        Ok(journal)
    }
    fn initialize(mut file: File) -> Result<Self> {
        file.try_lock().map_err(|e| e.to_string())?;
        let mut header = [0; HEADER as usize];
        header[..8].copy_from_slice(MAGIC);
        io(file.write_all(&header))?;
        io(file.sync_all())?;
        Ok(Self {
            file,
            failed_stage: None,
            path_lease: None,
            index: Index {
                generation: 0,
                end: HEADER,
                entries: BTreeMap::new(),
                damaged_slot: false,
            },
            writable: true,
            raster_proofs: RasterProofs::default(),
        })
    }
    fn open(path: &Path, writable: bool) -> Result<Self> {
        let lease = acquire_lease(path, writable)?;
        let mut file = io(OpenOptions::new().read(true).write(writable).open(path))?;
        if writable {
            file.try_lock()
        } else {
            file.try_lock_shared()
        }
        .map_err(|e| e.to_string())?;
        let index = read_index(&mut file)?;
        Ok(Self {
            file,
            failed_stage: None,
            path_lease: lease,
            index,
            writable,
            raster_proofs: RasterProofs::default(),
        })
    }
    fn read(&mut self, key: u64) -> Result<Option<Vec<u8>>> {
        let Some(record) = self.index.entries.get(&key) else {
            return Ok(None);
        };
        io(self.file.seek(SeekFrom::Start(record.offset)))?;
        let mut bytes = vec![0; record.length as usize];
        io(self.file.read_exact(&mut bytes))?;
        if crc(&bytes) != record.checksum {
            return Err("Damaged indexed record checksum".into());
        }
        Ok(Some(bytes))
    }
    fn commit(
        &mut self,
        changes: &[(u64, Option<&[u8]>)],
        mut checkpoint: impl FnMut(Phase) -> Result<()>,
    ) -> Result<()> {
        if !self.writable || self.index.damaged_slot {
            return Err("Indexed journal requires explicit recovery before writing".into());
        }
        if changes.len() > MAX_KEYS {
            return Err("Too many indexed changes".into());
        }
        let generation = self
            .index
            .generation
            .checked_add(1)
            .ok_or("Indexed generation overflow")?;
        let mut entries = self.index.entries.clone();
        let mut position = self.index.end;
        let mut unique = std::collections::BTreeSet::new();
        for (key, bytes) in changes {
            if !unique.insert(*key) {
                return Err("Duplicate indexed change key".into());
            }
            if let Some(bytes) = bytes {
                if bytes.is_empty() || bytes.len() > MAX_RECORD {
                    return Err("Indexed record exceeds budget".into());
                }
                entries.insert(
                    *key,
                    Record {
                        offset: position,
                        length: bytes.len() as u32,
                        checksum: crc(bytes),
                    },
                );
                position = position
                    .checked_add(bytes.len() as u64)
                    .ok_or("Indexed offset overflow")?;
            } else {
                entries.remove(key);
            }
        }
        if entries.len() > MAX_KEYS
            || entries.values().map(|r| r.length as u64).sum::<u64>() > MAX_LIVE
        {
            return Err("Indexed live payload exceeds budget".into());
        }
        let manifest = encode_index(&entries);
        let end = position
            .checked_add(manifest.len() as u64)
            .ok_or("Indexed length overflow")?;
        if end > MAX_FILE {
            return Err("Indexed journal requires compaction".into());
        }
        // No writes before every size/key constraint has passed. Retire only the
        // uncommitted tail, retaining both previous generations' referenced data.
        self.writable = false;
        self.raster_proofs = RasterProofs::default();
        let result = (|| {
            io(self.file.set_len(self.index.end))?;
            io(self.file.seek(SeekFrom::Start(self.index.end)))?;
            for (_, bytes) in changes {
                if let Some(bytes) = bytes {
                    io(self.file.write_all(bytes))?;
                }
            }
            self.publish(generation, position, &manifest, &mut checkpoint)
                .map(|_| ())
        })();
        // Any I/O/fault result is uncertain. Never retry from stale in-memory
        // pointers: close/reopen and independently validate the on-disk state.
        if result.is_err() {
            self.writable = false;
            return result;
        }
        self.index = Index {
            generation,
            end,
            entries,
            damaged_slot: false,
        };
        self.writable = true;
        Ok(())
    }
}

pub(crate) fn study(output: Option<&Path>) -> Result<()> {
    #[derive(serde::Serialize)]
    struct Report {
        scope: &'static str,
        generations: u64,
        first_bytes: u64,
        changed_tile_append_bytes: u64,
        unchanged_records_reused: bool,
        exact_reopen: bool,
    }
    let directory = tempfile::tempdir().map_err(|e| e.to_string())?;
    let path = directory.path().join("synthetic.index-study");
    let tile = vec![37; 256 * 256 * 4];
    let edited = vec![81; tile.len()];
    let mut journal = Journal::create(&path)?;
    journal.commit(
        &[
            (1, Some(b"synthetic metadata")),
            (10, Some(&tile)),
            (11, Some(&tile)),
        ],
        |_| Ok(()),
    )?;
    let first_bytes = journal.index.end;
    let unchanged = journal.index.entries[&11].clone();
    journal.commit(&[(10, Some(&edited))], |_| Ok(()))?;
    let end = journal.index.end;
    let reused = journal.index.entries[&11] == unchanged;
    drop(journal);
    let mut reopened = Journal::open(&path, false)?;
    let exact = reopened.read(10)?.as_deref() == Some(edited.as_slice())
        && reopened.read(11)?.as_deref() == Some(tile.as_slice());
    crate::clip_container::write_report(
        &Report {
            scope: "Synthetic experimental indexed commit primitive only; no document format, tile compression, thumbnail, compaction, editor integration or power-cut certification.",
            generations: reopened.index.generation,
            first_bytes,
            changed_tile_append_bytes: end - first_bytes,
            unchanged_records_reused: reused,
            exact_reopen: exact,
        },
        output,
    )
}

pub(crate) fn study_tiles(output: Option<&Path>) -> Result<()> {
    #[derive(serde::Serialize)]
    struct Case {
        name: &'static str,
        geometry: [u16; 2],
        raw_bytes: usize,
        record_bytes: usize,
        omitted: bool,
        exact: bool,
    }
    #[derive(serde::Serialize)]
    struct Report {
        scope: &'static str,
        cases: Vec<Case>,
        exact_journal_reopen: bool,
        changed_record_append_bytes: u64,
        unchanged_record_reused: bool,
        empty_tile_deletion_exact: bool,
    }
    let mut random = vec![0; 256 * 256 * 4];
    let mut state = 0x12345678u32;
    for byte in &mut random {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        *byte = state as u8;
    }
    let mut cases = Vec::new();
    for (name, size, raw) in [
        ("solid_rgba", [256, 256], vec![77; 256 * 256 * 4]),
        ("deterministic_noise", [256, 256], random),
        ("partial_edge", [3, 17], vec![121; 3 * 17 * 4]),
        ("hidden_rgb", [1, 1], vec![23, 45, 67, 0]),
        ("empty", [256, 256], vec![0; 256 * 256 * 4]),
    ] {
        let encoded = crate::indexed_tile::encode(size, &raw)?;
        let exact = match &encoded {
            Some(bytes) => crate::indexed_tile::decode(bytes, size)? == raw,
            None => raw.iter().all(|byte| *byte == 0),
        };
        if !exact {
            return Err("Indexed tile study round trip differs".into());
        }
        cases.push(Case {
            name,
            geometry: size,
            raw_bytes: raw.len(),
            record_bytes: encoded.as_ref().map_or(0, Vec::len),
            omitted: encoded.is_none(),
            exact,
        });
    }
    let directory = tempfile::tempdir().map_err(|e| e.to_string())?;
    let path = directory.path().join("synthetic-tiles.index-study");
    let original = vec![77; 256 * 256 * 4];
    let changed = vec![81; 256 * 256 * 4];
    let first = crate::indexed_tile::encode([256, 256], &original)?.unwrap();
    let next = crate::indexed_tile::encode([256, 256], &changed)?.unwrap();
    let mut journal = Journal::create(&path)?;
    journal.commit(&[(10, Some(&first)), (11, Some(&first))], |_| Ok(()))?;
    let previous_end = journal.index.end;
    let reused = journal.index.entries[&11].clone();
    journal.commit(&[(10, Some(&next))], |_| Ok(()))?;
    let appended = journal.index.end - previous_end;
    let unchanged = journal.index.entries[&11] == reused;
    drop(journal);
    let mut reopened = Journal::open(&path, false)?;
    let a = crate::indexed_tile::decode(
        &reopened.read(10)?.ok_or("Missing changed tile")?,
        [256, 256],
    )?;
    let b = crate::indexed_tile::decode(
        &reopened.read(11)?.ok_or("Missing retained tile")?,
        [256, 256],
    )?;
    let exact = a == changed && b == original;
    if !exact || !unchanged {
        return Err("Indexed tile journal round trip differs".into());
    }
    drop(reopened);
    let empty = crate::indexed_tile::encode([256, 256], &vec![0; 256 * 256 * 4])?;
    let mut journal = Journal::open(&path, true)?;
    journal.commit(&[(10, empty.as_deref())], |_| Ok(()))?;
    drop(journal);
    let mut reopened = Journal::open(&path, false)?;
    let deletion_exact = reopened.read(10)?.is_none()
        && crate::indexed_tile::decode(
            &reopened
                .read(11)?
                .ok_or("Missing retained tile after deletion")?,
            [256, 256],
        )? == original;
    if !deletion_exact {
        return Err("Indexed empty tile deletion differs".into());
    }
    crate::clip_container::write_report(
        &Report {
            scope: "Synthetic bounded tile codec and journal integration; no document format, thumbnail, compaction, editor save integration or power-cut certification. Append count excludes root overwrite; no latency benchmark.",
            cases,
            exact_journal_reopen: exact,
            changed_record_append_bytes: appended,
            unchanged_record_reused: unchanged,
            empty_tile_deletion_exact: deletion_exact,
        },
        output,
    )
}

pub(crate) fn study_compaction(output: Option<&Path>) -> Result<()> {
    #[derive(serde::Serialize)]
    struct Report {
        scope: &'static str,
        compaction: CompactStats,
        exact_reopen: bool,
        exact_following_save: bool,
    }
    let directory = tempfile::tempdir().map_err(|e| e.to_string())?;
    let path = directory.path().join("synthetic-compact.index-study");
    let mut journal = Journal::create(&path)?;
    let mut doc = crate::indexed_document::fixture();
    let (mut keys, _) =
        journal.replace_document(&doc, &Default::default(), &|| false, |_| Ok(()))?;
    for step in 0..6 {
        doc.layers[0].name = format!("Generated compaction revision {step}");
        doc.layers[0]
            .pixels
            .writer()
            .rgba_mut(0)
            .copy_from_slice(&[step * 13, 70, 90, 180]);
        keys = journal
            .replace_document(&doc, &keys, &|| false, |_| Ok(()))?
            .0;
    }
    let stats = journal.compact_document(&path, &|| false, |_| Ok(()))?;
    drop(journal);
    let mut journal = Journal::open(&path, true)?;
    let objects = journal.index.entries.keys().copied().collect::<Vec<_>>();
    let (mut loaded, keys) = crate::indexed_document::decode(
        &objects,
        &mut |key| journal.read(key)?.ok_or("Missing compacted object".into()),
        &|| false,
    )?;
    let exact = crate::indexed_document::equivalent(&doc, &loaded)?;
    loaded.layers[0].name = "Generated after compaction".into();
    journal.replace_document(&loaded, &keys, &|| false, |_| Ok(()))?;
    drop(journal);
    let mut journal = Journal::open(&path, false)?;
    let objects = journal.index.entries.keys().copied().collect::<Vec<_>>();
    let (after, _) = crate::indexed_document::decode(
        &objects,
        &mut |key| {
            journal
                .read(key)?
                .ok_or("Missing post-compaction object".into())
        },
        &|| false,
    )?;
    let exact_following = crate::indexed_document::equivalent(&loaded, &after)?;
    if !exact || !exact_following || stats.after_bytes >= stats.before_bytes {
        return Err("Compaction study differs or does not reclaim obsolete records".into());
    }
    crate::clip_container::write_report(
        &Report {
            scope: "Generated small-document same-directory compaction with stable pathname lease, locked staged/replacement file, codec/record/index verification and atomic TempPath persistence. No normal save integration, directory/device power-loss certification or dense-workload measurement.",
            compaction: stats,
            exact_reopen: exact,
            exact_following_save: exact_following,
        },
        output,
    )
}

pub(crate) fn study_thumbnails(output: Option<&Path>) -> Result<()> {
    fn preview(journal: &mut Journal) -> Result<(crate::indexed_thumbnail::Thumbnail, Vec<u64>)> {
        let keys = journal.index.entries.keys().copied().collect::<Vec<_>>();
        let mut reads = Vec::new();
        let thumbnail = crate::indexed_document::read_thumbnail(
            &keys,
            &mut |key| {
                reads.push(key);
                journal.read(key)?.ok_or("Missing thumbnail object".into())
            },
            &|| false,
        )?
        .ok_or("Missing indexed thumbnail")?;
        Ok((thumbnail, reads))
    }
    #[derive(serde::Serialize)]
    struct Report {
        scope: &'static str,
        size: [u16; 2],
        record_bytes: usize,
        preview_read_keys: Vec<u64>,
        exact_initial: bool,
        metadata_only_reuses_thumbnail: bool,
        pixel_edit_changes_thumbnail: bool,
        exact_edited: bool,
    }
    let directory = tempfile::tempdir().map_err(|e| e.to_string())?;
    let path = directory.path().join("synthetic-thumbnail.index-study");
    let mut doc = crate::indexed_document::fixture();
    doc.layers[1].visible = true;
    doc.layers[0]
        .pixels
        .writer()
        .rgba_mut(256)
        .copy_from_slice(&[20, 60, 100, 255]);
    let expected = crate::indexed_thumbnail::decode(
        &crate::indexed_thumbnail::encode(&doc, &|| false)?,
        [doc.width, doc.height],
    )?;
    let mut journal = Journal::create(&path)?;
    let (keys, _) = journal.replace_document(&doc, &Default::default(), &|| false, |_| Ok(()))?;
    let first = journal.index.entries[&2].clone();
    drop(journal);
    let mut journal = Journal::open(&path, true)?;
    let (initial, read_keys) = preview(&mut journal)?;
    let exact_initial = initial.rgba == expected.rgba;
    doc.layers[0].name = "Generated thumbnail metadata edit".into();
    let (keys, _) = journal.replace_document(&doc, &keys, &|| false, |_| Ok(()))?;
    let reused = journal.index.entries[&2] == first;
    doc.layers[0]
        .pixels
        .writer()
        .rgba_mut(0)
        .copy_from_slice(&[255, 0, 0, 255]);
    journal.replace_document(&doc, &keys, &|| false, |_| Ok(()))?;
    let changed = journal.index.entries[&2] != first;
    drop(journal);
    let mut journal = Journal::open(&path, false)?;
    let (edited, reads) = preview(&mut journal)?;
    let expected = crate::indexed_thumbnail::decode(
        &crate::indexed_thumbnail::encode(&doc, &|| false)?,
        [doc.width, doc.height],
    )?;
    let exact_edited = edited.rgba == expected.rgba && edited.rgba != initial.rgba;
    if !exact_initial
        || !reused
        || !changed
        || !exact_edited
        || read_keys != [0, 1, 2]
        || reads != [0, 1, 2]
    {
        return Err("Indexed thumbnail study differs".into());
    }
    crate::clip_container::write_report(
        &Report {
            scope: "Generated bounded opaque-white-canvas thumbnails committed with a versioned document schema. Preview accesses only manifest/metadata/thumbnail; this is not full raster health validation, UI integration or device durability certification. No image export.",
            size: edited.size,
            record_bytes: journal.index.entries[&2].length as usize,
            preview_read_keys: reads,
            exact_initial,
            metadata_only_reuses_thumbnail: reused,
            pixel_edit_changes_thumbnail: changed,
            exact_edited,
        },
        output,
    )
}

pub(crate) fn study_stream(output: Option<&Path>) -> Result<()> {
    fn read(
        journal: &mut Journal,
    ) -> Result<(crate::document::Document, crate::indexed_document::Keys)> {
        let keys = journal.index.entries.keys().copied().collect::<Vec<_>>();
        crate::indexed_document::decode(
            &keys,
            &mut |key| {
                journal
                    .read(key)?
                    .ok_or("Missing streamed document object".into())
            },
            &|| false,
        )
    }
    #[derive(serde::Serialize)]
    struct Report {
        scope: &'static str,
        first_commit_bytes: u64,
        unchanged_append_bytes: u64,
        edit_append_bytes: u64,
        removed_objects: usize,
        first: StreamStats,
        unchanged: StreamStats,
        edit: StreamStats,
        exact_initial: bool,
        exact_edited: bool,
        exact_after_removal: bool,
    }
    let directory = tempfile::tempdir().map_err(|e| e.to_string())?;
    let path = directory.path().join("synthetic-stream.index-study");
    let doc = crate::indexed_document::fixture();
    let mut journal = Journal::create(&path)?;
    let (_, first) = journal.replace_document(&doc, &Default::default(), &|| false, |_| Ok(()))?;
    let first_bytes = journal.index.end;
    drop(journal);
    let mut journal = Journal::open(&path, true)?;
    let (mut doc, keys) = read(&mut journal)?;
    let exact_initial =
        crate::indexed_document::equivalent(&crate::indexed_document::fixture(), &doc)?;
    let (keys, unchanged) = journal.replace_document(&doc, &keys, &|| false, |_| Ok(()))?;
    let unchanged_bytes = journal.index.end - first_bytes;
    let before = journal.index.end;
    doc.layers.swap(0, 1);
    doc.layers[0].name = "Generated stream rename".into();
    doc.layers[0]
        .pixels
        .writer()
        .rgba_mut(256)
        .copy_from_slice(&[10, 20, 30, 128]);
    let (_, edit) = journal.replace_document(&doc, &keys, &|| false, |_| Ok(()))?;
    let edit_bytes = journal.index.end - before;
    drop(journal);
    let mut journal = Journal::open(&path, true)?;
    let (mut loaded, keys) = read(&mut journal)?;
    let exact_edited = crate::indexed_document::equivalent(&doc, &loaded)?;
    loaded.layers.remove(0);
    loaded.active = 1;
    loaded.selection = None;
    loaded.selection_mask = None;
    let (_, removed) = journal.replace_document(&loaded, &keys, &|| false, |_| Ok(()))?;
    drop(journal);
    let mut journal = Journal::open(&path, false)?;
    let (final_doc, _) = read(&mut journal)?;
    let exact_removal = crate::indexed_document::equivalent(&loaded, &final_doc)?;
    if !exact_initial
        || !exact_edited
        || !exact_removal
        || unchanged.changed != 0
        || unchanged.reused != first.objects
    {
        return Err("Streaming document study differs".into());
    }
    crate::clip_container::write_report(
        &Report {
            scope: "Generated small-document streaming journal study; no complete encoded object maps retained. Sink memory metric covers incoming and comparison record allocations only, excluding encoder scratch/index/source memory. Append bytes exclude root overwrite. No compaction, normal save integration or power-cut certification.",
            first_commit_bytes: first_bytes,
            unchanged_append_bytes: unchanged_bytes,
            edit_append_bytes: edit_bytes,
            removed_objects: removed.removed,
            first,
            unchanged,
            edit,
            exact_initial,
            exact_edited,
            exact_after_removal: exact_removal,
        },
        output,
    )
}

pub(crate) fn study_documents(output: Option<&Path>) -> Result<()> {
    fn objects(
        doc: &crate::document::Document,
        keys: &crate::indexed_document::Keys,
    ) -> Result<(BTreeMap<u64, Vec<u8>>, crate::indexed_document::Keys)> {
        let mut objects = BTreeMap::new();
        let next = crate::indexed_document::visit(doc, keys, &|| false, &mut |key, bytes| {
            if objects.insert(key, bytes).is_some() {
                return Err("Duplicate document object".into());
            }
            Ok(())
        })?;
        Ok((objects, next))
    }
    fn read(
        journal: &mut Journal,
    ) -> Result<(crate::document::Document, crate::indexed_document::Keys)> {
        let keys = journal.index.entries.keys().copied().collect::<Vec<_>>();
        crate::indexed_document::decode(
            &keys,
            &mut |key| journal.read(key)?.ok_or("Missing document object".into()),
            &|| false,
        )
    }
    #[derive(serde::Serialize)]
    struct Report {
        scope: &'static str,
        canvas: [usize; 2],
        entries: usize,
        objects: usize,
        first_commit_bytes: u64,
        edit_append_bytes: u64,
        changed_objects: usize,
        reused_objects: usize,
        exact_initial: bool,
        exact_edited: bool,
        stable_keys_after_reopen: bool,
    }
    let directory = tempfile::tempdir().map_err(|e| e.to_string())?;
    let path = directory.path().join("synthetic-document.index-study");
    let mut doc = crate::indexed_document::fixture();
    let (first, keys) = objects(&doc, &Default::default())?;
    let mut journal = Journal::create(&path)?;
    let references = first
        .iter()
        .map(|(key, value)| (*key, Some(value.as_slice())))
        .collect::<Vec<_>>();
    journal.commit(&references, |_| Ok(()))?;
    let first_bytes = journal.index.end;
    drop(journal);
    let mut journal = Journal::open(&path, false)?;
    let (loaded, loaded_keys) = read(&mut journal)?;
    let exact_initial = crate::indexed_document::equivalent(&doc, &loaded)?;
    let (reencoded, _) = objects(&loaded, &loaded_keys)?;
    let stable = reencoded == first;
    drop(journal);
    // Reorder leaves, change metadata, and change only one source tile.
    doc.layers.swap(0, 1);
    doc.layers[0].name = "Generated renamed leaf".into();
    doc.layers[0]
        .pixels
        .writer()
        .rgba_mut(256)
        .copy_from_slice(&[10, 20, 30, 128]);
    let (second, _) = objects(&doc, &keys)?;
    let mut changes = second
        .iter()
        .filter(|(key, bytes)| first.get(key) != Some(*bytes))
        .map(|(key, bytes)| (*key, Some(bytes.as_slice())))
        .collect::<Vec<_>>();
    changes.extend(
        first
            .keys()
            .filter(|key| !second.contains_key(key))
            .map(|key| (*key, None)),
    );
    let changed = changes.len();
    let reused = second.len()
        - second
            .iter()
            .filter(|(key, bytes)| first.get(key) != Some(*bytes))
            .count();
    let mut journal = Journal::open(&path, true)?;
    journal.commit(&changes, |_| Ok(()))?;
    let appended = journal.index.end - first_bytes;
    drop(journal);
    let mut journal = Journal::open(&path, false)?;
    let (edited, _) = read(&mut journal)?;
    let exact_edited = crate::indexed_document::equivalent(&doc, &edited)?;
    if !exact_initial || !exact_edited || !stable {
        return Err("Indexed document study round trip differs".into());
    }
    crate::clip_container::write_report(
        &Report {
            scope: "Generated small document schema/journal round trips including selection, thumbnail and metadata; prototype driver retains complete encoded object maps for comparison. No bounded streaming save integration, compaction or power-cut certification.",
            canvas: [doc.width, doc.height],
            entries: doc.layers.len(),
            objects: second.len(),
            first_commit_bytes: first_bytes,
            edit_append_bytes: appended,
            changed_objects: changed,
            reused_objects: reused,
            exact_initial,
            exact_edited,
            stable_keys_after_reopen: stable,
        },
        output,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn locked_journal_proofs_move_between_snapshot_workers_without_retaining_pixels() {
        use crate::snapshot_memory::{Registry, SourceWorker};
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("journal");
        let mut live = crate::indexed_document::fixture();
        let mut j = Journal::create(&path).unwrap();
        let (keys, _) = j
            .replace_document(&live, &Default::default(), &|| false, |_| Ok(()))
            .unwrap();
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        let (go_tx, go_rx) = std::sync::mpsc::channel();
        let worker = SourceWorker::spawn(live.clone(), move |snapshot| {
            ready_tx.send(()).unwrap();
            go_rx.recv().unwrap();
            let (keys, stats) = j
                .replace_document(&snapshot, &keys, &|| false, |_| Ok(()))
                .unwrap();
            (j, keys, stats)
        });
        ready_rx.recv().unwrap();
        let mut before_registry = Registry::new(&live);
        worker.register_source(&mut before_registry);
        let before = before_registry.additional_bytes();
        live.layers[0]
            .pixels
            .writer()
            .rgba_mut(0)
            .copy_from_slice(&[211, 3, 99, 77]);
        let mut registry = Registry::new(&live);
        worker.register_source(&mut registry);
        assert!(registry.additional_bytes() >= before + 256 * 256 * 4);
        go_tx.send(()).unwrap();
        let (mut j, keys, stats) = worker.join().unwrap();
        assert_eq!(stats.raster_records_reused_without_read, 3);
        assert_eq!(stats.raster_records_read_for_comparison, 0);
        assert_eq!(stats.raster_records_encoded, 0);
        assert!(stats.raster_proof_descriptor_bytes < MAX_PROOF_DESCRIPTORS);
        let worker = SourceWorker::spawn(live.clone(), move |snapshot| {
            let (keys, stats) = j
                .replace_document(&snapshot, &keys, &|| false, |_| Ok(()))
                .unwrap();
            (j, keys, stats)
        });
        let (mut j, keys, stats) = worker.join().unwrap();
        assert_eq!(stats.raster_records_reused_without_read, 2);
        assert_eq!(stats.raster_records_read_for_comparison, 1);
        assert_eq!(stats.raster_records_encoded, 1);
        assert!(crate::indexed_document::equivalent(&live, &document(&mut j)).unwrap());
        drop(j);
        let mut j = Journal::open(&path, true).unwrap();
        assert!(j.raster_proofs.entries.is_empty());
        let (_, stats) = j
            .replace_document(&live, &keys, &|| false, |_| Ok(()))
            .unwrap();
        assert_eq!(stats.raster_records_reused_without_read, 0);
        assert_eq!(stats.raster_records_read_for_comparison, 3);
        assert_eq!(stats.raster_records_encoded, 0);
        // No proof owns pixel payloads after the document dies.
        drop(live);
        assert!(
            j.raster_proofs
                .entries
                .iter()
                .all(|(_, proof)| !proof.stamp.source_is_alive())
        );
    }
    #[test]
    fn journal_proofs_invalidate_for_flat_replacement_undo_compaction_and_low_level_writes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("journal");
        let mut doc = crate::indexed_document::fixture();
        let mut j = Journal::create(&path).unwrap();
        let (keys, _) = j
            .replace_document(&doc, &Default::default(), &|| false, |_| Ok(()))
            .unwrap();
        let original = doc.layers[0].pixels.clone();
        doc.layers[0]
            .pixels
            .writer()
            .rgba_mut(0)
            .copy_from_slice(&[11, 22, 33, 44]);
        j.replace_document(&doc, &keys, &|| false, |_| Ok(()))
            .unwrap();
        doc.layers[0].pixels = original;
        let (_, stats) = j
            .replace_document(&doc, &keys, &|| false, |_| Ok(()))
            .unwrap();
        assert_eq!(stats.raster_records_reused_without_read, 2);
        assert_eq!(stats.raster_records_encoded, 1);
        doc.layers[0].pixels = doc.layers[0]
            .pixels
            .region(doc.width, [0, 0, doc.width, doc.height])
            .into();
        let (_, stats) = j
            .replace_document(&doc, &keys, &|| false, |_| Ok(()))
            .unwrap();
        assert_eq!(stats.raster_records_reused_without_read, 1);
        assert_eq!(stats.raster_records_encoded, 0);
        j.compact_document(&path, &|| false, |_| Ok(())).unwrap();
        assert!(j.raster_proofs.entries.is_empty());
        let (_, stats) = j
            .replace_document(&doc, &keys, &|| false, |_| Ok(()))
            .unwrap();
        assert_eq!(stats.raster_records_reused_without_read, 0);
        j.commit(&[], |_| Ok(())).unwrap();
        assert!(j.raster_proofs.entries.is_empty());
        let (_, stats) = j
            .replace_document(&doc, &keys, &|| false, |_| Ok(()))
            .unwrap();
        assert_eq!(stats.raster_records_reused_without_read, 0);
        assert!(crate::indexed_document::equivalent(&doc, &document(&mut j)).unwrap());
    }
    #[test]
    fn source_geometry_and_generation_are_required_even_for_the_same_flat_allocation() {
        use crate::indexed_document::Object;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("journal");
        let mut j = Journal::create(&path).unwrap();
        let pixels: crate::pixels::Pixels = [1, 2, 3, 4].repeat(256).into();
        let key = (1u64 << 62) | (1 << 16);
        let save = |j: &mut Journal, canvas: [usize; 2]| {
            j.replace_objects(
                |emit| {
                    emit(
                        key,
                        Object::Source {
                            pixels: pixels.clone(),
                            canvas,
                            region: [0, 0, canvas[0], canvas[1]],
                        },
                    )
                },
                |_| Ok(()),
            )
            .unwrap()
            .1
        };
        save(&mut j, [256, 1]);
        assert_eq!(save(&mut j, [256, 1]).raster_records_reused_without_read, 1);
        let resized = save(&mut j, [1, 256]);
        assert_eq!(resized.raster_records_reused_without_read, 0);
        assert_eq!(resized.raster_records_encoded, 1);
        j.raster_proofs.generation -= 1;
        let stale = save(&mut j, [1, 256]);
        assert_eq!(stale.raster_records_reused_without_read, 0);
        assert_eq!(stale.raster_records_read_for_comparison, 1);
    }
    #[test]
    fn failed_publication_never_publishes_proofs_and_reopened_save_validates_records() {
        for fault in [
            Phase::PayloadWritten,
            Phase::PayloadSynced,
            Phase::RootWritten,
            Phase::RootSynced,
        ] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("journal");
            let doc = crate::indexed_document::fixture();
            let mut j = Journal::create(&path).unwrap();
            let (keys, _) = j
                .replace_document(&doc, &Default::default(), &|| false, |_| Ok(()))
                .unwrap();
            assert!(
                j.replace_document(&doc, &keys, &|| false, |phase| if phase == fault {
                    Err("injected fault".into())
                } else {
                    Ok(())
                })
                .is_err()
            );
            assert!(!j.writable);
            assert!(j.raster_proofs.entries.is_empty());
            drop(j);
            let mut j = Journal::open(&path, true).unwrap();
            let (_, stats) = j
                .replace_document(&doc, &keys, &|| false, |_| Ok(()))
                .unwrap();
            assert_eq!(stats.raster_records_reused_without_read, 0);
            assert_eq!(stats.raster_records_read_for_comparison, 3);
        }
    }
    #[test]
    fn decoded_source_reuse_survives_fresh_allocations_and_background_snapshot_lifetimes() {
        use crate::snapshot_memory::{Registry, SourceWorker};
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("journal");
        let doc = crate::indexed_document::fixture();
        let mut j = Journal::create(&path).unwrap();
        let (_, first) = j
            .replace_document(&doc, &Default::default(), &|| false, |_| Ok(()))
            .unwrap();
        assert_eq!(first.raster_records_encoded, 3);
        drop(j);
        let mut j = Journal::open(&path, false).unwrap();
        let inventory = j.index.entries.keys().copied().collect::<Vec<_>>();
        let (mut live, keys) = crate::indexed_document::decode(
            &inventory,
            &mut |key| j.read(key)?.ok_or("missing object".into()),
            &|| false,
        )
        .unwrap();
        assert_ne!(live.layers[0].id, doc.layers[0].id);
        drop(j);
        drop(doc);
        let original_pixel = live.layers[0].pixels.rgba(0);
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        let (go_tx, go_rx) = std::sync::mpsc::channel();
        let worker_path = path.clone();
        let worker = SourceWorker::spawn(live.clone(), move |snapshot| {
            ready_tx.send(()).unwrap();
            go_rx.recv().unwrap();
            let mut j = Journal::open(&worker_path, true).unwrap();
            j.replace_document(&snapshot, &keys, &|| false, |_| Ok(()))
                .unwrap()
        });
        ready_rx.recv().unwrap();
        let mut before = Registry::new(&live);
        worker.register_source(&mut before);
        live.layers[0]
            .pixels
            .writer()
            .rgba_mut(0)
            .copy_from_slice(&[150, 100, 20, 200]);
        let mut after = Registry::new(&live);
        worker.register_source(&mut after);
        assert!(after.additional_bytes() >= before.additional_bytes() + 256 * 256 * 4);
        go_tx.send(()).unwrap();
        let (keys, unchanged) = worker.join().unwrap();
        assert_eq!(unchanged.raster_records_encoded, 0);
        assert_eq!(unchanged.raster_records_reused_without_encoding, 3);
        let mut j = Journal::open(&path, false).unwrap();
        assert_eq!(document(&mut j).layers[0].pixels.rgba(0), original_pixel);
        drop(j);
        let worker_path = path.clone();
        let worker = SourceWorker::spawn(live.clone(), move |snapshot| {
            let mut j = Journal::open(&worker_path, true).unwrap();
            j.replace_document(&snapshot, &keys, &|| false, |_| Ok(()))
                .unwrap()
                .1
        });
        let edited = worker.join().unwrap();
        assert_eq!(edited.raster_records_encoded, 1);
        assert_eq!(edited.raster_records_reused_without_encoding, 2);
        let mut j = Journal::open(&path, false).unwrap();
        assert!(crate::indexed_document::equivalent(&live, &document(&mut j)).unwrap());
    }
    #[test]
    fn raw_raster_comparison_rejects_crc_only_equality_and_handles_resize_and_empty_deletion() {
        use crate::indexed_document::Object;
        let a = vec![0x64, 0xe1, 0xcd, 0xb0, 0x49, 0x63, 0x2f, 0xdd];
        let b = vec![0xac, 0x18, 0x6d, 0x24, 0x4b, 0x06, 0xa6, 0x8d];
        assert_eq!(crc(&a), crc(&b));
        let key = (1u64 << 62) | (1 << 16);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("journal");
        let mut j = Journal::create(&path).unwrap();
        j.replace_objects(
            |emit| {
                emit(
                    key,
                    Object::Rgba {
                        size: [2, 1],
                        bytes: a,
                    },
                )
            },
            |_| Ok(()),
        )
        .unwrap();
        let old = j.index.entries[&key].clone();
        drop(j);
        let mut j = Journal::open(&path, true).unwrap();
        let (_, changed) = j
            .replace_objects(
                |emit| {
                    emit(
                        key,
                        Object::Rgba {
                            size: [2, 1],
                            bytes: b.clone(),
                        },
                    )
                },
                |_| Ok(()),
            )
            .unwrap();
        assert_eq!(changed.raster_records_encoded, 1);
        assert_eq!(changed.raster_records_reused_without_encoding, 0);
        assert_ne!(j.index.entries[&key].offset, old.offset);
        assert_eq!(
            crate::indexed_tile::decode_record(&j.read(key).unwrap().unwrap())
                .unwrap()
                .1,
            b
        );
        let (_, resized) = j
            .replace_objects(
                |emit| {
                    emit(
                        key,
                        Object::Rgba {
                            size: [1, 2],
                            bytes: b,
                        },
                    )
                },
                |_| Ok(()),
            )
            .unwrap();
        assert_eq!(resized.raster_records_encoded, 1);
        assert_eq!(
            crate::indexed_tile::decode_record(&j.read(key).unwrap().unwrap())
                .unwrap()
                .0,
            [1, 2]
        );
        let (_, deleted) = j
            .replace_objects(
                |emit| {
                    emit(
                        key,
                        Object::Rgba {
                            size: [1, 2],
                            bytes: vec![0; 8],
                        },
                    )
                },
                |_| Ok(()),
            )
            .unwrap();
        assert_eq!(deleted.removed, 1);
        assert!(j.read(key).unwrap().is_none());
        assert!(
            j.replace_objects(
                |emit| {
                    emit(
                        key,
                        Object::Rgba {
                            size: [1, 1],
                            bytes: vec![0; 4],
                        },
                    )?;
                    emit(
                        key,
                        Object::Rgba {
                            size: [1, 1],
                            bytes: vec![0; 4],
                        },
                    )
                },
                |_| Ok(())
            )
            .is_err()
        );
    }
    #[test]
    fn committed_codec_damage_cannot_be_reused_or_silently_healed_by_source_comparison() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("journal");
        let mut j = Journal::create(&path).unwrap();
        let doc = crate::indexed_document::fixture();
        let (keys, _) = j
            .replace_document(&doc, &Default::default(), &|| false, |_| Ok(()))
            .unwrap();
        let key = *j.index.entries.keys().find(|key| **key >> 62 == 1).unwrap();
        let mut damaged = j.read(key).unwrap().unwrap();
        *damaged.last_mut().unwrap() ^= 1;
        j.commit(&[(key, Some(&damaged))], |_| Ok(())).unwrap();
        let before = bytes(&mut j);
        let generation = j.index.generation;
        assert!(
            j.replace_document(&doc, &keys, &|| false, |_| Ok(()))
                .is_err()
        );
        assert!(!j.writable);
        drop(j);
        assert_eq!(std::fs::read(&path).unwrap(), before);
        let j = Journal::open(&path, false).unwrap();
        assert_eq!(j.index.generation, generation);
    }
    fn bytes(journal: &mut Journal) -> Vec<u8> {
        journal.file.seek(SeekFrom::Start(0)).unwrap();
        let mut bytes = Vec::new();
        journal.file.read_to_end(&mut bytes).unwrap();
        bytes
    }
    fn populated(
        path: &Path,
    ) -> (
        Journal,
        crate::document::Document,
        crate::indexed_document::Keys,
    ) {
        let mut j = Journal::create(path).unwrap();
        let mut doc = crate::indexed_document::fixture();
        let (mut keys, _) = j
            .replace_document(&doc, &Default::default(), &|| false, |_| Ok(()))
            .unwrap();
        for revision in 0..3 {
            doc.layers[0].name = format!("Generated obsolete revision {revision}");
            keys = j
                .replace_document(&doc, &keys, &|| false, |_| Ok(()))
                .unwrap()
                .0;
        }
        (j, doc, keys)
    }
    #[test]
    fn compaction_panics_keep_leases_and_copied_read_only_files_remain_readable() {
        for fault in [CompactPhase::RecordCopied, CompactPhase::Replaced] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("journal");
            let (mut j, doc, _) = populated(&path);
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let _ = j.compact_document(&path, &|| false, |phase| {
                    assert!(phase != fault, "injected compaction panic");
                    Ok(())
                });
            }));
            assert!(result.is_err());
            assert!(!j.writable);
            assert!(Journal::open(&path, true).is_err());
            drop(j);
            let mut j = Journal::open(&path, false).unwrap();
            assert!(crate::indexed_document::equivalent(&doc, &document(&mut j)).unwrap());
            drop(j);
            assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 2);
        }
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source");
        let (j, doc, _) = populated(&source);
        drop(j);
        let copied = dir.path().join("copied");
        std::fs::copy(&source, &copied).unwrap();
        let mut reader = Journal::open(&copied, false).unwrap();
        assert!(reader.path_lease.is_none());
        assert!(crate::indexed_document::equivalent(&doc, &document(&mut reader)).unwrap());
        assert!(Journal::open(&copied, true).is_err());
        drop(reader);
        let writer = Journal::open(&copied, true).unwrap();
        assert!(writer.path_lease.is_some());
        #[cfg(windows)]
        assert!(std::fs::remove_file(lease_path(&copied).unwrap()).is_err());
        drop(writer);
        let original = std::fs::read(&copied).unwrap();
        std::fs::write(lease_path(&copied).unwrap(), b"foreign lock data").unwrap();
        assert!(Journal::open(&copied, true).is_err());
        assert_eq!(std::fs::read(&copied).unwrap(), original);
    }
    #[test]
    fn compaction_verifies_reclaims_and_keeps_writer_locks_across_replacement() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("journal");
        let (mut j, mut doc, keys) = populated(&path);
        let old = j.index.generation;
        let stats = j
            .compact_document(&path, &|| false, |phase| {
                if matches!(phase, CompactPhase::BeforeReplace | CompactPhase::Replaced) {
                    assert!(Journal::open(&path, true).is_err());
                    assert!(Journal::open(&path, false).is_err());
                }
                Ok(())
            })
            .unwrap();
        assert_eq!(stats.new_generation, old + 1);
        assert!(stats.after_bytes < stats.before_bytes);
        assert_eq!(j.file.metadata().unwrap().len(), stats.after_bytes);
        assert!(j.writable);
        assert!(crate::indexed_document::equivalent(&doc, &document(&mut j)).unwrap());
        doc.layers[0].name = "Generated append after compact".into();
        j.replace_document(&doc, &keys, &|| false, |_| Ok(()))
            .unwrap();
        drop(j);
        let mut j = Journal::open(&path, false).unwrap();
        assert_eq!(j.index.generation, old + 2);
        assert!(crate::indexed_document::equivalent(&doc, &document(&mut j)).unwrap());
        drop(j);
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 2);
    }
    #[test]
    fn compaction_failures_preserve_original_until_replacement_and_require_reopen() {
        for failure in [
            CompactPhase::RecordCopied,
            CompactPhase::StagedSynced,
            CompactPhase::Verified,
            CompactPhase::BeforeReplace,
            CompactPhase::Replaced,
            CompactPhase::FinalSynced,
        ] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("journal");
            let (mut j, doc, _) = populated(&path);
            let original = bytes(&mut j);
            let generation = j.index.generation;
            assert!(
                j.compact_document(&path, &|| false, |phase| if phase == failure {
                    Err("injected compact failure".into())
                } else {
                    Ok(())
                })
                .is_err()
            );
            assert!(!j.writable);
            assert!(Journal::open(&path, true).is_err());
            drop(j);
            let after = std::fs::read(&path).unwrap();
            if !matches!(failure, CompactPhase::Replaced | CompactPhase::FinalSynced) {
                assert_eq!(original, after);
            }
            let mut j = Journal::open(&path, false).unwrap();
            assert!(crate::indexed_document::equivalent(&doc, &document(&mut j)).unwrap());
            assert_eq!(
                j.index.generation,
                if matches!(failure, CompactPhase::Replaced | CompactPhase::FinalSynced) {
                    generation + 1
                } else {
                    generation
                }
            );
            drop(j);
            assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 2);
        }
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cancel");
        let (mut j, _, _) = populated(&path);
        let original = bytes(&mut j);
        let cancel = std::cell::Cell::new(false);
        assert!(
            j.compact_document(&path, &|| cancel.get(), |phase| {
                if phase == CompactPhase::BeforeReplace {
                    cancel.set(true);
                }
                Ok(())
            })
            .is_err()
        );
        drop(j);
        assert_eq!(std::fs::read(&path).unwrap(), original);
        #[cfg(windows)]
        {
            let path = dir.path().join("blocked-replacement");
            let (mut j, doc, _) = populated(&path);
            let original = bytes(&mut j);
            let blocker = OpenOptions::new()
                .read(true)
                .write(true)
                .open(&path)
                .unwrap();
            assert!(j.compact_document(&path, &|| false, |_| Ok(())).is_err());
            assert!(!j.writable);
            let temporary = j.failed_stage.as_ref().unwrap().to_path_buf();
            assert!(temporary.exists());
            assert!(Journal::open(&path, true).is_err());
            drop(blocker);
            drop(j);
            assert!(!temporary.exists());
            assert_eq!(std::fs::read(&path).unwrap(), original);
            let mut j = Journal::open(&path, false).unwrap();
            assert!(crate::indexed_document::equivalent(&doc, &document(&mut j)).unwrap());
        }
    }
    #[test]
    fn compaction_refuses_destination_changes_hard_links_and_corrupt_native_records() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("journal");
        let moved = dir.path().join("moved");
        let (mut j, _, _) = populated(&path);
        let original = bytes(&mut j);
        assert!(
            j.compact_document(&path, &|| false, |phase| {
                if phase == CompactPhase::BeforeReplace {
                    std::fs::rename(&path, &moved).unwrap();
                    std::fs::write(&path, b"foreign artwork").unwrap();
                }
                Ok(())
            })
            .is_err()
        );
        drop(j);
        assert_eq!(std::fs::read(&path).unwrap(), b"foreign artwork");
        assert_eq!(std::fs::read(&moved).unwrap(), original);
        let path = dir.path().join("linked");
        let alias = dir.path().join("alias");
        let (mut j, _, _) = populated(&path);
        let original = bytes(&mut j);
        std::fs::hard_link(&path, &alias).unwrap();
        assert!(j.compact_document(&path, &|| false, |_| Ok(())).is_err());
        drop(j);
        assert_eq!(std::fs::read(&path).unwrap(), original);
        assert_eq!(std::fs::read(&alias).unwrap(), original);
        let path = dir.path().join("bad-codec");
        let mut j = Journal::create(&path).unwrap();
        let doc = crate::indexed_document::fixture();
        let mut damaged = false;
        j.replace_stream(
            |emit| {
                crate::indexed_document::visit(
                    &doc,
                    &Default::default(),
                    &|| false,
                    &mut |key, mut data| {
                        if key >> 62 == 1 && !damaged {
                            *data.last_mut().unwrap() ^= 1;
                            damaged = true;
                        }
                        emit(key, data)
                    },
                )
            },
            |_| Ok(()),
        )
        .unwrap();
        let original = bytes(&mut j);
        assert!(j.compact_document(&path, &|| false, |_| Ok(())).is_err());
        drop(j);
        assert_eq!(std::fs::read(&path).unwrap(), original);
    }
    fn document(journal: &mut Journal) -> crate::document::Document {
        let keys = journal.index.entries.keys().copied().collect::<Vec<_>>();
        crate::indexed_document::decode(
            &keys,
            &mut |key| journal.read(key)?.ok_or("missing object".into()),
            &|| false,
        )
        .unwrap()
        .0
    }
    #[test]
    fn unwinding_during_encoding_or_publication_poison_handles_until_reopen() {
        for (stream, after_root) in [(true, false), (true, true), (false, true)] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("journal");
            let mut j = initial(&path);
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                if stream {
                    let _ = j.replace_stream(
                        |emit| {
                            emit(1, b"new".to_vec())?;
                            assert!(after_root, "injected encoder panic");
                            emit(3, b"added".to_vec())
                        },
                        |phase| {
                            assert!(phase != Phase::RootWritten, "injected publication panic");
                            Ok(())
                        },
                    );
                } else {
                    let _ = j.commit(
                        &[(1, Some(b"new")), (2, None), (3, Some(b"added"))],
                        |phase| {
                            assert!(phase != Phase::RootWritten, "injected publication panic");
                            Ok(())
                        },
                    );
                }
            }));
            assert!(result.is_err());
            assert!(!j.writable);
            assert!(j.commit(&[], |_| Ok(())).is_err());
            drop(j);
            let mut j = Journal::open(&path, false).unwrap();
            if j.index.generation == 1 {
                assert_eq!(j.read(1).unwrap().unwrap(), b"old");
                assert_eq!(j.read(2).unwrap().unwrap(), b"keep");
            } else {
                assert_eq!(j.index.generation, 2);
                assert_eq!(j.read(1).unwrap().unwrap(), b"new");
                assert!(j.read(2).unwrap().is_none());
                assert_eq!(j.read(3).unwrap().unwrap(), b"added");
            }
        }
    }
    #[test]
    fn streamed_documents_reuse_exact_bytes_with_crc_collisions_and_no_cursor_corruption() {
        // Equal-length CRC32 collision independently generated with Python zlib.
        let a = [0x64, 0xe1, 0xcd, 0xb0, 0x49, 0x63, 0x2f, 0xdd];
        let b = [0xac, 0x18, 0x6d, 0x24, 0x4b, 0x06, 0xa6, 0x8d];
        assert_ne!(a, b);
        assert_eq!(crc(&a), 0x6f162d3f);
        assert_eq!(crc(&a), crc(&b));
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("collision");
        let mut j = Journal::create(&path).unwrap();
        j.replace_stream(
            |emit| {
                emit(1, a.to_vec())?;
                emit(2, b"keep".to_vec())
            },
            |_| Ok(()),
        )
        .unwrap();
        let old = j.index.entries[&1].clone();
        let retained = j.index.entries[&2].clone();
        let (_, stats) = j
            .replace_stream(
                |emit| {
                    emit(1, b.to_vec())?;
                    emit(2, b"keep".to_vec())
                },
                |_| Ok(()),
            )
            .unwrap();
        assert_eq!((stats.changed, stats.reused), (1, 1));
        assert_ne!(j.index.entries[&1].offset, old.offset);
        assert_eq!(j.index.entries[&2], retained);
        let end = j.index.end;
        let (_, stats) = j
            .replace_stream(
                |emit| {
                    emit(2, b"keep".to_vec())?;
                    emit(1, b.to_vec())
                },
                |_| Ok(()),
            )
            .unwrap();
        assert_eq!((stats.changed, stats.reused), (0, 2));
        assert_eq!(j.index.end - end, 4 + 2 * 24);
        drop(j);
        let mut j = Journal::open(&path, false).unwrap();
        assert_eq!(j.read(1).unwrap().unwrap(), b);
        assert_eq!(j.read(2).unwrap().unwrap(), b"keep");
    }
    #[test]
    fn late_stream_failures_and_cancellation_keep_the_previous_document_and_forbid_retry() {
        for failure in 0..4 {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("journal");
            let mut j = Journal::create(&path).unwrap();
            let original = crate::indexed_document::fixture();
            let (keys, _) = j
                .replace_document(&original, &Default::default(), &|| false, |_| Ok(()))
                .unwrap();
            let old_end = j.index.end;
            let mut edited = original.clone();
            edited.layers[0].name = "Generated changed before failure".into();
            let result = if failure == 0 {
                j.replace_stream(
                    |emit| {
                        emit(1, b"partial replacement".to_vec())?;
                        Err::<(), _>("injected late encoder failure".into())
                    },
                    |_| Ok(()),
                )
                .map(|_| ())
            } else if failure == 1 {
                let checks = std::cell::Cell::new(0);
                j.replace_document(
                    &edited,
                    &keys,
                    &|| {
                        let value = checks.get() + 1;
                        checks.set(value);
                        value >= 4
                    },
                    |_| Ok(()),
                )
                .map(|_| ())
            } else if failure == 2 {
                j.replace_stream(
                    |emit| {
                        emit(1, b"partial replacement".to_vec())?;
                        emit(1, b"duplicate".to_vec())
                    },
                    |_| Ok(()),
                )
                .map(|_| ())
            } else {
                j.replace_stream(
                    |emit| {
                        emit(1, b"partial replacement".to_vec())?;
                        let mut excessive = Vec::with_capacity(MAX_RECORD * 2 + 1);
                        excessive.push(1);
                        emit(2, excessive)
                    },
                    |_| Ok(()),
                )
                .map(|_| ())
            };
            assert!(result.is_err());
            assert!(!j.writable);
            assert!(j.file.metadata().unwrap().len() > old_end);
            assert!(
                j.replace_document(&edited, &keys, &|| false, |_| Ok(()))
                    .is_err()
            );
            drop(j);
            let mut j = Journal::open(&path, true).unwrap();
            assert_eq!(j.index.generation, 1);
            assert!(crate::indexed_document::equivalent(&original, &document(&mut j)).unwrap());
            j.replace_document(&edited, &keys, &|| false, |_| Ok(()))
                .unwrap();
            drop(j);
            let mut j = Journal::open(&path, false).unwrap();
            assert!(crate::indexed_document::equivalent(&edited, &document(&mut j)).unwrap());
        }
    }
    #[test]
    fn streamed_document_faults_at_all_commit_boundaries_expose_one_complete_document() {
        for fault in [
            Phase::PayloadWritten,
            Phase::PayloadSynced,
            Phase::RootWritten,
            Phase::RootSynced,
        ] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("journal");
            let mut j = Journal::create(&path).unwrap();
            let original = crate::indexed_document::fixture();
            let (keys, _) = j
                .replace_document(&original, &Default::default(), &|| false, |_| Ok(()))
                .unwrap();
            let mut edited = original.clone();
            edited.layers.remove(0);
            edited.active = 1;
            edited.selection = None;
            edited.selection_mask = None;
            assert!(
                j.replace_document(&edited, &keys, &|| false, |phase| if phase == fault {
                    Err("injected publication failure".into())
                } else {
                    Ok(())
                })
                .is_err()
            );
            drop(j);
            let mut j = Journal::open(&path, false).unwrap();
            let generation = j.index.generation;
            let loaded = document(&mut j);
            let expected = if generation == 1 {
                &original
            } else {
                assert_eq!(generation, 2);
                &edited
            };
            assert!(crate::indexed_document::equivalent(expected, &loaded).unwrap());
        }
        // Cancellation after data sync but before pointer publication is safe.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cancel");
        let mut j = Journal::create(&path).unwrap();
        let original = crate::indexed_document::fixture();
        let (keys, _) = j
            .replace_document(&original, &Default::default(), &|| false, |_| Ok(()))
            .unwrap();
        let cancel = std::cell::Cell::new(false);
        let mut edited = original.clone();
        edited.layers[0].name = "Generated canceled".into();
        assert!(
            j.replace_document(&edited, &keys, &|| cancel.get(), |phase| {
                if phase == Phase::PayloadWritten {
                    cancel.set(true);
                }
                Ok(())
            })
            .is_err()
        );
        drop(j);
        let mut j = Journal::open(&path, false).unwrap();
        assert_eq!(j.index.generation, 1);
        assert!(crate::indexed_document::equivalent(&original, &document(&mut j)).unwrap());
    }
    fn initial(path: &Path) -> Journal {
        let mut j = Journal::create(path).unwrap();
        j.commit(&[(1, Some(b"old")), (2, Some(b"keep"))], |_| Ok(()))
            .unwrap();
        j
    }
    #[test]
    fn changed_records_append_unchanged_offsets_survive_and_deletions_are_index_only() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("journal");
        let mut j = initial(&path);
        let old = j.index.entries[&2].clone();
        let end = j.index.end;
        j.commit(&[(1, Some(b"new"))], |_| Ok(())).unwrap();
        assert_eq!(j.index.entries[&2], old);
        assert_eq!(j.index.end - end, 3 + 4 + 2 * 24);
        j.commit(&[(1, None)], |_| Ok(())).unwrap();
        drop(j);
        let mut j = Journal::open(&path, false).unwrap();
        assert_eq!(j.index.generation, 3);
        assert!(j.read(1).unwrap().is_none());
        assert_eq!(j.read(2).unwrap().unwrap(), b"keep");
    }
    #[test]
    fn interrupted_commit_recovers_one_whole_generation_and_requires_reopen() {
        for fault in [
            Phase::PayloadWritten,
            Phase::PayloadSynced,
            Phase::RootWritten,
            Phase::RootSynced,
        ] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("journal");
            let mut j = initial(&path);
            assert!(
                j.commit(
                    &[(1, Some(b"new")), (2, None), (3, Some(b"added"))],
                    |phase| if phase == fault {
                        Err("injected".into())
                    } else {
                        Ok(())
                    }
                )
                .is_err()
            );
            assert!(j.commit(&[], |_| Ok(())).is_err());
            drop(j);
            let mut j = Journal::open(&path, false).unwrap();
            match j.index.generation {
                1 => {
                    assert_eq!(j.read(1).unwrap().unwrap(), b"old");
                    assert_eq!(j.read(2).unwrap().unwrap(), b"keep");
                    assert!(j.read(3).unwrap().is_none());
                }
                2 => {
                    assert_eq!(j.read(1).unwrap().unwrap(), b"new");
                    assert!(j.read(2).unwrap().is_none());
                    assert_eq!(j.read(3).unwrap().unwrap(), b"added");
                }
                _ => panic!("partial generation"),
            }
        }
    }
    #[test]
    fn torn_latest_pointer_falls_back_but_cannot_silently_overwrite_recovery_evidence() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("journal");
        let mut j = initial(&path);
        j.commit(&[(1, Some(b"new"))], |_| Ok(())).unwrap();
        drop(j);
        let mut file = OpenOptions::new().write(true).open(&path).unwrap();
        file.seek(SeekFrom::Start(ROOT_OFFSETS[0] + 44)).unwrap();
        file.write_all(&[0; 4]).unwrap();
        drop(file);
        let before = std::fs::read(&path).unwrap();
        let mut j = Journal::open(&path, true).unwrap();
        assert!(j.index.damaged_slot);
        assert_eq!(j.index.generation, 1);
        assert_eq!(j.read(1).unwrap().unwrap(), b"old");
        assert!(j.commit(&[(1, Some(b"retry"))], |_| Ok(())).is_err());
        drop(j);
        assert_eq!(std::fs::read(&path).unwrap(), before);
    }
    #[test]
    fn corrupted_payload_is_an_error_and_reader_never_silently_rolls_back() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("journal");
        let j = initial(&path);
        let offset = j.index.entries[&1].offset;
        drop(j);
        let mut file = OpenOptions::new().write(true).open(&path).unwrap();
        file.seek(SeekFrom::Start(offset)).unwrap();
        file.write_all(b"bad").unwrap();
        drop(file);
        let mut j = Journal::open(&path, false).unwrap();
        assert!(j.read(1).is_err());
        assert_eq!(j.index.generation, 1);
    }
    #[test]
    fn every_partial_pointer_and_append_prefix_exposes_a_complete_generation() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source");
        let mut j = initial(&source);
        let old_end = j.index.end;
        j.commit(&[(1, Some(b"new")), (2, None), (3, Some(b"added"))], |_| {
            Ok(())
        })
        .unwrap();
        drop(j);
        let complete = std::fs::read(&source).unwrap();
        let path = dir.path().join("interrupted");
        // Generation two overwrites slot zero, which was previously empty.
        for prefix in 0..=64 {
            let mut damaged = complete.clone();
            let root = ROOT_OFFSETS[0] as usize;
            damaged[root + prefix..root + 64].fill(0);
            std::fs::write(&path, damaged).unwrap();
            let mut j = Journal::open(&path, false).unwrap();
            if j.index.generation == 1 {
                assert_eq!(j.read(1).unwrap().unwrap(), b"old");
                assert_eq!(j.read(2).unwrap().unwrap(), b"keep");
                assert!(j.read(3).unwrap().is_none());
            } else {
                assert_eq!(j.index.generation, 2);
                assert_eq!(j.read(1).unwrap().unwrap(), b"new");
                assert!(j.read(2).unwrap().is_none());
                assert_eq!(j.read(3).unwrap().unwrap(), b"added");
            }
        }
        for end in old_end..complete.len() as u64 {
            std::fs::write(&path, &complete[..end as usize]).unwrap();
            let mut j = Journal::open(&path, false).unwrap();
            assert_eq!(j.index.generation, 1);
            assert_eq!(j.read(1).unwrap().unwrap(), b"old");
        }
    }
    #[test]
    fn checksummed_malicious_manifest_references_are_bounded_and_ordered() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("journal");
        let j = initial(&path);
        drop(j);
        let original = std::fs::read(&path).unwrap();
        let root = ROOT_OFFSETS[1] as usize;
        let offset = u64_at(&original[root..root + 64], 16) as usize;
        for mutation in 0..3 {
            let mut bytes = original.clone();
            match mutation {
                0 => bytes[offset..offset + 4]
                    .copy_from_slice(&((MAX_KEYS + 1) as u32).to_le_bytes()),
                1 => bytes[offset + 12..offset + 20].copy_from_slice(&u64::MAX.to_le_bytes()),
                _ => {
                    let key = bytes[offset + 4..offset + 12].to_vec();
                    bytes[offset + 28..offset + 36].copy_from_slice(&key);
                }
            }
            let checksum = crc(&bytes[offset..]);
            bytes[root + 40..root + 44].copy_from_slice(&checksum.to_le_bytes());
            let checksum = root_crc(bytes[root..root + 64].try_into().unwrap());
            bytes[root + 44..root + 48].copy_from_slice(&checksum.to_le_bytes());
            std::fs::write(&path, bytes).unwrap();
            assert!(Journal::open(&path, false).is_err());
        }
    }
    #[test]
    fn competing_writer_and_invalid_transactions_cannot_change_committed_data() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("journal");
        let mut j = initial(&path);
        assert!(Journal::create(&path).is_err());
        assert!(Journal::open(&path, true).is_err());
        let end = j.index.end;
        for changes in [vec![(1, Some(&b""[..]))], vec![(1, None), (1, None)]] {
            assert!(j.commit(&changes, |_| Ok(())).is_err());
        }
        assert_eq!(j.index.end, end);
        assert_eq!(j.file.metadata().unwrap().len(), end);
        assert_eq!(j.read(1).unwrap().unwrap(), b"old");
    }
}
