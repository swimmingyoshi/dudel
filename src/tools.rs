use crate::icons::Icon;

/// What of the eraser is in the brush slot while the Eraser tool is active.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EraserSwap {
    /// The whole eraser tip (Eraser tip mode).
    Tip,
    /// Only the eraser size; the shape stays the brush's (Brush shape mode).
    Size,
}

/// The separate eraser tip: round, firm and pressure-sized.
pub fn default_eraser() -> crate::brush::Brush {
    crate::brush::Brush {
        name: "Eraser".into(),
        size: 30.0,
        hardness: 0.9,
        spacing: 0.08,
        pressure_size: true,
        ..Default::default()
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    Brush,
    Eraser,
    Fill,
    Picker,
    Select,
    /// Fills or erases a drawn shape; the mode lives in `lasso_paint::Options`.
    Lasso,
    Move,
    /// Adds and edits text layers.
    Text,
    /// Moves guide lines and the symmetry center and pans the view; never paints.
    Pointer,
}
impl Tool {
    pub const ALL: [Tool; 9] = [
        Tool::Brush,
        Tool::Eraser,
        Tool::Fill,
        Tool::Picker,
        Tool::Select,
        Tool::Lasso,
        Tool::Move,
        Tool::Text,
        Tool::Pointer,
    ];
    pub fn name(self) -> &'static str {
        match self {
            Self::Brush => "Brush",
            Self::Eraser => "Eraser",
            Self::Fill => "Fill",
            Self::Picker => "Color picker",
            Self::Select => "Selection",
            Self::Lasso => "Lasso",
            Self::Move => "Move",
            Self::Text => "Text",
            Self::Pointer => "Pointer",
        }
    }
    #[cfg(test)]
    pub fn key(self) -> eframe::egui::Key {
        use eframe::egui::Key;
        match self {
            Self::Brush => Key::B,
            Self::Eraser => Key::E,
            Self::Fill => Key::G,
            Self::Picker => Key::I,
            Self::Select => Key::M,
            Self::Lasso => Key::L,
            Self::Move => Key::V,
            Self::Text => Key::T,
            Self::Pointer => Key::O,
        }
    }
    pub fn icon(self) -> Icon {
        match self {
            Self::Brush => Icon::Brush,
            Self::Eraser => Icon::Eraser,
            Self::Fill => Icon::Fill,
            Self::Picker => Icon::Picker,
            Self::Select => Icon::Select,
            Self::Lasso => Icon::LassoFill,
            Self::Move => Icon::Move,
            Self::Text => Icon::Text,
            Self::Pointer => Icon::Pointer,
        }
    }
    pub fn hint(self) -> &'static str {
        match self {
            Self::Brush => "Paint with the selected preset · Shift+click draws a straight line",
            Self::Eraser => "Erase with the current brush's shape, or with its own eraser tip",
            Self::Fill => {
                "Fill an area of similar color, or drag a gradient · tap its shortcut again to switch"
            }
            Self::Picker => "Sample visible artwork · hold Alt with any tool",
            Self::Select => {
                "Rectangle, lasso, polygon or magic wand · Deselect removes the selection · Delete clears"
            }
            Self::Move => "Drag to move the selection, or the whole active layer",
            Self::Text => {
                "Click to type, or drag a box · on the selected text, click edits and drag moves · double-click other text to open it · Ctrl+Enter finishes"
            }
            Self::Lasso => {
                "Draw a freehand, rectangle or ellipse shape to fill or erase · tap its shortcut again to switch fill/erase · Esc cancels"
            }
            Self::Pointer => {
                "Move guide lines, text and the symmetry center, even when locked · double-click text to edit it · drag empty canvas to pan"
            }
        }
    }
    pub(crate) fn target_hint(self, mask: bool) -> &'static str {
        if mask {
            match self {
                Self::Brush => {
                    return "Paint mask coverage: black conceals, white reveals · Shift+click draws a straight line";
                }
                Self::Eraser => return "Reveal artwork with the selected preset's shape",
                Self::Fill => return "Fill a contiguous mask area with the current coverage",
                Self::Picker => {
                    return "Sample mask coverage, including disabled masks · hold Alt with any tool";
                }
                Self::Move => return "Drag to move selected mask coverage, or the whole mask",
                Self::Lasso => {
                    return "Draw a shape to fill with the current mask coverage, or erase to reveal artwork · Esc cancels";
                }
                _ => {}
            }
        }
        self.hint()
    }
    pub fn is_lasso_paint(self) -> bool {
        self == Self::Lasso
    }
}
