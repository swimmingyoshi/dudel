//! Dependency-free reader for the preview in a project's gzip header. Shared
//! verbatim with the Explorer thumbnail DLL, so it uses only `core`.

/// Gzip extra subfield id (SI1, SI2) of the preview image.
pub const SUBFIELD: [u8; 2] = *b"DP";
/// Writer and shell reader share the maximum embedded preview dimensions.
pub const MAX_SIDE: u32 = 256;
/// Bytes needed to find any preview: magic, gzip header and a full FEXTRA.
pub const PREFIX_LIMIT: usize = 7 + 10 + 2 + u16::MAX as usize;

/// The PNG or JPEG preview of a v4/v5/v6 project, from the first
/// `PREFIX_LIMIT` bytes of the file; None for any other file.
pub fn read(file: &[u8]) -> Option<&[u8]> {
    let magic = file.get(..7)?;
    if !matches!(magic, b"DUDL\x04\r\n" | b"DUDL\x05\r\n" | b"DUDL\x06\r\n") {
        return None;
    }
    let gzip = &file[7..];
    // ID1 ID2, deflate, FEXTRA set.
    if gzip.get(..3)? != [0x1f, 0x8b, 8] || gzip.get(3)? & 4 == 0 {
        return None;
    }
    let length = u16::from_le_bytes([*gzip.get(10)?, *gzip.get(11)?]) as usize;
    let mut extra = gzip.get(12..12 + length)?;
    while extra.len() >= 4 {
        let size = u16::from_le_bytes([extra[2], extra[3]]) as usize;
        let data = extra.get(4..4 + size)?;
        if extra[..2] == SUBFIELD {
            return Some(data);
        }
        extra = &extra[4 + size..];
    }
    None
}
