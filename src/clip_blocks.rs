//! Validates the observed 256-square, five-byte block envelope without assigning color semantics.
use crate::clip_container;
use flate2::{Decompress, FlushDecompress, Status};
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::Path,
};

const TILE_BYTES: usize = 256 * 256 * 5;
/// Gray layers store one color byte per pixel after the alpha plane.
const GRAY_TILE_BYTES: usize = 256 * 256 * 2;
const MAX_SLOTS: usize = 256;
const MAX_COMPRESSED: usize = 1024 * 1024;
const MAX_BODY: u64 = 64 * 1024 * 1024;
pub(crate) struct Budget {
    slots: usize,
    decoded: usize,
}
impl Default for Budget {
    /// Enough for the largest document Dudel can hold: every raster layer a
    /// full-size canvas. Tiles inflate one at a time, so this bounds work,
    /// not memory held at once.
    fn default() -> Self {
        let slots = crate::document::MAX_RASTER_LAYERS * MAX_SLOTS;
        Self {
            slots,
            decoded: slots * TILE_BYTES,
        }
    }
}

#[derive(Debug, serde::Serialize)]
struct Block {
    index: u32,
    populated: bool,
    compressed_bytes: usize,
    decoded_bytes: usize,
    /// Relative to the start of the external body, including its length prefix.
    compressed_offset: Option<u64>,
    envelope_adler32: u32,
}
#[derive(Debug, serde::Serialize)]
struct Blocks {
    body_bytes: u64,
    blocks: Vec<Block>,
    status_values: Vec<u32>,
    populated: usize,
    decoded_bytes: usize,
}
pub(crate) fn validate_layout(
    reader: &mut impl Read,
    bytes: u64,
    sizes: &[u32],
    tile_bytes: usize,
    budget: &mut Budget,
) -> Result<(), String> {
    let parsed = scan_each(
        reader,
        bytes,
        Some(tile_bytes),
        budget,
        &|| false,
        &mut |_, _| Ok(()),
    )?;
    if parsed.blocks.len() != sizes.len()
        || parsed.blocks.iter().zip(sizes).any(|(block, size)| {
            *size as usize
                != if block.populated {
                    112 + block.compressed_bytes
                } else {
                    104
                }
        })
    {
        return Err("Offscreen BlockSize table does not match external block records".into());
    }
    Ok(())
}
#[derive(serde::Serialize)]
struct External {
    id: String,
    chunk_offset: u64,
    blocks: Blocks,
}
#[derive(serde::Serialize)]
struct Report {
    scope: &'static str,
    externals: Vec<External>,
}

fn bytes<const N: usize>(reader: &mut impl Read) -> Result<[u8; N], String> {
    let mut data = [0; N];
    reader
        .read_exact(&mut data)
        .map_err(|error| format!("Truncated clip block envelope: {error}"))?;
    Ok(data)
}
fn word(reader: &mut impl Read) -> Result<u32, String> {
    Ok(u32::from_be_bytes(bytes(reader)?))
}
fn name(reader: &mut impl Read, expected: &str) -> Result<(), String> {
    if word(reader)? as usize != expected.len() {
        return Err("Unexpected clip block tag length".into());
    }
    for character in expected.bytes() {
        if bytes::<2>(reader)? != [0, character] {
            return Err("Unexpected clip block tag".into());
        }
    }
    Ok(())
}

// RFC 1950 Adler-32, with reductions in bounded batches to avoid u32 overflow.
fn adler32(data: &[u8]) -> u32 {
    let mut a = 1u32;
    let mut b = 0u32;
    for batch in data.chunks(5552) {
        for byte in batch {
            a += *byte as u32;
            b += a;
        }
        a %= 65521;
        b %= 65521;
    }
    (b << 16) | a
}

fn decode_zlib(data: &[u8], tile_bytes: usize) -> Result<Vec<u8>, String> {
    if data.len() > MAX_COMPRESSED {
        return Err("Clip compressed block exceeds limit".into());
    }
    // One extra byte detects expansion beyond the exact observed block layout.
    let mut output = vec![0; tile_bytes + 1];
    let mut decoder = Decompress::new(true);
    loop {
        let before = (decoder.total_in(), decoder.total_out());
        let status = decoder
            .decompress(
                &data[before.0 as usize..],
                &mut output[before.1 as usize..],
                FlushDecompress::Finish,
            )
            .map_err(|error| format!("Invalid clip zlib stream: {error}"))?;
        if status == Status::StreamEnd {
            if decoder.total_in() != data.len() as u64 || decoder.total_out() != tile_bytes as u64 {
                return Err("Clip zlib block has trailing data or wrong decoded length".into());
            }
            output.truncate(tile_bytes);
            return Ok(output);
        }
        if decoder.total_out() > tile_bytes as u64
            || before == (decoder.total_in(), decoder.total_out())
        {
            return Err("Clip zlib stream is truncated or exceeds decoded block size".into());
        }
    }
}

fn array(reader: &mut impl Read, tag: &str, count: usize) -> Result<Vec<u32>, String> {
    name(reader, tag)?;
    if word(reader)? != 12 || word(reader)? as usize != count || word(reader)? != 4 {
        return Err("Invalid clip block array header/count/element width".into());
    }
    (0..count).map(|_| word(reader)).collect()
}

fn scan(reader: &mut impl Read, body_bytes: u64, budget: &mut Budget) -> Result<Blocks, String> {
    scan_each(reader, body_bytes, None, budget, &|| false, &mut |_, _| {
        Ok(())
    })
}

// Consumers must keep their result unpublished and discard it on any error,
// including the status/checksum arrays that follow the pixel records.
pub(crate) fn decode_layout(
    reader: &mut impl Read,
    body_bytes: u64,
    sizes: &[u32],
    tile_bytes: usize,
    budget: &mut Budget,
    cancelled: &impl Fn() -> bool,
    consume: &mut impl FnMut(u32, &[u8]) -> Result<(), String>,
) -> Result<(), String> {
    let parsed = scan_each(
        reader,
        body_bytes,
        Some(tile_bytes),
        budget,
        cancelled,
        consume,
    )?;
    if parsed.blocks.len() != sizes.len()
        || parsed.blocks.iter().zip(sizes).any(|(block, size)| {
            *size as usize
                != if block.populated {
                    112 + block.compressed_bytes
                } else {
                    104
                }
        })
    {
        return Err("Offscreen BlockSize table does not match external block records".into());
    }
    Ok(())
}

/// `tile_bytes` is the layer's decoded tile size when known; a bare block
/// inspection accepts either observed size as each block declares it.
fn scan_each(
    reader: &mut impl Read,
    body_bytes: u64,
    tile_bytes: Option<usize>,
    budget: &mut Budget,
    cancelled: &impl Fn() -> bool,
    consume: &mut impl FnMut(u32, &[u8]) -> Result<(), String>,
) -> Result<Blocks, String> {
    if !(8..=MAX_BODY).contains(&body_bytes) {
        return Err("Clip external block body exceeds inspection bounds".into());
    }
    let mut reader = reader.take(body_bytes);
    if u64::from_be_bytes(bytes(&mut reader)?) != body_bytes - 8 {
        return Err("Clip external body declared length mismatch".into());
    }
    let mut blocks = Vec::<Block>::new();
    loop {
        if cancelled() {
            return Err("Clip raster decoding canceled".into());
        }
        let start = body_bytes - reader.limit();
        let prefix = word(&mut reader)?;
        // The observed status array follows the complete sequential block list.
        if prefix == 11 {
            let mut status_reader = std::io::Cursor::new(prefix.to_be_bytes()).chain(&mut reader);
            let status_values = array(&mut status_reader, "BlockStatus", blocks.len())?;
            if blocks.is_empty()
                || ![0, 1].contains(&status_values[0])
                || status_values.iter().any(|value| *value != status_values[0])
            {
                return Err(
                    "Unsupported clip block status: expected a uniform opaque 0 or 1".into(),
                );
            }
            let checksums = array(&mut reader, "BlockCheckSum", blocks.len())?;
            for (block, expected) in blocks.iter().zip(checksums) {
                if block.envelope_adler32 != expected {
                    return Err(format!(
                        "Clip block {} envelope checksum mismatch",
                        block.index
                    ));
                }
            }
            if reader.limit() != 0 {
                return Err("Trailing clip block envelope data".into());
            }
            let populated = blocks.iter().filter(|block| block.populated).count();
            let decoded_bytes = blocks.iter().map(|block| block.decoded_bytes).sum();
            return Ok(Blocks {
                body_bytes,
                blocks,
                status_values,
                populated,
                decoded_bytes,
            });
        }
        if blocks.len() >= MAX_SLOTS || prefix < 104 || prefix as u64 > reader.limit() + 4 {
            return Err("Invalid clip block size/count".into());
        }
        budget.slots = budget
            .slots
            .checked_sub(1)
            .ok_or("Clip block inspection exceeds total slot budget")?;
        let mut record = (&mut reader).take(prefix as u64 - 4);
        name(&mut record, "BlockDataBeginChunk")?;
        let index = word(&mut record)?;
        let declared = word(&mut record)? as usize;
        if index as usize != blocks.len()
            || ![TILE_BYTES, GRAY_TILE_BYTES].contains(&declared)
            || tile_bytes.is_some_and(|expected| expected != declared)
            || word(&mut record)? != 256
            || word(&mut record)? != 256
        {
            return Err("Unsupported clip block index/dimensions/decoded size".into());
        }
        let present = word(&mut record)?;
        let mut block = Block {
            index,
            populated: present == 1,
            compressed_bytes: 0,
            decoded_bytes: 0,
            compressed_offset: None,
            envelope_adler32: 0,
        };
        match present {
            0 => {
                if prefix != 104 {
                    return Err("Empty clip block has unexpected payload".into());
                }
            }
            1 => {
                budget.decoded = budget
                    .decoded
                    .checked_sub(declared)
                    .ok_or("Clip block inspection exceeds total inflate budget")?;
                let wrapper_size = word(&mut record)? as usize;
                let little_size = bytes::<4>(&mut record)?;
                let compressed_size = u32::from_le_bytes(little_size) as usize;
                if compressed_size == 0
                    || compressed_size > MAX_COMPRESSED
                    || wrapper_size != compressed_size + 4
                    || prefix as usize != 112 + compressed_size
                {
                    return Err("Invalid clip compressed lengths".into());
                }
                let mut compressed = vec![0; compressed_size + 4];
                compressed[..4].copy_from_slice(&little_size);
                record
                    .read_exact(&mut compressed[4..])
                    .map_err(|error| error.to_string())?;
                block.envelope_adler32 = adler32(&compressed);
                // No raw artwork is retained or written by this inspector.
                let decoded = decode_zlib(&compressed[4..], declared)?;
                if cancelled() {
                    return Err("Clip raster decoding canceled".into());
                }
                consume(index, &decoded)?;
                block.compressed_bytes = compressed_size;
                block.decoded_bytes = decoded.len();
                block.compressed_offset = Some(start + 74);
            }
            _ => return Err("Unsupported clip block presence flag".into()),
        }
        name(&mut record, "BlockDataEndChunk")?;
        if record.limit() != 0 {
            return Err("Trailing clip block record data".into());
        }
        blocks.push(block);
    }
}

pub(crate) fn inspect(input: &Path, output: Option<&Path>) -> Result<(), String> {
    let mut source = File::open(input).map_err(|error| error.to_string())?;
    let length = source.metadata().map_err(|error| error.to_string())?.len();
    let inventory = clip_container::scan(&mut source, length)?;
    let ids = inventory.external_ids(&mut source)?;
    let mut externals = Vec::new();
    let mut budget = Budget::default();
    for (id, chunk_offset) in ids {
        let (offset, body_bytes) = inventory.external_body_bounds(chunk_offset, id.len())?;
        source
            .seek(SeekFrom::Start(offset))
            .map_err(|error| error.to_string())?;
        let blocks = scan(&mut source, body_bytes, &mut budget)
            .map_err(|error| format!("External chunk {chunk_offset}: {error}"))?;
        externals.push(External {
            id,
            chunk_offset,
            blocks,
        });
    }
    clip_container::write_report(
        &Report {
            scope: "Observed 256x256 five-byte block framing, zlib streams and envelope checksums only; no color/channel interpretation, complete feature validation or document import.",
            externals,
        },
        output,
    )
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::io::Write;
    fn tag(out: &mut Vec<u8>, name: &str) {
        out.extend((name.len() as u32).to_be_bytes());
        for byte in name.bytes() {
            out.extend([0, byte]);
        }
    }
    fn zlib(raw: &[u8]) -> Vec<u8> {
        let mut writer = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
        writer.write_all(raw).unwrap();
        writer.finish().unwrap()
    }
    pub(crate) fn fixture(streams: &[Option<Vec<u8>>]) -> Vec<u8> {
        fixture_sized(streams, TILE_BYTES)
    }
    pub(crate) fn fixture_sized(streams: &[Option<Vec<u8>>], tile_bytes: usize) -> Vec<u8> {
        let mut data = vec![0; 8];
        let mut checksums = Vec::new();
        for (index, stream) in streams.iter().enumerate() {
            let size = 104 + stream.as_ref().map_or(0, |stream| 8 + stream.len());
            data.extend((size as u32).to_be_bytes());
            tag(&mut data, "BlockDataBeginChunk");
            for value in [
                index as u32,
                tile_bytes as u32,
                256,
                256,
                u32::from(stream.is_some()),
            ] {
                data.extend(value.to_be_bytes());
            }
            if let Some(stream) = stream {
                data.extend((stream.len() as u32 + 4).to_be_bytes());
                let mut wrapper = (stream.len() as u32).to_le_bytes().to_vec();
                wrapper.extend(stream);
                checksums.push(adler32(&wrapper));
                data.extend(wrapper);
            } else {
                checksums.push(0);
            }
            tag(&mut data, "BlockDataEndChunk");
        }
        for (name, values) in [
            ("BlockStatus", vec![1; streams.len()]),
            ("BlockCheckSum", checksums),
        ] {
            tag(&mut data, name);
            for value in [12, streams.len() as u32, 4] {
                data.extend(value.to_be_bytes());
            }
            for value in values {
                data.extend(value.to_be_bytes());
            }
        }
        let size = (data.len() - 8) as u64;
        data[..8].copy_from_slice(&size.to_be_bytes());
        data
    }
    fn checked(data: &[u8]) -> Result<Blocks, String> {
        scan(&mut &data[..], data.len() as u64, &mut Budget::default())
    }
    fn set_word(data: &mut [u8], at: usize, value: u32) {
        data[at..at + 4].copy_from_slice(&value.to_be_bytes());
    }

    #[test]
    fn sparse_slots_stream_offsets_checksums_and_exact_raw_payload() {
        let raw: Vec<_> = (0..TILE_BYTES)
            .map(|index| (index * 17 + index / 256) as u8)
            .collect();
        let stream = zlib(&raw);
        let data = fixture(&[None, Some(stream.clone()), None]);
        let report = checked(&data).unwrap();
        assert_eq!(
            (report.blocks.len(), report.populated, report.decoded_bytes),
            (3, 1, TILE_BYTES)
        );
        assert_eq!(report.status_values, [1, 1, 1]);
        assert!(report.blocks[0].compressed_offset.is_none());
        let block = &report.blocks[1];
        assert_eq!(block.compressed_offset, Some(186));
        let start = block.compressed_offset.unwrap() as usize;
        assert_eq!(&data[start..start + block.compressed_bytes], stream);
        assert_eq!(decode_zlib(&stream, TILE_BYTES).unwrap(), raw);
        assert_eq!(
            block.envelope_adler32,
            adler32(&data[start - 4..start + stream.len()])
        );
    }
    #[test]
    fn uniform_zero_status_preserves_payload_and_size_table_must_match() {
        let stream = zlib(&vec![19; TILE_BYTES]);
        let mut data = fixture(&[None, Some(stream.clone())]);
        let status_tag: Vec<_> = "BlockStatus".bytes().flat_map(|byte| [0, byte]).collect();
        let start = data
            .windows(status_tag.len())
            .position(|value| value == status_tag)
            .unwrap()
            + status_tag.len()
            + 12;
        set_word(&mut data, start, 0);
        assert!(
            checked(&data).is_err(),
            "mixed statuses must remain unsupported"
        );
        set_word(&mut data, start + 4, 0);
        let report = checked(&data).unwrap();
        assert_eq!(report.status_values, [0, 0]);
        assert_eq!(report.populated, 1);
        assert_eq!(
            decode_zlib(&stream, TILE_BYTES).unwrap(),
            vec![19; TILE_BYTES]
        );
        let sizes = [104, 112 + stream.len() as u32];
        validate_layout(
            &mut &data[..],
            data.len() as u64,
            &sizes,
            TILE_BYTES,
            &mut Budget::default(),
        )
        .unwrap();
        for sizes in [
            vec![104],
            vec![104, 104],
            vec![108, 112 + stream.len() as u32],
        ] {
            assert!(
                validate_layout(
                    &mut &data[..],
                    data.len() as u64,
                    &sizes,
                    TILE_BYTES,
                    &mut Budget::default()
                )
                .is_err()
            );
        }
        // A layer whose packing says Gray must not accept RGBA tiles.
        assert!(
            validate_layout(
                &mut &data[..],
                data.len() as u64,
                &sizes,
                GRAY_TILE_BYTES,
                &mut Budget::default()
            )
            .is_err()
        );
    }

    #[test]
    fn truncations_bad_geometry_flags_order_and_array_headers_are_rejected() {
        let data = fixture(&[None]);
        for end in 0..data.len() {
            assert!(checked(&data[..end]).is_err(), "accepted prefix {end}");
        }
        for (at, value) in [
            (8, 103),
            (8, u32::MAX),
            (12, 18),
            (54, 1),
            (58, TILE_BYTES as u32 + 1),
            (62, 128),
            (66, 128),
            (70, 2),
            (74, 16),
            (112, 10),
            (138, 13),
            (142, 2),
            (146, 8),
            (150, 2),
        ] {
            let mut changed = data.clone();
            set_word(&mut changed, at, value);
            assert!(checked(&changed).is_err(), "accepted change {at}={value}");
        }
        let mut trailing = data.clone();
        trailing.push(0);
        let len = (trailing.len() - 8) as u64;
        trailing[..8].copy_from_slice(&len.to_be_bytes());
        assert!(checked(&trailing).unwrap_err().contains("Trailing"));
        let mut bad_checksum = data.clone();
        *bad_checksum.last_mut().unwrap() = 1;
        assert!(checked(&bad_checksum).unwrap_err().contains("checksum"));
        assert!(checked(&fixture(&[])).is_err());
    }

    #[test]
    fn outer_checksum_inner_checksum_and_compressed_lengths_are_independent() {
        let stream = zlib(&vec![21; TILE_BYTES]);
        let data = fixture(&[Some(stream.clone())]);
        let mut bad_checksum = data.clone();
        *bad_checksum.last_mut().unwrap() ^= 1;
        assert!(
            checked(&bad_checksum)
                .unwrap_err()
                .contains("envelope checksum")
        );
        for at in [74, 78] {
            let mut changed = data.clone();
            changed[at] ^= 1;
            assert!(checked(&changed).is_err());
        }
        // Rebuild the correct envelope checksum around a corrupt zlib trailer.
        let mut broken_stream = stream.clone();
        *broken_stream.last_mut().unwrap() ^= 1;
        assert!(
            checked(&fixture(&[Some(broken_stream)]))
                .unwrap_err()
                .contains("zlib")
        );
        let mut extra = stream.clone();
        extra.push(0);
        assert!(
            checked(&fixture(&[Some(extra)]))
                .unwrap_err()
                .contains("trailing")
        );
        let mut concatenated = stream.clone();
        concatenated.extend(&stream);
        assert!(checked(&fixture(&[Some(concatenated)])).is_err());
        assert!(checked(&fixture(&[Some(stream[..stream.len() - 1].to_vec())])).is_err());
        let mut invalid_empty = fixture(&[None]);
        set_word(&mut invalid_empty, 8, 108);
        assert!(checked(&invalid_empty).is_err());
    }

    #[test]
    fn inflation_and_count_limits_reject_before_unbounded_work() {
        assert!(decode_zlib(&zlib(&vec![0; TILE_BYTES - 1]), TILE_BYTES).is_err());
        assert!(decode_zlib(&zlib(&vec![0; TILE_BYTES + 1]), TILE_BYTES).is_err());
        assert!(decode_zlib(&zlib(&vec![0; TILE_BYTES * 4]), TILE_BYTES).is_err());
        assert!(decode_zlib(&vec![0; MAX_COMPRESSED + 1], TILE_BYTES).is_err());
        assert!(scan(&mut std::io::empty(), MAX_BODY + 1, &mut Budget::default()).is_err());
        assert!(
            checked(&fixture(&vec![None; MAX_SLOTS + 1]))
                .unwrap_err()
                .contains("size/count")
        );
        let empty = fixture(&[None]);
        let populated = fixture(&[Some(zlib(&vec![0; TILE_BYTES]))]);
        let mut slot_budget = Budget {
            slots: 0,
            decoded: usize::MAX,
        };
        assert!(
            scan(&mut &empty[..], empty.len() as u64, &mut slot_budget)
                .unwrap_err()
                .contains("slot budget")
        );
        let mut inflate_budget = Budget {
            slots: 10,
            decoded: TILE_BYTES - 1,
        };
        assert!(
            scan(
                &mut &populated[..],
                populated.len() as u64,
                &mut inflate_budget
            )
            .unwrap_err()
            .contains("inflate budget")
        );
        let mut shared = Budget {
            slots: 2,
            decoded: TILE_BYTES,
        };
        scan(&mut &populated[..], populated.len() as u64, &mut shared).unwrap();
        assert!(scan(&mut &populated[..], populated.len() as u64, &mut shared).is_err());
    }

    #[test]
    fn adler32_matches_independent_known_vectors_and_handles_max_batches() {
        assert_eq!(adler32(b""), 1);
        assert_eq!(adler32(b"Wikipedia"), 0x11e60398);
        assert_eq!(adler32(&vec![255; 5552]), 0xf18f9b8c);
        // Verified independently with Python zlib.adler32, not this fixture writer.
        assert_eq!(adler32(&vec![255; TILE_BYTES]), 0x17ce4ab6);
    }
}
