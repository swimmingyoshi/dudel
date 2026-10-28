use crate::{Dudel, brush::Brush};

fn replace(library: &mut [Brush], index: usize, brush: &Brush) -> Result<(), String> {
    brush.validate()?;
    let preset = library.get_mut(index).ok_or("Preset no longer exists")?;
    *preset = brush.clone();
    Ok(())
}

fn remove(library: &mut Vec<Brush>, index: usize) -> Result<usize, String> {
    if library.len() <= 1 {
        return Err("Keep at least one brush in the library".into());
    }
    if index >= library.len() {
        return Err("Preset no longer exists".into());
    }
    library.remove(index);
    Ok(index.min(library.len() - 1))
}

impl Dudel {
    pub(crate) fn brush_preset_modified(&self) -> bool {
        self.brushes.get(self.selected_brush) != Some(self.paint_brush().as_ref())
    }
    pub(crate) fn revert_brush_preset(&mut self) {
        if let Some(saved) = self.brushes.get(self.selected_brush).cloned() {
            self.unswap_eraser();
            self.brush = saved;
            self.sync_eraser_slot();
            self.scratchpad.reset_stroke();
            self.status = "Brush settings restored".into();
        }
    }
    pub(crate) fn toggle_brush_favorite(&mut self, index: usize) {
        if let Some(brush) = self.brushes.get_mut(index) {
            brush.favorite = !brush.favorite;
            let favorite = brush.favorite;
            if index == self.selected_brush {
                self.unswap_eraser();
                self.brush.favorite = favorite;
                self.sync_eraser_slot();
            }
        }
    }
    pub(crate) fn save_new_brush_preset(&mut self) {
        let index = self.brushes.len();
        let brush = self.paint_brush().into_owned();
        match insert_copy(&mut self.brushes, brush, index, false) {
            Ok(index) => self.finish_new_brush(index, "New brush preset saved"),
            Err(error) => self.status = error,
        }
    }
    pub(crate) fn duplicate_brush_preset(&mut self, index: usize) {
        let Some(brush) = self.brushes.get(index).cloned() else {
            self.status = "Preset no longer exists".into();
            return;
        };
        match insert_copy(&mut self.brushes, brush, index + 1, true) {
            Ok(index) => self.finish_new_brush(index, "Brush preset duplicated"),
            Err(error) => self.status = error,
        }
    }
    fn finish_new_brush(&mut self, index: usize, status: &str) {
        self.choose_brush(index);
        self.brush_search.clear();
        self.brush_favorites_only = false;
        self.scratchpad.reset_stroke();
        self.status = format!("{status}: {}", self.brush.name);
    }
    pub(crate) fn update_brush_preset(&mut self) {
        if let Err(error) = self.brush.validate() {
            self.status = error;
            return;
        }
        let name = &self.brushes[self.selected_brush].name;
        let text = format!(
            "Replace the saved settings for ‘{name}’ with the current brush? This also updates exported packs."
        );
        self.confirm(
            "Update brush preset",
            text,
            "Update",
            Self::replace_brush_preset,
        );
    }
    fn replace_brush_preset(&mut self) {
        // The library's star belongs to the entry, not the edited settings.
        self.unswap_eraser();
        let mut brush = self.brush.clone();
        brush.favorite = self.brushes[self.selected_brush].favorite;
        self.brush.favorite = brush.favorite;
        self.status = match replace(&mut self.brushes, self.selected_brush, &brush) {
            Ok(()) => "Preset updated".into(),
            Err(error) => error,
        };
        self.brush_search.clear();
        self.sync_eraser_slot();
    }

    pub(crate) fn delete_brush_preset(&mut self) {
        if self.brushes.len() <= 1 {
            return;
        }
        let name = &self.brushes[self.selected_brush].name;
        let text = format!(
            "Delete ‘{name}’ from your library? Export the library first if you want a backup. Artwork is unaffected."
        );
        self.confirm(
            "Delete brush preset",
            text,
            "Delete",
            Self::remove_brush_preset,
        );
    }
    fn remove_brush_preset(&mut self) {
        match remove(&mut self.brushes, self.selected_brush) {
            Ok(index) => {
                self.choose_brush(index);
                self.brush_search.clear();
                self.scratchpad.reset_stroke();
                self.status = "Preset deleted".into();
            }
            Err(error) => self.status = error,
        }
    }
}

fn insert_copy(
    library: &mut Vec<Brush>,
    mut brush: Brush,
    index: usize,
    force_copy: bool,
) -> Result<usize, String> {
    brush.validate()?;
    if library.len() >= 512 {
        return Err("Brush library is full (512 presets)".into());
    }
    if index > library.len() {
        return Err("Preset no longer exists".into());
    }
    let names: std::collections::HashSet<_> =
        library.iter().map(|b| b.name.to_lowercase()).collect();
    let taken = |name: &str| names.contains(&name.to_lowercase());
    if force_copy || taken(&brush.name) {
        let original = brush.name.clone();
        for number in 1..=library.len() + 1 {
            let suffix = if number == 1 {
                " copy".to_owned()
            } else {
                format!(" copy {number}")
            };
            let mut length = original.len().min(200 - suffix.len());
            while !original.is_char_boundary(length) {
                length -= 1;
            }
            let name = format!("{}{suffix}", original[..length].trim_end());
            if !taken(&name) {
                brush.name = name;
                break;
            }
        }
    }
    brush.validate()?;
    library.insert(index, brush);
    Ok(index)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn deleting_a_preset_while_erasing_preserves_eraser_and_restores_the_replacement_pen() {
        for own in [false, true] {
            let mut app = Dudel::default();
            app.choose_brush(0);
            app.set_eraser_own(own);
            app.select_tool(crate::tools::Tool::Eraser);
            app.brush.size = 83.0;
            app.brush.opacity = 0.37;
            let eraser = app.eraser_tip().into_owned();
            let remaining = app.brushes[1].clone();
            app.remove_brush_preset();
            assert!(app.tool == crate::tools::Tool::Eraser);
            assert_eq!(app.brush.size, 83.0);
            assert!(app.paint_brush().as_ref() == &remaining);
            assert!(app.eraser_tip().as_ref() == &eraser);
            assert_eq!(app.brush_group, remaining.group());
            assert!(!app.brush_preset_modified());
            let saved = crate::preferences::Preferences::decode(&app.encode_preferences().unwrap())
                .unwrap();
            assert!(saved.current_brush == remaining);
            assert!(saved.tools.eraser.as_ref() == Some(&eraser));
            app.select_tool(crate::tools::Tool::Brush);
            assert!(app.brush == remaining);
            app.select_tool(crate::tools::Tool::Eraser);
            assert_eq!(app.brush.size, 83.0);
        }
    }

    #[test]
    fn favoriting_the_paint_preset_while_erasing_does_not_edit_the_eraser_tip() {
        let mut app = Dudel::default();
        app.choose_brush(0);
        app.set_eraser_own(true);
        app.select_tool(crate::tools::Tool::Eraser);
        let eraser = app.eraser_tip().into_owned();
        let favorite = app.paint_brush().favorite;
        app.toggle_brush_favorite(0);
        assert_eq!(app.paint_brush().favorite, !favorite);
        assert!(!app.brush_preset_modified());
        assert!(app.eraser_tip().as_ref() == &eraser);
        app.select_tool(crate::tools::Tool::Brush);
        assert_eq!(app.brush.favorite, !favorite);
    }

    #[test]
    fn duplicate_names_remain_valid_unique_and_utf8_safe_at_the_name_limit() {
        for name in ["A".repeat(200), "é".repeat(100), "🖌".repeat(50)] {
            let mut app = Dudel {
                brushes: vec![Brush {
                    name,
                    ..Default::default()
                }],
                ..Default::default()
            };
            app.choose_brush(0);
            let original = app.brush.clone();
            app.duplicate_brush_preset(0);
            app.duplicate_brush_preset(0);
            assert_eq!(app.brushes.len(), 3);
            assert!(app.brushes[0] == original);
            let names: std::collections::HashSet<_> =
                app.brushes.iter().map(|b| b.name.to_lowercase()).collect();
            assert_eq!(names.len(), 3);
            assert!(
                app.brushes
                    .iter()
                    .all(|b| b.name.len() <= 200 && b.validate().is_ok())
            );
            assert!(!app.brush_preset_modified());
        }
        let mut library = vec![Brush {
            name: "INK".into(),
            ..Default::default()
        }];
        insert_copy(
            &mut library,
            Brush {
                name: "ink".into(),
                ..Default::default()
            },
            1,
            false,
        )
        .unwrap();
        assert_ne!(
            library[0].name.to_lowercase(),
            library[1].name.to_lowercase()
        );
    }
    #[test]
    fn stabilization_changes_are_detected_independently_of_preview_pixels_and_stars() {
        let mut app = Dudel::default();
        app.choose_brush(0);
        let preview = crate::brush_previews::fingerprint(&app.brush);
        app.brush.stabilization = 12.0;
        assert_eq!(preview, crate::brush_previews::fingerprint(&app.brush));
        assert!(app.brush_preset_modified());
        app.revert_brush_preset();
        assert!(!app.brush_preset_modified());
        app.brush.stabilization_space = crate::brush::StabilizationSpace::Screen;
        assert!(app.brush_preset_modified());
        app.revert_brush_preset();
        app.brush.stabilization_by_speed = !app.brush.stabilization_by_speed;
        assert!(app.brush_preset_modified());
        app.revert_brush_preset();
        app.toggle_brush_favorite(0);
        assert!(!app.brush_preset_modified());
        assert!(app.brush.favorite == app.brushes[0].favorite);
    }
    #[test]
    fn invalid_or_full_library_creation_preserves_selection_entries_and_artwork() {
        let mut app = Dudel::default();
        let doc = app.doc.clone();
        let library = app.brushes.clone();
        let selected = app.selected_brush;
        app.brush.name.clear();
        app.save_new_brush_preset();
        assert!(app.brushes == library && app.selected_brush == selected);
        app.brushes = vec![Brush::default(); 512];
        app.choose_brush(0);
        app.save_new_brush_preset();
        app.duplicate_brush_preset(0);
        assert_eq!(app.brushes.len(), 512);
        assert_eq!(app.selected_brush, 0);
        assert!(crate::document::same_snapshot(&doc, &app.doc));
        assert!(!app.modified && !app.history.can_undo());
    }
    #[test]
    fn created_ink_pencil_and_image_presets_survive_preferences_and_pack_files_with_matching_pixels()
     {
        #[derive(Default)]
        struct MemoryStorage(std::collections::HashMap<String, String>);
        impl eframe::Storage for MemoryStorage {
            fn get_string(&self, key: &str) -> Option<String> {
                self.0.get(key).cloned()
            }
            fn set_string(&mut self, key: &str, value: String) {
                self.0.insert(key.into(), value);
            }
            fn flush(&mut self) {}
        }
        let mut app = Dudel::default();
        let original = app.brushes.clone();
        let document = app.doc.clone();
        let image = crate::brush::TipImage {
            width: 2,
            height: 2,
            alpha: (vec![255, 0, 128, 64]).into(),
        };
        let prototypes = vec![
            Brush {
                name: "QA ink".into(),
                size: 8.0,
                stabilization: 12.0,
                stabilization_space: crate::brush::StabilizationSpace::Screen,
                stabilization_by_speed: true,
                ..Default::default()
            },
            Brush {
                name: "QA pencil".into(),
                size: 12.0,
                opacity: 0.6,
                grain: Some(image.clone()),
                grain_amount: 0.45,
                grain_scale: 8.0,
                scatter_seed: 735,
                ..Default::default()
            },
            Brush {
                name: "QA stamp".into(),
                size: 20.0,
                tip: crate::brush::Tip::Image,
                image: Some(image),
                angle_source: crate::brush::AngleSource::Direction,
                angle_jitter: 12.0,
                ..Default::default()
            },
        ];
        for prototype in &prototypes {
            app.brush = prototype.clone();
            app.save_new_brush_preset();
            assert!(app.brush == *prototype && !app.brush_preset_modified());
        }
        assert!(app.brushes[..original.len()] == original);
        assert!(crate::document::same_snapshot(&document, &app.doc));
        assert!(!app.modified && !app.history.can_undo());
        let mut storage = MemoryStorage::default();
        eframe::App::save(&mut app, &mut storage);
        let directory = tempfile::tempdir().unwrap();
        let preferences_path = directory.path().join("preferences.json");
        std::fs::write(&preferences_path, &storage.0[crate::preferences::KEY]).unwrap();
        let preferences = crate::preferences::Preferences::decode(
            &std::fs::read_to_string(preferences_path).unwrap(),
        )
        .unwrap();
        let mut restored = Dudel::default();
        restored.apply_preferences(preferences);
        assert!(restored.brushes == app.brushes && restored.brush == app.brush);
        let pack = crate::brush_pack::BrushPack::new(prototypes.clone());
        pack.validate().unwrap();
        let pack_path = directory.path().join("qa.dudelbrushes");
        crate::storage::atomic_write(&pack_path, &serde_json::to_vec(&pack).unwrap()).unwrap();
        let imported: crate::brush_pack::BrushPack =
            serde_json::from_str(&crate::read_bounded(&pack_path, 128 * 1024 * 1024).unwrap())
                .unwrap();
        let mut fresh = Dudel::default();
        let first = imported.append_to(&mut fresh.brushes).unwrap();
        for (i, prototype) in prototypes.iter().enumerate() {
            restored.choose_brush(original.len() + i);
            fresh.choose_brush(first + i);
            assert!(restored.brush == *prototype && fresh.brush == *prototype);
            let expected = crate::brush_studio::render(prototype, [200, 72]);
            assert_eq!(
                crate::brush_studio::render(&restored.brush, [200, 72]),
                expected
            );
            assert_eq!(
                crate::brush_studio::render(&fresh.brush, [200, 72]),
                expected
            );
        }
        // Legacy files can contain a stale star on current_brush. Restore its
        // selected entry's metadata while retaining all edited paint settings.
        let mut value: serde_json::Value =
            serde_json::from_str(&storage.0[crate::preferences::KEY]).unwrap();
        value["current_brush"]["favorite"] = serde_json::json!(!app.brush.favorite);
        let normalized = crate::preferences::Preferences::decode(&value.to_string()).unwrap();
        assert_eq!(
            normalized.current_brush.favorite,
            normalized.brushes[normalized.selected_brush].favorite
        );
    }
    #[test]
    fn invalid_update_is_atomic_and_removal_keeps_valid_selection() {
        let mut library = vec![
            Brush::default(),
            Brush {
                name: "Pencil".into(),
                ..Brush::default()
            },
        ];
        let invalid = Brush {
            size: f32::NAN,
            ..Brush::default()
        };
        assert!(replace(&mut library, 0, &invalid).is_err());
        assert_eq!(library[0].size, 18.0);
        let revised = Brush {
            name: "New ink".into(),
            size: 25.0,
            ..Brush::default()
        };
        replace(&mut library, 0, &revised).unwrap();
        assert_eq!(library[0].name, "New ink");
        assert_eq!(remove(&mut library, 1).unwrap(), 0);
        assert!(remove(&mut library, 0).is_err());
        assert_eq!(library.len(), 1);
    }
}
