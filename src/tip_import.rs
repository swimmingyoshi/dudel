use crate::{
    Dudel,
    brush::{Tip, TipImage},
};

pub fn from_png(path: &std::path::Path) -> Result<TipImage, String> {
    mask_from_png(path, true)
}

fn mask_from_png(path: &std::path::Path, square: bool) -> Result<TipImage, String> {
    use image::ImageDecoder;
    let mut reader = image::ImageReader::open(path).map_err(|e| e.to_string())?;
    reader.set_format(image::ImageFormat::Png);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(1024);
    limits.max_image_height = Some(1024);
    limits.max_alloc = Some(16 * 1024 * 1024);
    reader.limits(limits);
    let decoder = reader.into_decoder().map_err(|e| e.to_string())?;
    let (width, height) = decoder.dimensions();
    if width == 0 || height == 0 {
        return Err("Tip image must have positive dimensions".into());
    }
    let rgba = image::DynamicImage::from_decoder(decoder)
        .map_err(|e| e.to_string())?
        .into_rgba8();
    // Pad into a square so the source aspect is retained by the image tip.
    let scale = (128.0_f32 / width.max(height) as f32).min(1.0);
    let resized = image::imageops::resize(
        &rgba,
        (width as f32 * scale).round().max(1.0) as u32,
        (height as f32 * scale).round().max(1.0) as u32,
        image::imageops::FilterType::Triangle,
    );
    let side = resized.width().max(resized.height());
    let (mask_width, mask_height) = if square {
        (side, side)
    } else {
        (resized.width(), resized.height())
    };
    let mut alpha = vec![0; (mask_width * mask_height) as usize];
    let offset_x = (mask_width - resized.width()) / 2;
    let offset_y = (mask_height - resized.height()) / 2;
    for (x, y, pixel) in resized.enumerate_pixels() {
        let luminance =
            (u32::from(pixel[0]) * 30 + u32::from(pixel[1]) * 59 + u32::from(pixel[2]) * 11) / 100;
        alpha[((y + offset_y) * mask_width + x + offset_x) as usize] =
            ((255 - luminance) * u32::from(pixel[3]) / 255) as u8;
    }
    if alpha.iter().all(|a| *a == 0) {
        return Err("Tip is empty: use dark marks on white or transparent background".into());
    }
    let tip = TipImage {
        width: mask_width as usize,
        height: mask_height as usize,
        alpha: alpha.into(),
    };
    tip.validate()?;
    Ok(tip)
}
impl Dudel {
    pub(crate) fn import_grain(&mut self) {
        let dialog = rfd::AsyncFileDialog::new().add_filter("PNG paper grain", &["png"]);
        self.ask_open_file(dialog, |app, path| match mask_from_png(&path, false) {
            Ok(mask) => {
                app.brush.grain = Some(mask);
                app.brush.grain_amount = 0.5;
                app.scratchpad.reset_stroke();
                app.status = "Grain imported — Save as new to keep this brush".into();
            }
            Err(error) => app.status = format!("Grain import: {error}"),
        });
    }
    pub(crate) fn import_tip(&mut self) {
        let dialog = rfd::AsyncFileDialog::new().add_filter("PNG brush tip", &["png"]);
        self.ask_open_file(dialog, |app, path| match from_png(&path) {
            Ok(image) => {
                app.brush.image = Some(image);
                app.brush.tip = Tip::Image;
                app.scratchpad.reset_stroke();
                app.status = "Tip imported — Save as new to keep this brush".into();
            }
            Err(e) => app.status = format!("Tip import: {e}"),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tip_import_uses_luminance_alpha_and_preserves_aspect() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("tip.png");
        image::save_buffer_with_format(
            &path,
            &[0, 0, 0, 255, 255, 255, 255, 255],
            2,
            1,
            image::ColorType::Rgba8,
            image::ImageFormat::Png,
        )
        .unwrap();
        let tip = from_png(&path).unwrap();
        assert_eq!((tip.width, tip.height), (2, 2));
        assert_eq!(tip.alpha, vec![255, 0, 0, 0]);
        let grain = mask_from_png(&path, false).unwrap();
        assert_eq!((grain.width, grain.height), (2, 1));
        assert_eq!(grain.alpha, vec![255, 0]);
        image::save_buffer_with_format(
            &path,
            &[255, 255, 255, 255],
            1,
            1,
            image::ColorType::Rgba8,
            image::ImageFormat::Png,
        )
        .unwrap();
        assert!(from_png(&path).is_err());
    }
}
