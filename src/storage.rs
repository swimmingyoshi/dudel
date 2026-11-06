use crate::document::Document;
use flate2::read::GzDecoder;
use serde::{Deserialize, Serialize};
use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
};

const LIMIT: u64 = 256 * 1024 * 1024;
const PROJECT_MAGIC: &[u8] = b"DUDL\x02\r\n";
const RECOVERY_MAGIC: &[u8] = b"DRCV\x01\r\n";
const PATH_LIMIT: usize = 256 * 1024;
pub(crate) const SAVE_CANCELED: &str = "Save canceled";

pub(crate) fn check_save_cancel(
    cancel: Option<&std::sync::atomic::AtomicBool>,
) -> Result<(), String> {
    if cancel.is_some_and(|flag| flag.load(std::sync::atomic::Ordering::Relaxed)) {
        Err(SAVE_CANCELED.into())
    } else {
        Ok(())
    }
}

#[cfg(test)]
#[path = "storage_interrupt_tests.rs"]
mod interruption_tests;

/// Same-directory staging, flush, then atomic replacement. Never delete the old
/// destination before committing. The tempfile crate supplies Windows replacement.
pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    atomic_stream(path, |file| {
        file.write_all(bytes).map_err(|e| e.to_string())
    })
}
pub(crate) fn atomic_stream(
    path: &Path,
    write: impl FnOnce(&mut std::fs::File) -> Result<(), String>,
) -> Result<(), String> {
    atomic_stream_observed(path, write, |_| Ok(()))
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum CommitStage {
    Written,
    Synced,
    Published,
}

fn atomic_stream_observed(
    path: &Path,
    write: impl FnOnce(&mut std::fs::File) -> Result<(), String>,
    mut observe: impl FnMut(CommitStage) -> Result<(), String>,
) -> Result<(), String> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut staged = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
    write(staged.as_file_mut())?;
    observe(CommitStage::Written)?;
    staged.as_file().sync_all().map_err(|e| e.to_string())?;
    observe(CommitStage::Synced)?;
    staged.persist(path).map_err(|e| e.to_string())?;
    observe(CommitStage::Published)?;
    Ok(())
}

pub fn save_project(path: &Path, doc: &Document) -> Result<(), String> {
    save_project_controlled(path, doc, None)
}

pub(crate) fn save_project_cancelable(
    path: &Path,
    doc: &Document,
    cancel: &std::sync::atomic::AtomicBool,
) -> Result<(), String> {
    save_project_controlled(path, doc, Some(cancel))
}

fn save_project_controlled(
    path: &Path,
    doc: &Document,
    cancel: Option<&std::sync::atomic::AtomicBool>,
) -> Result<(), String> {
    check_save_cancel(cancel)?;
    if path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            ["clip", "psd", "psb"]
                .iter()
                .any(|source| extension.eq_ignore_ascii_case(source))
        })
    {
        return Err("Choose a .dudel filename; Dudel cannot save over a CSP/PSD source".into());
    }
    // Also protect a CSP file renamed/linked to a different extension.
    match std::fs::File::open(path) {
        Ok(mut file) => {
            let mut magic = [0u8; 8];
            match file.read_exact(&mut magic) {
                Ok(()) if &magic == b"DIDX\x02\r\n\0" => {
                    return Err(
                        "Choose a new .dudel file; this destination contains an indexed project"
                            .into(),
                    );
                }
                Ok(()) if &magic == b"CSFCHUNK" => {
                    return Err(
                        "Choose a new .dudel file; this destination contains CSP artwork".into(),
                    );
                }
                Ok(()) if &magic[..4] == b"8BPS" => {
                    return Err(
                        "Choose a new .dudel file; this destination contains Photoshop artwork"
                            .into(),
                    );
                }
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => {}
                Err(error) => return Err(error.to_string()),
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.to_string()),
    }
    atomic_stream_observed(
        path,
        |file| crate::tile_project::encode_cancelable(doc, file, cancel),
        |stage| {
            if stage == CommitStage::Synced {
                check_save_cancel(cancel)
            } else {
                Ok(())
            }
        },
    )
}
#[cfg(test)]
#[test]
fn saving_native_project_cannot_overwrite_csp_sources() {
    let dir = tempfile::tempdir().unwrap();
    let doc = Document::new(1, 1);
    for name in ["source.clip", "source.CLIP"] {
        let path = dir.path().join(name);
        std::fs::write(&path, b"original source").unwrap();
        assert!(save_project(&path, &doc).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"original source");
    }
    save_project(&dir.path().join("imported.dudel"), &doc).unwrap();
    let disguised = dir.path().join("renamed.dudel");
    std::fs::write(&disguised, b"CSFCHUNK untouched source").unwrap();
    assert!(save_project(&disguised, &doc).is_err());
    assert_eq!(
        std::fs::read(&disguised).unwrap(),
        b"CSFCHUNK untouched source"
    );
}

pub fn load_project(path: &Path) -> Result<Document, String> {
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let size = file.metadata().map_err(|e| e.to_string())?.len();
    let mut reader = std::io::BufReader::new(file);
    use std::io::BufRead;
    let prefix = reader.fill_buf().map_err(|e| e.to_string())?;
    let v8 = prefix.starts_with(crate::tile_project::TEXT_MAGIC);
    let v7 = prefix.starts_with(crate::tile_project::VECTOR_MAGIC);
    let v6 = prefix.starts_with(crate::tile_project::STYLE_MAGIC);
    let v5 = prefix.starts_with(crate::tile_project::MASK_MAGIC);
    let v4 = prefix.starts_with(crate::tile_project::MAGIC);
    let v3 = prefix.starts_with(crate::binary_project::MAGIC);
    if v8 || v7 || v6 || v5 || v4 || v3 {
        // Decoding is bounded by the document payload budget; this only stops
        // reading files no Dudel save could have produced.
        if size > 4 << 30 {
            return Err("Binary project exceeds file limit".into());
        }
        reader.consume(crate::tile_project::MAGIC.len());
        return if v8 || v7 {
            crate::tile_project::decode_vector(reader)
        } else if v6 {
            crate::tile_project::decode_styled(reader)
        } else if v5 {
            crate::tile_project::decode_masked(reader)
        } else if v4 {
            crate::tile_project::decode(reader)
        } else {
            crate::binary_project::decode(reader)
        };
    }
    let mut data = Vec::new();
    reader
        .take(LIMIT + 1)
        .read_to_end(&mut data)
        .map_err(|e| e.to_string())?;
    if data.len() as u64 > LIMIT {
        return Err("Project file exceeds supported size".into());
    }
    let decoded = if data.starts_with(PROJECT_MAGIC) {
        let mut decoded = Vec::new();
        GzDecoder::new(&data[PROJECT_MAGIC.len()..])
            .take(LIMIT + 1)
            .read_to_end(&mut decoded)
            .map_err(|e| format!("Damaged compressed project: {e}"))?;
        if decoded.len() as u64 > LIMIT {
            return Err("Decompressed project exceeds supported size".into());
        }
        decoded
    } else if data.starts_with(b"DUDL") {
        return Err("Unsupported project container version".into());
    } else {
        data
    };
    let mut doc: Document = serde_json::from_slice(&decoded).map_err(|e| e.to_string())?;
    doc.validate()?;
    sparsify(&mut doc)?;
    Ok(doc)
}

fn sparsify(doc: &mut Document) -> Result<(), String> {
    for layer in doc.layers.iter_mut().filter(|layer| !layer.folder) {
        layer.pixels =
            crate::pixels::Pixels::read_rgba(&mut layer.pixels.as_slice(), doc.width, doc.height)
                .map_err(|e| e.to_string())?;
    }
    Ok(())
}

pub fn import_png(path: &Path) -> Result<Document, String> {
    use image::ImageDecoder;
    let mut reader = image::ImageReader::open(path).map_err(|e| e.to_string())?;
    reader.set_format(image::ImageFormat::Png);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(crate::document::MAX_SIDE as u32);
    limits.max_image_height = Some(crate::document::MAX_SIDE as u32);
    // 16-bit RGBA decodes at 8 bytes per pixel before conversion.
    limits.max_alloc = Some(crate::document::MAX_PIXELS as u64 * 8 + (64 << 20));
    reader.limits(limits);
    let decoder = reader.into_decoder().map_err(|e| e.to_string())?;
    let (width, height) = decoder.dimensions();
    if width == 0
        || height == 0
        || u64::from(width) * u64::from(height) > crate::document::MAX_PIXELS as u64
    {
        return Err("PNG exceeds supported canvas dimensions".into());
    }
    let image = image::DynamicImage::from_decoder(decoder)
        .map_err(|e| e.to_string())?
        .into_rgba8();
    let mut doc = Document::new(width as usize, height as usize);
    doc.dpi = crate::document::default_dpi();
    // Read pHYs through the bounded PNG header decoder, without a second
    // pixel decode. Unitless aspect ratios do not specify a print resolution.
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let metadata = png::Decoder::new_with_limits(
        std::io::BufReader::new(file),
        png::Limits { bytes: 1 << 20 },
    );
    if let Ok(reader) = metadata.read_info()
        && let Some(dims) = reader.info().pixel_dims
        && dims.unit == png::Unit::Meter
    {
        let dpi = f64::from(dims.xppu) * 0.0254;
        // pHYs stores whole pixels/metre; allow its half-unit rounding at
        // the supported endpoints (1 DPI encodes as 39 pixels/metre).
        if (1.0 - 0.0127..=12000.0 + 0.0127).contains(&dpi) {
            doc.dpi = dpi.clamp(1.0, 12000.0);
        }
    }

    doc.layers[0].pixels = crate::pixels::Pixels::read_rgba(
        &mut std::io::Cursor::new(image.into_raw()),
        doc.width,
        doc.height,
    )
    .map_err(|e| e.to_string())?;
    doc.layers[0].name = path
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    doc.validate()?;
    Ok(doc)
}

#[derive(Serialize, Deserialize)]
pub struct Recovery {
    pub document: Document,
    pub original_path: Option<PathBuf>,
}

pub fn save_recovery(path: &Path, recovery: &Recovery) -> Result<(), String> {
    recovery.document.validate()?;
    let metadata = serde_json::to_vec(&recovery.original_path).map_err(|e| e.to_string())?;
    if metadata.len() > PATH_LIMIT {
        return Err("Recovery path exceeds supported size".into());
    }
    atomic_stream(path, |file| {
        file.write_all(RECOVERY_MAGIC).map_err(|e| e.to_string())?;
        file.write_all(&(metadata.len() as u32).to_le_bytes())
            .map_err(|e| e.to_string())?;
        file.write_all(&metadata).map_err(|e| e.to_string())?;
        crate::tile_project::encode(&recovery.document, file)
    })
}

pub fn read_recovery(path: &Path) -> Result<Recovery, String> {
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let size = file.metadata().map_err(|e| e.to_string())?.len();
    let mut reader = std::io::BufReader::new(file);
    use std::io::BufRead;
    if reader
        .fill_buf()
        .map_err(|e| e.to_string())?
        .starts_with(RECOVERY_MAGIC)
    {
        if size > 551 * 1024 * 1024 {
            return Err("Recovery exceeds supported size".into());
        }
        reader.consume(RECOVERY_MAGIC.len());
        let mut length = [0; 4];
        reader.read_exact(&mut length).map_err(|e| e.to_string())?;
        let length = u32::from_le_bytes(length) as usize;
        if length == 0 || length > PATH_LIMIT {
            return Err("Invalid recovery path metadata size".into());
        }
        let mut metadata = vec![0; length];
        reader
            .read_exact(&mut metadata)
            .map_err(|e| e.to_string())?;
        let original_path = serde_json::from_slice(&metadata).map_err(|e| e.to_string())?;
        let mut magic = vec![0; crate::binary_project::MAGIC.len()];
        reader.read_exact(&mut magic).map_err(|e| e.to_string())?;
        let document = if magic == crate::tile_project::TEXT_MAGIC
            || magic == crate::tile_project::VECTOR_MAGIC
        {
            crate::tile_project::decode_vector(reader)?
        } else if magic == crate::tile_project::STYLE_MAGIC {
            crate::tile_project::decode_styled(reader)?
        } else if magic == crate::tile_project::MASK_MAGIC {
            crate::tile_project::decode_masked(reader)?
        } else if magic == crate::tile_project::MAGIC {
            crate::tile_project::decode(reader)?
        } else if magic == crate::binary_project::MAGIC {
            crate::binary_project::decode(reader)?
        } else {
            return Err("Unsupported recovery project format".into());
        };
        return Ok(Recovery {
            document,
            original_path,
        });
    }
    if size > LIMIT {
        return Err("Legacy recovery exceeds supported size".into());
    }
    let mut bytes = Vec::new();
    GzDecoder::new(reader)
        .take(LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > LIMIT {
        return Err("Recovery exceeds supported size".into());
    }
    let mut recovery: Recovery = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    recovery.document.validate()?;
    sparsify(&mut recovery.document)?;
    Ok(recovery)
}

pub fn recovery_directory() -> Result<PathBuf, String> {
    let dirs = directories::ProjectDirs::from("org", "Dudel", "Dudel")
        .ok_or("User data directory unavailable")?;
    let path = dirs.data_local_dir().join("recovery");
    std::fs::create_dir_all(&path).map_err(|e| e.to_string())?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::{Compression, write::GzEncoder};
    #[test]
    fn compressed_projects_preserve_metadata_and_legacy_projects_open() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("compressed.dudel");
        let mut doc = Document::new(64, 64);
        doc.add_layer();
        doc.layers[1].clipped = true;
        doc.layers[1].locked = true;
        doc.selection = Some(crate::selection::Selection {
            left: 2,
            top: 3,
            right: 10,
            bottom: 12,
        });
        let original = serde_json::to_vec(&doc).unwrap();
        save_project(&path, &doc).unwrap();
        assert!(std::fs::metadata(&path).unwrap().len() < original.len() as u64);
        assert_eq!(
            serde_json::to_vec(&load_project(&path).unwrap()).unwrap(),
            original
        );
        let legacy = directory.path().join("legacy.dudel");
        std::fs::write(&legacy, &original).unwrap();
        assert_eq!(
            serde_json::to_vec(&load_project(&legacy).unwrap()).unwrap(),
            original
        );
        let mut v2 = GzEncoder::new(Vec::new(), Compression::fast());
        v2.write_all(&original).unwrap();
        let mut v2_container = PROJECT_MAGIC.to_vec();
        v2_container.extend(v2.finish().unwrap());
        std::fs::write(&legacy, v2_container).unwrap();
        assert_eq!(
            serde_json::to_vec(&load_project(&legacy).unwrap()).unwrap(),
            original
        );
        let mut damaged = std::fs::read(&path).unwrap();
        damaged.truncate(damaged.len() - 8);
        std::fs::write(&path, damaged).unwrap();
        assert!(load_project(&path).is_err());
        std::fs::write(&path, b"DUDL\x09\r\n").unwrap();
        assert!(load_project(&path).is_err());
    }
    #[test]
    fn png_import_retains_rgba_and_rejects_invalid_data() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("image.png");
        let rgba = [10, 20, 30, 64, 100, 120, 140, 255];
        image::save_buffer_with_format(
            &path,
            &rgba,
            2,
            1,
            image::ColorType::Rgba8,
            image::ImageFormat::Png,
        )
        .unwrap();
        let doc = import_png(&path).unwrap();
        assert_eq!(doc.layers[0].pixels, rgba);
        assert_eq!(doc.composite_transparent(), rgba);
        std::fs::write(&path, b"not a PNG").unwrap();
        assert!(import_png(&path).is_err());
    }
    #[test]
    fn replacing_project_preserves_readable_document() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("art.dudel");
        let mut doc = Document::new(8, 8);
        save_project(&path, &doc).unwrap();
        doc.layers[0].pixels[3] = 255;
        save_project(&path, &doc).unwrap();
        let restored = load_project(&path).unwrap();
        assert_eq!(restored.layers[0].pixels[3], 255);
        doc.layers[0].pixels.pop();
        assert!(save_project(&path, &doc).is_err());
        let intact = load_project(&path).unwrap();
        intact.validate().unwrap();
    }
    #[test]
    fn recovery_roundtrip_and_truncated_input() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("recovery.gz");
        let recovery = Recovery {
            document: Document::new(8, 8),
            original_path: Some("art.dudel".into()),
        };
        save_recovery(&path, &recovery).unwrap();
        let restored = read_recovery(&path).unwrap();
        assert_eq!(restored.original_path, recovery.original_path);
        assert_eq!(restored.document.composite(), recovery.document.composite());
        let intact = std::fs::read(&path).unwrap();
        for end in [RECOVERY_MAGIC.len(), intact.len() - 8] {
            std::fs::write(&path, &intact[..end]).unwrap();
            assert!(read_recovery(&path).is_err());
        }
        let mut oversized = RECOVERY_MAGIC.to_vec();
        oversized.extend_from_slice(&u32::MAX.to_le_bytes());
        std::fs::write(&path, oversized).unwrap();
        assert!(read_recovery(&path).is_err());
        let mut legacy = GzEncoder::new(Vec::new(), Compression::fast());
        legacy
            .write_all(&serde_json::to_vec(&recovery).unwrap())
            .unwrap();
        std::fs::write(&path, legacy.finish().unwrap()).unwrap();
        assert_eq!(
            read_recovery(&path).unwrap().original_path,
            recovery.original_path
        );
        std::fs::write(&path, [31, 139]).unwrap();
        assert!(read_recovery(&path).is_err());
    }
    #[test]
    fn failed_commit_does_not_remove_destination() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("existing_directory");
        std::fs::create_dir(&path).unwrap();
        assert!(atomic_write(&path, b"new contents").is_err());
        assert!(path.is_dir());
    }
}
