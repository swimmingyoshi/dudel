//! Target feedback beside the canvas, with explicit, undoable recovery actions.
use crate::{Dudel, document::LayerId, document_edits::Property, egui, tools::Tool};

pub(crate) struct Feedback {
    pub message: &'static str,
    pub action: Option<Recovery>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ux_feedback_recovers_projected_visibility_and_lock_without_losing_pixels() {
        let mut app = Dudel {
            doc: crate::document::Document::new(8, 8),
            ..Default::default()
        };
        app.doc.layers[0].pixels = [20, 40, 60, 255].repeat(64).into();
        app.doc.group_active().unwrap();
        app.doc.active = 0;
        let pixels = app.doc.layers[0].pixels.clone();
        // An unfinished edit keeps subsequent property edits queued.
        app.render_queue
            .edit(crate::document_edits::Edit::Selection(Some(
                crate::selection::Selection {
                    left: 0,
                    top: 0,
                    right: 8,
                    bottom: 8,
                },
            )))
            .unwrap();
        let id = app.doc.layers[0].id;
        let folder = app.doc.layers[1].id;
        app.set_canvas_property(folder, Property::Visible(false));
        app.set_canvas_property(id, Property::Locked(true));
        assert!(
            app.doc.layers[1].visible,
            "The authoritative document is still visible"
        );
        assert_eq!(
            app.editing_feedback().unwrap().action,
            Some(Recovery::Unlock)
        );
        app.recover_editing_target(Recovery::Unlock);
        assert_eq!(app.editing_feedback().unwrap().action, Some(Recovery::Show));
        app.recover_editing_target(Recovery::Show);
        assert!(app.editing_feedback().is_none());
        app.flush_render_queue();
        assert!(app.doc.shown(0));
        assert!(!app.doc.layers[0].locked);
        assert_eq!(app.doc.layers[0].pixels, pixels);
        app.undo();
        app.flush_render_queue();
        assert!(!app.doc.layers[1].visible);
    }

    #[test]
    fn ux_feedback_preserves_vectors_and_does_not_warn_on_drawable_masks() {
        let mut app = Dudel {
            doc: crate::document::Document::new(8, 8),
            ..Default::default()
        };
        app.create_vector_layer();
        app.flush_render_queue();
        assert!(app.editing_feedback().is_none(), "Brush draws vectors");
        let vector = app.doc.layers[app.doc.active].id;
        app.select_tool(Tool::Fill);
        assert_eq!(
            app.editing_feedback().unwrap().action,
            Some(Recovery::NewLayer)
        );
        app.recover_editing_target(Recovery::NewLayer);
        app.flush_render_queue();
        assert!(
            app.doc
                .layers
                .iter()
                .find(|l| l.id == vector)
                .unwrap()
                .vector
                .is_some()
        );
        let active = app.doc.active;
        let mask = crate::document::LayerId::new();
        app.doc.layers[active].mask = Some(crate::layer_mask::LayerMask {
            id: mask,
            enabled: true,
            linked: true,
            coverage: crate::layer_mask::Mask::new(8, 8, 255).unwrap(),
        });
        app.doc.layers[active].alpha_locked = true;
        app.select_tool(Tool::Eraser);
        assert_eq!(
            app.editing_feedback().unwrap().action,
            Some(Recovery::UnlockAlpha)
        );
        app.focus_current_mask(true);
        assert!(
            app.editing_feedback().is_none(),
            "Alpha lock does not block mask erasing"
        );
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Recovery {
    Unlock,
    Show,
    UnlockAlpha,
    NewLayer,
    Mask,
}
impl Recovery {
    fn label(self) -> &'static str {
        match self {
            Self::Unlock => "Unlock layer",
            Self::Show => "Show layer and folders",
            Self::UnlockAlpha => "Unlock transparency",
            Self::NewLayer => "New raster layer",
            Self::Mask => "Paint mask",
        }
    }
}

impl Dudel {
    pub(crate) fn editing_feedback(&self) -> Option<Feedback> {
        if self.reference_layers.active.is_some()
            || !matches!(
                self.tool,
                Tool::Brush | Tool::Eraser | Tool::Fill | Tool::Lasso
            )
        {
            return None;
        }
        let properties = self.input_properties()?;
        let raster = self.input_stroke_target().is_raster();
        let blocked = |message, action| {
            Some(Feedback {
                message,
                action: Some(action),
            })
        };
        if properties.locked {
            return blocked("Layer locked", Recovery::Unlock);
        }
        if raster && properties.folder {
            return blocked("Folder selected", Recovery::NewLayer);
        }
        if !self.hidden_target_ancestors().is_empty() {
            return blocked("Layer or folder hidden", Recovery::Show);
        }
        if raster && self.tool == Tool::Eraser && properties.alpha_locked {
            return blocked("Transparency locked", Recovery::UnlockAlpha);
        }
        if raster && let Some(layer) = self.input_layer().map(|i| &self.doc.layers[i]) {
            if layer.adjustment.is_some() {
                return blocked(
                    "Adjustment layer selected",
                    if properties.mask.is_some() {
                        Recovery::Mask
                    } else {
                        Recovery::NewLayer
                    },
                );
            }
            if layer.vector.is_some() && matches!(self.tool, Tool::Fill | Tool::Lasso) {
                return blocked("This tool needs raster artwork", Recovery::NewLayer);
            }
        }
        self.canvas_paint_blocker().map(|message| Feedback {
            message,
            action: None,
        })
    }

    fn hidden_target_ancestors(&self) -> Vec<LayerId> {
        if let Some(context) = &self.pending_layers {
            return self
                .input_target()
                .map_or_else(Vec::new, |id| context.hidden_ancestors(id));
        }
        let mut hidden = Vec::new();
        let mut current = self.input_layer();
        while let Some(i) = current {
            if !self.doc.layers[i].visible {
                hidden.push(self.doc.layers[i].id);
            }
            current = self.doc.parent(i);
        }
        hidden
    }

    pub(crate) fn recover_editing_target(&mut self, recovery: Recovery) {
        // Recheck against projected state rather than applying a stale UI action.
        if !self
            .editing_feedback()
            .is_some_and(|f| f.action == Some(recovery))
        {
            return;
        }
        let Some(target) = self.input_target() else {
            return;
        };
        match recovery {
            Recovery::Unlock => {
                self.set_canvas_property(target, Property::Locked(false));
            }
            Recovery::UnlockAlpha => {
                self.set_canvas_property(target, Property::AlphaLocked(false));
            }
            Recovery::Show => {
                for id in self.hidden_target_ancestors() {
                    self.set_canvas_property(id, Property::Visible(true));
                }
            }
            Recovery::NewLayer => self.new_layer(),
            Recovery::Mask => self.focus_current_mask(true),
        }
    }

    pub(crate) fn editing_target_ui(&mut self, ui: &mut egui::Ui) {
        let reference = self.reference_layers.active.is_some();
        let raster = self.input_stroke_target().is_raster();
        let target = if reference {
            "Reference"
        } else if !raster {
            "Mask"
        } else if self
            .input_layer()
            .is_some_and(|i| self.doc.layers[i].vector.is_some())
        {
            "Vector artwork"
        } else {
            "Artwork"
        };
        let name = if reference {
            self.reference_layers
                .layers
                .iter()
                .find(|b| Some(b.id) == self.reference_layers.active)
                .map(|b| b.name.as_str())
        } else {
            self.input_layer().map(|i| self.doc.layers[i].name.as_str())
        }
        .unwrap_or("Pending layer");
        let title = format!("{target} · {name}");
        ui.horizontal_wrapped(|ui| {
            ui.add(egui::Label::new(&title).truncate())
                .on_hover_text(&title);
            if (reference || !raster) && ui.small_button("Switch to artwork").clicked() {
                self.deselect_reference();
                self.focus_current_mask(false);
            }
            if let Some(feedback) = self.editing_feedback() {
                ui.colored_label(ui.visuals().warn_fg_color, feedback.message);
                if let Some(action) = feedback.action
                    && ui
                        .add_enabled(
                            action != Recovery::NewLayer || self.doc.has_room(1, 1),
                            egui::Button::new(action.label()).small(),
                        )
                        .clicked()
                {
                    self.recover_editing_target(action);
                }
            }
            if self.doc.selection.is_some() || self.doc.selection_mask.is_some() {
                ui.label("Selection active");
                if ui.small_button("Deselect").clicked() {
                    self.set_selection(None);
                }
            }
        });
    }
}
