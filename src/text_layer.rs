//! App side of `text_layer`: the parts that need the window, the app state or
//! app-only modules. The engine part is `dudel_core::text_layer`, re-exported
//! here so `crate::text_layer::…` paths keep working.
pub(crate) use dudel_core::text_layer::*;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{document::Document, text_render::Align};
    use crate::{document::LayerId, document_edits::Edit, text_render::TEST_FONT};

    fn text(string: &str) -> TextLayer {
        TextLayer {
            string: string.into(),
            font: TEST_FONT.into(),
            size: 40.0,
            color: [200, 10, 30],
            align: Align::Left,
            vertical: false,
            rightward: false,
            letters: Default::default(),
            format: Default::default(),
            wrap: None,
            box_height: None,
            line_spacing: 1.0,
            start: [20.0, 20.0],
            along: [1.0, 0.0],
            down: [0.0, 1.0],
            csp: None,
            spans: Vec::new(),
        }
    }

    fn ink(doc: &Document, index: usize) -> usize {
        let pixels = &doc.layers[index].pixels;
        (0..doc.width * doc.height)
            .filter(|p| pixels.rgba(*p)[3] != 0)
            .count()
    }

    #[test]
    fn the_queue_draws_text_and_undo_steps_back_through_it() {
        let mut app = crate::Dudel {
            doc: Document::new(300, 200),
            ..Default::default()
        };
        let layers = app.doc.layers.len();
        let id = LayerId::new();
        let new = Edit::NewText {
            id,
            text: Box::new(text("Hemlo")),
        };
        app.dispatch_document_edit(new, "Added", "Not added");
        app.flush_render_queue();
        assert_eq!(app.doc.layers.len(), layers + 1);
        let short = ink(&app.doc, app.doc.active);
        assert!(short > 0);
        // A brush on the layer stops before any stroke begins.
        assert_eq!(app.vector_stroke_blocker(), Some(BLOCKER));

        let set = Edit::SetText {
            target: id,
            text: Box::new(text("Hemlo Hemlo")),
        };
        app.dispatch_document_edit(set, "Changed", "Not changed");
        app.flush_render_queue();
        assert!(ink(&app.doc, app.doc.active) > short);

        app.undo();
        app.flush_render_queue();
        let layer = &app.doc.layers[app.doc.active];
        assert_eq!(layer.text.as_ref().unwrap().string, "Hemlo");
        assert_eq!(ink(&app.doc, app.doc.active), short);

        app.undo();
        app.flush_render_queue();
        assert_eq!(app.doc.layers.len(), layers);
        assert!(app.doc.layers.iter().all(|l| l.text.is_none()));
    }
}
