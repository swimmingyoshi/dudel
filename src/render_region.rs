use crate::brush::Brush;

/// Conservative half-open bounds, including rotated square/image corners.
pub fn dab_bounds(x: f32, y: f32, brush: &Brush, width: usize, height: usize) -> [usize; 4] {
    let extent = brush.size * 0.5 * std::f32::consts::SQRT_2 + 1.0;
    [
        (x - extent).floor().clamp(0.0, width as f32) as usize,
        (y - extent).floor().clamp(0.0, height as f32) as usize,
        (x + extent).ceil().clamp(0.0, width as f32) as usize,
        (y + extent).ceil().clamp(0.0, height as f32) as usize,
    ]
}

pub fn include(region: &mut Option<[usize; 4]>, bounds: [usize; 4]) {
    if bounds[0] >= bounds[2] || bounds[1] >= bounds[3] {
        return;
    }
    *region = Some(match *region {
        Some(old) => [
            old[0].min(bounds[0]),
            old[1].min(bounds[1]),
            old[2].max(bounds[2]),
            old[3].max(bounds[3]),
        ],
        None => bounds,
    });
}

fn area(rect: [usize; 4]) -> usize {
    (rect[2] - rect[0]) * (rect[3] - rect[1])
}
fn union(a: [usize; 4], b: [usize; 4]) -> [usize; 4] {
    [
        a[0].min(b[0]),
        a[1].min(b[1]),
        a[2].max(b[2]),
        a[3].max(b[3]),
    ]
}

/// Changed areas as a few rectangles instead of one bounding box. Symmetry
/// copies land far apart, often diagonally at an angle; a single box around
/// them covers mostly untouched canvas and forces the display to recomposite
/// all of it. Rectangles merge only when that wastes little area.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Damage {
    rects: Vec<[usize; 4]>,
}

impl Damage {
    /// Separate rectangles kept before the closest pair is merged.
    const LIMIT: usize = 8;

    pub fn include(&mut self, rect: [usize; 4]) {
        if rect[0] >= rect[2] || rect[1] >= rect[3] {
            return;
        }
        let compact = |old: [usize; 4], rect: [usize; 4]| {
            area(union(old, rect)) <= (area(old) + area(rect)) * 5 / 4
        };
        // Fast path for runs of adjacent rectangles (pixels along a row, dabs
        // along a stroke): grow the latest rectangle in place.
        if let Some(last) = self.rects.last_mut()
            && compact(*last, rect)
        {
            *last = union(*last, rect);
            return;
        }
        let mut rect = rect;
        // Absorb every rectangle the new one overlaps or nearly touches.
        while let Some(index) = self.rects.iter().position(|old| compact(*old, rect)) {
            rect = union(self.rects.swap_remove(index), rect);
        }
        self.rects.push(rect);
        while self.rects.len() > Self::LIMIT {
            // Merge the pair that adds the least untouched area.
            let mut best = (usize::MAX, 0, 1);
            for i in 0..self.rects.len() {
                for j in i + 1..self.rects.len() {
                    let (a, b) = (self.rects[i], self.rects[j]);
                    let waste = area(union(a, b)).saturating_sub(area(a) + area(b));
                    if waste < best.0 {
                        best = (waste, i, j);
                    }
                }
            }
            let (_, i, j) = best;
            let merged = union(self.rects[i], self.rects.swap_remove(j));
            self.rects[i] = merged;
        }
    }
    pub fn merge(&mut self, other: Damage) {
        for rect in other.rects {
            self.include(rect);
        }
    }
    pub fn is_empty(&self) -> bool {
        self.rects.is_empty()
    }
    pub fn rects(&self) -> &[[usize; 4]] {
        &self.rects
    }
    /// One box around everything, for consumers that need a single rectangle.
    pub fn bounds(&self) -> Option<[usize; 4]> {
        self.rects.iter().copied().reduce(union)
    }
    pub fn take(&mut self) -> Damage {
        std::mem::take(self)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn damage_keeps_distant_copies_apart_and_merges_neighbours() {
        let mut damage = Damage::default();
        // Two mirrored dabs on a diagonal stay two small rectangles.
        damage.include([100, 100, 120, 120]);
        damage.include([900, 700, 920, 720]);
        assert_eq!(damage.rects().len(), 2);
        // Consecutive overlapping dabs along a stroke merge.
        damage.include([110, 100, 130, 120]);
        assert_eq!(damage.rects().len(), 2);
        assert!(damage.rects().contains(&[100, 100, 130, 120]));
        assert_eq!(damage.bounds(), Some([100, 100, 920, 720]));
        damage.include([5, 5, 5, 9]);
        assert_eq!(damage.rects().len(), 2);
        // The cap merges the cheapest pairs, never dropping coverage.
        for k in 0..20 {
            damage.include([k * 100, 1000, k * 100 + 10, 1010]);
        }
        assert!(damage.rects().len() <= Damage::LIMIT);
        let bounds = damage.bounds().unwrap();
        assert_eq!(bounds, [0, 100, 1910, 1010]);
        for k in 0..20 {
            let point = [k * 100 + 5, 1005];
            assert!(damage.rects().iter().any(|r| r[0] <= point[0]
                && point[0] < r[2]
                && r[1] <= point[1]
                && point[1] < r[3]));
        }
    }
    use super::*;
    use crate::{brush::Tip, document::Document};
    #[test]
    fn partial_updates_reconstruct_full_canvas_after_rotated_pressure_dabs() {
        let mut doc = Document::new(64, 64);
        let mut cached = doc.composite();
        let brush = Brush {
            size: 25.0,
            angle: 45.0,
            tip: Tip::Square,
            ..Brush::default()
        };
        for (x, y, pressure) in [(0.0, 0.0, 0.1), (30.0, 32.0, 1.0), (63.0, 63.0, 0.6)] {
            for erase in [false, true] {
                doc.dab(x, y, pressure, &brush, [211, 78, 91], erase);
                let [left, top, right, bottom] = dab_bounds(x, y, &brush, 64, 64);
                let region = doc.composite_region(left, top, right, bottom);
                let stride = (right - left) * 4;
                for row in 0..bottom - top {
                    let offset = ((top + row) * 64 + left) * 4;
                    cached[offset..offset + stride]
                        .copy_from_slice(&region[row * stride..(row + 1) * stride]);
                }
                assert_eq!(cached, doc.composite());
            }
        }
        let mut region = None;
        include(&mut region, [4, 8, 20, 30]);
        include(&mut region, [1, 10, 40, 22]);
        include(&mut region, [0, 0, 0, 0]);
        assert_eq!(region, Some([1, 8, 40, 30]));
    }
}
