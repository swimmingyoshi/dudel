//! Validated, portable shortcut overrides. Missing entries retain defaults;
//! an empty entry explicitly unbinds a command.
use crate::{
    commands::{Action, COMMANDS},
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
        let mut used: Vec<(Chord, &str)> = Vec::new();
        for command in COMMANDS {
            for chord in self.effective(command.action) {
                if let Some(reason) = chord.reserved(command.action) {
                    return Err(format!("{}: {reason}", chord.label()));
                }
                if let Some((_, owner)) = used.iter().find(|(other, _)| *other == chord) {
                    return Err(format!("{} is already assigned to {owner}", chord.label()));
                }
                used.push((chord, command.label));
            }
        }
        Ok(())
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
