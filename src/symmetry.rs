//! Symmetry drawing guide: every brush or eraser dab is repeated as mirrored
//! or rotated copies about a center point.

#[derive(Clone, Copy, PartialEq, Eq, Default, Debug, serde::Serialize, serde::Deserialize)]
pub enum Mode {
    #[default]
    Off,
    /// Mirror across a vertical axis: left and right halves match.
    LeftRight,
    /// Mirror across a horizontal axis.
    TopBottom,
    /// Both axes: four mirrored quarters.
    FourWay,
    /// Rotational copies around the center.
    Radial,
}

impl Mode {
    pub const ALL: [Mode; 5] = [
        Mode::Off,
        Mode::LeftRight,
        Mode::TopBottom,
        Mode::FourWay,
        Mode::Radial,
    ];
    pub fn name(self) -> &'static str {
        match self {
            Mode::Off => "Off",
            Mode::LeftRight => "Left / right",
            Mode::TopBottom => "Top / bottom",
            Mode::FourWay => "Four-way",
            Mode::Radial => "Radial",
        }
    }
}

/// Row-major 2 × 2 map applied to a dab's pixel offsets before tip lookup.
pub type Map = [f32; 4];
pub const IDENTITY: Map = [1.0, 0.0, 0.0, 1.0];

#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Symmetry {
    pub mode: Mode,
    /// Copies for radial symmetry, 2..=16.
    pub segments: u8,
    /// Document-space center; None follows the canvas center.
    pub center: Option<[f32; 2]>,
    /// Clockwise rotation of the mirror axes or radial spokes, in degrees.
    pub angle: f32,
}

impl Default for Symmetry {
    fn default() -> Self {
        Self {
            mode: Mode::Off,
            segments: 6,
            center: None,
            angle: 0.0,
        }
    }
}

impl Symmetry {
    pub fn active(&self) -> bool {
        self.mode != Mode::Off
    }
    pub fn center(&self, width: usize, height: usize) -> [f32; 2] {
        self.center
            .unwrap_or([width as f32 / 2.0, height as f32 / 2.0])
    }
    /// Directions of the guide lines through the center, in radians in
    /// screen orientation (y down, clockwise positive). Mirror axes run both
    /// ways; radial spokes start at the top.
    pub fn axes(&self) -> Vec<f32> {
        let turn = self.angle.to_radians();
        let vertical = std::f32::consts::FRAC_PI_2 + turn;
        match self.mode {
            Mode::Off => vec![],
            Mode::LeftRight => vec![vertical],
            Mode::TopBottom => vec![turn],
            Mode::FourWay => vec![vertical, turn],
            Mode::Radial => {
                let n = self.segments.clamp(2, 16);
                (0..n)
                    .map(|k| {
                        std::f32::consts::TAU * k as f32 / n as f32 - std::f32::consts::FRAC_PI_2
                            + turn
                    })
                    .collect()
            }
        }
    }
    /// Every copy of a dab at (x, y), the original first. The map mirrors or
    /// rotates the tip so angled and image tips are reflected too.
    pub fn copies(&self, x: f32, y: f32, width: usize, height: usize) -> Vec<([f32; 2], Map)> {
        let [cx, cy] = self.center(width, height);
        // Reflection across the line through the center at `axis` radians.
        // The matrix is its own inverse, so it doubles as the tip map.
        let reflect = |axis: f32| {
            let (sin, cos) = (2.0 * axis).sin_cos();
            // Snap rounding noise so axis-aligned mirrors stay pixel exact.
            let snap = |v: f32| if v.abs() < 1e-6 { 0.0 } else { v };
            let (sin, cos) = (snap(sin), snap(cos));
            let map = [cos, sin, sin, -cos];
            let (dx, dy) = (x - cx, y - cy);
            (
                [
                    cx + map[0] * dx + map[1] * dy,
                    cy + map[2] * dx + map[3] * dy,
                ],
                map,
            )
        };
        let turn = self.angle.to_radians();
        let vertical = std::f32::consts::FRAC_PI_2 + turn;
        match self.mode {
            Mode::Off => vec![([x, y], IDENTITY)],
            Mode::LeftRight => vec![([x, y], IDENTITY), reflect(vertical)],
            Mode::TopBottom => vec![([x, y], IDENTITY), reflect(turn)],
            Mode::FourWay => vec![
                ([x, y], IDENTITY),
                reflect(vertical),
                reflect(turn),
                // Both reflections compose to a half turn, whatever the angle.
                ([2.0 * cx - x, 2.0 * cy - y], [-1.0, 0.0, 0.0, -1.0]),
            ],
            Mode::Radial => {
                let n = self.segments.clamp(2, 16);
                (0..n)
                    .map(|k| {
                        let (sin, cos) = (std::f32::consts::TAU * k as f32 / n as f32).sin_cos();
                        let (dx, dy) = (x - cx, y - cy);
                        (
                            [cx + dx * cos - dy * sin, cy + dx * sin + dy * cos],
                            // Tip lookup uses the inverse rotation.
                            [cos, sin, -sin, cos],
                        )
                    })
                    .collect()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{brush::Brush, document::Document};

    #[test]
    fn copies_mirror_and_rotate_about_the_center() {
        let mut symmetry = Symmetry {
            mode: Mode::LeftRight,
            ..Symmetry::default()
        };
        let copies = symmetry.copies(70.0, 20.0, 100, 50);
        assert_eq!(copies[1].0, [30.0, 20.0]);
        symmetry.mode = Mode::FourWay;
        symmetry.center = Some([10.0, 10.0]);
        let copies = symmetry.copies(15.0, 12.0, 100, 50);
        assert_eq!(copies.len(), 4);
        assert_eq!(copies[3].0, [5.0, 8.0]);
        symmetry.mode = Mode::Radial;
        symmetry.segments = 4;
        let copies = symmetry.copies(20.0, 10.0, 100, 50);
        let quarter = copies[1].0;
        assert!((quarter[0] - 10.0).abs() < 1e-4 && (quarter[1] - 20.0).abs() < 1e-4);
    }

    #[test]
    fn rotated_mirrors_reflect_across_the_turned_axis() {
        let mut symmetry = Symmetry {
            mode: Mode::LeftRight,
            center: Some([50.0, 50.0]),
            angle: 45.0,
            ..Symmetry::default()
        };
        // A vertical axis turned 45° clockwise runs top-right to bottom-left,
        // so a point right of center lands above it.
        let close =
            |a: [f32; 2], b: [f32; 2]| (a[0] - b[0]).abs() < 1e-4 && (a[1] - b[1]).abs() < 1e-4;
        let copies = symmetry.copies(60.0, 50.0, 100, 100);
        assert!(close(copies[1].0, [50.0, 40.0]), "{:?}", copies[1].0);
        // Mirroring twice returns the original point, and the map is a reflection.
        let [a, b, c, d] = copies[1].1;
        assert!((a * d - b * c + 1.0).abs() < 1e-5);
        symmetry.angle = 90.0;
        let copies = symmetry.copies(60.0, 40.0, 100, 100);
        assert!(close(copies[1].0, [60.0, 60.0]), "{:?}", copies[1].0);
        symmetry.mode = Mode::FourWay;
        symmetry.angle = 30.0;
        let copies = symmetry.copies(70.0, 45.0, 100, 100);
        assert!(close(copies[3].0, [30.0, 55.0]));
        // Every copy stays the same distance from the center.
        for (point, _) in &copies {
            let r = (point[0] - 50.0).hypot(point[1] - 50.0);
            assert!((r - 20.0f32.hypot(5.0)).abs() < 1e-3);
        }
    }

    #[test]
    fn mirrored_dabs_reflect_angled_tips_exactly() {
        let brush = Brush {
            size: 16.0,
            hardness: 1.0,
            aspect: 0.25,
            angle: 30.0,
            pressure_size: false,
            ..Brush::default()
        };
        let symmetry = Symmetry {
            mode: Mode::LeftRight,
            ..Symmetry::default()
        };
        let mut doc = Document::new(64, 32);
        for ([x, y], map) in symmetry.copies(44.0, 16.0, 64, 32) {
            doc.dab_mapped(x, y, 1.0, &brush, [0, 0, 0], false, map);
        }
        let alpha = |x: usize, y: usize| doc.layers[0].pixels[(y * 64 + x) * 4 + 3];
        for y in 0..32 {
            for x in 0..64 {
                assert_eq!(alpha(x, y), alpha(63 - x, y), "pixel {x},{y}");
            }
        }
        assert!(alpha(44, 16) > 0 && alpha(19, 16) > 0);
    }
}
