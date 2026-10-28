//! Experimental independent RGBA tile codec, outside normal project saves.
use flate2::{Compression, Decompress, FlushDecompress, Status, write::ZlibEncoder};
use std::io::Write;

const MAGIC: &[u8; 8] = b"DITILE01";
const HEADER: usize = 32;
const MAX_RAW: usize = 256 * 256 * 4;

fn crc(bytes: &[u8]) -> u32 {
    let mut crc = flate2::Crc::new();
    crc.update(bytes);
    crc.sum()
}
fn geometry([width, height]: [u16; 2]) -> Result<usize, String> {
    if !(1..=256).contains(&width) || !(1..=256).contains(&height) {
        return Err("Indexed tile geometry exceeds bounds".into());
    }
    Ok(width as usize * height as usize * 4)
}
fn word(bytes: &[u8], at: usize) -> usize {
    u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap()) as usize
}

/// All-zero bytes are represented by absence. Nonzero hidden RGB is retained.
pub(crate) fn encode(size: [u16; 2], rgba: &[u8]) -> Result<Option<Vec<u8>>, String> {
    validate_rgba(size, rgba)?;
    if rgba.iter().all(|byte| *byte == 0) {
        return Ok(None);
    }
    let mut compressor = ZlibEncoder::new(Vec::new(), Compression::fast());
    compressor
        .write_all(rgba)
        .map_err(|error| error.to_string())?;
    let compressed = compressor.finish().map_err(|error| error.to_string())?;
    let (codec, payload) = if compressed.len() < rgba.len() {
        (1, compressed.as_slice())
    } else {
        (0, rgba)
    };
    let mut bytes = vec![0; HEADER];
    bytes[..8].copy_from_slice(MAGIC);
    bytes[8..10].copy_from_slice(&size[0].to_le_bytes());
    bytes[10..12].copy_from_slice(&size[1].to_le_bytes());
    bytes[12] = 4;
    bytes[13] = codec;
    bytes[16..20].copy_from_slice(&(rgba.len() as u32).to_le_bytes());
    bytes[20..24].copy_from_slice(&(payload.len() as u32).to_le_bytes());
    bytes[24..28].copy_from_slice(&crc(rgba).to_le_bytes());
    let checksum = crc(&bytes[..28]);
    bytes[28..32].copy_from_slice(&checksum.to_le_bytes());
    bytes.extend_from_slice(payload);
    Ok(Some(bytes))
}
pub(crate) fn validate_rgba(size: [u16; 2], rgba: &[u8]) -> Result<(), String> {
    if rgba.len() != geometry(size)? {
        return Err("Indexed tile RGBA length differs from geometry".into());
    }
    Ok(())
}
/// Validate a committed record using its own bounded geometry. Callers compare
/// that geometry with the new source before deciding whether it can be reused.
pub(crate) fn decode_record(bytes: &[u8]) -> Result<([u16; 2], Vec<u8>), String> {
    if bytes.len() < HEADER {
        return Err("Truncated indexed tile record".into());
    }
    let size = [
        u16::from_le_bytes(bytes[8..10].try_into().unwrap()),
        u16::from_le_bytes(bytes[10..12].try_into().unwrap()),
    ];
    Ok((size, decode(bytes, size)?))
}

pub(crate) fn decode(bytes: &[u8], expected: [u16; 2]) -> Result<Vec<u8>, String> {
    let raw_length = geometry(expected)?;
    if !(HEADER..=HEADER + MAX_RAW).contains(&bytes.len()) || &bytes[..8] != MAGIC {
        return Err("Invalid indexed tile record size or version".into());
    }
    let size = [
        u16::from_le_bytes(bytes[8..10].try_into().unwrap()),
        u16::from_le_bytes(bytes[10..12].try_into().unwrap()),
    ];
    if size != expected
        || bytes[12] != 4
        || bytes[14..16] != [0, 0]
        || word(bytes, 16) != raw_length
        || word(bytes, 20) != bytes.len() - HEADER
        || word(bytes, 28) as u32 != crc(&bytes[..28])
    {
        return Err("Indexed tile header/geometry/checksum mismatch".into());
    }
    let payload = &bytes[HEADER..];
    let decoded = match bytes[13] {
        0 if payload.len() == raw_length => payload.to_vec(),
        1 if !payload.is_empty() && payload.len() < raw_length => {
            // One extra byte detects oversized streams without expanding them.
            let mut output = vec![0; raw_length + 1];
            let mut decoder = Decompress::new(true);
            loop {
                let before = (decoder.total_in(), decoder.total_out());
                let status = decoder
                    .decompress(
                        &payload[before.0 as usize..],
                        &mut output[before.1 as usize..],
                        FlushDecompress::Finish,
                    )
                    .map_err(|error| format!("Invalid indexed tile zlib: {error}"))?;
                if status == Status::StreamEnd {
                    if decoder.total_in() != payload.len() as u64
                        || decoder.total_out() != raw_length as u64
                    {
                        return Err(
                            "Indexed tile zlib has trailing bytes or wrong decoded size".into()
                        );
                    }
                    output.truncate(raw_length);
                    break output;
                }
                if decoder.total_out() > raw_length as u64
                    || before == (decoder.total_in(), decoder.total_out())
                {
                    return Err("Indexed tile zlib is truncated or exceeds decoded size".into());
                }
            }
        }
        _ => return Err("Unknown indexed tile codec or noncanonical payload length".into()),
    };
    if crc(&decoded) != word(bytes, 24) as u32 || decoded.iter().all(|byte| *byte == 0) {
        return Err("Indexed tile payload checksum differs or an empty tile was stored".into());
    }
    Ok(decoded)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn reseal(bytes: &mut [u8]) {
        let checksum = crc(&bytes[..28]);
        bytes[28..32].copy_from_slice(&checksum.to_le_bytes());
    }
    #[test]
    fn edges_alpha_hidden_rgb_and_incompressible_tiles_round_trip_exactly() {
        for size in [[1, 1], [3, 17], [256, 1], [1, 256], [255, 256], [256, 256]] {
            let mut state = 0x12345678u32;
            let raw: Vec<u8> = (0..geometry(size).unwrap())
                .map(|_| {
                    state ^= state << 13;
                    state ^= state >> 17;
                    state ^= state << 5;
                    state as u8
                })
                .collect();
            let encoded = encode(size, &raw).unwrap().unwrap();
            assert_eq!(decode(&encoded, size).unwrap(), raw);
            if size == [256, 256] {
                assert_eq!(encoded[13], 0);
                assert_eq!(encoded.len(), HEADER + MAX_RAW);
            }
        }
        let raw = [23, 45, 67, 0];
        let encoded = encode([1, 1], &raw).unwrap().unwrap();
        assert_eq!(decode(&encoded, [1, 1]).unwrap(), raw);
        assert!(encode([256, 256], &vec![0; MAX_RAW]).unwrap().is_none());
        let raw = vec![77; MAX_RAW];
        let encoded = encode([256, 256], &raw).unwrap().unwrap();
        assert_eq!(encoded[13], 1);
        assert!(encoded.len() < MAX_RAW / 10);
        assert_eq!(decode(&encoded, [256, 256]).unwrap(), raw);
    }
    #[test]
    fn every_truncation_and_header_corruption_fails_without_partial_output() {
        let encoded = encode([32, 32], &vec![89; 32 * 32 * 4]).unwrap().unwrap();
        for end in 0..encoded.len() {
            assert!(
                decode(&encoded[..end], [32, 32]).is_err(),
                "accepted prefix {end}"
            );
        }
        for at in 0..HEADER {
            let mut damaged = encoded.clone();
            damaged[at] ^= 1;
            assert!(decode(&damaged, [32, 32]).is_err());
        }
        assert!(decode(&encoded, [31, 32]).is_err());
        for size in [[0, 1], [1, 0], [257, 1], [1, 257]] {
            assert!(encode(size, &[]).is_err());
            assert!(decode(&encoded, size).is_err());
        }
        assert!(encode([1, 1], &[0; 3]).is_err());
    }
    #[test]
    fn checksummed_headers_cannot_enable_unknown_codecs_unbounded_sizes_or_bad_payloads() {
        let encoded = encode([32, 32], &vec![89; 32 * 32 * 4]).unwrap().unwrap();
        for mutation in 0..5 {
            let mut damaged = encoded.clone();
            match mutation {
                0 => damaged[13] = 2,
                1 => damaged[14] = 1,
                2 => damaged[16..20].copy_from_slice(&u32::MAX.to_le_bytes()),
                3 => damaged[20..24].copy_from_slice(&u32::MAX.to_le_bytes()),
                _ => damaged[24] ^= 1,
            }
            reseal(&mut damaged);
            assert!(decode(&damaged, [32, 32]).is_err());
        }
        let mut damaged = encoded.clone();
        *damaged.last_mut().unwrap() ^= 1;
        assert!(decode(&damaged, [32, 32]).is_err());
        let mut extra = encoded.clone();
        extra.push(0);
        let size = (extra.len() - HEADER) as u32;
        extra[20..24].copy_from_slice(&size.to_le_bytes());
        reseal(&mut extra);
        assert!(decode(&extra, [32, 32]).is_err());
        let mut oversized = encoded;
        oversized[8..10].copy_from_slice(&16u16.to_le_bytes());
        oversized[10..12].copy_from_slice(&16u16.to_le_bytes());
        oversized[16..20].copy_from_slice(&1024u32.to_le_bytes());
        reseal(&mut oversized);
        assert!(
            decode(&oversized, [16, 16])
                .unwrap_err()
                .contains("exceeds decoded size")
        );
        let mut undersized = oversized;
        undersized[8..10].copy_from_slice(&64u16.to_le_bytes());
        undersized[10..12].copy_from_slice(&64u16.to_le_bytes());
        undersized[16..20].copy_from_slice(&16384u32.to_le_bytes());
        reseal(&mut undersized);
        assert!(
            decode(&undersized, [64, 64])
                .unwrap_err()
                .contains("wrong decoded size")
        );
    }
    #[test]
    fn independent_zlib_vector_concatenated_streams_and_noncanonical_empty_records() {
        // Python zlib.compress(bytes([17])*256), crc32 == 0x1dc7b163.
        let vector = [120, 156, 19, 20, 28, 217, 0, 0, 137, 248, 17, 1];
        let mut encoded = encode([8, 8], &[17; 256]).unwrap().unwrap();
        encoded.truncate(HEADER);
        encoded.extend_from_slice(&vector);
        encoded[20..24].copy_from_slice(&(vector.len() as u32).to_le_bytes());
        encoded[24..28].copy_from_slice(&0x1dc7b163u32.to_le_bytes());
        reseal(&mut encoded);
        assert_eq!(decode(&encoded, [8, 8]).unwrap(), [17; 256]);
        let mut concatenated = encoded;
        concatenated.extend_from_slice(&vector);
        concatenated[20..24].copy_from_slice(&((2 * vector.len()) as u32).to_le_bytes());
        reseal(&mut concatenated);
        assert!(
            decode(&concatenated, [8, 8])
                .unwrap_err()
                .contains("trailing bytes")
        );
        let mut empty = encode([1, 1], &[1, 2, 3, 4]).unwrap().unwrap();
        empty[HEADER..].fill(0);
        empty[24..28].copy_from_slice(&crc(&[0; 4]).to_le_bytes());
        reseal(&mut empty);
        assert!(decode(&empty, [1, 1]).unwrap_err().contains("empty tile"));
    }
}
