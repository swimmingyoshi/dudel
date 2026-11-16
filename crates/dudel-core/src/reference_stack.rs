//! View-only reference boards in the artwork stack. Saved positions bind to
//! process-local layer identities on load, so folders/reorders carry boards.
//! Display snapshots share image allocations; exports never receive them.
use crate::{
    document::{Document, LayerId, MAX_DEPTH},
    reference_layers::ReferenceLayer,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Clone, Serialize, Deserialize)]
pub struct Position {
    pub gap: usize,
    pub depth: u8,
    #[serde(skip)]
    anchor: Option<LayerId>,
    #[serde(skip)]
    parent: Option<LayerId>,
    #[serde(skip)]
    pub bound: bool,
}
impl Position {
    pub fn validate(&self) -> Result<(), String> {
        if self.gap > crate::document::MAX_ENTRIES || self.depth > MAX_DEPTH {
            Err("Invalid reference stack position".into())
        } else {
            Ok(())
        }
    }
    pub fn at(doc: &Document, gap: usize, depth: u8) -> Self {
        let gap = gap.min(doc.layers.len());
        let parent = if depth > 0 {
            (gap..doc.layers.len())
                .find(|&i| doc.layers[i].folder && doc.layers[i].depth + 1 == depth)
        } else {
            None
        };
        Self {
            gap,
            depth: parent.map_or(0, |_| depth),
            anchor: doc.layers.get(gap).map(|l| l.id),
            parent: parent.map(|i| doc.layers[i].id),
            bound: true,
        }
    }
    pub fn resolved(&self, doc: &Document) -> (usize, u8, Option<usize>) {
        let mut gap = self
            .anchor
            .and_then(|id| doc.layers.iter().position(|l| l.id == id))
            .unwrap_or_else(|| self.gap.min(doc.layers.len()));
        if self.anchor.is_none() && self.bound {
            gap = doc.layers.len();
        }
        let parent = self
            .parent
            .and_then(|id| doc.layers.iter().position(|l| l.id == id && l.folder));
        if let Some(parent) = parent {
            let block = doc.block(parent);
            gap = gap.clamp(block.start, parent);
            let depth = doc.layers[parent].depth + 1;
            (valid_gap(doc, gap, depth), depth, Some(parent))
        } else {
            (valid_gap(doc, gap, 0), 0, None)
        }
    }
}

fn valid_gap(doc: &Document, mut gap: usize, depth: u8) -> usize {
    while let Some(layer) = doc.layers.get(gap) {
        if layer.depth < depth || (layer.depth == depth && !layer.folder) {
            break;
        }
        let index = if layer.depth > depth {
            doc.parent(gap).unwrap_or(gap)
        } else {
            gap
        };
        let start = doc.block(index).start;
        if start == gap {
            break;
        }
        gap = start;
    }
    gap
}

pub struct ReferenceDrag(pub u64);

pub struct Sample {
    pub image: Arc<crate::reference_layers::Image>,
    pub center: [f32; 2],
    pub x: [f32; 2],
    pub y: [f32; 2],
    pub bounds: emath::Rect,
    pub flip: bool,
    pub opacity: f32,
}
pub struct Scene {
    pub images: Vec<Sample>,
    pub key: usize,
}
impl Scene {
    pub fn register_allocations(&self, registry: &mut crate::snapshot_memory::Registry) {
        self.visit_allocations(|id, bytes| registry.add_buffer(id, bytes));
    }
    pub fn visit_allocations(&self, mut visit: impl FnMut(usize, usize)) {
        visit(
            self.images.as_ptr() as usize,
            self.images.capacity() * std::mem::size_of::<Sample>(),
        );
        for sample in &self.images {
            visit(
                sample.image.rgba.as_ptr() as usize,
                sample.image.rgba.capacity(),
            );
        }
    }
    pub fn new(board: &ReferenceLayer) -> Self {
        let images = board
            .placements()
            .enumerate()
            .filter(|(i, p)| *i == 0 || p.visible)
            .map(|(i, p)| {
                let (s, c) = p.rotation.to_radians().sin_cos();
                Sample {
                    image: p.image.clone(),
                    center: p.center,
                    x: [c / p.scale, s / p.scale],
                    y: [-s / p.scale, c / p.scale],
                    bounds: emath::Rect::from_points(&p.corners()),
                    flip: p.flip,
                    opacity: if i == 0 { 1.0 } else { p.opacity },
                }
            })
            .collect();
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(1);
        Self {
            images,
            key: NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
        }
    }
    pub fn intersects(&self, area: [usize; 4]) -> bool {
        let rect = emath::Rect::from_min_max(
            emath::pos2(area[0] as f32, area[1] as f32),
            emath::pos2(area[2] as f32, area[3] as f32),
        );
        self.images.iter().any(|i| i.bounds.intersects(rect))
    }
    pub fn pixel(&self, x: usize, y: usize) -> [u8; 4] {
        let p = emath::pos2(x as f32 + 0.5, y as f32 + 0.5);
        let mut out = [0.0_f32; 4];
        for sample in &self.images {
            if !sample.bounds.contains(p) {
                continue;
            }
            let dx = p.x - sample.center[0];
            let dy = p.y - sample.center[1];
            let mut sx = dx * sample.x[0] + dy * sample.x[1] + sample.image.width as f32 / 2.0;
            let sy = dx * sample.y[0] + dy * sample.y[1] + sample.image.height as f32 / 2.0;
            if sx < 0.0
                || sy < 0.0
                || sx >= sample.image.width as f32
                || sy >= sample.image.height as f32
            {
                continue;
            }
            if sample.flip {
                sx = sample.image.width as f32 - sx;
            }
            // Bilinear filtering in premultiplied space avoids colored halos
            // at transparent image edges when scaled or rotated.
            let x = (sx - 0.5).clamp(0.0, (sample.image.width - 1) as f32);
            let y = (sy - 0.5).clamp(0.0, (sample.image.height - 1) as f32);
            let ix = x as usize;
            let iy = y as usize;
            let fx = x - ix as f32;
            let fy = y - iy as f32;
            let mut rgba = [0.0_f32; 4];
            for (px, py, weight) in [
                (ix, iy, (1.0 - fx) * (1.0 - fy)),
                ((ix + 1).min(sample.image.width - 1), iy, fx * (1.0 - fy)),
                (ix, (iy + 1).min(sample.image.height - 1), (1.0 - fx) * fy),
                (
                    (ix + 1).min(sample.image.width - 1),
                    (iy + 1).min(sample.image.height - 1),
                    fx * fy,
                ),
            ] {
                let pixel = &sample.image.rgba[(py * sample.image.width + px) * 4..][..4];
                let a = pixel[3] as f32 / 255.0 * weight;
                for c in 0..3 {
                    rgba[c] += pixel[c] as f32 * a;
                }
                rgba[3] += a;
            }
            let a = rgba[3] * sample.opacity;
            for c in 0..3 {
                out[c] = out[c] * (1.0 - a) + rgba[c] * sample.opacity;
            }
            out[3] = out[3] * (1.0 - a) + a;
        }
        if out[3] == 0.0 {
            [0; 4]
        } else {
            [
                (out[0] / out[3]).round() as u8,
                (out[1] / out[3]).round() as u8,
                (out[2] / out[3]).round() as u8,
                (out[3] * 255.0).round() as u8,
            ]
        }
    }
}
