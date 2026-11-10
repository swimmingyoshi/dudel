use crate::{Dudel, brush::Brush, read_bounded, storage};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
pub struct BrushPack {
    pub format: String,
    pub version: u32,
    pub brushes: Vec<Brush>,
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
        }
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.format != "dudel-brush-pack" || !matches!(self.version, 1..=3) {
            return Err("Unsupported brush pack format/version".into());
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
    pub(crate) fn import_brush_pack(&mut self, path: std::path::PathBuf) {
        let result = (|| {
            let source = read_bounded(&path, 128 * 1024 * 1024)?;
            let pack: BrushPack = serde_json::from_str(&source).map_err(|e| e.to_string())?;
            pack.append_to(&mut self.brushes)
        })();
        match result {
            Ok(first) => {
                self.selected_brush = first;
                self.brush = self.brushes[first].clone();
                self.brush_search.clear();
                self.status = "Brush pack imported".into();
            }
            Err(e) => self.status = format!("Brush pack: {e}"),
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
