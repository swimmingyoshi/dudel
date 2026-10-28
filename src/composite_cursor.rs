//! Immutable-source display compositing, bounded by pixel work between yields.
use crate::document::Document;

pub(crate) struct CompositeCursor {
    source: Document,
    rect: [usize; 4],
    pixels: Vec<u8>,
    next: usize,
    transparent: bool,
    pub(crate) snapshot: crate::snapshot_memory::Snapshot,
}
impl CompositeCursor {
    pub(crate) fn new(source: &Document, rect: [usize; 4], transparent: bool) -> Self {
        let rect = [
            rect[0].min(source.width),
            rect[1].min(source.height),
            rect[2].min(source.width),
            rect[3].min(source.height),
        ];
        let count = rect[2].saturating_sub(rect[0]) * rect[3].saturating_sub(rect[1]);
        let source = source.clone();
        let snapshot = crate::snapshot_memory::Snapshot::capture(&source);
        Self {
            source,
            snapshot,
            rect,
            pixels: Vec::with_capacity(count * 4),
            next: 0,
            transparent,
        }
    }
    pub(crate) fn step(&mut self, max_pixels: usize) -> bool {
        let [left, top, right, bottom] = self.rect;
        let width = right.saturating_sub(left);
        let count = width * bottom.saturating_sub(top);
        let end = self.next.saturating_add(max_pixels).min(count);
        for offset in self.next..end {
            let index = (top + offset / width) * self.source.width + left + offset % width;
            self.pixels
                .extend_from_slice(&self.source.composite_display_pixel(index, self.transparent));
        }
        self.next = end;
        self.next == count
    }
    pub(crate) fn capacity_bytes(&self) -> usize {
        self.pixels.capacity()
    }
    pub(crate) fn finish(self) -> Vec<u8> {
        assert_eq!(
            self.pixels.len(),
            self.rect[2].saturating_sub(self.rect[0])
                * self.rect[3].saturating_sub(self.rect[1])
                * 4,
            "Composite is unfinished"
        );
        self.pixels
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sliced_composite_keeps_immutable_pixels_and_matches_clamped_region_oracle() {
        for budget in [1, 127, 4096] {
            let mut doc = Document::new(257, 35);
            doc.layers[0].pixels = [17, 83, 201, 128].repeat(257 * 35).into();
            doc.group_active().unwrap();
            doc.layers[doc.active].opacity = 0.6;
            doc.add_layer();
            doc.layers[doc.active].pixels = [201, 83, 17, 127].repeat(257 * 35).into();
            doc.layers[doc.active].clipped = true;
            doc.layers[doc.active].blend = crate::blend::BlendMode::Multiply;
            for rect in [[2, 3, 255, 34], [0, 0, 999, 999], [20, 20, 10, 10]] {
                let expected = doc.composite_region(rect[0], rect[1], rect[2], rect[3]);
                let mut cursor = CompositeCursor::new(&doc, rect, false);
                let mut complete = cursor.step(budget);
                let old = doc.layers[doc.active].pixels.clone();
                doc.layers[doc.active]
                    .pixels
                    .writer()
                    .rgba_mut(5 * 257 + 5)
                    .copy_from_slice(&[0; 4]);
                doc.layers[doc.active].opacity = 0.1;
                while !complete {
                    complete = cursor.step(budget);
                }
                assert_eq!(cursor.finish(), expected);
                doc.layers[doc.active].pixels = old;
                doc.layers[doc.active].opacity = 1.0;
            }
        }
    }
}
