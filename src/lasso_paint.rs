//! App side of `lasso_paint`: the parts that need the window, the app state or
//! app-only modules. The engine part is `dudel_core::lasso_paint`, re-exported
//! here so `crate::lasso_paint::…` paths keep working.
pub(crate) use dudel_core::lasso_paint::*;

#[cfg(test)]
mod tests {

    use crate::document::Document;

    use crate::document_edits::Edit;

    #[test]
    fn pending_layer_mask_lasso_and_duplicate_capture_independent_slots() {
        let mut app = crate::Dudel {
            doc: Document::new(8, 8),
            ..Default::default()
        };
        app.render_queue.edit(Edit::SelectLayer(0)).unwrap();
        app.new_layer();
        let original = app.input_target().unwrap();
        assert!(app.create_canvas_mask(crate::layer_mask_edits::Seed::HideAll));
        app.select_tool(crate::tools::Tool::Lasso);
        app.lasso.erase = true;
        app.brush.opacity = 1.0;
        assert!(app.input_layer().is_none());
        let points = [[0.0, 0.0], [8.0, 0.0], [8.0, 8.0], [0.0, 8.0]];
        for (i, p) in points.into_iter().enumerate() {
            app.update_selection_drag(p, [p[0] as usize, p[1] as usize], i == 0, i == 3, true);
        }
        app.duplicate_layer();
        let copied = app.input_target().unwrap();
        assert_eq!(
            app.input_stroke_target(),
            crate::paint_target::Target::Mask(copied)
        );
        app.select_tool(crate::tools::Tool::Lasso);
        app.lasso.erase = false;
        app.color = [0; 3];
        let points = [[0.0, 0.0], [4.0, 0.0], [4.0, 8.0], [0.0, 8.0]];
        for (i, p) in points.into_iter().enumerate() {
            app.update_selection_drag(p, [p[0] as usize, p[1] as usize], i == 0, i == 3, true);
        }
        app.flush_render_queue();
        let original = app.doc.layers.iter().find(|l| l.id == original).unwrap();
        let copy = app.doc.layers.iter().find(|l| l.id == copied).unwrap();
        assert_eq!(original.mask.as_ref().unwrap().coverage.at(2, 2), 255);
        assert_eq!(copy.mask.as_ref().unwrap().coverage.at(2, 2), 0);
        assert_eq!(copy.mask.as_ref().unwrap().coverage.at(6, 2), 255);
        assert!(original.pixels.iter().all(|b| *b == 0) && copy.pixels.iter().all(|b| *b == 0));
        app.undo();
        assert_eq!(
            app.doc.layers[app.doc.active]
                .mask
                .as_ref()
                .unwrap()
                .coverage
                .at(2, 2),
            255
        );
        app.undo();
        assert_eq!(app.doc.layers.len(), 2);
        app.undo();
        assert_eq!(
            app.doc.layers[app.doc.active]
                .mask
                .as_ref()
                .unwrap()
                .coverage
                .at(2, 2),
            0
        );
    }
}
