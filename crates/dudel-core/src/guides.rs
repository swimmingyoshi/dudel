//! Drawing guides saved with a project: symmetry (shared and per layer), the
//! grid, ruler guide lines and the guide lock. The app owns the live state;
//! a `Guides` record travels inside the saved document only, so undo and
//! history never revert guide edits.
use crate::{document::Document, symmetry::Symmetry, view::View};
use emath::{Pos2, pos2};

/// Width of the rulers along the top and left of the canvas, in points.
pub const RULER: f32 = 18.0;
/// Screen distance within which a guide line can be picked up.
pub const PICK: f32 = 6.0;
pub const MAX_LINES: usize = 256;

#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Grid {
    pub visible: bool,
    /// Distance between major lines in canvas pixels.
    pub spacing: u32,
    /// Minor divisions per major cell; 1 draws major lines only.
    pub subdivisions: u8,
    pub opacity: f32,
    /// Outline every canvas pixel when zoomed in to `PIXEL_GRID_ZOOM` or
    /// closer, independent of the grid above.
    pub pixels: bool,
}

impl Default for Grid {
    fn default() -> Self {
        Self {
            visible: false,
            spacing: 100,
            subdivisions: 4,
            opacity: 0.35,
            pixels: false,
        }
    }
}

/// Zoom from which the pixel grid shows: 800%, where a pixel is eight screen
/// points wide. It fades in over the next 50%.
pub const PIXEL_GRID_ZOOM: f32 = 8.0;

/// A straight guide across the whole canvas at a canvas-pixel position.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct GuideLine {
    /// Vertical lines sit at an x position; horizontal lines at a y position.
    pub vertical: bool,
    pub position: f32,
}

/// The saved form of every guide.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Guides {
    pub symmetry: Symmetry,
    /// Layer stack index paired with that layer's own symmetry.
    pub layer_symmetry: Vec<(usize, Symmetry)>,
    pub grid: Grid,
    pub lines: Vec<GuideLine>,
    pub locked: bool,
    pub rulers: bool,
    /// Special rulers (line, parallel, focus, ellipse) strokes snap to.
    pub special_rulers: Vec<crate::rulers::Ruler>,
    /// Stack index of the layer each ruler belongs to; None for all layers.
    /// Parallel to `special_rulers`; missing entries mean all layers.
    pub ruler_layers: Vec<Option<usize>>,
    /// Strokes snap to rulers and guide lines.
    pub snapping: bool,
    /// Strokes starting near a grid line follow it.
    pub snap_grid: bool,
}

impl Default for Guides {
    fn default() -> Self {
        Self {
            symmetry: Symmetry::default(),
            layer_symmetry: Vec::new(),
            grid: Grid::default(),
            lines: Vec::new(),
            // Guides start locked so a stroke never picks one up by accident.
            locked: true,
            rulers: false,
            special_rulers: Vec::new(),
            ruler_layers: Vec::new(),
            snapping: true,
            snap_grid: false,
        }
    }
}

fn finite(value: f32) -> bool {
    value.is_finite()
}

fn coordinate(value: f32) -> bool {
    finite(value) && value.abs() <= 1.0e9
}
fn clean_symmetry(symmetry: &mut Symmetry) {
    symmetry.segments = symmetry.segments.clamp(2, 16);
    if !finite(symmetry.angle) {
        symmetry.angle = 0.0;
    }
    if !(-180.0..=180.0).contains(&symmetry.angle) {
        symmetry.angle = (symmetry.angle + 180.0).rem_euclid(360.0) - 180.0;
    }
    symmetry.center = symmetry
        .center
        .filter(|[x, y]| coordinate(*x) && coordinate(*y));
}

impl Guides {
    /// Repair or drop anything a damaged or hand-edited file could contain,
    /// so loading never fails because of guides.
    pub fn sanitize(&mut self, doc: &Document) {
        clean_symmetry(&mut self.symmetry);
        self.layer_symmetry
            .retain(|(index, _)| *index < doc.layers.len());
        self.layer_symmetry.sort_by_key(|(index, _)| *index);
        self.layer_symmetry.dedup_by_key(|(index, _)| *index);
        for (_, symmetry) in &mut self.layer_symmetry {
            clean_symmetry(symmetry);
        }
        self.grid.spacing = self.grid.spacing.clamp(4, 4096);
        self.grid.subdivisions = self.grid.subdivisions.clamp(1, 16);
        if !finite(self.grid.opacity) {
            self.grid.opacity = Grid::default().opacity;
        }
        self.grid.opacity = self.grid.opacity.clamp(0.05, 1.0);
        // Retain off-canvas guides after cropping, using the lasso coordinate domain.
        self.lines.retain(|line| coordinate(line.position));
        self.lines.truncate(MAX_LINES);
        // Rulers may extend past the canvas (focus points often do), but not
        // without bound.
        let owners = std::mem::take(&mut self.ruler_layers);
        let (rulers, layers): (Vec<_>, Vec<_>) = std::mem::take(&mut self.special_rulers)
            .into_iter()
            .enumerate()
            .filter(|(_, ruler)| {
                ruler.is_finite() && ruler.handles().iter().flatten().all(|v| coordinate(*v))
            })
            .map(|(index, ruler)| {
                let owner = owners
                    .get(index)
                    .copied()
                    .flatten()
                    .filter(|layer| doc.layers.get(*layer).is_some_and(|l| !l.folder));
                (ruler, owner)
            })
            .take(crate::rulers::MAX_RULERS)
            .unzip();
        self.special_rulers = rulers;
        self.ruler_layers = layers;
    }
    pub fn map_canvas(
        &mut self,
        before: crate::canvas_size::Frame,
        after: crate::canvas_size::Frame,
    ) {
        map_symmetry(&mut self.symmetry, before, after);
        for (_, symmetry) in &mut self.layer_symmetry {
            map_symmetry(symmetry, before, after);
        }
        for line in &mut self.lines {
            map_line(line, before, after);
        }
        for ruler in &mut self.special_rulers {
            ruler.map_canvas(before, after);
        }
    }
}
pub fn map_symmetry(
    symmetry: &mut Symmetry,
    before: crate::canvas_size::Frame,
    after: crate::canvas_size::Frame,
) {
    symmetry.center = symmetry.center.map(|p| after.point_from(before, p));
    symmetry.angle += after.angle_from(before);
}
pub fn map_line(
    line: &mut GuideLine,
    before: crate::canvas_size::Frame,
    after: crate::canvas_size::Frame,
) {
    let p = if line.vertical {
        [line.position, 0.0]
    } else {
        [0.0, line.position]
    };
    let p = after.point_from(before, p);
    line.vertical ^= after.turn_from(before) % 2 == 1;
    line.position = if line.vertical { p[0] } else { p[1] };
}

/// Canvas-space endpoints that span the whole visible area for a line at
/// `position`, so rotated views still draw it edge to edge.
pub fn line_ends(line: GuideLine, doc: [usize; 2], reach: f32) -> [Pos2; 2] {
    let (w, h) = (doc[0] as f32, doc[1] as f32);
    if line.vertical {
        [pos2(line.position, -reach), pos2(line.position, h + reach)]
    } else {
        [pos2(-reach, line.position), pos2(w + reach, line.position)]
    }
}

/// Screen distance from `point` to a guide line.
pub fn distance(line: GuideLine, doc: [usize; 2], view: &View, point: Pos2) -> f32 {
    let [a, b] = line_ends(line, doc, 0.0).map(|p| view.to_screen(p));
    let along = b - a;
    let t = ((point - a).dot(along) / along.length_sq().max(1e-6)).clamp(-1e6, 1e6);
    (a + along * t - point).length()
}

/// Topmost guide line within pick distance of a screen point.
pub fn line_at(lines: &[GuideLine], doc: [usize; 2], view: &View, point: Pos2) -> Option<usize> {
    lines
        .iter()
        .enumerate()
        .rev()
        .map(|(index, line)| (index, distance(*line, doc, view, point)))
        .filter(|(_, d)| *d <= PICK)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(index, _)| index)
}

/// Which ruler a screen point is over: `Some(true)` for the left ruler
/// (it creates vertical guides), `Some(false)` for the top one.
pub fn ruler_at(area: emath::Rect, point: Pos2) -> Option<bool> {
    if !area.contains(point) {
        None
    } else if point.x < area.left() + RULER {
        Some(true)
    } else if point.y < area.top() + RULER {
        Some(false)
    } else {
        None
    }
}

/// Canvas-pixel distance between ruler ticks so labels stay about 60 points
/// apart: 1, 2 or 5 times a power of ten.
pub fn tick_step(zoom: f32) -> f32 {
    let target = 60.0 / zoom.max(1e-4);
    let mut step = 1.0f32;
    while step < target {
        for factor in [2.0, 2.5, 2.0] {
            step *= factor;
            if step >= target {
                break;
            }
        }
    }
    step
}

/// A guide line being dragged; `original` is None for one pulled from a ruler.
#[derive(Clone, Copy, Debug)]
pub struct GuideDrag {
    pub index: usize,
    pub original: Option<f32>,
}
