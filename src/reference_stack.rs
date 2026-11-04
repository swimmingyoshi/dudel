//! View-only reference boards in the artwork stack. Saved positions bind to
//! process-local layer identities on load, so folders/reorders carry boards.
//! Display snapshots share image allocations; exports never receive them.
use crate::{
    Dudel,
    document::{Document, Layer, LayerId, MAX_DEPTH},
    egui,
    reference_layers::ReferenceLayer,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct Position {
    pub gap: usize,
    pub depth: u8,
    #[serde(skip)]
    anchor: Option<LayerId>,
    #[serde(skip)]
    parent: Option<LayerId>,
    #[serde(skip)]
    bound: bool,
}
impl Position {
    pub(crate) fn validate(&self) -> Result<(), String> {
        if self.gap > crate::document::MAX_ENTRIES || self.depth > MAX_DEPTH {
            Err("Invalid reference stack position".into())
        } else {
            Ok(())
        }
    }
    fn at(doc: &Document, gap: usize, depth: u8) -> Self {
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
    fn resolved(&self, doc: &Document) -> (usize, u8, Option<usize>) {
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

pub(crate) struct ReferenceDrag(pub u64);

struct Sample {
    image: Arc<crate::reference_layers::Image>,
    center: [f32; 2],
    x: [f32; 2],
    y: [f32; 2],
    bounds: egui::Rect,
    flip: bool,
    opacity: f32,
}
pub(crate) struct Scene {
    images: Vec<Sample>,
    pub(crate) key: usize,
}
impl Scene {
    pub(crate) fn register_allocations(&self, registry: &mut crate::snapshot_memory::Registry) {
        self.visit_allocations(|id, bytes| registry.add_buffer(id, bytes));
    }
    pub(crate) fn visit_allocations(&self, mut visit: impl FnMut(usize, usize)) {
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
    fn new(board: &ReferenceLayer) -> Self {
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
                    bounds: egui::Rect::from_points(&p.corners()),
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
    pub(crate) fn intersects(&self, area: [usize; 4]) -> bool {
        let rect = egui::Rect::from_min_max(
            egui::pos2(area[0] as f32, area[1] as f32),
            egui::pos2(area[2] as f32, area[3] as f32),
        );
        self.images.iter().any(|i| i.bounds.intersects(rect))
    }
    pub(crate) fn pixel(&self, x: usize, y: usize) -> [u8; 4] {
        let p = egui::pos2(x as f32 + 0.5, y as f32 + 0.5);
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
pub(crate) struct CachedBoard {
    id: u64,
    key: Vec<u64>,
    layer: Layer,
}
impl CachedBoard {
    pub(crate) fn register_allocations(&self, registry: &mut crate::snapshot_memory::Registry) {
        registry.add_buffer(
            self.key.as_ptr() as usize,
            self.key.capacity() * std::mem::size_of::<u64>(),
        );
        if let Some(scene) = &self.layer.display_reference {
            scene.register_allocations(registry);
        }
    }
}
impl crate::reference_layers::State {
    pub(crate) fn bind_positions(&mut self, doc: &Document) {
        for board in &mut self.layers {
            if board.position.is_none() && !board.images.is_empty() {
                board.position = Some(Position::at(
                    doc,
                    if board.above { doc.layers.len() } else { 0 },
                    0,
                ));
            }
            if let Some(p) = &board.position
                && !p.bound
            {
                board.position = Some(Position::at(doc, p.gap, p.depth));
            }
        }
        self.blocked.clear();
        self.hidden.clear();
        let mut order = Vec::new();
        for (i, board) in self.layers.iter().enumerate() {
            let (gap, _, parent) = self.location(board, doc);
            order.push((gap, i, board.id));
            let mut current = parent;
            while let Some(index) = current {
                if !doc.layers[index].visible {
                    self.hidden.push(board.id);
                    self.blocked.push(board.id);
                    break;
                }
                if doc.layers[index].locked {
                    self.blocked.push(board.id);
                }
                current = doc.parent(index);
            }
        }
        order.sort_by_key(|(gap, index, _)| (*gap, *index));
        self.hit_order = order.into_iter().map(|(_, _, id)| id).collect();
        if let Some(target) = self.importing_target() {
            self.blocked.push(target);
        }
    }
    pub(crate) fn location(
        &self,
        board: &ReferenceLayer,
        doc: &Document,
    ) -> (usize, u8, Option<usize>) {
        board.position.as_ref().map_or(
            (if board.above { doc.layers.len() } else { 0 }, 0, None),
            |p| p.resolved(doc),
        )
    }
    pub(crate) fn display_document(&mut self, doc: &Document) -> Option<Document> {
        self.bind_positions(doc);
        if self.layers.iter().all(|r| r.position.is_none()) {
            self.scene_cache.clear();
            return None;
        }
        self.scene_cache
            .retain(|c| self.layers.iter().any(|r| r.id == c.id));
        let mut insertions = Vec::new();
        for board in &self.layers {
            let (gap, depth, _) = self.location(board, doc);
            let mut key = vec![doc.width as u64, doc.height as u64];
            for p in board.placements() {
                key.extend([
                    Arc::as_ptr(&p.image) as u64,
                    p.center[0].to_bits() as u64,
                    p.center[1].to_bits() as u64,
                    p.scale.to_bits() as u64,
                    p.rotation.to_bits() as u64,
                    p.opacity.to_bits() as u64,
                    p.visible as u64,
                    p.flip as u64,
                ]);
            }
            let cached = self.scene_cache.iter_mut().find(|c| c.id == board.id);
            let layer = if let Some(cached) = cached.filter(|c| c.key == key) {
                cached.layer.clone()
            } else {
                let mut layer = Layer::raster(board.name.clone(), doc.width, doc.height);
                layer.display_reference = Some(Arc::new(Scene::new(board)));
                self.scene_cache.retain(|c| c.id != board.id);
                self.scene_cache.push(CachedBoard {
                    id: board.id,
                    key,
                    layer: layer.clone(),
                });
                layer
            };
            let mut layer = layer;
            layer.depth = depth;
            layer.opacity = board.opacity;
            layer.visible = board.visible;
            insertions.push((gap, layer));
        }
        let mut scene = doc.clone();
        scene.layers.clear();
        for gap in 0..=doc.layers.len() {
            for (_, layer) in insertions.iter().filter(|(at, _)| *at == gap) {
                scene.layers.push(layer.clone());
            }
            if let Some(layer) = doc.layers.get(gap) {
                if gap == doc.active {
                    scene.active = scene.layers.len();
                }
                scene.layers.push(layer.clone());
            }
        }
        Some(scene)
    }
    pub(crate) fn saved_positions(&mut self, doc: &Document) {
        self.bind_positions(doc);
        let locations: Vec<_> = self.layers.iter().map(|b| self.location(b, doc)).collect();
        for (board, (gap, depth, _)) in self.layers.iter_mut().zip(locations) {
            if let Some(p) = &mut board.position {
                p.gap = gap;
                p.depth = depth;
            }
        }
    }
}
impl Dudel {
    pub(crate) fn reference_margin_shapes(&self, view: crate::view::View) -> Vec<egui::Shape> {
        self.reference_margin_shapes_with(view, |_| false)
    }
    pub(crate) fn reference_margin_shapes_with(
        &self,
        view: crate::view::View,
        excluded: impl Fn(crate::document::LayerId) -> bool,
    ) -> Vec<egui::Shape> {
        let mut shapes = Vec::new();
        let w = self.doc.width as f32;
        let h = self.doc.height as f32;
        if self
            .reference_layers
            .layers
            .iter()
            .all(|b| b.position.is_none())
        {
            return shapes;
        }
        let mut boards: Vec<_> = self
            .reference_layers
            .layers
            .iter()
            .enumerate()
            .filter(|(_, b)| b.visible)
            .collect();
        boards.sort_by_key(|(i, b)| (self.reference_layers.location(b, &self.doc).0, *i));
        for (_, board) in boards {
            let (_, _, parent) = self.reference_layers.location(board, &self.doc);
            let mut opacity = board.opacity;
            let mut current = parent;
            while let Some(index) = current {
                let folder = &self.doc.layers[index];
                if !folder.visible
                    || excluded(folder.id)
                    || folder.mask.as_ref().is_some_and(|m| m.enabled)
                {
                    opacity = 0.0;
                    break;
                }
                opacity *= folder.opacity;
                current = self.doc.parent(index);
            }
            if opacity == 0.0 {
                continue;
            }
            for (i, p) in board
                .placements()
                .enumerate()
                .filter(|(i, p)| *i == 0 || p.visible)
            {
                let Some(texture) = self.reference_layers.texture(p.id) else {
                    continue;
                };
                let corners = p.corners();
                if corners
                    .iter()
                    .all(|p| p.x >= 0.0 && p.x <= w && p.y >= 0.0 && p.y <= h)
                {
                    continue;
                }
                let uv = [
                    egui::pos2(0.0, 0.0),
                    egui::pos2(1.0, 0.0),
                    egui::pos2(1.0, 1.0),
                    egui::pos2(0.0, 1.0),
                ];
                let polygon: Vec<_> = corners
                    .into_iter()
                    .zip(uv)
                    .map(|(pos, mut uv)| {
                        if p.flip {
                            uv.x = 1.0 - uv.x;
                        }
                        (pos, uv)
                    })
                    .collect();
                // Four disjoint regions outside the document rectangle. Clip
                // in document space, so rotated and mirrored views also work.
                for region in [
                    vec![(0, 0.0, false)],
                    vec![(0, w, true)],
                    vec![(0, 0.0, true), (0, w, false), (1, 0.0, false)],
                    vec![(0, 0.0, true), (0, w, false), (1, h, true)],
                ] {
                    let mut clipped = polygon.clone();
                    for (axis, edge, greater) in region {
                        clipped = clip_polygon(clipped, axis, edge, greater);
                    }
                    if clipped.len() < 3 {
                        continue;
                    }
                    let mut mesh = egui::Mesh::with_texture(texture.id());
                    let color = egui::Color32::WHITE
                        .gamma_multiply(opacity * if i == 0 { 1.0 } else { p.opacity });
                    for (pos, uv) in clipped {
                        mesh.vertices.push(egui::epaint::Vertex {
                            pos: view.to_screen(pos),
                            uv,
                            color,
                        });
                    }
                    for n in 1..mesh.vertices.len() - 1 {
                        mesh.indices.extend([0, n as u32, n as u32 + 1]);
                    }
                    shapes.push(egui::Shape::mesh(mesh));
                }
            }
        }
        shapes
    }
    pub(crate) fn reorder_reference_board(&mut self, from: u64, to: u64, above: bool) {
        if from == to {
            return;
        }
        let Some(target) = self.reference_layers.index(Some(to)) else {
            return;
        };
        let position = self.reference_layers.layers[target].position.clone();
        let legacy_above = self.reference_layers.layers[target].above;
        let Some(source) = self.reference_layers.index(Some(from)) else {
            return;
        };
        let mut board = self.reference_layers.layers.remove(source);
        board.position = position;
        board.above = legacy_above;
        let target = self.reference_layers.index(Some(to)).unwrap();
        self.reference_layers
            .layers
            .insert(target + usize::from(above), board);
        self.references_edited();
        self.select_reference(from);
    }
    pub(crate) fn place_reference(&mut self, id: u64, gap: usize, depth: u8) {
        let Some(index) = self.reference_layers.index(Some(id)) else {
            return;
        };
        if depth > MAX_DEPTH {
            self.status = "Folder nesting limit reached".into();
            return;
        }
        self.reference_layers.layers[index].position = Some(Position::at(&self.doc, gap, depth));
        self.references_edited();
        self.select_reference(id);
    }
    pub(crate) fn reference_rows_at(&mut self, ui: &mut egui::Ui, gap: usize) {
        let ids = self
            .reference_layers
            .layers
            .iter()
            .rev()
            .filter(|b| {
                let (at, _, parent) = self.reference_layers.location(b, &self.doc);
                if at != gap {
                    return false;
                }
                if self.layer_navigation.active() {
                    return self.layer_navigation.matches_reference(&b.name);
                }
                let mut current = parent;
                while let Some(index) = current {
                    if self.doc.layers[index].collapsed {
                        return false;
                    }
                    current = self.doc.parent(index);
                }
                true
            })
            .map(|b| b.id)
            .collect();
        self.reference_rows_for_ids(ui, ids);
    }
}

fn clip_polygon(
    input: Vec<(egui::Pos2, egui::Pos2)>,
    axis: usize,
    edge: f32,
    greater: bool,
) -> Vec<(egui::Pos2, egui::Pos2)> {
    let Some(&last) = input.last() else {
        return input;
    };
    let value = |p: egui::Pos2| if axis == 0 { p.x } else { p.y };
    let inside = |p: egui::Pos2| {
        if greater {
            value(p) >= edge
        } else {
            value(p) <= edge
        }
    };
    let mut previous = last;
    let mut out = Vec::new();
    for point in input {
        if inside(previous.0) != inside(point.0) {
            let t = (edge - value(previous.0)) / (value(point.0) - value(previous.0));
            out.push((
                previous.0 + (point.0 - previous.0) * t,
                previous.1 + (point.1 - previous.1) * t,
            ));
        }
        if inside(point.0) {
            out.push(point);
        }
        previous = point;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reference_layers::{Image, MAX_IMAGES};
    fn image(color: [u8; 4]) -> Image {
        Image {
            width: 2,
            height: 2,
            rgba: color.repeat(4),
        }
    }
    fn board(app: &mut Dudel) -> u64 {
        app.insert_reference("Hair references".into(), image([0, 255, 0, 255]));
        let b = &mut app.reference_layers.layers[0];
        b.opacity = 1.0;
        b.center = [1.0, 1.0];
        b.scale = 1.0;
        b.id
    }
    #[test]
    fn frozen_reference_accounting_deduplicates_without_retaining_removed_images() {
        let mut app = Dudel {
            doc: Document::new(64, 64),
            ..Default::default()
        };
        app.insert_reference(
            "Study".into(),
            Image {
                width: 256,
                height: 256,
                rgba: [10, 20, 30, 255].repeat(256 * 256),
            },
        );
        let id = app.reference_layers.layers[0].id;
        app.place_reference(id, 0, 0);
        let image = Arc::downgrade(&app.reference_layers.layers[0].image);
        let display = app.reference_layers.display_document(&app.doc).unwrap();
        let strong_before = image.strong_count();
        let descriptor = crate::snapshot_memory::Snapshot::capture(&display);
        assert_eq!(image.strong_count(), strong_before);
        let mut registry = crate::snapshot_memory::Registry::new(&app.doc);
        registry.add_snapshot(&descriptor);
        let first = registry.additional_bytes();
        assert!(
            first >= 256 * 256 * 4,
            "frozen reference pixels must be counted"
        );
        registry.add_snapshot(&descriptor);
        assert!(
            registry.additional_bytes() - first < 256 * 256 * 4,
            "shared image is counted once"
        );
        app.reference_layers.install(Vec::new());
        drop(display);
        assert!(
            image.upgrade().is_none(),
            "descriptors must not keep pixels alive"
        );
    }
    #[test]
    fn reference_boards_composite_in_order_inherit_folder_masks_and_never_change_exports() {
        let mut app = Dudel {
            doc: Document::new(2, 2),
            ..Default::default()
        };
        app.doc.layers[0].pixels = [255, 0, 0, 255].repeat(4).into();
        app.doc.add_layer();
        app.doc.layers[1].pixels = [0, 0, 255, 128].repeat(4).into();
        let id = board(&mut app);
        let artwork = app.doc.composite();
        app.place_reference(id, 1, 0);
        let scene = app.reference_layers.display_document(&app.doc).unwrap();
        assert_eq!(&scene.composite()[..4], &[0, 127, 128, 255]);
        assert_eq!(app.doc.composite(), artwork);
        app.doc.group_active().unwrap();
        app.doc.version = 2;
        app.doc.layers[2].opacity = 0.5;
        app.doc.layers[2].mask = Some(crate::layer_mask::LayerMask {
            id: LayerId::new(),
            enabled: true,
            linked: true,
            coverage: crate::layer_mask::Mask::new(2, 2, 160).unwrap(),
        });
        app.place_reference(id, 1, 1);
        let mut oracle = app.doc.clone();
        let mut expected = Layer::raster("reference oracle".into(), 2, 2);
        expected.depth = 1;
        expected.pixels = [0, 255, 0, 255].repeat(4).into();
        oracle.layers.insert(1, expected);
        let scene = app.reference_layers.display_document(&app.doc).unwrap();
        assert_eq!(scene.composite(), oracle.composite());
        let mut prefix = crate::prefix_cache::PrefixCache::default();
        assert_eq!(
            prefix.composite_region(&scene, [0, 0, 2, 2], false),
            oracle.composite()
        );
        app.doc.layers[2].visible = false;
        let scene = app.reference_layers.display_document(&app.doc).unwrap();
        assert_eq!(scene.composite(), app.doc.composite());
        assert!(app.reference_layers.hidden.contains(&id));
    }
    #[test]
    fn twelve_images_round_trip_with_positions_and_reject_a_thirteenth_without_an_edit() {
        let mut app = Dudel {
            doc: Document::new(8, 8),
            ..Default::default()
        };
        let id = board(&mut app);
        for i in 1..MAX_IMAGES {
            app.insert_reference_image(id, format!("Hair {i}"), image([i as u8, 90, 100, 200]))
                .unwrap();
        }
        let revision = app.revision;
        assert!(
            app.insert_reference_image(id, "overflow".into(), image([1; 4]))
                .is_err()
        );
        assert_eq!(app.revision, revision);
        app.arrange_reference_images(id);
        app.place_reference(id, 0, 0);
        app.reference_layers.saved_positions(&app.doc);
        let artwork = app.doc.composite();
        let mut saved = app.doc.clone();
        saved.references = app.reference_layers.begin_save();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("boards.dudel");
        crate::storage::save_project(&path, &saved).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        let mut payload = Vec::new();
        std::io::Read::read_to_end(
            &mut flate2::read::GzDecoder::new(&bytes[crate::tile_project::MAGIC.len()..]),
            &mut payload,
        )
        .unwrap();
        assert!(
            payload
                .windows(5)
                .any(|tag| tag == crate::reference_layers::STACK_TRAILER),
            "older readers must reject the new board/order payload instead of discarding it"
        );
        let loaded = crate::storage::load_project(&path).unwrap();
        assert_eq!(loaded.references[0].images.len() + 1, MAX_IMAGES);
        assert_eq!(loaded.references[0].name, "Hair references");
        assert_eq!(loaded.references[0].position.as_ref().unwrap().gap, 0);
        for (a, b) in saved.references[0]
            .placements()
            .zip(loaded.references[0].placements())
        {
            assert_eq!(a.center, b.center);
            assert_eq!(a.scale, b.scale);
            assert!(a.image == b.image);
        }
        assert_eq!(loaded.composite(), artwork);
    }
    #[test]
    fn cached_reference_scene_reuses_images_and_invalidates_geometry_and_prefix() {
        let mut app = Dudel {
            doc: Document::new(32, 32),
            ..Default::default()
        };
        let id = board(&mut app);
        app.place_reference(id, 0, 0);
        let a = app.reference_layers.display_document(&app.doc).unwrap();
        let b = app.reference_layers.display_document(&app.doc).unwrap();
        assert_eq!(a.layers[0].display_key(), b.layers[0].display_key());
        assert!(Arc::ptr_eq(
            &app.reference_layers.layers[0].image,
            &a.layers[0].display_reference.as_ref().unwrap().images[0].image
        ));
        let mut prefix = crate::prefix_cache::PrefixCache::default();
        assert_eq!(
            prefix.composite_region(&a, [0, 0, 16, 16], false),
            a.composite_display_region([0, 0, 16, 16], false)
        );
        app.reference_layers.layers[0].center = [8.0, 8.0];
        let c = app.reference_layers.display_document(&app.doc).unwrap();
        assert_ne!(a.layers[0].display_key(), c.layers[0].display_key());
        assert_eq!(
            prefix.composite_region(&c, [0, 0, 16, 16], false),
            c.composite_display_region([0, 0, 16, 16], false)
        );
        assert_ne!(a.composite(), c.composite());
        assert_eq!(a.layers[0].display_region_key([20, 20, 32, 32]), 0);
        assert_eq!(c.layers[0].display_region_key([20, 20, 32, 32]), 0);
        assert!(
            c.validate().is_err(),
            "view-only references must never serialize as artwork layers"
        );
    }
    #[test]
    fn ordered_references_keep_margin_images_outside_rotated_canvas_without_double_drawing_inside()
    {
        let mut app = Dudel {
            doc: Document::new(8, 8),
            ..Default::default()
        };
        let id = board(&mut app);
        app.place_reference(id, 0, 0);
        app.reference_layers.layers[0].center = [0.0, 1.0];
        let ctx = egui::Context::default();
        let _ = ctx.run(Default::default(), |ctx| app.prepare_reference_layers(ctx));
        let view = crate::view::View {
            origin: egui::pos2(200.0, 200.0),
            zoom: 2.0,
            rotation: 0.4,
            mirror: true,
            half: egui::vec2(4.0, 4.0),
        };
        let shapes = app.reference_margin_shapes(view);
        assert!(!shapes.is_empty());
        let mut area = 0.0;
        for shape in shapes {
            let egui::Shape::Mesh(mesh) = shape else {
                panic!("reference mesh")
            };
            for triangle in mesh.indices.chunks_exact(3) {
                let [a, b, c] = [triangle[0], triangle[1], triangle[2]]
                    .map(|i| view.to_doc(mesh.vertices[i as usize].pos));
                let part = ((b - a).x * (c - a).y - (b - a).y * (c - a).x).abs() / 2.0;
                area += part;
                if part > 0.00001 {
                    assert!(
                        a.x <= 0.001 && b.x <= 0.001 && c.x <= 0.001,
                        "margin shape overlaps the canvas interior"
                    );
                }
            }
        }
        assert!((area - 2.0).abs() < 0.01);
    }
    #[test]
    fn folder_identity_carries_reference_position_through_moves_and_reopen() {
        let mut app = Dudel {
            doc: Document::new(8, 8),
            ..Default::default()
        };
        app.doc.group_active().unwrap();
        let folder = app.doc.layers[1].id;
        let id = board(&mut app);
        app.place_reference(id, 1, 1);
        app.doc
            .layers
            .insert(0, Layer::raster("new bottom".into(), 8, 8));
        app.doc.active = 2;
        let (gap, depth, parent) = app
            .reference_layers
            .location(&app.reference_layers.layers[0], &app.doc);
        assert_eq!((gap, depth, parent), (2, 1, Some(2)));
        assert_eq!(app.doc.layers[parent.unwrap()].id, folder);
        app.reference_layers.saved_positions(&app.doc);
        let mut loaded = app.reference_layers.layers.clone();
        loaded = serde_json::from_str(&serde_json::to_string(&loaded).unwrap()).unwrap();
        let mut state = crate::reference_layers::State::default();
        state.install(loaded);
        state.bind_positions(&app.doc);
        assert_eq!(state.location(&state.layers[0], &app.doc), (2, 1, Some(2)));
    }
}
