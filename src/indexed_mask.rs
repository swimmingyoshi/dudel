//! Experimental independent R8 selection-page codec, outside normal project saves.
use flate2::{Compression, Decompress, FlushDecompress, Status, write::ZlibEncoder};
use std::io::Write;

const MAGIC: &[u8; 8] = b"DIMASK02";
const HEADER: usize = 32;
pub(crate) const MAX_BYTES: usize = HEADER + 256 * 256;
const MAX_RAW: usize = 256 * 256;

fn crc(bytes: &[u8]) -> u32 {
    let mut crc = flate2::Crc::new();
    crc.update(bytes);
    crc.sum()
}
fn geometry([width, height]: [u16; 2]) -> Result<usize, String> {
    if !(1..=256).contains(&width) || !(1..=256).contains(&height) {
        return Err("Indexed mask geometry exceeds bounds".into());
    }
    Ok(width as usize * height as usize)
}
fn word(bytes: &[u8], at: usize) -> usize {
    u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap()) as usize
}

/// All-zero bytes are represented by absence. All nonzero feathered coverage values are retained.
pub(crate) fn encode(size: [u16; 2], coverage: &[u8]) -> Result<Option<Vec<u8>>, String> {
    validate_coverage(size, coverage)?;
    if coverage.iter().all(|byte| *byte == 0) {
        return Ok(None);
    }
    let mut compressor = ZlibEncoder::new(Vec::new(), Compression::fast());
    compressor
        .write_all(coverage)
        .map_err(|error| error.to_string())?;
    let compressed = compressor.finish().map_err(|error| error.to_string())?;
    let (codec, payload) = if compressed.len() < coverage.len() {
        (1, compressed.as_slice())
    } else {
        (0, coverage)
    };
    let mut bytes = vec![0; HEADER];
    bytes[..8].copy_from_slice(MAGIC);
    bytes[8..10].copy_from_slice(&size[0].to_le_bytes());
    bytes[10..12].copy_from_slice(&size[1].to_le_bytes());
    bytes[12] = 1;
    bytes[13] = codec;
    bytes[16..20].copy_from_slice(&(coverage.len() as u32).to_le_bytes());
    bytes[20..24].copy_from_slice(&(payload.len() as u32).to_le_bytes());
    bytes[24..28].copy_from_slice(&crc(coverage).to_le_bytes());
    let checksum = crc(&bytes[..28]);
    bytes[28..32].copy_from_slice(&checksum.to_le_bytes());
    bytes.extend_from_slice(payload);
    Ok(Some(bytes))
}
pub(crate) fn validate_coverage(size: [u16; 2], coverage: &[u8]) -> Result<(), String> {
    if coverage.len() != geometry(size)? {
        return Err("Indexed mask coverage length differs from geometry".into());
    }
    Ok(())
}
pub(crate) fn decode(bytes: &[u8], expected: [u16; 2]) -> Result<Vec<u8>, String> {
    let raw_length = geometry(expected)?;
    if !(HEADER..=HEADER + MAX_RAW).contains(&bytes.len()) || &bytes[..8] != MAGIC {
        return Err("Invalid indexed mask record size or version".into());
    }
    let size = [
        u16::from_le_bytes(bytes[8..10].try_into().unwrap()),
        u16::from_le_bytes(bytes[10..12].try_into().unwrap()),
    ];
    if size != expected
        || bytes[12] != 1
        || bytes[14..16] != [0, 0]
        || word(bytes, 16) != raw_length
        || word(bytes, 20) != bytes.len() - HEADER
        || word(bytes, 28) as u32 != crc(&bytes[..28])
    {
        return Err("Indexed mask header/geometry/checksum mismatch".into());
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
                    .map_err(|error| format!("Invalid indexed mask zlib: {error}"))?;
                if status == Status::StreamEnd {
                    if decoder.total_in() != payload.len() as u64
                        || decoder.total_out() != raw_length as u64
                    {
                        return Err(
                            "Indexed mask zlib has trailing bytes or wrong decoded size".into()
                        );
                    }
                    output.truncate(raw_length);
                    break output;
                }
                if decoder.total_out() > raw_length as u64
                    || before == (decoder.total_in(), decoder.total_out())
                {
                    return Err("Indexed mask zlib is truncated or exceeds decoded size".into());
                }
            }
        }
        _ => return Err("Unknown indexed mask codec or noncanonical payload length".into()),
    };
    if crc(&decoded) != word(bytes, 24) as u32 || decoded.iter().all(|byte| *byte == 0) {
        return Err("Indexed mask payload checksum differs or an empty page was stored".into());
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
    fn feathered_edges_empty_solid_and_noise_round_trip_with_bounded_records() {
        for shape in [[1, 1], [1, 256], [256, 1], [3, 17], [255, 256], [256, 256]] {
            let count = geometry(shape).unwrap();
            let mut state = 0x12345678u32;
            let noise: Vec<u8> = (0..count)
                .map(|_| {
                    state ^= state << 13;
                    state ^= state >> 17;
                    state ^= state << 5;
                    state as u8
                })
                .collect();
            for raw in [
                noise,
                vec![127; count],
                (0..count).map(|n| n as u8).collect(),
            ] {
                let encoded = encode(shape, &raw).unwrap();
                if raw.iter().all(|v| *v == 0) {
                    assert!(encoded.is_none());
                } else {
                    let encoded = encoded.unwrap();
                    assert!(encoded.len() <= HEADER + count);
                    assert_eq!(decode(&encoded, shape).unwrap(), raw);
                }
            }
            assert!(encode(shape, &vec![0; count]).unwrap().is_none());
        }
        let solid = encode([256, 256], &vec![255; MAX_RAW]).unwrap().unwrap();
        assert_eq!(solid[13], 1);
        assert!(solid.len() < 1024);
        assert!(encode([0, 1], &[]).is_err());
        assert!(encode([257, 1], &[1; 257]).is_err());
        assert!(encode([2, 2], &[1; 3]).is_err());
    }
    #[test]
    fn independent_python_zlib_vector_and_hostile_streams_are_strictly_bounded() {
        // Python zlib.compress(bytes([0,1,127,255])*64,1), not this encoder.
        let payload = [
            0x78, 0x01, 0x63, 0x60, 0xac, 0xff, 0xcf, 0x30, 0x82, 0x31, 0x00, 0xa4, 0x41, 0x5f,
            0xc1,
        ];
        let raw = [0, 1, 127, 255].repeat(64);
        let mut encoded = vec![0; HEADER];
        encoded[..8].copy_from_slice(MAGIC);
        encoded[8..10].copy_from_slice(&16u16.to_le_bytes());
        encoded[10..12].copy_from_slice(&16u16.to_le_bytes());
        encoded[12] = 1;
        encoded[13] = 1;
        encoded[16..20].copy_from_slice(&256u32.to_le_bytes());
        encoded[20..24].copy_from_slice(&(payload.len() as u32).to_le_bytes());
        encoded[24..28].copy_from_slice(&crc(&raw).to_le_bytes());
        reseal(&mut encoded);
        encoded.extend(payload);
        assert_eq!(decode(&encoded, [16, 16]).unwrap(), raw);
        for end in 0..encoded.len() {
            assert!(decode(&encoded[..end], [16, 16]).is_err(), "prefix {end}");
        }
        for at in 0..encoded.len() {
            let mut bad = encoded.clone();
            bad[at] ^= 1;
            assert!(decode(&bad, [16, 16]).is_err(), "byte {at}");
        }
        for alteration in 0..6 {
            let mut bad = encoded.clone();
            match alteration {
                0 => bad.push(0),         // trailing stream garbage, header length corrected below
                1 => bad.extend(payload), // concatenated streams
                2 => {
                    bad.pop();
                } // truncated stream, header length corrected below
                3 => bad[8..10].copy_from_slice(&15u16.to_le_bytes()),
                4 => bad[8..10].copy_from_slice(&17u16.to_le_bytes()),
                _ => bad[13] = 2,
            }
            if alteration <= 2 {
                let len = (bad.len() - HEADER) as u32;
                bad[20..24].copy_from_slice(&len.to_le_bytes());
            } else if alteration <= 4 {
                let len = if alteration == 3 { 240u32 } else { 272u32 };
                bad[16..20].copy_from_slice(&len.to_le_bytes());
            }
            reseal(&mut bad);
            let shape = if alteration == 3 {
                [15, 16]
            } else if alteration == 4 {
                [17, 16]
            } else {
                [16, 16]
            };
            assert!(decode(&bad, shape).is_err(), "alteration {alteration}");
        }
        let mut empty = encode([1, 1], &[1]).unwrap().unwrap();
        empty[HEADER] = 0;
        empty[24..28].copy_from_slice(&crc(&[0]).to_le_bytes());
        reseal(&mut empty);
        assert!(decode(&empty, [1, 1]).is_err());
    }
}
