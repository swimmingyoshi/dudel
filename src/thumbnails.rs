use crate::{Dudel, document::Layer, egui};

const SIDE: usize = 40;

/// Raw coverage preview; disabled masks retain their editable grayscale values.
pub(crate) fn mask_pixels(mask: &crate::layer_mask::Mask, side: usize) -> Vec<u8> {
    let [width, height] = mask.dimensions();
    let scale = side as f32 / width.max(height) as f32;
    let left = (side as f32 - width as f32 * scale) / 2.0;
    let top = (side as f32 - height as f32 * scale) / 2.0;
    let mut output = vec![255; side * side * 4];
    for y in 0..side {
        for x in 0..side {
            let at = [
                ((x as f32 + 0.5 - left) / scale).floor(),
                ((y as f32 + 0.5 - top) / scale).floor(),
            ];
            let value =
                if at[0] >= 0.0 && at[1] >= 0.0 && at[0] < width as f32 && at[1] < height as f32 {
                    mask.at(at[0] as usize, at[1] as usize)
                } else {
                    45
                };
            output[(y * side + x) * 4..(y * side + x) * 4 + 3].fill(value);
        }
    }
    output
}

/// Small checkerboard preview of raw layer contents. Bounded subsampling keeps
/// preview work independent of large document dimensions; fine detail is approximate.
pub fn pixels(layer: &Layer, width: usize, height: usize, side: usize) -> Vec<u8> {
    let mut output = vec![255; side * side * 4];
    if layer.folder {
        // Folder rows draw an icon instead; keep one texture per stack entry.
        output.fill(0);
        return output;
    }
    let scale = side as f32 / width.max(height) as f32;
    let shown_width = width as f32 * scale;
    let shown_height = height as f32 * scale;
    let left = (side as f32 - shown_width) / 2.0;
    let top = (side as f32 - shown_height) / 2.0;
    for y in 0..side {
        for x in 0..side {
            let out = &mut output[(y * side + x) * 4..(y * side + x) * 4 + 4];
            if x as f32 + 0.5 < left
                || x as f32 + 0.5 >= left + shown_width
                || y as f32 + 0.5 < top
                || y as f32 + 0.5 >= top + shown_height
            {
                out[..3].fill(45);
                continue;
            }
            let count = ((1.0 / scale).ceil() as usize).clamp(1, 4);
            let mut rgba = [0.0_f32; 4];
            for sy in 0..count {
                for sx in 0..count {
                    let px = ((x as f32 - left + (sx as f32 + 0.5) / count as f32) / scale)
                        .floor()
                        .clamp(0.0, (width - 1) as f32) as usize;
                    let py = ((y as f32 - top + (sy as f32 + 0.5) / count as f32) / scale)
                        .floor()
                        .clamp(0.0, (height - 1) as f32) as usize;
                    let pixel = layer.pixels.rgba(py * width + px);
                    let alpha =
                        pixel[3] as f32 / 255.0 * layer.mask_coverage(py * width + px, width);
                    for c in 0..3 {
                        rgba[c] += pixel[c] as f32 * alpha;
                    }
                    rgba[3] += alpha;
                }
            }
            let divisor = (count * count) as f32;
            let background = if (x / 5 + y / 5) % 2 == 0 {
                210.0
            } else {
                170.0
            };
            for c in 0..3 {
                out[c] = (rgba[c] / divisor + background * (1.0 - rgba[3] / divisor)).round() as u8;
            }
        }
    }
    output
}
impl Dudel {
    pub(crate) fn refresh_thumbnails(&mut self, ctx: &egui::Context) {
        let stale = self.thumbnail_revision != Some(self.revision)
            || self.layer_thumbnails.len() != self.doc.layers.len();
        if stale
            && self.layer_thumbnails.len() == self.doc.layers.len()
            && (self.stroking || self.render_queue.busy() || self.performance.defer_optional())
        {
            ctx.request_repaint_after(std::time::Duration::from_millis(150));
            return;
        }
        if !stale {
            return;
        }
        if self.thumbnail_revision.is_some()
            && self.layer_thumbnails.len() == self.doc.layers.len()
            && self.thumbnail_timer.elapsed().as_millis() < 200
        {
            ctx.request_repaint_after(std::time::Duration::from_millis(200));
            return;
        }
        self.layer_thumbnails.truncate(self.doc.layers.len());
        self.mask_thumbnails
            .resize_with(self.doc.layers.len(), || None);
        self.mask_thumbnails.truncate(self.doc.layers.len());
        self.thumbnail_stamps.truncate(self.doc.layers.len());
        let mut prepared = 0;
        for (index, layer) in self.doc.layers.iter().enumerate() {
            // Only layers whose pixels changed are redrawn; painting one layer
            // no longer regenerates every thumbnail every 200ms.
            let current = index < self.layer_thumbnails.len()
                && self.mask_thumbnails[index].is_some() == layer.mask.is_some()
                && self
                    .thumbnail_stamps
                    .get(index)
                    .is_some_and(|(folder, stamp, mask)| {
                        *folder == layer.folder
                            && stamp.matches(&layer.pixels)
                            && mask.matches(layer.mask.as_ref())
                    });
            if current {
                continue;
            }
            // Large imports/reorders settle over several updates instead of
            // preparing every changed layer on the input thread at once.
            if prepared == 2 {
                ctx.request_repaint();
                return;
            }
            prepared += 1;
            if let Some(mask) = &layer.mask {
                let image = egui::ColorImage::from_rgba_unmultiplied(
                    [SIDE, SIDE],
                    &mask_pixels(&mask.coverage, SIDE),
                );
                if let Some(texture) = &mut self.mask_thumbnails[index] {
                    texture.set(image, egui::TextureOptions::LINEAR);
                } else {
                    self.mask_thumbnails[index] = Some(ctx.load_texture(
                        format!("mask preview {index}"),
                        image,
                        egui::TextureOptions::LINEAR,
                    ));
                }
            } else {
                self.mask_thumbnails[index] = None;
            }
            let rect = if layer.folder {
                [0; 4]
            } else {
                [0, 0, self.doc.width, self.doc.height]
            };
            let stamp = (
                layer.folder,
                layer.pixels.region_stamp(rect),
                crate::layer_mask::LayerStamp::new(
                    layer.mask.as_ref(),
                    [0, 0, self.doc.width, self.doc.height],
                ),
            );
            if index < self.thumbnail_stamps.len() {
                self.thumbnail_stamps[index] = stamp;
            } else {
                self.thumbnail_stamps.push(stamp);
            }
            let image = egui::ColorImage::from_rgba_unmultiplied(
                [SIDE, SIDE],
                &pixels(layer, self.doc.width, self.doc.height, SIDE),
            );
            if let Some(texture) = self.layer_thumbnails.get_mut(index) {
                texture.set(image, egui::TextureOptions::LINEAR);
            } else {
                self.layer_thumbnails.push(ctx.load_texture(
                    format!("layer preview {index}"),
                    image,
                    egui::TextureOptions::LINEAR,
                ));
            }
        }
        self.thumbnail_revision = Some(self.revision);
        self.thumbnail_timer = std::time::Instant::now();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    #[test]
    fn mask_only_changes_update_the_affected_thumbnail_and_disabled_mask_restores_artwork() {
        let ctx = egui::Context::default();
        let mut app = crate::Dudel {
            doc: Document::new(1, 1),
            ..Default::default()
        };
        app.doc.version = 2;
        app.doc.layers[0].pixels = vec![201, 17, 83, 255].into();
        app.doc.layers[0].mask = Some(crate::layer_mask::LayerMask {
            id: crate::document::LayerId::new(),
            enabled: true,
            linked: true,
            coverage: crate::layer_mask::Mask::new(1, 1, 255).unwrap(),
        });
        let _ = ctx.run(Default::default(), |ctx| app.refresh_thumbnails(ctx));
        let id = app.layer_thumbnails[0].id();
        app.doc.layers[0]
            .mask
            .as_mut()
            .unwrap()
            .coverage
            .writer()
            .set(0, 0, 0);
        app.changed();
        app.thumbnail_timer = std::time::Instant::now() - std::time::Duration::from_secs(1);
        let output = ctx.run(Default::default(), |ctx| app.refresh_thumbnails(ctx));
        assert!(
            output
                .textures_delta
                .set
                .iter()
                .any(|(updated, _)| *updated == id)
        );
        assert_eq!(
            &pixels(&app.doc.layers[0], 1, 1, 10)[..4],
            &[210, 210, 210, 255]
        );
        app.doc.layers[0].mask.as_mut().unwrap().enabled = false;
        assert!(
            pixels(&app.doc.layers[0], 1, 1, 10)
                .chunks_exact(4)
                .all(|p| p == [201, 17, 83, 255])
        );
    }
    #[test]
    fn preview_preserves_opaque_color_and_shows_transparency() {
        let mut doc = Document::new(1, 1);
        doc.layers[0].pixels = vec![255, 0, 0, 255].into();
        let preview = pixels(&doc.layers[0], 1, 1, 10);
        assert!(preview.chunks_exact(4).all(|p| p == [255, 0, 0, 255]));
        doc.layers[0].pixels[3] = 0;
        let preview = pixels(&doc.layers[0], 1, 1, 10);
        assert_eq!(&preview[..4], &[210, 210, 210, 255]);
        assert_eq!(&preview[20..24], &[170, 170, 170, 255]);
    }
    #[test]
    fn painting_one_layer_redraws_only_its_thumbnail() {
        let ctx = egui::Context::default();
        let mut doc = Document::new(64, 64);
        doc.add_layer();
        doc.add_layer();
        let mut app = crate::Dudel {
            doc,
            ..Default::default()
        };
        let uploads = |app: &mut crate::Dudel| {
            app.thumbnail_timer = std::time::Instant::now() - std::time::Duration::from_secs(1);
            let output = ctx.run(Default::default(), |ctx| app.refresh_thumbnails(ctx));
            let ids: Vec<_> = app
                .layer_thumbnails
                .iter()
                .map(egui::TextureHandle::id)
                .collect();
            output
                .textures_delta
                .set
                .iter()
                .filter(|(id, _)| ids.contains(id))
                .count()
        };
        assert_eq!(uploads(&mut app), 2);
        assert_ne!(app.thumbnail_revision, Some(app.revision));
        assert_eq!(uploads(&mut app), 1);
        assert_eq!(app.thumbnail_revision, Some(app.revision));
        app.doc.layers[1].pixels.writer().write_region(
            64,
            [5, 5, 9, 9],
            &[9, 99, 199, 255].repeat(16),
        );
        app.revision += 1;
        app.stroking = true;
        assert_eq!(uploads(&mut app), 0);
        app.stroking = false;
        assert_eq!(uploads(&mut app), 1);
        // Reordering the painted layer with a blank one changes two positions.
        app.doc.layers.swap(0, 1);
        app.revision += 1;
        assert_eq!(uploads(&mut app), 2);
        app.revision += 1;
        assert_eq!(uploads(&mut app), 0);
    }
}
