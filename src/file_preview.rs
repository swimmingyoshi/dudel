//! Small flattened preview stored in a v4/v5 project's gzip header (FEXTRA),
//! so file browsers can show artwork without decompressing any layers.
//! Readers that predate it skip the extra field, so the format is unchanged.
use crate::document::Document;

use header::SUBFIELD;
#[cfg(test)]
use header::read;
// The app writes previews; reading them is for tests and the Explorer DLL.
#[allow(dead_code)]
#[path = "file_preview_header.rs"]
mod header;

/// Longest preview side; Explorer's largest icon view asks for 256.
pub const SIDE: usize = header::MAX_SIDE as usize;
/// Gzip XLEN is a u16 and the subfield header takes four bytes of it.
const MAX_IMAGE: usize = u16::MAX as usize - 4;

/// Gzip extra-field bytes holding one preview subfield, or None when the
/// document is empty or no encoding fits the header.
pub(crate) fn gzip_extra(doc: &Document) -> Option<Vec<u8>> {
    let image = encode(doc)?;
    let mut extra = Vec::with_capacity(image.len() + 4);
    extra.extend_from_slice(&SUBFIELD);
    extra.extend_from_slice(&(image.len() as u16).to_le_bytes());
    extra.extend_from_slice(&image);
    Some(extra)
}

/// PNG for line art and flat colour; JPEG only when PNG would not fit.
fn encode(doc: &Document) -> Option<Vec<u8>> {
    let (width, height, rgb) = render(doc)?;
    let mut png = Vec::new();
    image::ImageEncoder::write_image(
        image::codecs::png::PngEncoder::new(&mut png),
        &rgb,
        width as u32,
        height as u32,
        image::ExtendedColorType::Rgb8,
    )
    .ok()?;
    if png.len() <= MAX_IMAGE {
        return Some(png);
    }
    for quality in [88, 70, 50] {
        let mut jpeg = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, quality)
            .encode(
                &rgb,
                width as u32,
                height as u32,
                image::ExtendedColorType::Rgb8,
            )
            .ok()?;
        if jpeg.len() <= MAX_IMAGE {
            return Some(jpeg);
        }
    }
    None
}

/// Composite on paper, 2x2 supersampled per preview pixel. Sampling keeps the
/// cost to a quarter million composited pixels whatever the canvas size.
fn render(doc: &Document) -> Option<(usize, usize, Vec<u8>)> {
    if doc.width == 0 || doc.height == 0 {
        return None;
    }
    let expanded = if doc.needs_expansion() {
        Some(crate::layer_effects::expand(doc, false, &|| false).ok()?)
    } else {
        None
    };
    let doc = expanded.as_ref().unwrap_or(doc);
    let scale = (doc.width.max(doc.height) as f32 / SIDE as f32).max(1.0);
    let width = ((doc.width as f32 / scale).round() as usize).clamp(1, SIDE);
    let height = ((doc.height as f32 / scale).round() as usize).clamp(1, SIDE);
    let [sx, sy] = [
        doc.width as f32 / width as f32,
        doc.height as f32 / height as f32,
    ];
    let row = |y: usize| {
        let mut out = Vec::with_capacity(width * 3);
        for x in 0..width {
            let mut sum = [0u32; 3];
            for [ox, oy] in [[0.25, 0.25], [0.75, 0.25], [0.25, 0.75], [0.75, 0.75]] {
                let px = (((x as f32 + ox) * sx) as usize).min(doc.width - 1);
                let py = (((y as f32 + oy) * sy) as usize).min(doc.height - 1);
                let pixel = doc.composite_rgba_pixel(py * doc.width + px);
                for c in 0..3 {
                    sum[c] += pixel[c] as u32;
                }
            }
            out.extend(sum.map(|value| ((value + 2) / 4) as u8));
        }
        out
    };
    let rows = crate::parallel::run(height, row);
    let mut rgb = Vec::with_capacity(width * height * 3);
    for line in rows {
        rgb.extend(line.ok()?);
    }
    Some((width, height, rgb))
}

#[test]
fn saved_preview_includes_native_layer_effects() {
    use crate::layer_effects::{Effect, LayerEffect, Paint};
    let mut doc = Document::new(1, 1);
    doc.layers[0].pixels = vec![255, 0, 0, 255].into();
    let mut overlay = Effect::preset("Color Overlay").unwrap();
    overlay.paint = Paint::Color([0, 0, 255]);
    doc.layers[0].effects.push(LayerEffect::new(overlay));
    assert_eq!(render(&doc).unwrap().2, vec![0, 0, 255]);
}

/// Registers (or removes) dudel_thumbnails.dll beside dudel.exe as the
/// current user's Explorer thumbnail provider. regsvr32 runs the DLL's own
/// per-user registration, so this needs no administrator rights.
#[cfg(windows)]
pub(crate) fn set_explorer_thumbnails(enable: bool) -> Result<String, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    register_thumbnails(&exe.with_file_name("dudel_thumbnails.dll"), enable)?;
    Ok(if enable {
        "File Explorer now shows previews of .dudel projects saved from now on".into()
    } else {
        "File Explorer previews for .dudel projects removed".into()
    })
}

/// Registers or removes the thumbnail library at `dll` for the current user.
#[cfg(windows)]
pub(crate) fn register_thumbnails(dll: &std::path::Path, enable: bool) -> Result<(), String> {
    if !dll.is_file() {
        return Err(format!("{} is missing", dll.display()));
    }
    let system = std::env::var_os("SystemRoot").unwrap_or_else(|| r"C:\Windows".into());
    let mut command =
        std::process::Command::new(std::path::Path::new(&system).join(r"System32\regsvr32.exe"));
    command.arg("/s");
    if !enable {
        command.arg("/u");
    }
    let status = command.arg(dll).status().map_err(|e| e.to_string())?;
    if !status.success() {
        return Err(format!("regsvr32 failed ({status})"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saved_projects_carry_a_readable_preview_and_still_load() {
        let mut doc = Document::new(1000, 500);
        let red = [200, 10, 10, 255].repeat(500 * 500);
        doc.layers[0]
            .pixels
            .writer()
            .write_region(1000, [0, 0, 500, 500], &red);
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("preview.dudel");
        crate::storage::save_project(&path, &doc).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        let preview = image::load_from_memory(read(&bytes).unwrap())
            .unwrap()
            .to_rgb8();
        assert_eq!(preview.dimensions(), (256, 128));
        assert_eq!(preview.get_pixel(10, 64).0, [200, 10, 10]);
        assert_eq!(preview.get_pixel(240, 64).0, [255, 255, 255]);
        // Text projects carry the same preview under their own header.
        let mut text = bytes.clone();
        text[..7].copy_from_slice(crate::tile_project::TEXT_MAGIC);
        assert_eq!(read(&text), read(&bytes));
        let loaded = crate::storage::load_project(&path).unwrap();
        assert_eq!(loaded.layers[0].pixels.rgba(0), [200, 10, 10, 255]);
    }

    #[test]
    fn small_canvases_keep_their_size_and_noise_falls_back_to_jpeg() {
        let small = Document::new(40, 30);
        let (width, height, _) = render(&small).unwrap();
        assert_eq!((width, height), (40, 30));
        let mut noise = Document::new(512, 512);
        let mut state = 0x2545_f491u32;
        let pixels: Vec<u8> = (0..512 * 512)
            .flat_map(|_| {
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                let [r, g, b, _] = state.to_le_bytes();
                [r, g, b, 255]
            })
            .collect();
        noise.layers[0]
            .pixels
            .writer()
            .write_region(512, [0, 0, 512, 512], &pixels);
        let extra = gzip_extra(&noise).unwrap();
        assert!(extra.len() <= u16::MAX as usize);
        assert!(extra[4..].starts_with(&[0xff, 0xd8]));
    }

    #[test]
    fn header_reader_rejects_other_files() {
        assert!(read(b"DUDL\x04\r\n").is_none());
        assert!(read(b"CSFCHUNK").is_none());
        let mut v4 = crate::tile_project::MAGIC.to_vec();
        v4.extend_from_slice(&[0x1f, 0x8b, 8, 4, 0, 0, 0, 0, 0, 255, 10, 0]);
        v4.extend_from_slice(b"DP\x10\x00short");
        assert!(read(&v4).is_none());
    }
}
