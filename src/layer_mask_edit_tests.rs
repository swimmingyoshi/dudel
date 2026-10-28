use crate::{
    document::{Document, LayerId},
    document_edits::Edit,
    layer_mask::{LayerMask, Mask},
    layer_mask_edits::{Command, Seed},
};
fn command(doc: &Document, command: Command) -> Edit {
    Edit::LayerMask {
        target: doc.layers[doc.active].id,
        command,
    }
}
fn create(doc: &Document, seed: Seed) -> Document {
    command(
        doc,
        Command::Create {
            id: LayerId::new(),
            seed,
        },
    )
    .prepare(doc)
    .unwrap()
    .0
}
fn id(doc: &Document) -> LayerId {
    doc.layers[doc.active].mask.as_ref().unwrap().id
}
fn json(doc: &Document) -> serde_json::Value {
    serde_json::to_value(doc).unwrap()
}
#[test]
fn mask_creation_uses_exact_selection_coverage_and_has_constant_reveal_hide_seeds() {
    let mut source = Document::new(65, 3);
    let coverage: Vec<u8> = (0..195).map(|i| ((i * 37) % 256) as u8).collect();
    let selection = crate::selection_mask::Mask::from_coverage(65, 3, coverage.clone()).unwrap();
    source.selection = selection.bounds();
    source.selection_mask = Some(selection);
    let result = create(&source, Seed::Selection);
    assert_eq!(result.version, 2);
    assert_eq!(result.selection_mask, source.selection_mask);
    for (i, expected) in coverage.into_iter().enumerate() {
        assert_eq!(
            result.layers[0]
                .mask
                .as_ref()
                .unwrap()
                .coverage
                .at(i % 65, i / 65),
            expected
        );
    }
    assert!(result.layers[0].pixels.shares(&source.layers[0].pixels));
    for (seed, value) in [(Seed::RevealAll, 255), (Seed::HideAll, 0)] {
        let result = create(&source, seed);
        let mask = result.layers[0].mask.as_ref().unwrap();
        assert_eq!(mask.coverage.payload_bytes(), 0);
        assert_eq!(mask.coverage.at(64, 2), value);
    }
    let blank = Document::new(1, 1);
    assert!(
        command(
            &blank,
            Command::Create {
                id: LayerId::new(),
                seed: Seed::Selection
            }
        )
        .prepare(&blank)
        .is_err()
    );
    assert!(
        command(
            &result,
            Command::Create {
                id: LayerId::new(),
                seed: Seed::HideAll
            }
        )
        .prepare(&result)
        .is_err()
    );
}
#[test]
fn mask_identity_is_stable_for_edits_fresh_for_duplicate_and_blocks_replacement_targets() {
    let original = create(&Document::new(2, 2), Seed::HideAll);
    let mask_id = id(&original);
    let toggled = command(
        &original,
        Command::Enabled {
            id: mask_id,
            value: false,
        },
    )
    .prepare(&original)
    .unwrap()
    .0;
    let toggled = command(
        &toggled,
        Command::Linked {
            id: mask_id,
            value: false,
        },
    )
    .prepare(&toggled)
    .unwrap()
    .0;
    let inverted = command(&toggled, Command::Invert(mask_id))
        .prepare(&toggled)
        .unwrap()
        .0;
    assert_eq!(id(&inverted), mask_id);
    assert_eq!(
        inverted.layers[0].mask.as_ref().unwrap().coverage.at(0, 0),
        255
    );
    assert!(
        !inverted.layers[0].mask.as_ref().unwrap().enabled
            && !inverted.layers[0].mask.as_ref().unwrap().linked
    );
    let mut replacement = original.clone();
    replacement.layers[0].mask.as_mut().unwrap().id = LayerId::new();
    let frozen = replacement.clone();
    assert!(!crate::document::same_snapshot(&replacement, &original));
    assert!(
        command(&replacement, Command::Invert(mask_id))
            .prepare(&replacement)
            .is_err()
    );
    assert!(crate::document::same_snapshot(&replacement, &frozen));
    let mut duplicate = original.clone();
    duplicate.duplicate_active().unwrap();
    assert_ne!(id(&duplicate), mask_id);
    assert!(
        duplicate.layers[1]
            .mask
            .as_ref()
            .unwrap()
            .coverage
            .shares(&original.layers[0].mask.as_ref().unwrap().coverage)
    );
    let reopened: Document = serde_json::from_value(json(&original)).unwrap();
    assert_ne!(id(&reopened), mask_id);
    assert_eq!(json(&reopened), json(&original));
}
#[test]
fn apply_mask_bakes_alpha_once_preserves_hidden_rgb_and_ignores_the_selection() {
    let mut source = Document::new(4, 1);
    source.layers[0].pixels = vec![
        11, 23, 37, 255, 41, 53, 67, 129, 71, 83, 97, 0, 101, 113, 127, 128,
    ]
    .into();
    source.version = 2;
    let slot = LayerId::new();
    source.layers[0].mask = Some(LayerMask {
        id: slot,
        enabled: true,
        linked: true,
        coverage: Mask::from_coverage(4, 1, &[0, 128, 255, 255], None).unwrap(),
    });
    source.selection = Some(crate::selection::Selection {
        left: 1,
        top: 0,
        right: 2,
        bottom: 1,
    });
    let expected_selection = source.selection;
    let result = command(&source, Command::Apply(slot))
        .prepare(&source)
        .unwrap()
        .0;
    assert!(result.layers[0].mask.is_none());
    assert_eq!(result.selection, expected_selection);
    assert_eq!(
        result.layers[0].pixels.as_slice(),
        &[
            11, 23, 37, 0, 41, 53, 67, 65, 71, 83, 97, 0, 101, 113, 127, 128
        ]
    );
    let mut disabled = source.clone();
    disabled.layers[0].mask.as_mut().unwrap().enabled = false;
    disabled.layers[0].alpha_locked = true;
    let applied = command(&disabled, Command::Apply(slot))
        .prepare(&disabled)
        .unwrap()
        .0;
    assert!(applied.layers[0].pixels.shares(&disabled.layers[0].pixels));
    assert!(applied.layers[0].mask.is_none());
    let mut locked = source.clone();
    locked.layers[0].alpha_locked = true;
    assert!(
        command(&locked, Command::Apply(slot))
            .prepare(&locked)
            .is_err()
    );
}
#[test]
fn queued_mask_commands_keep_one_undo_each_and_preserve_redo_on_noop() {
    let mut app = crate::Dudel {
        doc: Document::new(4, 2),
        ..Default::default()
    };
    let original = json(&app.doc);
    let slot = LayerId::new();
    let operations = [
        Command::Create {
            id: slot,
            seed: Seed::HideAll,
        },
        Command::Invert(slot),
        Command::Enabled {
            id: slot,
            value: false,
        },
        Command::Linked {
            id: slot,
            value: false,
        },
        Command::Remove(slot),
    ];
    let mut expected = app.doc.clone();
    let mut states = vec![original];
    for operation in operations {
        let edit = command(&expected, operation);
        expected = edit.prepare(&expected).unwrap().0;
        states.push(json(&expected));
        app.render_queue.edit(edit).unwrap();
    }
    crate::workflow_tests::finish_jobs(&mut app);
    assert_eq!(json(&app.doc), *states.last().unwrap());
    for expected in states[..states.len() - 1].iter().rev() {
        app.undo();
        crate::workflow_tests::finish_jobs(&mut app);
        assert_eq!(&json(&app.doc), expected);
    }
    assert!(!app.history.can_undo());
    for expected in &states[1..] {
        app.redo();
        crate::workflow_tests::finish_jobs(&mut app);
        assert_eq!(&json(&app.doc), expected);
    }
    app.undo();
    crate::workflow_tests::finish_jobs(&mut app);
    assert!(app.history.can_redo());
    let revision = app.revision;
    let edit = command(
        &app.doc,
        Command::Enabled {
            id: slot,
            value: false,
        },
    );
    app.render_queue.edit(edit).unwrap();
    crate::workflow_tests::finish_jobs(&mut app);
    assert_eq!(app.revision, revision);
    assert!(app.history.can_redo());
}
#[test]
fn canceled_or_locked_mask_commands_never_mutate_source_and_folders_require_merge_before_apply() {
    let source = create(&Document::new(2, 2), Seed::HideAll);
    let slot = id(&source);
    let cancel = std::sync::atomic::AtomicBool::new(true);
    for operation in [
        Command::Invert(slot),
        Command::Apply(slot),
        Command::Remove(slot),
        Command::Enabled {
            id: slot,
            value: false,
        },
    ] {
        let mut actual = source.clone();
        assert!(crate::layer_mask_edits::apply(&mut actual, 0, operation, Some(&cancel)).is_err());
        assert!(crate::document::same_snapshot(&actual, &source));
    }
    let mut locked = source.clone();
    locked.layers[0].locked = true;
    assert!(
        command(&locked, Command::Remove(slot))
            .prepare(&locked)
            .is_err()
    );
    let mut folder = Document::new(2, 2);
    folder.group_active().unwrap();
    let mut folder = create(&folder, Seed::HideAll);
    let frozen = folder.clone();
    assert!(folder.ungroup_active().is_err());
    assert!(crate::document::same_snapshot(&folder, &frozen));
    assert!(
        command(&folder, Command::Apply(id(&folder)))
            .prepare(&folder)
            .is_err()
    );
    folder.merge_folder().unwrap();
    assert!(folder.layers[folder.active].mask.is_some());
    let result = command(&folder, Command::Apply(id(&folder)))
        .prepare(&folder)
        .unwrap()
        .0;
    assert!(result.layers[result.active].mask.is_none());
}
#[test]
fn merge_down_bakes_both_enabled_masks_once_and_disabled_masks_do_not_contribute() {
    for clipped in [false, true] {
        for enabled in [false, true] {
            let mut doc = create(&Document::new(1, 1), Seed::HideAll);
            doc.layers[0].mask.as_mut().unwrap().coverage = Mask::new(1, 1, 128).unwrap();
            doc.layers[0].pixels = vec![200, 30, 70, 255].into();
            doc.add_layer();
            doc.layers[1].pixels = vec![20, 100, 220, 128].into();
            doc.layers[1].clipped = clipped;
            doc = create(&doc, Seed::HideAll);
            doc.layers[1].mask.as_mut().unwrap().coverage = Mask::new(1, 1, 64).unwrap();
            doc.layers[1].mask.as_mut().unwrap().enabled = enabled;
            let expected = doc.composite_transparent();
            let source = doc.clone();
            let mut history = crate::document::History::new();
            history.checkpoint(&doc);
            doc.merge_down().unwrap();
            history.settle(&doc);
            assert!(doc.layers[0].mask.is_none());
            assert_eq!(doc.composite_transparent(), expected);
            assert!(history.undo(&mut doc));
            assert_eq!(json(&doc), json(&source));
        }
    }
}
#[test]
fn merge_folder_keeps_its_mask_and_bakes_child_masks_without_compounding_root_coverage() {
    let mut doc = create(&Document::new(1, 1), Seed::RevealAll);
    doc.layers[0].pixels = vec![200, 30, 70, 255].into();
    doc.layers[0].mask.as_mut().unwrap().coverage = Mask::new(1, 1, 128).unwrap();
    doc.group_active().unwrap();
    doc = create(&doc, Seed::HideAll);
    doc.layers[doc.active].mask.as_mut().unwrap().coverage = Mask::new(1, 1, 128).unwrap();
    let source = doc.clone();
    let expected = doc.composite_transparent();
    let root_id = id(&doc);
    doc.merge_folder().unwrap();
    assert_eq!(id(&doc), root_id);
    assert!(!doc.layers[doc.active].folder);
    assert_eq!(doc.composite_transparent(), expected);
    let applied = command(&doc, Command::Apply(root_id))
        .prepare(&doc)
        .unwrap()
        .0;
    assert!(applied.layers[0].mask.is_none());
    assert_eq!(applied.composite_transparent(), expected);
    assert!(source.layers[0].mask.is_some());
}
