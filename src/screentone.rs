//! CSP screentone: a layer at one dot density drawn as round black dots on
//! a rotated grid, the way Clip Studio Paint's tone effect draws it.
//!
//! CSP snaps the grid to whole pixels so it repeats exactly: a cell's side
//! is the pixel vector (a, b) nearest to (cos, sin) of the angle (modulo
//! 90°) times resolution ÷ lines, and the cells are the squares spanned by
//! (a, b) and (-b, a) from the canvas origin, each pixel belonging to one.
//! In every cell the dot is the disc around the cell's center whose radius
//! is binary-searched until it covers the density's share of the cell's
//! pixels; when two pixels tie, the search ends with too many and the dot
//! keeps the first ones in row order. Pixel positions are their top-left
//! corners nudged by -0.1, which breaks most ties toward the lower right.
//! Checked pixel for pixel against a tone CSP saved at 300 dpi, 85 lines,
//! 45° and 40%.
use crate::pixels::Pixels;

/// The largest cell side drawn as dots; past it (a corrupt resolution) the
/// caller shades evenly.
const MAX_CELL: i64 = 512;
/// CSP stops refining the dot's radius after this many tries.
const SEARCHES: usize = 20;

/// A tone's grid.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Screen {
    /// Lines per inch; CSP keeps it within 5..=85.
    pub(crate) lines: f64,
    /// Pixels per inch the tone was laid out for.
    pub(crate) resolution: f64,
    /// Degrees.
    pub(crate) angle: u32,
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
fn dot([a, b]: [i64; 2], density: f64) -> Vec<bool> {
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
    let mut radius = (d as f64).sqrt();
    let mut step = radius;
    let mut previous = 0;
    let mut last = false;
    let mut covered = vec![false; n * n];
    for _ in 0..SEARCHES {
        covered.fill(false);
        let mut hits = 0;
        'rows: for i in 0..n {
            let dy = (i as f64 + half) - 0.1;
            for j in 0..n {
                if owned(i, j) {
                    let dx = (j as f64 + half) - 0.1;
                    if dx * dx + dy * dy < radius * radius {
                        covered[i * n + j] = true;
                        hits += 1;
                    }
                }
                if last && target <= hits {
                    break 'rows;
                }
            }
        }
        if last {
            break;
        }
        if previous == hits && target < hits {
            last = true;
        } else {
            if hits == target {
                break;
            }
            previous = hits;
            step *= 0.5;
            if hits < target {
                radius += step;
            } else {
                radius -= step;
            }
        }
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
    let covered = dot([a, b], f64::from(percent.min(100)) * 0.01);
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

    #[test]
    fn dots_match_csp_at_300_dpi_85_lines_45_degrees() {
        // The repeating 4 x 4 corner of CSP's saved preview of a 40% tone:
        // two three-pixel dots, 37.5% black.
        let csp = ["..##", "..#.", "##..", "#..."];
        let screen = Screen {
            lines: 85.0,
            resolution: 300.0,
            angle: 45,
        };
        let pixels = render(&screen, 40, [8, 8], &|| false).unwrap();
        for y in 0..8 {
            for x in 0..8 {
                let black = csp[y % 4].as_bytes()[x % 4] == b'#';
                let expected = if black { [0, 0, 0, 255] } else { [0; 4] };
                assert_eq!(pixels.rgba(y * 8 + x), expected, "pixel {x}, {y}");
            }
        }
    }

    #[test]
    fn dots_cover_the_density_share_of_each_cell() {
        // A 10-pixel axis-aligned cell has 100 pixels: the dot covers
        // exactly the requested share, growing from the center.
        for percent in [0, 10, 25, 50, 75, 100] {
            let covered = dot([10, 0], f64::from(percent) / 100.0);
            assert_eq!(covered.iter().filter(|c| **c).count(), percent as usize);
        }
        let small = dot([10, 0], 0.05);
        assert!(small[5 * 10 + 5] && !small[0]);
        // Every pixel of the canvas lands in exactly one cell position.
        let screen = Screen {
            lines: 60.0,
            resolution: 600.0,
            angle: 30,
        };
        let full = render(&screen, 100, [64, 48], &|| false).unwrap();
        assert!((0..64 * 48).all(|k| full.rgba(k) == [0, 0, 0, 255]));
        assert!(render(&screen, 50, [64, 48], &|| true).is_err());
    }
}
