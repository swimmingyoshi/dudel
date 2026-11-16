use serde::{Deserialize, Serialize};

/// Half-open rectangle in document pixel coordinates.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Selection {
    pub left: usize,
    pub top: usize,
    pub right: usize,
    pub bottom: usize,
}

impl Selection {
    pub fn contains(self, x: usize, y: usize) -> bool {
        x >= self.left && x < self.right && y >= self.top && y < self.bottom
    }
    pub fn from_drag(a: [usize; 2], b: [usize; 2], width: usize, height: usize) -> Self {
        Self {
            left: a[0].min(b[0]).min(width - 1),
            top: a[1].min(b[1]).min(height - 1),
            right: a[0].max(b[0]).saturating_add(1).min(width),
            bottom: a[1].max(b[1]).saturating_add(1).min(height),
        }
    }
    pub fn valid(self, width: usize, height: usize) -> bool {
        self.left < self.right
            && self.top < self.bottom
            && self.right <= width
            && self.bottom <= height
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reverse_drag_clamps_to_canvas_and_uses_half_open_bounds() {
        let selection = Selection::from_drag([9, 7], [2, 3], 8, 6);
        assert_eq!(
            selection,
            Selection {
                left: 2,
                top: 3,
                right: 8,
                bottom: 6
            }
        );
        assert!(selection.valid(8, 6));
        assert!(selection.contains(7, 5));
        assert!(!selection.contains(8, 5));
    }
}
