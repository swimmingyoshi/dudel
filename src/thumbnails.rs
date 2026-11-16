use crate::{
    Dudel,
    document::{Document, Layer},
    egui,
};

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
    if layer.folder {
        // Folder rows draw an icon instead; keep one texture per stack entry.
        return vec![0; side * side * 4];
    }
    preview(width, height, side, |index| {
        let pixel = layer.pixels.rgba(index);
        (
            [pixel[0], pixel[1], pixel[2]],
            pixel[3] as f32 / 255.0 * layer.mask_coverage(index, width),
        )
    })
}

/// A top-level folder's contents composited the way the canvas shows them,
/// for its frame in the animation timeline. Only the folder is composited,
/// sampled at a few times the preview's resolution.
pub(crate) fn folder_pixels(doc: &Document, folder: usize, side: usize) -> Vec<u8> {
    let mut view = doc.clone();
    view.layers = doc.layers[doc.block(folder)].to_vec();
    view.active = 0;
    if let Some(header) = view.layers.last_mut() {
        header.visible = true;
    }
    let step = (doc.width.max(doc.height) / (side * 4)).max(1);
    let Ok((rgba, [columns, _])) = view.composite_reduced(step, true, || Ok(())) else {
        return vec![0; side * side * 4];
    };
    preview(doc.width, doc.height, side, |index| {
        let (x, y) = (index % doc.width / step, index / doc.width / step);
        let pixel = &rgba[(y * columns + x) * 4..][..4];
        (
            [pixel[0], pixel[1], pixel[2]],
            f32::from(pixel[3]) / 255.0,
        )
    })
}

/// A key for what a frame folder's preview shows besides its layers'
/// pixels, which `refresh_thumbnails` tracks layer by layer: its layers and
/// how they are composited. Never 0 (a layer) or 1 (any other folder).
fn folder_key(doc: &Document, folder: usize) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for layer in &doc.layers[doc.block(folder)] {
        layer.id.hash(&mut hasher);
        (layer.visible, layer.clipped, layer.folder, layer.depth).hash(&mut hasher);
        (layer.opacity.to_bits(), layer.fill.to_bits()).hash(&mut hasher);
        layer.mask.as_ref().map(|mask| mask.enabled).hash(&mut hasher);
        format!("{:?}{:?}{:?}", layer.blend, layer.effects, layer.adjustment).hash(&mut hasher);
    }
    hasher.finish().max(2)
}

/// A `side` square preview of a `width` × `height` picture on a
/// checkerboard, letterboxed. `sample` gives a pixel's color and coverage
/// by its index; a few are averaged per preview pixel.
fn preview(
    width: usize,
    height: usize,
    side: usize,
    sample: impl Fn(usize) -> ([u8; 3], f32),
) -> Vec<u8> {
    let mut output = vec![255; side * side * 4];
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
                    let (pixel, alpha) = sample(py * width + px);
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
        // A layer without a preview (new, or from a replaced document) is
        // drawn without waiting for the throttles below.
        let complete = self.thumbnail_stamps.len() == self.doc.layers.len()
            && self
                .doc
                .layers
                .iter()
                .all(|layer| self.thumbnail_stamps.contains_key(&layer.id));
        let stale = self.thumbnail_revision != Some(self.revision) || !complete;
        if stale
            && complete
            && (self.stroking || self.render_queue.busy() || self.performance.defer_optional())
        {
            ctx.request_repaint_after(std::time::Duration::from_millis(150));
            return;
        }
        if !stale {
            return;
        }
        if self.thumbnail_revision.is_some()
            && complete
            && self.thumbnail_timer.elapsed().as_millis() < 200
        {
            ctx.request_repaint_after(std::time::Duration::from_millis(200));
            return;
        }
        // Keyed by layer identity: inserting, deleting or reordering layers
        // keeps every other layer's thumbnail instead of shifting them all.
        let present: std::collections::HashSet<_> =
            self.doc.layers.iter().map(|layer| layer.id).collect();
        self.layer_thumbnails.retain(|id, _| present.contains(id));
        self.mask_thumbnails.retain(|id, _| present.contains(id));
        self.thumbnail_stamps.retain(|id, _| present.contains(id));
        let mut prepared = 0;
        let animated = self.doc.animation.is_some();
        for (index, layer) in self.doc.layers.iter().enumerate() {
            // A top-level folder is a frame in an animation, and the timeline
            // shows its contents; anywhere else a folder shows an icon.
            let frame = animated
                && layer.folder
                && layer.depth == 0
                && layer.display_reference.is_none();
            let kind = match (layer.folder, frame) {
                (false, _) => 0,
                (true, false) => 1,
                (true, true) => folder_key(&self.doc, index),
            };
            // Only layers whose pixels changed are redrawn; painting one layer
            // no longer regenerates every thumbnail every 200ms.
            let current = self.layer_thumbnails.contains_key(&layer.id)
                && self.mask_thumbnails.contains_key(&layer.id) == layer.mask.is_some()
                && self
                    .thumbnail_stamps
                    .get(&layer.id)
                    .is_some_and(|(drawn, stamp, mask)| {
                        *drawn == kind
                            && stamp.matches(&layer.pixels)
                            && mask.matches(layer.mask.as_ref())
                    });
            if current {
                continue;
            }
            // A changed layer inside a folder changes the folder's frame:
            // its header comes later in the stack and is redrawn after it.
            if animated && layer.depth > 0 {
                let top = (index + 1..self.doc.layers.len())
                    .find(|&above| self.doc.layers[above].depth == 0);
                if let Some(top) = top {
                    self.thumbnail_stamps.remove(&self.doc.layers[top].id);
                }
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
                if let Some(texture) = self.mask_thumbnails.get_mut(&layer.id) {
                    texture.set(image, egui::TextureOptions::LINEAR);
                } else {
                    self.mask_thumbnails.insert(
                        layer.id,
                        ctx.load_texture(
                            format!("mask preview {:?}", layer.id),
                            image,
                            egui::TextureOptions::LINEAR,
                        ),
                    );
                }
            } else {
                self.mask_thumbnails.remove(&layer.id);
            }
            let rect = if layer.folder {
                [0; 4]
            } else {
                [0, 0, self.doc.width, self.doc.height]
            };
            self.thumbnail_stamps.insert(
                layer.id,
                (
                    kind,
                    layer.pixels.region_stamp(rect),
                    crate::layer_mask::LayerStamp::new(
                        layer.mask.as_ref(),
                        [0, 0, self.doc.width, self.doc.height],
                    ),
                ),
            );
            let preview = if frame {
                folder_pixels(&self.doc, index, SIDE)
            } else {
                pixels(layer, self.doc.width, self.doc.height, SIDE)
            };
            let image = egui::ColorImage::from_rgba_unmultiplied([SIDE, SIDE], &preview);
            if let Some(texture) = self.layer_thumbnails.get_mut(&layer.id) {
                texture.set(image, egui::TextureOptions::LINEAR);
            } else {
                self.layer_thumbnails.insert(
                    layer.id,
                    ctx.load_texture(
                        format!("layer preview {:?}", layer.id),
                        image,
                        egui::TextureOptions::LINEAR,
                    ),
                );
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
        let id = app.layer_thumbnails[&app.doc.layers[0].id].id();
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
    fn a_frame_folder_previews_its_contents_and_follows_their_changes() {
        let ctx = egui::Context::default();
        let mut doc = Document::new(8, 8);
        doc.layers = vec![
            Layer::raster("Frame 1".into(), 8, 8),
            Layer {
                depth: 1,
                ..Layer::raster("Color".into(), 8, 8)
            },
            Layer {
                depth: 1,
                ..Layer::raster("Lines".into(), 8, 8)
            },
            Layer::new_folder("Frame 2".into()),
        ];
        doc.active = 1;
        doc.layers[1]
            .pixels
            .writer()
            .write_region(8, [0, 0, 8, 8], &[200, 0, 0, 255].repeat(64));
        let mut app = crate::Dudel {
            doc,
            ..Default::default()
        };
        let refresh = |app: &mut crate::Dudel| {
            app.revision += 1;
            app.thumbnail_timer = std::time::Instant::now() - std::time::Duration::from_secs(1);
            let mut set = Vec::new();
            for _ in 0..4 {
                let output = ctx.run(Default::default(), |ctx| app.refresh_thumbnails(ctx));
                set.extend(output.textures_delta.set.into_iter().map(|(id, _)| id));
            }
            set
        };
        let folder = app.doc.layers[3].id;
        // Outside an animation a folder keeps its empty preview.
        refresh(&mut app);
        assert!(folder_pixels(&app.doc, 3, 10)[..4] != [0; 4]);
        assert!(
            pixels(&app.doc.layers[3], 8, 8, 10)
                .chunks_exact(4)
                .all(|p| p == [0; 4])
        );
        // As a frame it shows the red layer inside it.
        app.doc.animation = Some(Default::default());
        let texture = app.layer_thumbnails[&folder].id();
        assert!(refresh(&mut app).contains(&texture));
        let preview = folder_pixels(&app.doc, 3, 10);
        assert!(preview.chunks_exact(4).all(|p| p == [200, 0, 0, 255]));
        // Painting a layer inside redraws the frame; nothing else does.
        app.doc.layers[2]
            .pixels
            .writer()
            .write_region(8, [0, 0, 8, 8], &[0, 0, 200, 255].repeat(64));
        assert!(refresh(&mut app).contains(&texture));
        assert!(
            folder_pixels(&app.doc, 3, 10)
                .chunks_exact(4)
                .all(|p| p == [0, 0, 200, 255])
        );
        assert!(!refresh(&mut app).contains(&texture));
        // So does hiding a layer inside.
        app.doc.layers[2].visible = false;
        assert!(refresh(&mut app).contains(&texture));
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
                .values()
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
        // Previews follow layer identity: reordering redraws nothing, and a
        // layer inserted below the others redraws only itself.
        app.doc.layers.swap(0, 1);
        app.revision += 1;
        assert_eq!(uploads(&mut app), 0);
        app.doc.active = 0;
        app.doc.insert_layer_above().unwrap();
        app.revision += 1;
        assert_eq!(uploads(&mut app), 1);
        assert_eq!(app.layer_thumbnails.len(), app.doc.layers.len());
        // A deleted layer's preview is released.
        let removed = app.doc.layers.remove(1).id;
        app.doc.active = 0;
        app.revision += 1;
        assert_eq!(uploads(&mut app), 0);
        assert!(!app.layer_thumbnails.contains_key(&removed));
        assert_eq!(app.thumbnail_revision, Some(app.revision));
    }
}
