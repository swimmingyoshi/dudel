//! Validated, portable shortcut overrides. Missing entries retain defaults;
//! an empty entry explicitly unbinds a command.
use crate::{
    commands::{Action, COMMANDS, Command},
    egui,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Chord {
    pub key: egui::Key,
    #[serde(default)]
    pub command: bool,
    #[serde(default)]
    pub shift: bool,
    #[serde(default)]
    pub alt: bool,
}

impl Chord {
    pub fn new(key: egui::Key, modifiers: egui::Modifiers) -> Self {
        Self {
            key,
            command: modifiers.command || modifiers.ctrl,
            shift: modifiers.shift,
            alt: modifiers.alt,
        }
    }
    pub fn label(self) -> String {
        format!(
            "{}{}{}{}",
            if self.command { "Ctrl+" } else { "" },
            if self.shift { "Shift+" } else { "" },
            if self.alt { "Alt+" } else { "" },
            self.key.symbol_or_name()
        )
    }
    pub fn matches(self, key: egui::Key, modifiers: egui::Modifiers) -> bool {
        self == Self::new(key, modifiers)
    }
    fn parse(value: &str) -> Self {
        let parts: Vec<_> = value.split('+').collect();
        Self {
            key: egui::Key::from_name(parts.last().unwrap()).expect("valid default key"),
            command: parts.contains(&"Ctrl"),
            shift: parts.contains(&"Shift"),
            alt: parts.contains(&"Alt"),
        }
    }
    fn reserved(self, action: Action) -> Option<&'static str> {
        use egui::Key;
        if self.alt {
            return Some("Alt is reserved for color picking and canvas gestures");
        }
        if matches!(
            self.key,
            Key::Escape
                | Key::Enter
                | Key::Space
                | Key::ArrowUp
                | Key::ArrowDown
                | Key::ArrowLeft
                | Key::ArrowRight
                | Key::Backspace
                | Key::Delete
                | Key::F6
                | Key::F11
        ) {
            return Some("This key is reserved for navigation, editing or window controls");
        }
        if self.key == Key::Tab && !(action == Action::Panels && !self.command && !self.shift) {
            return Some("Tab is reserved for panel, control and project navigation");
        }
        if self.key == Key::R && !self.command {
            return Some("R is reserved for rotating the canvas view while held");
        }
        if self.key == Key::F && !self.command {
            return Some("F is reserved for resizing the brush or reshape reach");
        }
        if self.command && matches!(self.key, Key::C | Key::X | Key::V | Key::Insert)
            || self.shift && self.key == Key::Insert
        {
            return Some("Clipboard shortcuts remain available to artwork and text fields");
        }
        None
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub(crate) struct Bindings(pub BTreeMap<String, Vec<Chord>>);

impl Bindings {
    pub fn effective(&self, action: Action) -> Vec<Chord> {
        let entry = crate::commands::get(action);
        self.0.get(entry.id).cloned().unwrap_or_else(|| {
            entry
                .defaults
                .split(';')
                .filter(|s| !s.is_empty())
                .map(Chord::parse)
                .collect()
        })
    }
    pub fn label(&self, action: Action) -> String {
        self.effective(action)
            .iter()
            .map(|c| c.label())
            .collect::<Vec<_>>()
            .join(" / ")
    }
    pub fn event_matches(&self, action: Action, event: &egui::Event) -> bool {
        matches!(event, egui::Event::Key { key, modifiers, pressed: true, repeat, .. }
            if (!repeat || matches!(action, Action::Undo | Action::Redo | Action::SmallerBrush | Action::LargerBrush | Action::ZoomIn | Action::ZoomOut | Action::RotateLeft | Action::RotateRight))
            && self.effective(action).iter().any(|c| c.matches(*key, *modifiers)))
    }
    pub fn consume(&self, ctx: &egui::Context, action: Action) -> bool {
        ctx.input_mut(|input| {
            let mut found = false;
            input.events.retain(|event| {
                // Real panel Tab presses are handled in raw_input_hook before
                // focus traversal. Leave the synthetic Tab from F6 to egui.
                if action == Action::Panels
                    && matches!(
                        event,
                        egui::Event::Key {
                            key: egui::Key::Tab,
                            ..
                        }
                    )
                {
                    return true;
                }
                let matches = self.event_matches(action, event);
                found |= matches;
                !matches
            });
            found
        })
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.0.len() > COMMANDS.len() {
            return Err("Too many shortcut entries".into());
        }
        for (id, bindings) in &self.0 {
            if !COMMANDS.iter().any(|c| c.id == id) {
                return Err(format!("Unknown command: {id}"));
            }
            if bindings.len() > 3 {
                return Err("Use at most three shortcuts per command".into());
            }
        }
        self.problem().map_or(Ok(()), |(reason, _)| Err(reason))
    }
    /// The first reserved or doubly assigned key, and the saved entry behind it.
    fn problem(&self) -> Option<(String, Option<&'static str>)> {
        let saved = |command: &Command| self.0.contains_key(command.id).then_some(command.id);
        let mut used: Vec<(Chord, &Command)> = Vec::new();
        for command in COMMANDS {
            for chord in self.effective(command.action) {
                if let Some(reason) = chord.reserved(command.action) {
                    return Some((format!("{}: {reason}", chord.label()), saved(command)));
                }
                if let Some((_, owner)) = used.iter().find(|(other, _)| *other == chord) {
                    let reason =
                        format!("{} is already assigned to {}", chord.label(), owner.label);
                    return Some((reason, saved(command).or(saved(owner))));
                }
                used.push((chord, command));
            }
        }
        None
    }
    /// Put saved entries that no longer validate back to their defaults, so
    /// that a new default taking a key costs one shortcut and not the settings.
    /// Returns how many went.
    pub fn repair(&mut self) -> usize {
        let before = self.0.len();
        self.0
            .retain(|id, chords| chords.len() <= 3 && COMMANDS.iter().any(|c| c.id == id));
        while let Some((_, Some(id))) = self.problem() {
            self.0.remove(id);
        }
        before - self.0.len()
    }
    pub fn changed(&self, action: Action, chords: Option<Vec<Chord>>) -> Result<Self, String> {
        let mut next = self.clone();
        let id = crate::commands::get(action).id;
        if let Some(chords) = chords {
            next.0.insert(id.into(), chords);
        } else {
            next.0.remove(id);
        }
        next.validate()?;
        Ok(next)
    }
    pub fn export(&self) -> Result<Vec<u8>, String> {
        self.validate()?;
        // Full effective bindings make profiles independent of later default changes.
        let bindings = Self(
            COMMANDS
                .iter()
                .map(|c| (c.id.into(), self.effective(c.action)))
                .collect(),
        );
        serde_json::to_vec_pretty(&Profile {
            version: 1,
            bindings,
        })
        .map_err(|e| e.to_string())
    }
    pub fn import(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() > MAX_PROFILE_BYTES {
            return Err("Shortcut profile exceeds 64 KiB".into());
        }
        let profile: Profile =
            serde_json::from_slice(bytes).map_err(|e| format!("Invalid shortcut profile: {e}"))?;
        if profile.version != 1 {
            return Err("Unsupported shortcut profile version".into());
        }
        profile.bindings.validate()?;
        Ok(profile.bindings)
    }
}

/// Saved bindings, leaving out entries this build cannot read.
pub(crate) fn lenient<'de, D: serde::Deserializer<'de>>(source: D) -> Result<Bindings, D::Error> {
    let serde_json::Value::Object(saved) = serde_json::Value::deserialize(source)? else {
        return Ok(Bindings::default());
    };
    let readable = saved
        .into_iter()
        .filter_map(|(id, chords)| Some((id, serde_json::from_value(chords).ok()?)));
    Ok(Bindings(readable.collect()))
}

pub(crate) const MAX_PROFILE_BYTES: usize = 64 * 1024;
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Profile {
    version: u32,
    bindings: Bindings,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Dudel, tools::Tool};

    #[test]
    fn shortcut_defaults_and_profiles_validate_and_roundtrip() {
        let defaults = Bindings::default();
        defaults.validate().unwrap();
        let ids: std::collections::HashSet<_> = COMMANDS.iter().map(|c| c.id).collect();
        assert_eq!(ids.len(), COMMANDS.len());
        let custom = defaults
            .changed(Action::Tool(Tool::Eraser), Some(vec![]))
            .unwrap()
            .changed(
                Action::NewLayer,
                Some(vec![Chord::new(egui::Key::E, egui::Modifiers::NONE)]),
            )
            .unwrap();
        let restored = Bindings::import(&custom.export().unwrap()).unwrap();
        for command in COMMANDS {
            assert_eq!(
                custom.effective(command.action),
                restored.effective(command.action)
            );
        }
        assert!(restored.effective(Action::Tool(Tool::Eraser)).is_empty());
        assert!(
            custom
                .changed(Action::Tool(Tool::Eraser), None)
                .unwrap_err()
                .contains("already assigned")
        );
    }

    #[test]
    fn shortcut_conflicts_include_aliases_and_fixed_gestures() {
        let bindings = Bindings::default();
        for chord in [
            Chord::parse("Ctrl+Z"),
            Chord::parse("Ctrl+Y"),
            Chord::parse("Ctrl+Shift+P"),
        ] {
            assert!(
                bindings
                    .changed(Action::NewLayer, Some(vec![chord]))
                    .unwrap_err()
                    .contains("already assigned")
            );
        }
        for key in [
            "Escape", "Enter", "Tab", "F6", "Alt+Q", "Ctrl+V", "R", "F", "Shift+F",
        ] {
            assert!(
                bindings
                    .changed(Action::NewLayer, Some(vec![Chord::parse(key)]))
                    .is_err(),
                "{key}"
            );
        }
        let custom = bindings
            .changed(Action::NewLayer, Some(vec![Chord::parse("Ctrl+Shift+L")]))
            .unwrap();
        assert!(
            !custom
                .effective(Action::NewLayer)
                .iter()
                .any(|c| c.matches(egui::Key::L, egui::Modifiers::CTRL))
        );
        assert!(
            custom.effective(Action::NewLayer)[0]
                .matches(egui::Key::L, egui::Modifiers::CTRL | egui::Modifiers::SHIFT)
        );
    }

    #[test]
    fn a_bad_saved_shortcut_keeps_the_other_settings_and_shortcuts() {
        let app = Dudel {
            shortcuts: Bindings::default()
                .changed(Action::NewFolder, Some(vec![Chord::parse("Ctrl+Shift+L")]))
                .unwrap(),
            ..Default::default()
        };
        let source = app.encode_preferences().unwrap();
        let mut saved: serde_json::Value = serde_json::from_str(&source).unwrap();
        let shortcuts = saved["shortcuts"].as_object_mut().unwrap();
        // A key another command now has by default, one that is reserved, a
        // command this build lacks and a key it cannot read.
        for (id, chords) in [
            ("layer.effects", r#"[{"key":"Z","command":true}]"#),
            ("app.settings", r#"[{"key":"F"}]"#),
            ("tool.future", r#"[{"key":"Q"}]"#),
            ("brush.editor", r#"[{"key":"NoSuchKey"}]"#),
        ] {
            shortcuts.insert(id.into(), serde_json::from_str(chords).unwrap());
        }
        let p = crate::preferences::Preferences::decode(&saved.to_string()).unwrap();
        assert_eq!(p.shortcuts_dropped, 3);
        assert_eq!(p.shortcuts, app.shortcuts);
        p.shortcuts.validate().unwrap();
        // Two saved entries that clash lose one, and the result is valid.
        let chord = Chord::parse("Ctrl+Shift+L");
        let mut clash = Bindings::default();
        clash.0.insert("layer.new_vector".into(), vec![chord]);
        clash.0.insert("layer.new_folder".into(), vec![chord]);
        assert_eq!(clash.repair(), 1);
        clash.validate().unwrap();
        // Swapped keys are valid together and stay.
        let mut swapped = Bindings::default();
        swapped.0.insert("tool.eraser".into(), vec![]);
        swapped
            .0
            .insert("layer.new".into(), vec![Chord::parse("E")]);
        assert_eq!(swapped.repair(), 0);
    }

    #[test]
    fn shortcut_import_is_bounded_atomic_and_preferences_remain_backward_compatible() {
        let mut app = Dudel::default();
        app.shortcuts = app
            .shortcuts
            .changed(Action::NewLayer, Some(vec![Chord::parse("Ctrl+Shift+L")]))
            .unwrap();
        let original = app.shortcuts.clone();
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("profile.dudelkeys");
        for bytes in [
            b"not json".to_vec(),
            vec![b' '; MAX_PROFILE_BYTES + 1],
            br#"{"version":2,"bindings":{}}"#.to_vec(),
            br#"{"version":1,"bindings":{"unknown.command":[]}}"#.to_vec(),
            br#"{"version":1,"bindings":{"file.new":[{"key":"Z","command":true}]}}"#.to_vec(),
        ] {
            std::fs::write(&path, bytes).unwrap();
            assert!(app.import_shortcut_profile(&path).is_err());
            assert_eq!(app.shortcuts, original);
        }
        std::fs::write(&path, Bindings::default().export().unwrap()).unwrap();
        app.import_shortcut_profile(&path).unwrap();
        assert_eq!(
            app.shortcuts.label(Action::NewLayer),
            Bindings::default().label(Action::NewLayer)
        );
        app.shortcuts = original.clone();
        let source = app.encode_preferences().unwrap();
        assert_eq!(
            crate::preferences::Preferences::decode(&source)
                .unwrap()
                .shortcuts,
            original
        );
        let mut legacy: serde_json::Value = serde_json::from_str(&source).unwrap();
        legacy.as_object_mut().unwrap().remove("shortcuts");
        assert_eq!(
            crate::preferences::Preferences::decode(&legacy.to_string())
                .unwrap()
                .shortcuts,
            Bindings::default()
        );
    }
}
