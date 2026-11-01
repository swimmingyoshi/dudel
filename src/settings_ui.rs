//! A stable, scrollable home for settings instead of nested transient menus.
use crate::{Dudel, egui};

/// Settings pages: tab name, heading, introduction and what resetting does.
const PAGES: [(&str, &str, &str, &str); 4] = [
    (
        "Workspace",
        "Make room for your work",
        "Choose what stays visible while you draw.",
        "Show the panels, return to the automatic panel layout and allow 8 open projects.",
    ),
    (
        "Drawing",
        "Drawing and pen input",
        "Brush-specific pressure and stabilization settings live in Brush Studio.",
        "Use native pen input, stop recording timelapses for new canvases, and lock and snap to guides.",
    ),
    (
        "Display",
        "Display and performance",
        "",
        "Hide the performance monitor. The graphics API is chosen in its own window.",
    ),
    (
        "Storage",
        "Files and storage",
        "Keep projects, exports and recovery files where you expect them.",
        "Save to Documents\\Dudel and its usual subfolders again. Files already saved stay where they are.",
    ),
];

/// One setting; searching matches its label and extra keywords.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Item {
    Panels,
    PanelLayout,
    ProjectLimit,
    Theme,
    Extensions,
    NativePen,
    Timelapse,
    LockGuides,
    Snapping,
    Graphics,
    Performance,
    Folders,
    #[cfg(windows)]
    Thumbnails,
}

impl Item {
    const ALL: &[Item] = &[
        Item::Panels,
        Item::PanelLayout,
        Item::ProjectLimit,
        Item::Theme,
        Item::Extensions,
        Item::NativePen,
        Item::Timelapse,
        Item::LockGuides,
        Item::Snapping,
        Item::Graphics,
        Item::Performance,
        Item::Folders,
        #[cfg(windows)]
        Item::Thumbnails,
    ];
    fn page(self) -> usize {
        match self {
            Item::Panels
            | Item::PanelLayout
            | Item::ProjectLimit
            | Item::Theme
            | Item::Extensions => 0,
            Item::NativePen | Item::Timelapse | Item::LockGuides | Item::Snapping => 1,
            Item::Graphics | Item::Performance => 2,
            Item::Folders => 3,
            #[cfg(windows)]
            Item::Thumbnails => 3,
        }
    }
    /// Lowercase words a search can match.
    fn keywords(self) -> &'static str {
        match self {
            Item::Panels => "show workspace panels hide sidebar inspector tool panel",
            Item::PanelLayout => "restore automatic panel layout reset sidebar",
            Item::ProjectLimit => "open project limit tabs memory documents",
            Item::Theme => "choose customize theme colors dark light appearance",
            Item::Extensions => "manage extensions plugins add-ons filters install",
            Item::NativePen => "native pen input tablet wacom xp-pen windows ink pressure stylus",
            Item::Timelapse => "record timelapses new canvases speed paint",
            Item::LockGuides => "lock guide handles rulers symmetry",
            Item::Snapping => "snap strokes to guides rulers",
            Item::Graphics => {
                "graphics api latency renderer gpu opengl directx dx12 vulkan vsync tearing lag"
            }
            Item::Performance => "show performance monitor memory fps frame time",
            Item::Folders => "storage folders library documents recovery exports autosave location",
            #[cfg(windows)]
            Item::Thumbnails => "file explorer thumbnails previews windows shell",
        }
    }
    fn matches(self, query: &str) -> bool {
        let query = query.to_lowercase();
        query
            .split_whitespace()
            .all(|word| self.keywords().contains(word))
    }
}

impl Dudel {
    pub(crate) fn settings_ui(&mut self, ctx: &egui::Context) {
        let mut open = self.settings_open;
        egui::Window::new("Settings")
            .open(&mut open)
            .collapsible(false)
            .default_width(480.0)
            .default_height(460.0)
            .vscroll(true)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.settings_search)
                            .id(egui::Id::new("settings search"))
                            .hint_text("Search settings")
                            .char_limit(60)
                            .desired_width(ui.available_width() - 40.0),
                    );
                    if !self.settings_search.is_empty()
                        && ui.small_button("✕").on_hover_text("Clear search").clicked()
                    {
                        self.settings_search.clear();
                    }
                });
                ui.add_space(4.0);
                let query = self.settings_search.trim().to_owned();
                if !query.is_empty() {
                    let found: Vec<Item> = Item::ALL
                        .iter()
                        .copied()
                        .filter(|item| item.matches(&query))
                        .collect();
                    if found.is_empty() {
                        ui.weak(format!("No settings match “{query}”."));
                    }
                    for item in found {
                        ui.add_space(4.0);
                        ui.weak(PAGES[item.page()].0);
                        self.setting_ui(ui, item);
                    }
                    return;
                }
                ui.horizontal_wrapped(|ui| {
                    for (i, (name, ..)) in PAGES.iter().enumerate() {
                        ui.selectable_value(&mut self.settings_section, i, *name);
                    }
                });
                ui.add_space(8.0);
                ui.separator();
                ui.add_space(8.0);
                let page = self.settings_section.min(PAGES.len() - 1);
                let (name, heading, intro, reset) = PAGES[page];
                ui.heading(heading);
                if page == 2 {
                    ui.label(format!("Running now: {}", self.display_active));
                } else {
                    ui.label(intro);
                }
                ui.add_space(8.0);
                for &item in Item::ALL.iter().filter(|item| item.page() == page) {
                    self.setting_ui(ui, item);
                }
                ui.add_space(8.0);
                ui.separator();
                if ui
                    .small_button("Reset this page…")
                    .on_hover_text(reset)
                    .clicked()
                {
                    self.confirm(
                        &format!("Reset {name} settings"),
                        format!("{reset}\n\nOther pages are unchanged."),
                        "Reset",
                        move |app| app.reset_settings_page(page),
                    );
                }
            });
        self.settings_open = open;
    }

    fn setting_ui(&mut self, ui: &mut egui::Ui, item: Item) {
        match item {
            Item::Panels => {
                ui.checkbox(&mut self.show_panels, "Show workspace panels");
            }
            Item::PanelLayout => {
                if ui.button("Restore automatic panel layout").clicked() {
                    self.tool_panel_override = None;
                    self.inspector_override = None;
                    self.show_panels = true;
                }
            }
            Item::ProjectLimit => {
                ui.horizontal(|ui| {
                    ui.label("Open project limit");
                    let minimum = self.tabs.len().max(1);
                    ui.add(
                        egui::DragValue::new(&mut self.tabs.max)
                            .range(minimum..=crate::tabs::LIMIT),
                    );
                });
                ui.weak("Each open project uses memory, including parked tabs.");
            }
            Item::Theme => {
                ui.menu_button("Choose or customize a theme", |ui| self.theme_menu(ui));
            }
            Item::Extensions => {
                if ui.button("Manage extensions…").clicked() {
                    self.extension_manager = true;
                }
            }
            Item::NativePen => {
                ui.checkbox(&mut self.input.native_requested, "Use native pen input");
                ui.weak(&self.input.diagnostics.status);
            }
            Item::Timelapse => {
                ui.checkbox(
                    &mut self.timelapse.prefs.record_new,
                    "Record timelapses for new canvases",
                );
            }
            Item::LockGuides => {
                ui.checkbox(&mut self.guides_locked, "Lock guide handles");
            }
            Item::Snapping => {
                ui.checkbox(&mut self.snapping, "Snap strokes to guides");
            }
            Item::Graphics => {
                if ui.button("Graphics API and latency…").clicked() {
                    self.display_window = true;
                }
                ui.weak("Graphics changes take effect after restarting Dudel.");
            }
            Item::Performance => {
                ui.checkbox(&mut self.performance.open, "Show performance monitor");
            }
            Item::Folders => {
                if ui.button("Storage & folders…").clicked() {
                    self.storage_window = true;
                }
            }
            #[cfg(windows)]
            Item::Thumbnails => {
                ui.add_space(4.0);
                ui.strong("File Explorer thumbnails");
                ui.weak("Show artwork previews for .dudel files. Enable again after moving Dudel's folder.");
                ui.horizontal_wrapped(|ui| {
                    for (enable, label) in [(true, "Enable previews"), (false, "Remove previews")] {
                        if ui.button(label).clicked() {
                            self.status = crate::file_preview::set_explorer_thumbnails(enable)
                                .unwrap_or_else(|error| format!("File Explorer previews: {error}"));
                        }
                    }
                });
            }
        }
    }

    /// Return one settings page to the values a fresh installation uses.
    fn reset_settings_page(&mut self, page: usize) {
        match page {
            0 => {
                self.show_panels = true;
                self.tool_panel_override = None;
                self.inspector_override = None;
                self.tabs.max = crate::tabs::DEFAULT_MAX.max(self.tabs.len());
            }
            1 => {
                self.input.native_requested = true;
                self.timelapse.prefs.record_new = false;
                self.guides_locked = true;
                self.snapping = true;
            }
            2 => self.performance.open = false,
            _ => self.locations = Default::default(),
        }
        self.status = format!("{} settings reset", PAGES[page.min(PAGES.len() - 1)].0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_setting_has_a_page_and_search_matches_its_keywords() {
        for &item in Item::ALL {
            assert!(item.page() < PAGES.len());
            assert!(item.matches(""), "{item:?}");
        }
        let find = |query: &str| -> Vec<Item> {
            Item::ALL
                .iter()
                .copied()
                .filter(|i| i.matches(query))
                .collect()
        };
        assert_eq!(find("Wacom"), vec![Item::NativePen]);
        assert_eq!(find("vsync"), vec![Item::Graphics]);
        assert_eq!(find("snap guides"), vec![Item::Snapping]);
        assert!(find("plugins").contains(&Item::Extensions));
        assert!(find("nothing like this").is_empty());
    }

    #[test]
    fn resetting_a_page_restores_only_that_page() {
        let mut app = Dudel {
            show_panels: false,
            inspector_override: Some(false),
            snapping: false,
            ..Default::default()
        };
        app.performance.open = true;
        app.reset_settings_page(1);
        assert!(app.snapping && !app.show_panels && app.performance.open);
        app.reset_settings_page(0);
        assert!(app.show_panels && app.inspector_override.is_none());
        assert_eq!(app.tabs.max, crate::tabs::DEFAULT_MAX);
        app.reset_settings_page(2);
        assert!(!app.performance.open);
        app.locations.beside_project = false;
        app.reset_settings_page(3);
        assert!(app.locations.beside_project);
    }
}
