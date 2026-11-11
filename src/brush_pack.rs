use crate::{Dudel, brush::Brush, storage};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
pub struct BrushPack {
    pub format: String,
    pub version: u32,
    pub brushes: Vec<Brush>,
    /// Who made the brushes and how they may be used. Older Dudel ignores it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub info: Option<PackInfo>,
}

/// A shared brush's details, shown before it is imported.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PackInfo {
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub license: String,
    #[serde(default)]
    pub description: String,
}
impl PackInfo {
    pub fn validate(&self) -> Result<(), String> {
        let line = |s: &str| s.chars().count() <= 100 && !s.chars().any(char::is_control);
        if !line(&self.author)
            || !line(&self.license)
            || self.description.chars().count() > 1000
            || self
                .description
                .chars()
                .any(|c| c.is_control() && c != '\n')
        {
            return Err(
                "The author, license or description is too long or has control characters".into(),
            );
        }
        Ok(())
    }
    fn is_empty(&self) -> bool {
        [&self.author, &self.license, &self.description]
            .iter()
            .all(|s| s.trim().is_empty())
    }
}
impl BrushPack {
    /// Packs holding a brush graph are version 3, which older Dudel refuses
    /// instead of importing the brushes without their graphs.
    pub fn new(brushes: Vec<Brush>) -> Self {
        Self {
            format: "dudel-brush-pack".into(),
            version: if brushes.iter().any(|b| b.graph.is_some()) {
                3
            } else {
                2
            },
            brushes,
            info: None,
        }
    }
    /// One brush with its details: the contents of a `.dudelbrush` file.
    pub fn single(brush: Brush, info: PackInfo) -> Self {
        let mut pack = Self::new(vec![brush]);
        pack.info = (!info.is_empty()).then(|| PackInfo {
            author: info.author.trim().into(),
            license: info.license.trim().into(),
            description: info.description.trim().into(),
        });
        pack
    }
    /// A pack, or a single brush saved as plain JSON by older Dudel, checked.
    pub fn read(source: &str) -> Result<Self, String> {
        let value: serde_json::Value = serde_json::from_str(source).map_err(|e| e.to_string())?;
        let pack = if value.get("format").is_some() {
            if value["format"] == "dudel-brush-pack"
                && value["version"].as_u64().is_some_and(|v| v > 3)
            {
                return Err("This brush file was made by a newer Dudel".into());
            }
            serde_json::from_value(value).map_err(|e| e.to_string())?
        } else {
            let brush: Brush = serde_json::from_value(value).map_err(|e| e.to_string())?;
            Self::new(vec![brush])
        };
        pack.validate()?;
        Ok(pack)
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.format != "dudel-brush-pack" || !matches!(self.version, 1..=3) {
            return Err("Unsupported brush pack format/version".into());
        }
        if let Some(info) = &self.info {
            info.validate()?;
        }
        if self.version < 3 && self.brushes.iter().any(|b| b.graph.is_some()) {
            return Err("Brush graphs need a version 3 brush pack".into());
        }
        if self.brushes.is_empty() || self.brushes.len() > 512 {
            return Err("Brush packs must contain 1–512 presets".into());
        }
        for brush in &self.brushes {
            brush.validate()?;
        }
        Ok(())
    }
    pub fn append_to(self, library: &mut Vec<Brush>) -> Result<usize, String> {
        self.validate()?;
        if library.len() + self.brushes.len() > 512 {
            return Err("Import exceeds the 512-preset library limit".into());
        }
        let first = library.len();
        library.extend(self.brushes);
        Ok(first)
    }
}
impl Dudel {
    pub(crate) fn brush_pack_file(&mut self, import: bool) {
        let dialog = self.dialog_in(
            crate::locations::Kind::Brushes,
            rfd::AsyncFileDialog::new().add_filter("Dudel brush pack", &["dudelbrushes"]),
        );
        if import {
            self.ask_open_file(dialog, Self::import_brush_pack);
        } else {
            let dialog = dialog.set_file_name("My brushes.dudelbrushes");
            self.ask_save_file(dialog, Self::export_brush_pack);
        }
    }
    fn export_brush_pack(&mut self, path: std::path::PathBuf) {
        let result = (|| {
            let pack = BrushPack::new(self.brushes.clone());
            pack.validate()?;
            let bytes = serde_json::to_vec(&pack).map_err(|e| e.to_string())?;
            storage::atomic_write(&path, &bytes)
        })();
        self.status = match result {
            Ok(()) => "Brush pack exported".into(),
            Err(e) => format!("Brush pack: {e}"),
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn portable_pack_preserves_settings_and_imports_together() {
        let brush = Brush {
            name: "Custom pencil".into(),
            tags: "sketch, pencil".into(),
            pressure_gamma: 0.6,
            custom_pressure: true,
            pressure_curve: [0.0, 0.1, 0.7, 0.9, 1.0],
            stabilization: 8.0,
            tip: crate::brush::Tip::Square,
            aspect: 0.3,
            angle: 30.0,
            scatter: 0.6,
            scatter_seed: 123,
            ..Brush::default()
        };
        let pack = BrushPack::new(vec![brush, Brush::default()]);
        let encoded = serde_json::to_vec(&pack).unwrap();
        let decoded: BrushPack = serde_json::from_slice(&encoded).unwrap();
        let mut library = vec![Brush::default()];
        assert_eq!(decoded.append_to(&mut library).unwrap(), 1);
        assert_eq!(library.len(), 3);
        assert_eq!(library[1].pressure_gamma, 0.6);
        assert!(library[1].custom_pressure);
        assert_eq!(library[1].response(0.5), 0.7);
        assert_eq!(library[1].stabilization, 8.0);
        assert_eq!(library[1].tip, crate::brush::Tip::Square);
        assert_eq!(library[1].aspect, 0.3);
        assert_eq!(library[1].angle, 30.0);
        assert_eq!(library[1].scatter, 0.6);
        assert_eq!(library[1].scatter_seed, 123);
        assert_eq!(library[1].tags, "sketch, pencil");
    }
    #[test]
    fn invalid_packs_leave_library_unchanged() {
        let bad = Brush {
            size: 0.0,
            ..Brush::default()
        };
        let mut library = vec![Brush::default()];
        assert!(
            BrushPack::new(vec![Brush::default(), bad])
                .append_to(&mut library)
                .is_err()
        );
        assert_eq!(library.len(), 1);
        let mut pack = BrushPack::new(vec![Brush::default()]);
        pack.version = 999;
        assert!(pack.append_to(&mut library).is_err());
        assert_eq!(library.len(), 1);
    }
    #[test]
    fn image_tip_is_embedded_and_survives_pack_roundtrip() {
        let brush = Brush {
            tip: crate::brush::Tip::Image,
            image: Some(crate::brush::TipImage {
                width: 2,
                height: 2,
                alpha: (vec![255, 0, 128, 0]).into(),
            }),
            ..Brush::default()
        };
        let pack = BrushPack::new(vec![brush]);
        let encoded = serde_json::to_vec(&pack).unwrap();
        let decoded: BrushPack = serde_json::from_slice(&encoded).unwrap();
        decoded.validate().unwrap();
        assert_eq!(
            decoded.brushes[0].image.as_ref().unwrap().alpha,
            vec![255, 0, 128, 0]
        );
    }
}
