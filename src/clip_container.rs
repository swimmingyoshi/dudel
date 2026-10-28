//! Read-only inventory of the observed CSFCHUNK container; not a raster importer.
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom, Write},
    path::Path,
};

const MAX_FILE: u64 = 1024 * 1024 * 1024;
const MAX_SQL: u64 = 32 * 1024 * 1024;
const MAX_CHUNKS: usize = 65_536;

#[derive(Debug, serde::Serialize)]
pub(crate) struct Inventory {
    scope: &'static str,
    file_bytes: u64,
    head_code: u64,
    selected_sql_offset: u64,
    chunks: Vec<Chunk>,
    sqlite_headers: Vec<SqlHeader>,
    unknown_chunks: Vec<String>,
}
impl Inventory {
    pub(crate) fn external_body_bounds(
        &self,
        offset: u64,
        id_bytes: usize,
    ) -> Result<(u64, u64), String> {
        let chunk = self
            .chunks
            .iter()
            .find(|chunk| chunk.offset == offset && chunk.kind == "CHNKExta")
            .ok_or("Missing validated external chunk")?;
        let prefix = 8 + id_bytes as u64;
        let bytes = chunk
            .payload_bytes
            .checked_sub(prefix)
            .ok_or("Truncated external identifier")?;
        Ok((offset + 16 + prefix, bytes))
    }
    pub(crate) fn selected_sql_payload(&self) -> (u64, u64) {
        let sql = self
            .sqlite_headers
            .iter()
            .find(|sql| sql.chunk_offset == self.selected_sql_offset)
            .unwrap();
        (sql.chunk_offset + 16, sql.payload_bytes)
    }

    pub(crate) fn external_ids(
        &self,
        reader: &mut (impl Read + Seek),
    ) -> Result<Vec<(String, u64)>, String> {
        let chunks: Vec<_> = self
            .chunks
            .iter()
            .filter(|chunk| chunk.kind == "CHNKExta")
            .collect();
        if chunks.len() > 4096 {
            return Err("Too many external chunks for metadata inspection".into());
        }
        chunks
            .into_iter()
            .map(|chunk| {
                reader
                    .seek(SeekFrom::Start(chunk.offset + 16))
                    .map_err(|error| error.to_string())?;
                let size = be64(&read::<8>(reader)?);
                if size == 0 || size > 1024 || size > chunk.payload_bytes - 8 {
                    return Err("Invalid external identifier bounds after scan".into());
                }
                let size = size as usize;
                let mut id = vec![0; size];
                reader
                    .read_exact(&mut id)
                    .map_err(|error| error.to_string())?;
                if !id.iter().all(u8::is_ascii_alphanumeric) {
                    return Err("Unsupported external identifier encoding".into());
                }
                Ok((String::from_utf8(id).unwrap(), chunk.offset))
            })
            .collect()
    }
}
#[derive(Debug, serde::Serialize)]
struct Chunk {
    kind: String,
    offset: u64,
    payload_bytes: u64,
}
#[derive(Debug, serde::Serialize)]
struct SqlHeader {
    chunk_offset: u64,
    payload_bytes: u64,
    page_bytes: u32,
}

fn be64(bytes: &[u8]) -> u64 {
    u64::from_be_bytes(bytes.try_into().unwrap())
}
fn read<const N: usize>(reader: &mut impl Read) -> Result<[u8; N], String> {
    let mut bytes = [0; N];
    reader
        .read_exact(&mut bytes)
        .map_err(|error| format!("Truncated clip data: {error}"))?;
    Ok(bytes)
}
pub(crate) fn scan(reader: &mut (impl Read + Seek), file_bytes: u64) -> Result<Inventory, String> {
    if !(24..=MAX_FILE).contains(&file_bytes) {
        return Err("Clip container exceeds supported inspection bounds".into());
    }
    reader
        .seek(SeekFrom::Start(0))
        .map_err(|error| error.to_string())?;
    let header = read::<24>(reader)?;
    if &header[..8] != b"CSFCHUNK" {
        return Err("Not a CSFCHUNK clip container".into());
    }
    if be64(&header[8..16]) != file_bytes {
        return Err("Clip declared length does not match the file".into());
    }
    if be64(&header[16..24]) != 24 {
        return Err("Unsupported clip initial chunk offset".into());
    }
    let mut inventory = Inventory {
        scope: "Container framing, SQLite signatures and page geometry only. No complete SQLite header/schema, external block checksum, pixel/channel layout, layer features or raster decoding validation; not import support.",
        file_bytes,
        head_code: 0,
        selected_sql_offset: 0,
        chunks: Vec::new(),
        sqlite_headers: Vec::new(),
        unknown_chunks: Vec::new(),
    };
    let mut offset = 24;
    while offset < file_bytes {
        if inventory.chunks.len() >= MAX_CHUNKS {
            return Err("Clip chunk count exceeds inspection limit".into());
        }
        if file_bytes - offset < 16 {
            return Err("Truncated clip chunk header".into());
        }
        reader
            .seek(SeekFrom::Start(offset))
            .map_err(|error| error.to_string())?;
        let header = read::<16>(reader)?;
        if &header[..4] != b"CHNK" || !header[..8].iter().all(u8::is_ascii_alphanumeric) {
            return Err("Invalid clip chunk tag".into());
        }
        let kind = std::str::from_utf8(&header[..8]).unwrap().to_owned();
        let payload_bytes = be64(&header[8..]);
        let end = offset
            .checked_add(16)
            .and_then(|start| start.checked_add(payload_bytes))
            .filter(|end| *end <= file_bytes)
            .ok_or("Clip chunk extends beyond the file")?;
        if inventory.chunks.is_empty() && kind != "CHNKHead" {
            return Err("Clip head is not the first chunk".into());
        }
        match kind.as_str() {
            "CHNKHead" => {
                if !inventory.chunks.is_empty() || payload_bytes != 40 {
                    return Err("Unsupported or duplicate clip head".into());
                }
                let head = read::<40>(reader)?;
                inventory.head_code = be64(&head[..8]);
                if inventory.head_code != 256 || be64(&head[16..24]) != 16 {
                    return Err("Unsupported clip head fields".into());
                }
                inventory.selected_sql_offset = be64(&head[8..16]);
            }
            "CHNKSQLi" => {
                if !(100..=MAX_SQL).contains(&payload_bytes) {
                    return Err(
                        "Clip SQLite metadata exceeds inspection cap or is truncated".into(),
                    );
                }
                let sqlite = read::<100>(reader)?;
                if &sqlite[..16] != b"SQLite format 3\0" {
                    return Err("Invalid clip SQLite magic".into());
                }
                let encoded = u16::from_be_bytes(sqlite[16..18].try_into().unwrap());
                let page_bytes = if encoded == 1 { 65536 } else { encoded as u32 };
                if !(512..=65536).contains(&page_bytes)
                    || !page_bytes.is_power_of_two()
                    || !payload_bytes.is_multiple_of(page_bytes as u64)
                {
                    return Err("Invalid clip SQLite page geometry".into());
                }
                inventory.sqlite_headers.push(SqlHeader {
                    chunk_offset: offset,
                    payload_bytes,
                    page_bytes,
                });
            }
            "CHNKExta" => {
                if payload_bytes < 8 {
                    return Err("Truncated clip external-data header".into());
                }
                let id_bytes = be64(&read::<8>(reader)?);
                if id_bytes == 0 || id_bytes > 1024 || id_bytes > payload_bytes - 8 {
                    return Err("Invalid clip external identifier bounds".into());
                }
            }
            "CHNKFoot" => {
                if payload_bytes != 0 {
                    return Err("Unsupported clip footer payload".into());
                }
            }
            _ => {
                if !inventory.unknown_chunks.contains(&kind) {
                    inventory.unknown_chunks.push(kind.clone());
                }
            }
        }
        inventory.chunks.push(Chunk {
            kind,
            offset,
            payload_bytes,
        });
        offset = end;
    }
    if !inventory
        .chunks
        .last()
        .is_some_and(|chunk| chunk.kind == "CHNKFoot")
    {
        return Err("Missing final clip footer".into());
    }
    if !inventory
        .sqlite_headers
        .iter()
        .any(|sqlite| sqlite.chunk_offset == inventory.selected_sql_offset)
    {
        return Err("Clip head does not reference a validated SQLite chunk".into());
    }
    Ok(inventory)
}
pub(crate) fn inspect(input: &Path, output: Option<&Path>) -> Result<(), String> {
    let mut source = File::open(input).map_err(|error| error.to_string())?;
    let length = source.metadata().map_err(|error| error.to_string())?.len();
    let inventory = scan(&mut source, length)?;
    write_report(&inventory, output)
}
pub(crate) fn write_report(
    report: &impl serde::Serialize,
    output: Option<&Path>,
) -> Result<(), String> {
    let report = serde_json::to_vec_pretty(report).map_err(|error| error.to_string())?;
    if let Some(path) = output {
        // Exclusive creation also rejects an existing source alias/hard link.
        let mut file = File::options()
            .write(true)
            .create_new(true)
            .open(path)
            .map_err(|error| format!("Report must use a new file: {error}"))?;
        file.write_all(&report).map_err(|error| error.to_string())?;
    } else {
        println!("{}", std::str::from_utf8(&report).unwrap());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(extra: bool, multiple_sql: bool) -> Vec<u8> {
        let mut sql = vec![0; 512];
        sql[..16].copy_from_slice(b"SQLite format 3\0");
        sql[16..18].copy_from_slice(&512u16.to_be_bytes());
        let mut chunks = vec![(*b"CHNKHead", vec![0; 40])];
        if extra {
            chunks.push((*b"CHNKWild", vec![17; 7]));
        }
        if multiple_sql {
            chunks.push((*b"CHNKSQLi", sql.clone()));
            chunks.push((*b"CHNKFoot", Vec::new()));
        }
        chunks.push((*b"CHNKSQLi", sql));
        chunks.push((*b"CHNKFoot", Vec::new()));
        let total = 24
            + chunks
                .iter()
                .map(|(_, data)| 16 + data.len())
                .sum::<usize>();
        let mut offset = 24usize;
        let mut selected = 0;
        for (kind, data) in &chunks {
            if kind == b"CHNKSQLi" {
                selected = offset;
            }
            offset += 16 + data.len();
        }
        chunks[0].1[..8].copy_from_slice(&256u64.to_be_bytes());
        chunks[0].1[8..16].copy_from_slice(&(selected as u64).to_be_bytes());
        chunks[0].1[16..24].copy_from_slice(&16u64.to_be_bytes());
        let mut bytes = b"CSFCHUNK".to_vec();
        bytes.extend_from_slice(&(total as u64).to_be_bytes());
        bytes.extend_from_slice(&24u64.to_be_bytes());
        for (kind, data) in chunks {
            bytes.extend_from_slice(&kind);
            bytes.extend_from_slice(&(data.len() as u64).to_be_bytes());
            bytes.extend_from_slice(&data);
        }
        bytes
    }
    fn parse(bytes: &[u8]) -> Result<Inventory, String> {
        scan(&mut std::io::Cursor::new(bytes), bytes.len() as u64)
    }
    #[test]
    fn inventory_preserves_unknown_chunks_and_resolves_selected_database() {
        let bytes = fixture(true, true);
        let inventory = parse(&bytes).unwrap();
        assert_eq!(inventory.sqlite_headers.len(), 2);
        assert_eq!(
            inventory.selected_sql_offset,
            inventory.sqlite_headers[1].chunk_offset
        );
        assert_eq!(inventory.unknown_chunks, vec!["CHNKWild"]);
        assert_eq!(inventory.sqlite_headers[0].page_bytes, 512);
    }
    #[test]
    fn truncation_overflow_and_wrong_database_reference_are_rejected() {
        let bytes = fixture(false, false);
        for end in 0..bytes.len() {
            assert!(scan(&mut std::io::Cursor::new(&bytes[..end]), bytes.len() as u64).is_err());
        }
        let mut overflow = bytes.clone();
        overflow[32..40].copy_from_slice(&u64::MAX.to_be_bytes());
        assert!(parse(&overflow).is_err());
        let mut pointer = bytes.clone();
        pointer[48..56].copy_from_slice(&81u64.to_be_bytes());
        assert!(parse(&pointer).is_err());
        let mut size = bytes;
        size[8..16].copy_from_slice(&0u64.to_be_bytes());
        assert!(parse(&size).is_err());
    }
    #[test]
    fn sqlite_magic_geometry_footer_and_inspection_caps_are_checked() {
        let bytes = fixture(false, false);
        let mut magic = bytes.clone();
        magic[96] = 0;
        assert!(parse(&magic).is_err());
        let mut page = bytes.clone();
        page[112..114].copy_from_slice(&513u16.to_be_bytes());
        assert!(parse(&page).is_err());
        let mut footer = bytes.clone();
        let len = footer.len();
        footer[len - 16..len - 8].copy_from_slice(b"CHNKWild");
        assert!(parse(&footer).is_err());
        assert!(scan(&mut std::io::Cursor::new(&bytes), MAX_FILE + 1).is_err());
        let mut large = bytes;
        let claimed = MAX_SQL + 512 + 112;
        large[8..16].copy_from_slice(&claimed.to_be_bytes());
        large[88..96].copy_from_slice(&(MAX_SQL + 512).to_be_bytes());
        assert!(
            scan(&mut std::io::Cursor::new(large), claimed)
                .unwrap_err()
                .contains("metadata")
        );
    }
    #[test]
    fn report_creation_cannot_overwrite_source_or_an_existing_report() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.clip");
        let bytes = fixture(false, false);
        std::fs::write(&source, &bytes).unwrap();
        assert!(inspect(&source, Some(&source)).is_err());
        assert_eq!(std::fs::read(&source).unwrap(), bytes);
        let report = dir.path().join("report.json");
        inspect(&source, Some(&report)).unwrap();
        let first = std::fs::read(&report).unwrap();
        assert!(inspect(&source, Some(&report)).is_err());
        assert_eq!(std::fs::read(&report).unwrap(), first);
    }
    #[test]
    fn large_sqlite_pages_and_external_identifier_bounds_are_supported() {
        let mut bytes = fixture(false, false);
        let mut sql = vec![0; 65536];
        sql[..100].copy_from_slice(&bytes[96..196]);
        sql[16..18].copy_from_slice(&1u16.to_be_bytes());
        bytes.splice(96..608, sql);
        bytes[88..96].copy_from_slice(&65536u64.to_be_bytes());
        let length = bytes.len() as u64;
        bytes[8..16].copy_from_slice(&length.to_be_bytes());
        assert_eq!(parse(&bytes).unwrap().sqlite_headers[0].page_bytes, 65536);

        let mut bytes = fixture(false, false);
        let mut external = b"CHNKExta".to_vec();
        external.extend_from_slice(&12u64.to_be_bytes());
        external.extend_from_slice(&4u64.to_be_bytes());
        external.extend_from_slice(b"abcd");
        bytes.splice(80..80, external);
        bytes[48..56].copy_from_slice(&108u64.to_be_bytes());
        let length = bytes.len() as u64;
        bytes[8..16].copy_from_slice(&length.to_be_bytes());
        assert_eq!(parse(&bytes).unwrap().chunks[1].kind, "CHNKExta");
        bytes[96..104].copy_from_slice(&1000u64.to_be_bytes());
        assert!(parse(&bytes).unwrap_err().contains("identifier"));
    }
}
