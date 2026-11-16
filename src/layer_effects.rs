//! App side of `layer_effects`: the parts that need the window, the app state or
//! app-only modules. The engine part is `dudel_core::layer_effects`, re-exported
//! here so `crate::layer_effects::…` paths keep working.
pub(crate) use dudel_core::layer_effects::*;

#[cfg(test)]
mod tests {
    use super::*;

    use crate::document::Document;

    /// A 4 x 1 document with an opaque red pixel at x = 1.
    fn doc_with(effects: Vec<LayerEffect>) -> Document {
        let mut doc = Document::new(4, 1);
        doc.layers[0]
            .pixels
            .writer()
            .rgba_mut(1)
            .copy_from_slice(&[255, 0, 0, 255]);
        doc.layers[0].effects = effects;
        doc
    }

    fn overlay() -> LayerEffect {
        let mut effect = Effect::preset("Color Overlay").unwrap();
        effect.paint = Paint::Color([0, 0, 255]);
        LayerEffect::new(effect)
    }

    #[test]
    fn effect_edits_undo_and_reject_folders() {
        use crate::document_edits::{Edit, Property, Target};
        let mut app = crate::Dudel {
            doc: doc_with(Vec::new()),
            ..Default::default()
        };
        let target = app.doc.layers[0].id;
        assert!(app.set_canvas_property(target, Property::Effects(vec![overlay()])));
        app.flush_render_queue();
        assert_eq!(app.doc.layers[0].effects, vec![overlay()]);
        assert!(app.history.can_undo());
        app.undo();
        app.flush_render_queue();
        assert!(app.doc.layers[0].effects.is_empty());
        let edit = Edit::Property {
            target: Target::Identity(target),
            property: Property::Effects(vec![overlay(); crate::document::MAX_EFFECTS + 1]),
        };
        assert!(edit.validate_arguments().is_err());
    }
}
