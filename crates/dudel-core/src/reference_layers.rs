//! Reference layers: images placed on the canvas to draw from. They are app
//! state saved with projects, never artwork: compositing, export, fill and
//! history do not see them, and undoing artwork never moves them.
use emath::{Pos2, Vec2, pos2, vec2};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

pub const MAX_REFERENCES: usize = 8;
pub const MAX_IMAGES: usize = 12;
pub const IMPORT_BUDGET: usize = 128 * 1024 * 1024;
/// Imported images are reduced to this longest edge; references are for
/// viewing; new imports share a 128 MiB project-wide RGBA budget.
const IMPORT_SIDE: usize = 1536;
/// Largest stored side a project may carry.
const MAX_SIDE: usize = 4096;
pub const MAX_NAME_BYTES: usize = 256;
const MAX_PNG_BYTES: usize = 64 * 1024 * 1024;
pub const MIN_SCALE: f32 = 0.01;
pub const MAX_SCALE: f32 = 64.0;
/// Project trailer holding reference images after the artwork records.
pub const TRAILER: &[u8; 5] = b"REFS\x01";
/// New placement/board data must not be silently discarded by older readers.
pub const STACK_TRAILER: &[u8; 5] = b"REFS\x02";
/// Screen distance of the rotation handle beyond the reference's top edge.
pub const ROTATE_REACH: f32 = 26.0;
pub const HANDLE_RADIUS: f32 = 10.0;

/// Straight (unpremultiplied) RGBA pixels.
#[derive(Default, PartialEq)]
pub struct Image {
    pub width: usize,
    pub height: usize,
    pub rgba: Vec<u8>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct ReferenceLayer {
    /// Session identity for selection and textures; never saved.
    #[serde(skip)]
    pub id: u64,
    pub name: String,
    /// Image center in document pixels.
    pub center: [f32; 2],
    /// Document pixels per image pixel.
    pub scale: f32,
    /// Clockwise degrees.
    pub rotation: f32,
    pub opacity: f32,
    pub visible: bool,
    pub locked: bool,
    /// Drawn over the artwork rather than between paper and artwork.
    pub above: bool,
    #[serde(default)]
    pub flip: bool,
    #[serde(skip)]
    pub image: Arc<Image>,
    /// Additional placements on this reference board (the primary image above
    /// keeps existing project files compatible). No nested boards.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub images: Vec<ReferenceLayer>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<crate::reference_stack::Position>,
}

impl ReferenceLayer {
    pub fn placements(&self) -> impl Iterator<Item = &Self> {
        std::iter::once(self).chain(&self.images)
    }
    fn axes(&self) -> (Vec2, Vec2) {
        let (sin, cos) = self.rotation.to_radians().sin_cos();
        (vec2(cos, sin), vec2(-sin, cos))
    }
    /// Corners in document pixels: image top-left, top-right, bottom-right,
    /// bottom-left.
    pub fn corners(&self) -> [Pos2; 4] {
        let (x, y) = self.axes();
        let half = vec2(self.image.width as f32, self.image.height as f32) * self.scale / 2.0;
        let center = pos2(self.center[0], self.center[1]);
        [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)]
            .map(|(sx, sy)| center + x * half.x * sx + y * half.y * sy)
    }
    pub fn contains(&self, point: Pos2) -> bool {
        let (x, y) = self.axes();
        let offset = point - pos2(self.center[0], self.center[1]);
        let half = vec2(self.image.width as f32, self.image.height as f32) * self.scale / 2.0;
        offset.dot(x).abs() <= half.x && offset.dot(y).abs() <= half.y
    }
    /// Scale and center that fit the image inside a canvas with a margin.
    pub fn fit(&mut self, width: usize, height: usize) {
        let factor = (width as f32 / self.image.width as f32)
            .min(height as f32 / self.image.height as f32)
            * 0.9;
        self.scale = factor.clamp(MIN_SCALE, MAX_SCALE);
        self.center = [width as f32 / 2.0, height as f32 / 2.0];
        self.rotation = 0.0;
    }
}

/// Checks reference metadata, and the images too once they are attached.
pub fn validate(references: &[ReferenceLayer], images: bool) -> Result<(), String> {
    if references.len() > MAX_REFERENCES {
        return Err(format!(
            "At most {MAX_REFERENCES} reference layers are supported"
        ));
    }
    let mut total = 0usize;
    for board in references {
        if board.images.len() >= MAX_IMAGES
            || board
                .images
                .iter()
                .any(|i| !i.images.is_empty() || i.position.is_some())
        {
            return Err(format!(
                "Reference layers support at most {MAX_IMAGES} images, without nested boards"
            ));
        }
        if let Some(position) = &board.position {
            position.validate()?;
        }
        for reference in board.placements() {
            let finite = reference
                .center
                .iter()
                .all(|v| v.is_finite() && v.abs() <= 1.0e6)
                && reference.rotation.is_finite();
            if reference.name.len() > MAX_NAME_BYTES
                || !finite
                || !(MIN_SCALE..=MAX_SCALE).contains(&reference.scale)
                || !(0.0..=1.0).contains(&reference.opacity)
            {
                return Err("Invalid reference layer settings".into());
            }
            let image = &reference.image;
            total = total.saturating_add(image.rgba.len());
            if images
                && (image.width == 0
                    || image.height == 0
                    || image.width > MAX_SIDE
                    || image.height > MAX_SIDE
                    || image.rgba.len() != image.width * image.height * 4)
            {
                return Err("Invalid reference image".into());
            }
        }
    }
    // Retain compatibility with the old eight 4096-square image ceiling.
    if total > 512 * 1024 * 1024 {
        return Err("Reference images exceed the project memory limit".into());
    }
    Ok(())
}

/// Append the reference images as PNG records after the artwork payload.
pub fn encode_trailer(
    references: &[ReferenceLayer],
    writer: &mut impl std::io::Write,
    cancel: &dyn Fn() -> Result<(), String>,
) -> Result<(), String> {
    use image::ImageEncoder;
    if references.is_empty() {
        return Ok(());
    }
    validate(references, true)?;
    let tag = if references
        .iter()
        .any(|b| b.position.is_some() || !b.images.is_empty())
    {
        STACK_TRAILER
    } else {
        TRAILER
    };
    writer.write_all(tag).map_err(|e| e.to_string())?;
    writer
        .write_all(
            &(references.iter().map(|r| 1 + r.images.len()).sum::<usize>() as u32).to_le_bytes(),
        )
        .map_err(|e| e.to_string())?;
    for reference in references.iter().flat_map(ReferenceLayer::placements) {
        cancel()?;
        let image = &reference.image;
        let mut png = Vec::new();
        image::codecs::png::PngEncoder::new_with_quality(
            &mut png,
            image::codecs::png::CompressionType::Fast,
            image::codecs::png::FilterType::Adaptive,
        )
        .write_image(
            &image.rgba,
            image.width as u32,
            image.height as u32,
            image::ExtendedColorType::Rgba8,
        )
        .map_err(|e| format!("Reference image encoding failed: {e}"))?;
        writer
            .write_all(&(png.len() as u32).to_le_bytes())
            .map_err(|e| e.to_string())?;
        writer.write_all(&png).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Read the images for metadata-declared references, after the trailer tag.
pub fn decode_trailer(
    references: &mut [ReferenceLayer],
    reader: &mut impl std::io::Read,
) -> Result<(), String> {
    let mut word = [0u8; 4];
    let mut read_u32 = |reader: &mut dyn std::io::Read| -> Result<usize, String> {
        reader
            .read_exact(&mut word)
            .map_err(|e| format!("Damaged reference images: {e}"))?;
        Ok(u32::from_le_bytes(word) as usize)
    };
    if read_u32(reader)? != references.iter().map(|r| 1 + r.images.len()).sum::<usize>()
        || references.is_empty()
    {
        return Err("Reference images do not match project metadata".into());
    }
    let mut total = 0usize;
    for reference in references.iter_mut().flat_map(|r| {
        std::iter::once(&mut r.image).chain(r.images.iter_mut().map(|i| &mut i.image))
    }) {
        let length = read_u32(reader)?;
        if length == 0 || length > MAX_PNG_BYTES {
            return Err("Invalid reference image record".into());
        }
        let mut png = vec![0; length];
        reader
            .read_exact(&mut png)
            .map_err(|e| format!("Damaged reference images: {e}"))?;
        let mut decoder =
            image::ImageReader::with_format(std::io::Cursor::new(png), image::ImageFormat::Png);
        let mut limits = image::Limits::default();
        limits.max_image_width = Some(MAX_SIDE as u32);
        limits.max_image_height = Some(MAX_SIDE as u32);
        limits.max_alloc = Some((MAX_SIDE * MAX_SIDE * 4) as u64);
        decoder.limits(limits);
        let rgba = decoder
            .decode()
            .map_err(|e| format!("Damaged reference image: {e}"))?
            .into_rgba8();
        total += rgba.len();
        if total > 512 * 1024 * 1024 {
            return Err("Reference images exceed the project memory limit".into());
        }
        *reference = Arc::new(Image {
            width: rgba.width() as usize,
            height: rgba.height() as usize,
            rgba: rgba.into_raw(),
        });
    }
    validate(references, true)
}

pub fn validate_new_image(image: &Image) -> Result<(), String> {
    if image.width == 0
        || image.height == 0
        || image.width > MAX_SIDE
        || image.height > MAX_SIDE
        || image
            .width
            .checked_mul(image.height)
            .and_then(|p| p.checked_mul(4))
            != Some(image.rgba.len())
    {
        Err("Invalid reference image".into())
    } else {
        Ok(())
    }
}

pub fn downscale(image: Image) -> Result<Image, String> {
    let longest = image.width.max(image.height);
    if longest <= IMPORT_SIDE {
        return Ok(image);
    }
    let factor = IMPORT_SIDE as f32 / longest as f32;
    let source = image::RgbaImage::from_raw(image.width as u32, image.height as u32, image.rgba)
        .ok_or("Invalid reference pixels")?;
    let resized = image::imageops::resize(
        &source,
        ((image.width as f32 * factor).round() as u32).max(1),
        ((image.height as f32 * factor).round() as u32).max(1),
        image::imageops::FilterType::Triangle,
    );
    Ok(Image {
        width: resized.width() as usize,
        height: resized.height() as usize,
        rgba: resized.into_raw(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;

    fn image(width: usize, height: usize, seed: u8) -> Image {
        Image {
            width,
            height,
            rgba: (0..width * height * 4)
                .map(|i| (i as u8).wrapping_mul(31).wrapping_add(seed))
                .collect(),
        }
    }

    fn reference(seed: u8, above: bool) -> ReferenceLayer {
        ReferenceLayer {
            id: 0,
            name: format!("Pose {seed}"),
            center: [40.5, 22.0],
            scale: 0.75,
            rotation: -12.5,
            opacity: 0.4,
            visible: true,
            locked: seed.is_multiple_of(2),
            above,
            flip: true,
            image: Arc::new(image(13 + seed as usize, 7, seed)),
            images: Vec::new(),
            position: None,
        }
    }

    fn same(a: &ReferenceLayer, b: &ReferenceLayer) -> bool {
        a.name == b.name
            && a.center == b.center
            && a.scale == b.scale
            && a.rotation == b.rotation
            && a.opacity == b.opacity
            && (a.visible, a.locked, a.above, a.flip) == (b.visible, b.locked, b.above, b.flip)
            && a.image == b.image
    }

    #[test]
    fn projects_round_trip_reference_layers_beside_selection_masks() {
        let directory = tempfile::tempdir().unwrap();
        for masked in [false, true] {
            let mut doc = Document::new(64, 48);
            doc.layers[0]
                .pixels
                .writer()
                .rgba_mut(5)
                .copy_from_slice(&[9, 8, 7, 255]);
            if masked {
                let mask = crate::selection_mask::Mask::from_coverage(
                    64,
                    48,
                    (0..64 * 48).map(|i| (i % 256) as u8).collect(),
                )
                .unwrap();
                doc.selection = mask.bounds();
                doc.selection_mask = Some(mask);
            }
            doc.references = vec![reference(1, true), reference(2, false)];
            let path = directory.path().join(format!("refs-{masked}.dudel"));
            crate::storage::save_project(&path, &doc).unwrap();
            let loaded = crate::storage::load_project(&path).unwrap();
            assert_eq!(loaded.references.len(), 2);
            for (a, b) in loaded.references.iter().zip(&doc.references) {
                assert!(same(a, b));
            }
            assert_eq!(loaded.selection_mask, doc.selection_mask);
            assert_eq!(loaded.composite(), doc.composite());
        }
        // Documents without references keep their earlier bytes.
        let mut bytes = Vec::new();
        crate::tile_project::encode(&Document::new(32, 32), &mut bytes).unwrap();
        let mut payload = Vec::new();
        use std::io::Read;
        flate2::read::GzDecoder::new(&bytes[crate::tile_project::MAGIC.len()..])
            .read_to_end(&mut payload)
            .unwrap();
        assert!(!payload.windows(4).any(|w| w == b"REFS"));
    }

    #[test]
    fn damaged_or_mismatched_reference_records_are_rejected() {
        let mut doc = Document::new(16, 16);
        doc.references = vec![reference(3, true)];
        let mut bytes = Vec::new();
        crate::tile_project::encode(&doc, &mut bytes).unwrap();
        let header = crate::tile_project::MAGIC.len();
        assert!(crate::tile_project::decode(&bytes[header..]).is_ok());
        // Metadata that names a reference without its image trailer.
        let mut missing = Vec::new();
        {
            use std::io::Write;
            let mut gzip = flate2::write::GzEncoder::new(&mut missing, flate2::Compression::fast());
            let metadata = crate::binary_project::metadata(&doc).unwrap();
            gzip.write_all(&(metadata.len() as u32).to_le_bytes())
                .unwrap();
            gzip.write_all(&metadata).unwrap();
            gzip.write_all(&0u32.to_le_bytes()).unwrap();
            gzip.finish().unwrap();
        }
        assert!(crate::tile_project::decode(&missing[..]).is_err());
        // A record count that disagrees with the metadata.
        let mut images = Vec::new();
        encode_trailer(&doc.references, &mut images, &|| Ok(())).unwrap();
        images[5] = 2;
        let mut references = vec![reference(3, true)];
        assert!(decode_trailer(&mut references, &mut &images[5..]).is_err());
        // Invalid settings fail validation; legacy v3 refuses references.
        let mut bad = reference(4, true);
        bad.scale = f32::NAN;
        assert!(validate(&[bad], false).is_err());
        assert!(validate(&vec![reference(5, true); MAX_REFERENCES + 1], false).is_err());
        assert!(crate::binary_project::encode(&doc, Vec::new()).is_err());
    }
}
