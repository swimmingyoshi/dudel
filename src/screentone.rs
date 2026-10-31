//! CSP screentone: a layer at one dot density drawn as black dots or lines
//! on a rotated grid, the way Clip Studio Paint's tone effect draws it.
//!
//! CSP snaps the grid to whole pixels so it repeats exactly: a cell's side
//! is the pixel vector (a, b) nearest to (cos, sin) of the angle (modulo
//! 90°) times resolution ÷ lines, and the cells are the squares spanned by
//! (a, b) and (-b, a) from the canvas origin, each pixel belonging to one.
//! In every cell the dot is a shape around the cell's center (a disc, a
//! square along the cell's sides, or a band along one side for lines) whose
//! size is binary-searched toward the density's share of the cell's pixels.
//! Round and square dots break a tie by keeping the first pixels in row
//! order; lines shrink a little and keep what they have, which can leave
//! them off the share. Pixel positions are their top-left corners nudged by
//! -0.1, which breaks most ties toward the lower right. Checked pixel for
//! pixel against tones CSP saved: round dots at 300 dpi, 85 lines, 45° and
//! 40%; square dots and lines at 600 dpi, 60 lines, 45° and 70%.
use crate::pixels::Pixels;

/// The largest cell side drawn as dots; past it (a corrupt resolution) the
/// caller shades evenly.
const MAX_CELL: i64 = 512;
/// CSP stops refining the dot's radius after this many tries.
const SEARCHES: usize = 20;

/// A tone's dot shape.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Shape {
    Round,
    Square,
    Line,
}

/// A tone's grid.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Screen {
    /// Lines per inch; CSP keeps it within 5..=85.
    pub(crate) lines: f64,
    /// Pixels per inch the tone was laid out for.
    pub(crate) resolution: f64,
    /// Degrees.
    pub(crate) angle: u32,
    pub(crate) shape: Shape,
}

impl Screen {
    /// The cell's side (a, b) in whole pixels, or None when the cells are
    /// too small for dots: below 1.5 px CSP shades evenly instead.
    pub(crate) fn side(&self) -> Option<[i64; 2]> {
        if !(self.lines.is_finite() && self.resolution.is_finite() && self.resolution > 0.0) {
            return None;
        }
        let pitch = self.resolution / self.lines.clamp(5.0, 85.0);
        if pitch > MAX_CELL as f64 {
            return None;
        }
        let angle = f64::from(self.angle.min(359) % 90) * std::f64::consts::PI / 180.0;
        let side = [nearest(angle.cos() * pitch), nearest(angle.sin() * pitch)];
        let size = ((side[0] * side[0] + side[1] * side[1]) as f64).sqrt();
        (size > 1.5).then_some(side)
    }
}

/// Rounds half away from zero, as CSP does.
fn nearest(v: f64) -> i64 {
    if v < 0.0 {
        (v - 0.5) as i64
    } else {
        (v + 0.5) as i64
    }
}

/// One cell's dot over its (a + b)-square bounding box, row by row; the box
/// starts `b` pixels left of the cell's origin corner.
fn dot(shape: Shape, angle: u32, [a, b]: [i64; 2], density: f64) -> Vec<bool> {
    let n = (a + b) as usize;
    let d = a * a + b * b;
    // A pixel at (x, y) from the origin corner belongs to the cell when its
    // coordinates along both sides lie in [0, d).
    let owned = |i: usize, j: usize| {
        let (x, y) = (j as i64 - b, i as i64);
        (0..d).contains(&(x * a + y * b)) && (0..d).contains(&(y * a - x * b))
    };
    let count = (0..n * n).filter(|k| owned(k / n, k % n)).count();
    let target = nearest(count as f64 * density) as usize;
    // Corner offsets from the cell's center.
    let half = -((a + b) as f64) * 0.5;
    let size = (d as f64).sqrt();
    // The slopes of the cell's two sides; both 0 on an axis-aligned grid.
    let (along, across) = if a != 0 && b != 0 {
        (b as f64 / a as f64, -(a as f64 / b as f64))
    } else {
        (0.0, 0.0)
    };
    let band = |slope: f64, half_width: f64, dx: f64, dy: f64| {
        let middle = dx * slope;
        middle - half_width <= dy && dy < middle + half_width
    };
    let width = |slope: f64, r: f64| (slope * slope + 1.0).sqrt() * r;
    let angle = angle.min(359) % 180;
    // Whether the pixel at (dx, dy) from the center lies in the shape of
    // size `r`.
    let inside = |r: f64, dx: f64, dy: f64| match shape {
        Shape::Round => dx * dx + dy * dy < r * r,
        Shape::Square => {
            band(along, width(along, r), dx, dy)
                && if across == 0.0 {
                    let w = width(across, r);
                    -w <= dx && dx < w
                } else {
                    band(across, width(across, r), dx, dy)
                }
        }
        // Lines run along the first side below 90°, else the second.
        Shape::Line if angle < 90 => {
            if angle <= 45 || along != 0.0 || across != 0.0 {
                band(along, width(along, r), dx, dy)
            } else {
                -r <= dx && dx < r
            }
        }
        Shape::Line => {
            let w = width(across, r);
            if across != 0.0 {
                band(across, w, dx, dy)
            } else if angle <= 135 || along != 0.0 {
                -w <= dx && dx < w
            } else {
                -r <= dy && dy < r
            }
        }
    };
    // Lines past 90° are scanned column by column.
    let columns = shape == Shape::Line && angle >= 90;
    let mut radius = if shape == Shape::Round {
        size
    } else {
        size * 0.5
    };
    let mut step = radius;
    let mut previous = 0;
    let mut last = false;
    let mut shrinks = 0;
    let mut covered = vec![false; n * n];
    for _ in 0..if shape == Shape::Line {
        SEARCHES + 1
    } else {
        SEARCHES
    } {
        covered.fill(false);
        let mut hits = 0;
        'scan: for outer in 0..n {
            for inner in 0..n {
                let (i, j) = if columns {
                    (inner, outer)
                } else {
                    (outer, inner)
                };
                if owned(i, j) {
                    let dx = (j as f64 + half) - 0.1;
                    let dy = (i as f64 + half) - 0.1;
                    if inside(radius, dx, dy) {
                        covered[i * n + j] = true;
                        hits += 1;
                    }
                }
                if last && target <= hits {
                    break 'scan;
                }
            }
        }
        if last {
            break;
        }
        if previous == hits && target < hits {
            if shape != Shape::Line {
                last = true;
                continue;
            }
            // Lines shrink up to three more times, then keep what they have.
            if shrinks > 2 {
                break;
            }
            shrinks += 1;
            step *= 0.5;
            radius -= step;
        } else {
            if hits == target {
                break;
            }
            step *= 0.5;
            if hits < target {
                radius += step;
            } else {
                radius -= step;
            }
        }
        previous = hits;
    }
    covered
}

/// The tone over the whole canvas at `percent` density: opaque black dots,
/// transparent between them.
pub(crate) fn render(
    screen: &Screen,
    percent: u32,
    canvas: [usize; 2],
    cancelled: &impl Fn() -> bool,
) -> Result<Pixels, String> {
    let [width, height] = canvas;
    let [a, b] = screen
        .side()
        .ok_or("Screentone cells are too small for dots")?;
    let covered = dot(
        screen.shape,
        screen.angle,
        [a, b],
        f64::from(percent.min(100)) * 0.01,
    );
    let n = (a + b) as usize;
    let d = a * a + b * b;
    let mut pixels = Pixels::sparse(width, height);
    let mut writer = pixels.writer();
    let mut row = vec![0u8; width * 4];
    for y in 0..height {
        if cancelled() {
            return Err("Screentone drawing canceled".into());
        }
        row.fill(0);
        let y = y as i64;
        for x in 0..width {
            // Which cell the pixel is in, then where in that cell's box.
            let x = x as i64;
            let u = (x * a + y * b).div_euclid(d);
            let v = (y * a - x * b).div_euclid(d);
            let i = (y - u * b - v * a) as usize;
            let j = (x - u * a + (v + 1) * b) as usize;
            if covered[i * n + j] {
                row[x as usize * 4 + 3] = 255;
            }
        }
        writer.write_region(width, [0, y as usize, width, y as usize + 1], &row);
    }
    Ok(pixels)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cells_snap_to_whole_pixels() {
        let screen = |resolution, lines, angle| Screen {
            lines,
            resolution,
            angle,
            shape: Shape::Round,
        };
        // 300 / 85 = 3.53 px at 45° is (2.496, 2.496): cells of side (2, 2).
        assert_eq!(screen(300.0, 85.0, 45).side(), Some([2, 2]));
        assert_eq!(screen(600.0, 60.0, 0).side(), Some([10, 0]));
        // Angles repeat every 90° and lines stay within 5..=85.
        assert_eq!(
            screen(600.0, 60.0, 135).side(),
            screen(600.0, 60.0, 45).side()
        );
        assert_eq!(
            screen(600.0, 200.0, 0).side(),
            screen(600.0, 85.0, 0).side()
        );
        // 72 / 60 = 1.2 px cells are shaded evenly, as in CSP.
        assert_eq!(screen(72.0, 60.0, 80).side(), None);
        assert_eq!(screen(f64::NAN, 60.0, 0).side(), None);
        assert_eq!(screen(1e9, 60.0, 0).side(), None);
    }

    /// Renders two repeats of `csp`, the corner of a CSP saved preview.
    fn matches_csp(screen: Screen, percent: u32, csp: &[&str]) {
        let side = csp.len();
        let pixels = render(&screen, percent, [side * 2, side * 2], &|| false).unwrap();
        for y in 0..side * 2 {
            for x in 0..side * 2 {
                let black = csp[y % side].as_bytes()[x % side] == b'#';
                let expected = if black { [0, 0, 0, 255] } else { [0; 4] };
                assert_eq!(pixels.rgba(y * side * 2 + x), expected, "pixel {x}, {y}");
            }
        }
    }

    #[test]
    fn dots_match_csp_at_300_dpi_85_lines_45_degrees() {
        // Two three-pixel dots, 37.5% black.
        let screen = Screen {
            lines: 85.0,
            resolution: 300.0,
            angle: 45,
            shape: Shape::Round,
        };
        matches_csp(screen, 40, &["..##", "..#.", "##..", "#..."]);
    }

    #[test]
    fn squares_and_lines_match_csp_at_600_dpi_60_lines_45_degrees() {
        // At 70% the search stops at 66 (squares) and 77 (lines) of each
        // cell's 98 pixels, not 69.
        let screen = |shape| Screen {
            lines: 60.0,
            resolution: 600.0,
            angle: 45,
            shape,
        };
        matches_csp(
            screen(Shape::Square),
            70,
            &[
                "..###########.",
                "...##########.",
                "#...########..",
                "##...######..#",
                "###...####..##",
                "####...##..###",
                "#####.....####",
                "######...#####",
                "######....####",
                "#####..#...###",
                "####..###...##",
                "###..#####...#",
                "##..#######...",
                "...#########..",
            ],
        );
        matches_csp(
            screen(Shape::Line),
            70,
            &[
                "..###########.",
                "...###########",
                "#...##########",
                "##...#########",
                "###...########",
                "####...#######",
                "#####...######",
                "######...#####",
                "#######...####",
                "########...###",
                "#########...##",
                "##########...#",
                "###########...",
                ".###########..",
            ],
        );
        // Past 90° lines turn to the cell's other side.
        let turned = |angle| {
            let screen = Screen {
                angle,
                ..screen(Shape::Line)
            };
            render(&screen, 30, [14, 14], &|| false).unwrap()
        };
        let (a, b) = (turned(45), turned(135));
        assert!((0..14 * 14).any(|k| a.rgba(k) != b.rgba(k)));
    }

    #[test]
    fn dots_cover_the_density_share_of_each_cell() {
        // A 10-pixel axis-aligned cell has 100 pixels: the dot covers
        // exactly the requested share, growing from the center.
        for percent in [0, 10, 25, 50, 75, 100] {
            let covered = dot(Shape::Round, 0, [10, 0], f64::from(percent) / 100.0);
            assert_eq!(covered.iter().filter(|c| **c).count(), percent as usize);
        }
        let small = dot(Shape::Round, 0, [10, 0], 0.05);
        assert!(small[5 * 10 + 5] && !small[0]);
        // Every pixel of the canvas lands in exactly one cell position.
        let screen = Screen {
            lines: 60.0,
            resolution: 600.0,
            angle: 30,
            shape: Shape::Square,
        };
        let full = render(&screen, 100, [64, 48], &|| false).unwrap();
        assert!((0..64 * 48).all(|k| full.rgba(k) == [0, 0, 0, 255]));
        assert!(render(&screen, 50, [64, 48], &|| true).is_err());
    }
}
