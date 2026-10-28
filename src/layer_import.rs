use crate::Dudel;

pub(crate) fn resize_rgba(source: &image::RgbaImage, width: u32, height: u32) -> image::RgbaImage {
    let premultiplied = image::ImageBuffer::from_fn(source.width(), source.height(), |x, y| {
        let pixel = source.get_pixel(x, y);
        let alpha = pixel[3] as f32 / 255.0;
        image::Rgba([
            pixel[0] as f32 / 255.0 * alpha,
            pixel[1] as f32 / 255.0 * alpha,
            pixel[2] as f32 / 255.0 * alpha,
            alpha,
        ])
    });
    let resized = image::imageops::resize(
        &premultiplied,
        width,
        height,
        image::imageops::FilterType::Triangle,
    );
    image::ImageBuffer::from_fn(width, height, |x, y| {
        let pixel = resized.get_pixel(x, y);
        let color = |index| {
            if pixel[3] > 0.0 {
                (pixel[index] / pixel[3] * 255.0).round().clamp(0.0, 255.0) as u8
            } else {
                0
            }
        };
        image::Rgba([
            color(0),
            color(1),
            color(2),
            (pixel[3] * 255.0).round().clamp(0.0, 255.0) as u8,
        ])
    })
}

impl Dudel {
    pub(crate) fn import_png_layer(&mut self) {
        if !self.doc.has_room(1, 1) {
            self.status = "Layer limit reached (32)".into();
            return;
        }
        let dialog = self.dialog_in(
            crate::locations::Kind::Projects,
            rfd::AsyncFileDialog::new().add_filter("Images", &crate::image_paste::EXTENSIONS),
        );
        // Each chosen image becomes a layer above the current one.
        self.ask_open_files(dialog, |app, paths| app.paste_image_files(paths, None));
    }
}
