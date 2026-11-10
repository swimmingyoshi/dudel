use crate::{brush::Brush, workspace};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub const KEY: &str = "dudel.preferences.v1";

#[derive(Serialize, Deserialize)]
pub struct Preferences {
    #[serde(default, deserialize_with = "crate::shortcuts::lenient")]
    pub shortcuts: crate::shortcuts::Bindings,
    /// Saved shortcuts that decoding put back to their defaults.
    #[serde(skip)]
    pub shortcuts_dropped: usize,
    /// Brush graphs that failed validation and were removed on load.
    #[serde(skip)]
    pub graphs_dropped: usize,
    pub version: u32,
    #[serde(default)]
    pub theme: crate::themes::Theme,
    pub brushes: Vec<Brush>,
    pub current_brush: Brush,
    pub selected_brush: usize,
    pub color: [u8; 3],
    #[serde(default = "crate::palette::defaults")]
    pub palette: Vec<[u8; 3]>,
    pub show_panels: bool,
    #[serde(default = "default_native_pen")]
    pub native_pen: bool,
    #[serde(default)]
    pub inspector_override: Option<bool>,
    #[serde(default)]
    pub tool_panel_override: Option<bool>,
    pub recent: Vec<PathBuf>,
    #[serde(default = "default_secondary")]
    pub secondary: [u8; 3],
    #[serde(default)]
    pub recent_colors: Vec<[u8; 3]>,
    /// What new text looks like.
    #[serde(default)]
    pub text: crate::text_options::Style,
    /// How image exports are written.
    #[serde(default)]
    pub export: crate::export_options::ExportSettings,
    /// The commands in the middle of the pen quick menu.
    #[serde(default)]
    pub quick_menu: crate::quick_menu::QuickMenuPrefs,
    /// Open the welcome window at startup.
    #[serde(default = "default_welcome")]
    pub welcome: bool,
    /// Show transparent canvas areas as a checkerboard instead of white paper.
    #[serde(default)]
    pub show_transparency: bool,
    #[serde(default)]
    pub tools: ToolPrefs,
    #[serde(default)]
    pub storage: crate::locations::Locations,
    #[serde(default)]
    pub preview: crate::preview_window::Settings,
    #[serde(default)]
    pub tabs: TabPrefs,
    #[serde(default)]
    pub timelapse: crate::timelapse::Prefs,
    #[serde(default)]
    pub layout: LayoutPrefs,
    #[serde(default)]
    pub new_canvas: crate::new_canvas::Prefs,
}

/// Heights the artist dragged in the side panels; None keeps the automatic size.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Debug, Default)]
pub struct LayoutPrefs {
    #[serde(default)]
    pub library_height: Option<f32>,
    #[serde(default)]
    pub color_height: Option<f32>,
    #[serde(default)]
    pub navigator_height: Option<f32>,
    #[serde(default)]
    pub layers_height: Option<f32>,
    /// The right panel's Navigator tab is chosen instead of Color.
    #[serde(default)]
    pub navigator_tab: bool,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
pub struct TabPrefs {
    /// Most projects open at once.
    #[serde(default = "default_max_tabs")]
    pub max: usize,
}
fn default_max_tabs() -> usize {
    crate::tabs::DEFAULT_MAX
}
impl Default for TabPrefs {
    fn default() -> Self {
        Self {
            max: default_max_tabs(),
        }
    }
}

/// Tool choices that carry between sessions.
#[derive(Serialize, Deserialize, Default, Clone, PartialEq)]
pub struct ToolPrefs {
    #[serde(default)]
    pub lasso: crate::lasso_paint::Options,
    /// The Fill tool's Gradient mode and its choices.
    #[serde(default)]
    pub gradient: crate::gradient_tool::Options,
    #[serde(default)]
    pub brush_group: crate::brush::BrushGroup,
    /// Last preset name chosen in each brush group, by `BrushGroup::index`.
    #[serde(default)]
    pub group_presets: [String; 4],
    /// Erase with a separate eraser tip instead of the current brush's shape.
    #[serde(default)]
    pub eraser_own: bool,
    #[serde(default)]
    pub eraser: Option<crate::brush::Brush>,
}

fn default_secondary() -> [u8; 3] {
    [255, 255, 255]
}
fn default_native_pen() -> bool {
    true
}
fn default_welcome() -> bool {
    true
}

impl Preferences {
    pub fn decode(source: &str) -> Result<Self, String> {
        if source.len() > 128 * 1024 * 1024 {
            return Err("Preferences exceed size limit".into());
        }
        let mut p: Self = serde_json::from_str(source).map_err(|e| e.to_string())?;
        if p.version != 1 {
            return Err("Unsupported preferences version".into());
        }
        p.shortcuts_dropped = p.shortcuts.repair();
        if p.brushes.len() > 512 {
            return Err("Too many brush presets".into());
        }
        if p.brushes.is_empty() {
            p.brushes = workspace::default_brushes();
        }
        // A broken graph costs only the graph, not the whole settings file.
        for brush in p.brushes.iter_mut().chain([&mut p.current_brush]) {
            if brush.graph.as_ref().is_some_and(|g| g.validate().is_err()) {
                brush.graph = None;
                p.graphs_dropped += 1;
            }
        }
        for brush in &p.brushes {
            brush.validate()?;
        }
        p.current_brush.validate()?;
        p.theme.upgrade_builtin();
        p.theme.validate()?;
        crate::palette::validate(&p.palette)?;
        p.selected_brush = p.selected_brush.min(p.brushes.len() - 1);
        // Favourite state belongs to the library entry, including legacy files
        // saved while the selected entry's star and live brush differed.
        p.current_brush.favorite = p.brushes[p.selected_brush].favorite;
        p.recent.truncate(10);
        p.recent_colors.truncate(16);
        p.text.sanitize();
        p.storage.sanitize();
        for name in &mut p.tools.group_presets {
            name.truncate(256);
        }
        Ok(p)
    }
}

pub fn remember(recent: &mut Vec<PathBuf>, path: PathBuf) {
    recent.retain(|entry| entry != &path);
    recent.insert(0, path);
    recent.truncate(10);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preferences_preserve_custom_brush_and_workspace() {
        let brush = Brush {
            name: "My brush".into(),
            size: 42.0,
            ..Brush::default()
        };
        let p = Preferences {
            shortcuts: Default::default(),
            shortcuts_dropped: 0,
            graphs_dropped: 0,
            theme: crate::themes::Theme::presets().remove(1),
            version: 1,
            brushes: vec![brush.clone()],
            current_brush: brush,
            selected_brush: 100,
            color: [12, 34, 56],
            palette: vec![[12, 34, 56], [78, 132, 204]],
            show_panels: false,
            native_pen: false,
            inspector_override: None,
            tool_panel_override: Some(true),
            recent: vec!["painting.dudel".into()],
            secondary: [1, 2, 3],
            recent_colors: vec![[9, 9, 9]],
            text: Default::default(),
            export: crate::export_options::ExportSettings {
                format: crate::export_options::Format::Jpeg,
                quality: 70,
                ..Default::default()
            },
            quick_menu: crate::quick_menu::QuickMenuPrefs {
                actions: vec!["edit.undo".into()],
            },
            welcome: false,
            show_transparency: true,
            tools: ToolPrefs {
                brush_group: crate::brush::BrushGroup::Airbrush,
                ..Default::default()
            },
            storage: crate::locations::Locations {
                beside_project: false,
                ..Default::default()
            },
            preview: crate::preview_window::Settings {
                show_pointer: true,
                ..Default::default()
            },
            tabs: TabPrefs { max: 3 },
            layout: LayoutPrefs {
                library_height: Some(96.0),
                color_height: None,
                navigator_height: Some(150.0),
                layers_height: None,
                navigator_tab: true,
            },
            timelapse: crate::timelapse::Prefs {
                record_new: true,
                ..Default::default()
            },
            new_canvas: crate::new_canvas::Prefs {
                presets: vec![crate::new_canvas::Size {
                    name: "Comic page".into(),
                    width: 2894,
                    height: 4093,
                    dpi: 350.0,
                }],
                recent: vec![],
            },
        };
        let restored = Preferences::decode(&serde_json::to_string(&p).unwrap()).unwrap();
        assert_eq!(restored.theme, p.theme);
        assert_eq!(restored.brushes[0].size, 42.0);
        assert_eq!(restored.selected_brush, 0);
        assert!(restored.show_transparency);
        assert!(!restored.welcome);
        assert!(restored.tools == p.tools);
        assert!(!restored.storage.beside_project);
        assert!(restored.preview.show_pointer && restored.preview.hide_references);
        assert_eq!(restored.tabs.max, 3);
        assert!(restored.timelapse.record_new);
        assert_eq!(restored.new_canvas, p.new_canvas);
        assert!(!restored.show_panels);
        assert!(!restored.native_pen);
        assert_eq!(restored.color, [12, 34, 56]);
        assert_eq!(restored.palette, p.palette);
        assert_eq!(restored.recent, p.recent);
        assert_eq!(restored.secondary, [1, 2, 3]);
        assert_eq!(restored.recent_colors, vec![[9, 9, 9]]);
        let legacy = format!(
            r#"{{"version":1,"brushes":[],"current_brush":{},"selected_brush":0,"color":[0,0,0],"show_panels":true,"recent":[]}}"#,
            serde_json::to_string(&Brush::default()).unwrap()
        );
        let legacy = Preferences::decode(&legacy).unwrap();
        assert_eq!(legacy.secondary, [255, 255, 255]);
        assert!(legacy.recent_colors.is_empty());
        assert!(legacy.native_pen);
        assert_eq!(legacy.tabs.max, crate::tabs::DEFAULT_MAX);
        assert!(!legacy.timelapse.record_new);
        let mut invalid = p;
        invalid.current_brush.size = -1.0;
        assert!(Preferences::decode(&serde_json::to_string(&invalid).unwrap()).is_err());
    }
    #[test]
    fn recent_files_are_unique_and_bounded() {
        let mut paths = vec![];
        for index in 0..20 {
            remember(&mut paths, format!("{index}.dudel").into());
        }
        remember(&mut paths, "15.dudel".into());
        assert_eq!(paths.len(), 10);
        assert_eq!(paths[0], PathBuf::from("15.dudel"));
        assert_eq!(
            paths
                .iter()
                .filter(|p| p.as_path() == std::path::Path::new("15.dudel"))
                .count(),
            1
        );
    }
}
