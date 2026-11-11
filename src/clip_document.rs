//! Assemble a validated source plan without publishing it to the editor.
use crate::{
    document::{Document, Layer, MAX_DEPTH, MAX_ENTRIES, MAX_RASTER_LAYERS},
    pixels::Pixels,
};

pub(crate) enum Content {
    Raster,
    Folder,
    Paper([u8; 3]),
    /// A correction layer, as a Dudel adjustment layer.
    Adjustment(crate::adjustment::Settings),
}
pub(crate) struct Entry {
    pub(crate) source_id: i64,
    pub(crate) name: String,
    pub(crate) depth: u8,
    pub(crate) visible: bool,
    pub(crate) opacity: u16,
    pub(crate) clipped: bool,
    pub(crate) content: Content,
}

/// Entries are source root-child-next preorder, excluding the neutral root.
pub(crate) fn assemble(
    canvas: [usize; 2],
    entries: Vec<Entry>,
    active: i64,
    unsupported: &[String],
    cancelled: &impl Fn() -> bool,
    decode: &mut impl FnMut(i64) -> Result<Pixels, String>,
) -> Result<Document, String> {
    if !unsupported.is_empty() {
        return Err(format!(
            "Unsupported clip content: {}",
            unsupported.join("; ")
        ));
    }
    let [width, height] = canvas;
    if width == 0
        || height == 0
        || width > crate::document::MAX_SIDE
        || height > crate::document::MAX_SIDE
        || width * height > crate::document::MAX_PIXELS
        || entries.is_empty()
        || entries.len() > MAX_ENTRIES
    {
        return Err("Clip document exceeds Dudel limits".into());
    }
    if entries
        .iter()
        .filter(|entry| !matches!(entry.content, Content::Folder))
        .count()
        > MAX_RASTER_LAYERS
    {
        return Err("Clip document including paper exceeds raster layer limit".into());
    }
    let mut identities = std::collections::BTreeSet::new();
    let mut parents = Vec::<u8>::new();
    for entry in &entries {
        while parents.last().is_some_and(|depth| *depth >= entry.depth) {
            parents.pop();
        }
        if entry.depth > MAX_DEPTH
            || entry.depth as usize != parents.len()
            || entry.opacity > 256
            || entry.name.len() > 4096
            || entry.source_id <= 0
            || !identities.insert(entry.source_id)
        {
            return Err("Invalid clip document plan identity/properties/hierarchy".into());
        }
        if matches!(entry.content, Content::Folder) {
            parents.push(entry.depth);
        }
    }
    if !identities.contains(&active) {
        return Err("Clip current layer does not map to an imported entry".into());
    }
    let mut layers = Vec::new();
    let mut pending = Vec::<(i64, Layer)>::new();
    let mut active_index = None;
    fn push(
        layers: &mut Vec<Layer>,
        active_index: &mut Option<usize>,
        active: i64,
        id: i64,
        layer: Layer,
    ) {
        if id == active {
            *active_index = Some(layers.len());
        }
        layers.push(layer);
    }
    for entry in entries {
        if cancelled() {
            return Err("Clip document assembly canceled".into());
        }
        while pending
            .last()
            .is_some_and(|(_, layer)| layer.depth >= entry.depth)
        {
            let (id, layer) = pending.pop().unwrap();
            push(&mut layers, &mut active_index, active, id, layer);
        }
        let mut layer = match entry.content {
            Content::Folder => Layer::new_folder(entry.name),
            Content::Raster => {
                let pixels = decode(entry.source_id)?;
                if !pixels.has_geometry(width, height) {
                    return Err("Decoded clip raster geometry disagrees with canvas".into());
                }
                let mut layer = Layer::raster(entry.name, width, height);
                layer.pixels = pixels;
                layer
            }
            Content::Adjustment(settings) => {
                let mut layer = Layer::raster(entry.name, width, height);
                layer.adjustment = Some(crate::adjustment::Adjustment::try_from(settings)?);
                layer
            }
            Content::Paper(rgb) => {
                let mut layer = Layer::raster(entry.name, width, height);
                let row: Vec<u8> = [rgb[0], rgb[1], rgb[2], 255].repeat(width);
                let mut writer = layer.pixels.writer();
                for y in 0..height {
                    if cancelled() {
                        return Err("Clip document assembly canceled".into());
                    }
                    writer.write_region(width, [0, y, width, y + 1], &row);
                }
                layer
            }
        };
        layer.depth = entry.depth;
        layer.visible = entry.visible;
        layer.opacity = entry.opacity as f32 / 256.;
        layer.clipped = entry.clipped;
        // Folder group quantization awaits its own controlled CSP fixtures.
        layer.csp_byte_opacity = !layer.folder;
        if layer.folder {
            pending.push((entry.source_id, layer));
        } else {
            push(
                &mut layers,
                &mut active_index,
                active,
                entry.source_id,
                layer,
            );
        }
    }
    while let Some((id, layer)) = pending.pop() {
        push(&mut layers, &mut active_index, active, id, layer);
    }
    if cancelled() {
        return Err("Clip document assembly canceled".into());
    }
    let doc = Document {
        version: 1,
        width,
        height,
        dpi: crate::document::default_dpi(),
        layers,
        active: active_index.unwrap(),
        selection: None,
        selection_mask: None,
        guides: None,
        references: Vec::new(),
        paper: None,
        canvas_frame: crate::canvas_size::Frame::identity(),
    };
    doc.validate()?;
    Ok(doc)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn entry(id: i64, depth: u8, content: Content) -> Entry {
        Entry {
            source_id: id,
            name: format!("source {id}"),
            depth,
            visible: true,
            opacity: 256,
            clipped: false,
            content,
        }
    }
    fn raster(rgba: [u8; 4]) -> Pixels {
        let mut pixels = Pixels::sparse(1, 1);
        pixels.writer().rgba_mut(0).copy_from_slice(&rgba);
        pixels
    }
    #[test]
    fn postorder_keeps_siblings_nested_groups_active_identity_and_names() {
        let entries = vec![
            entry(1, 0, Content::Paper([12, 34, 56])),
            entry(2, 0, Content::Folder),
            entry(3, 1, Content::Raster),
            entry(4, 1, Content::Folder),
            entry(5, 2, Content::Raster),
            entry(6, 0, Content::Raster),
        ];
        let doc = assemble([1, 1], entries, 4, &[], &|| false, &mut |_| {
            Ok(raster([0; 4]))
        })
        .unwrap();
        assert_eq!(
            doc.layers
                .iter()
                .map(|layer| layer.name.as_str())
                .collect::<Vec<_>>(),
            [
                "source 1", "source 3", "source 5", "source 4", "source 2", "source 6"
            ]
        );
        assert_eq!(doc.active, 3);
        assert!(doc.layers[3].folder);
        assert_eq!(doc.layers[0].pixels.rgba(0), [12, 34, 56, 255]);
        doc.validate().unwrap();
    }
    #[test]
    fn opacity_visibility_and_clipping_are_applied_once_by_the_native_compositor() {
        let paper = entry(1, 0, Content::Paper([0, 0, 0]));
        let mut base = entry(2, 0, Content::Raster);
        base.opacity = 128;
        let mut clip = entry(3, 0, Content::Raster);
        clip.clipped = true;
        let mut doc = assemble(
            [1, 1],
            vec![paper, base, clip],
            2,
            &[],
            &|| false,
            &mut |id| {
                Ok(raster(if id == 2 {
                    [255, 0, 0, 255]
                } else {
                    [0, 255, 0, 255]
                }))
            },
        )
        .unwrap();
        assert_eq!(doc.layers[1].pixels.rgba(0), [255, 0, 0, 255]);
        assert_eq!(doc.layers[1].opacity, 0.5);
        assert!(doc.layers[1].csp_byte_opacity);
        assert_eq!(doc.composite_transparent(), [0, 127, 0, 255]);
        doc.layers[2].visible = false;
        assert_eq!(doc.composite_transparent(), [127, 0, 0, 255]);
    }
    #[test]
    fn unsupported_invalid_hierarchy_and_extra_paper_limit_fail_before_decoding() {
        let calls = std::cell::Cell::new(0);
        let mut decode = |_| {
            calls.set(calls.get() + 1);
            Ok(raster([0; 4]))
        };
        assert!(
            assemble(
                [1, 1],
                vec![entry(1, 0, Content::Raster)],
                1,
                &["animation".into()],
                &|| false,
                &mut decode
            )
            .is_err()
        );
        assert!(
            assemble(
                [1, 1],
                vec![entry(1, 1, Content::Raster)],
                1,
                &[],
                &|| false,
                &mut decode
            )
            .is_err()
        );
        let limit = MAX_RASTER_LAYERS as i64;
        let mut entries: Vec<_> = (1..=limit)
            .map(|id| entry(id, 0, Content::Raster))
            .collect();
        entries.push(entry(limit + 1, 0, Content::Paper([255; 3])));
        assert!(assemble([1, 1], entries, 1, &[], &|| false, &mut decode).is_err());
        assert_eq!(calls.get(), 0);
    }
    #[test]
    fn cancellation_and_late_decode_failure_never_return_a_partial_document() {
        let calls = std::cell::Cell::new(0);
        let result = assemble(
            [1, 1],
            vec![entry(1, 0, Content::Raster), entry(2, 0, Content::Raster)],
            1,
            &[],
            &|| false,
            &mut |_| {
                calls.set(calls.get() + 1);
                if calls.get() == 2 {
                    Err("broken later layer".into())
                } else {
                    Ok(raster([255; 4]))
                }
            },
        );
        assert!(result.is_err());
        assert_eq!(calls.get(), 2);
        let cancel = std::cell::Cell::new(false);
        assert!(
            assemble(
                [1, 1],
                vec![entry(1, 0, Content::Raster)],
                1,
                &[],
                &|| cancel.get(),
                &mut |_| {
                    cancel.set(true);
                    Ok(raster([255; 4]))
                }
            )
            .is_err()
        );
    }
    #[test]
    fn folder_opacity_is_applied_to_children_and_paper_fill_is_cancellable() {
        let mut folder = entry(2, 0, Content::Folder);
        folder.opacity = 128;
        let doc = assemble(
            [1, 1],
            vec![
                entry(1, 0, Content::Paper([0, 0, 255])),
                folder,
                entry(3, 1, Content::Raster),
            ],
            3,
            &[],
            &|| false,
            &mut |_| Ok(raster([0, 255, 0, 255])),
        )
        .unwrap();
        assert_eq!(doc.composite_transparent(), [0, 128, 128, 255]);
        let polls = std::cell::Cell::new(0);
        let cancel = || {
            polls.set(polls.get() + 1);
            polls.get() > 2
        };
        assert!(
            assemble(
                [1, 3],
                vec![entry(1, 0, Content::Paper([255; 3]))],
                1,
                &[],
                &cancel,
                &mut |_| panic!("paper must not decode a raster")
            )
            .is_err()
        );
    }
}
