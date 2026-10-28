use eframe::egui::{Pos2, Vec2, pos2, vec2};

pub const MIN_ZOOM: f32 = 0.05;
pub const MAX_ZOOM: f32 = 32.0;

/// Display-only canvas placement. Rotation and mirroring never change pixels;
/// they only map between screen points and document coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct View {
    /// Screen position of the document center.
    pub origin: Pos2,
    pub zoom: f32,
    /// Radians, clockwise on screen.
    pub rotation: f32,
    pub mirror: bool,
    /// Half the document size in pixels.
    pub half: Vec2,
}

pub fn rotate(v: Vec2, angle: f32) -> Vec2 {
    let (sin, cos) = angle.sin_cos();
    vec2(v.x * cos - v.y * sin, v.x * sin + v.y * cos)
}

impl View {
    pub fn to_screen(self, doc: Pos2) -> Pos2 {
        let mut v = (doc.to_vec2() - self.half) * self.zoom;
        if self.mirror {
            v.x = -v.x;
        }
        self.origin + rotate(v, self.rotation)
    }
    pub fn to_doc(self, screen: Pos2) -> Pos2 {
        let mut v = rotate(screen - self.origin, -self.rotation) / self.zoom;
        if self.mirror {
            v.x = -v.x;
        }
        (v + self.half).to_pos2()
    }
    /// Map a document-space offset (such as a brush outline point) to screen.
    pub fn offset_to_screen(self, offset: Vec2) -> Vec2 {
        let mut v = offset * self.zoom;
        if self.mirror {
            v.x = -v.x;
        }
        rotate(v, self.rotation)
    }
    pub fn corners(self) -> [Pos2; 4] {
        let size = self.half * 2.0;
        [
            pos2(0.0, 0.0),
            pos2(size.x, 0.0),
            pos2(size.x, size.y),
            pos2(0.0, size.y),
        ]
        .map(|p| self.to_screen(p))
    }
}

/// Zoom that fits the rotated document inside the viewport with a margin.
pub fn fit_zoom(viewport: Vec2, document: Vec2, rotation: f32) -> f32 {
    let (sin, cos) = rotation.sin_cos();
    let width = document.x * cos.abs() + document.y * sin.abs();
    let height = document.x * sin.abs() + document.y * cos.abs();
    ((viewport.x - 48.0) / width)
        .min((viewport.y - 48.0) / height)
        .clamp(MIN_ZOOM, 4.0)
}

/// Normalize to (-PI, PI] so the displayed angle stays readable.
pub fn normalize(angle: f32) -> f32 {
    let tau = std::f32::consts::TAU;
    let wrapped = angle.rem_euclid(tau);
    if wrapped > std::f32::consts::PI {
        wrapped - tau
    } else {
        wrapped
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn screen_and_document_mapping_round_trips_with_rotation_and_mirror() {
        for mirror in [false, true] {
            let view = View {
                origin: pos2(400.0, 300.0),
                zoom: 1.7,
                rotation: 0.6,
                mirror,
                half: vec2(512.0, 384.0),
            };
            let doc = pos2(100.0, 650.0);
            let back = view.to_doc(view.to_screen(doc));
            assert!((back - doc).length() < 1e-3, "{back:?}");
            // The document center always sits at the origin.
            assert!((view.to_screen(pos2(512.0, 384.0)) - view.origin).length() < 1e-3);
        }
    }
    #[test]
    fn mirroring_swaps_left_and_right_on_screen() {
        let mut view = View {
            origin: pos2(0.0, 0.0),
            zoom: 1.0,
            rotation: 0.0,
            mirror: false,
            half: vec2(10.0, 10.0),
        };
        assert!(view.to_screen(pos2(0.0, 10.0)).x < 0.0);
        view.mirror = true;
        assert!(view.to_screen(pos2(0.0, 10.0)).x > 0.0);
    }
    #[test]
    fn fit_accounts_for_rotation_and_angles_normalize() {
        let straight = fit_zoom(vec2(1048.0, 1048.0), vec2(1000.0, 500.0), 0.0);
        let turned = fit_zoom(vec2(1048.0, 1048.0), vec2(1000.0, 500.0), 0.7);
        assert!(turned < straight);
        assert!((normalize(3.0 * std::f32::consts::PI) - std::f32::consts::PI).abs() < 1e-5);
        assert!((normalize(-0.25) + 0.25).abs() < 1e-6);
    }
}
