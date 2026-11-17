//! Settings: a sidebar of categories, the chosen one on the right, and a
//! search over every setting at the top.
use crate::{Dudel, egui};

/// The settings pages, by name, for the code that opens one.
pub(crate) mod page {
    pub(crate) const INTERFACE: usize = 0;
    pub(crate) const DRAWING: usize = 1;
    pub(crate) const SHORTCUTS: usize = 2;
    pub(crate) const FILES: usize = 3;
    pub(crate) const DISPLAY: usize = 4;
    pub(crate) const EXTENSIONS: usize = 5;
}

/// Settings pages: sidebar name, heading, introduction and what resetting
/// does (empty when there is nothing to reset).
const PAGES: [(&str, &str, &str, &str); 6] = [
    (
        "Interface",
        "Look and layout",
        "The style, its colors, and which panels and palettes show. Every style keeps every feature.",
        "Return to the Classic style, show the panels in their automatic layout, and put every style's palettes back where they started.",
    ),
    (
        "Drawing & pen",
        "Drawing and pen input",
        "Brush-specific pressure and stabilization settings live in Brush Studio.",
        "Use native pen input, stop recording timelapses for new canvases, lock and snap to guides, and put the default commands back in the pen quick menu.",
    ),
    (
        "Shortcuts",
        "Keyboard and tablet shortcuts",
        "Bindings are saved with your settings and can be shared as profiles.",
        "Restore every command's default shortcuts.",
    ),
    (
        "Projects & files",
        "Projects and files",
        "How many projects stay open, and where projects, exports and recovery files go.",
        "Allow 8 open projects, and save to Documents\\Dudel and its usual subfolders again. Files already saved stay where they are.",
    ),
    (
        "Display",
        "Display and performance",
        "",
        "Hide the performance monitor. The graphics API is chosen in its own window.",
    ),
    (
        "Extensions",
        "Extensions",
        "Filters, brushes and tools others have made, and your own.",
        "",
    ),
];

/// One setting; searching matches its label and extra keywords.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Item {
    Shortcuts,
    Panels,
    PanelLayout,
    Palettes,
    ProjectLimit,
    Interface,
    Theme,
    Extensions,
    NativePen,
    QuickMenu,
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
        // Quick switches first; the style picker is long.
        Item::Panels,
        Item::PanelLayout,
        Item::Palettes,
        Item::Interface,
        Item::Theme,
        Item::NativePen,
        Item::QuickMenu,
        Item::Snapping,
        Item::LockGuides,
        Item::Timelapse,
        Item::Shortcuts,
        Item::ProjectLimit,
        Item::Folders,
        #[cfg(windows)]
        Item::Thumbnails,
        Item::Graphics,
        Item::Performance,
        Item::Extensions,
    ];
    fn page(self) -> usize {
        match self {
            Item::Interface | Item::Theme | Item::Panels | Item::Palettes | Item::PanelLayout => {
                page::INTERFACE
            }
            Item::NativePen
            | Item::QuickMenu
            | Item::Timelapse
            | Item::LockGuides
            | Item::Snapping => page::DRAWING,
            Item::Shortcuts => page::SHORTCUTS,
            Item::ProjectLimit | Item::Folders => page::FILES,
            #[cfg(windows)]
            Item::Thumbnails => page::FILES,
            Item::Graphics | Item::Performance => page::DISPLAY,
            Item::Extensions => page::EXTENSIONS,
        }
    }
    /// Lowercase words a search can match.
    fn keywords(self) -> &'static str {
        match self {
            Item::Shortcuts => {
                "keyboard shortcuts bindings keys remap tablet express profile import export"
            }
            Item::Panels => "show workspace panels hide sidebar inspector tool panel",
            Item::PanelLayout => "restore automatic panel layout reset sidebar classic",
            Item::Palettes => {
                "palettes dock docking tabs window drag move float lock arrange arrangement reset panels workspace"
            }
            Item::ProjectLimit => "open project limit tabs memory documents",
            Item::Interface => {
                "interface style layout slate float atelier classic icons line duotone solid density compact touch corners colors palette appearance tablet left handed"
            }
            Item::Theme => "choose customize theme colors dark light appearance",
            Item::Extensions => "manage extensions plugins add-ons filters install",
            Item::NativePen => "native pen input tablet wacom xp-pen windows ink pressure stylus",
            Item::QuickMenu => {
                "pen quick menu ring radial popup palette side barrel button right click favorite brushes colors"
            }
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
        // A fixed size, so the categories and the page lay out inside the
        // frame; a resizable window lets wide contents push past its edge.
        let screen = ctx.content_rect().size();
        let size = egui::vec2(
            (screen.x - 40.0).clamp(420.0, 760.0),
            (screen.y - 120.0).clamp(300.0, 540.0),
        );
        egui::Window::new("Settings")
            .open(&mut open)
            .collapsible(false)
            .fixed_size(size)
            // Opens in the middle like the other dialogs, and stays where
            // the artist drags it.
            .pivot(egui::Align2::CENTER_CENTER)
            .default_pos(ctx.content_rect().center())
            .show(ctx, |ui| {
                // A window takes the size of its contents, so every part
                // here has an exact size: nothing can push past the frame.
                ui.set_min_size(size);
                ui.set_max_size(size);
                let top = ui.next_widget_position().y;
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.settings_search)
                            .id(egui::Id::new("settings search"))
                            .hint_text("Search settings")
                            .char_limit(60)
                            .desired_width(size.x - 40.0),
                    );
                    if !self.settings_search.is_empty()
                        && ui.small_button("✕").on_hover_text("Clear search").clicked()
                    {
                        self.settings_search.clear();
                    }
                });
                ui.add_space(6.0);
                let query = self.settings_search.trim().to_owned();
                let height = (size.y - (ui.next_widget_position().y - top)).max(120.0);
                let sidebar = 150.0;
                let gap = 2.0 * ui.spacing().item_spacing.x + 8.0;
                ui.horizontal_top(|ui| {
                    // The categories.
                    ui.allocate_ui_with_layout(
                        egui::vec2(sidebar, height),
                        egui::Layout::top_down_justified(egui::Align::Min),
                        |ui| {
                            ui.set_min_size(egui::vec2(sidebar, height));
                            ui.spacing_mut().button_padding = egui::vec2(10.0, 7.0);
                            for (i, (name, ..)) in PAGES.iter().enumerate() {
                                let chosen = query.is_empty() && self.settings_section == i;
                                if ui.selectable_label(chosen, *name).clicked() {
                                    self.settings_section = i;
                                    self.settings_search.clear();
                                }
                            }
                        },
                    );
                    ui.separator();
                    // The page, scrolling within its own width.
                    let width = size.x - sidebar - gap;
                    ui.allocate_ui_with_layout(
                        egui::vec2(width, height),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| {
                            ui.set_max_width(width);
                            egui::ScrollArea::vertical()
                                .id_salt(("settings page", self.settings_section, query.is_empty()))
                                .auto_shrink([false, false])
                                .max_height(height)
                                .show(ui, |ui| {
                                    ui.set_max_width(width - 16.0);
                                    if query.is_empty() {
                                        self.settings_page_ui(ui);
                                    } else {
                                        self.settings_results_ui(ui, &query);
                                    }
                                });
                        },
                    );
                });
            });
        self.settings_open = open;
    }

    fn settings_page_ui(&mut self, ui: &mut egui::Ui) {
        let page = self.settings_section.min(PAGES.len() - 1);
        let (name, heading, intro, reset) = PAGES[page];
        ui.horizontal(|ui| {
            ui.heading(heading);
            // Beside the heading, so a long page never hides it.
            if !reset.is_empty() {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
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
            }
        });
        if page == page::DISPLAY {
            ui.label(format!("Running now: {}", self.display_active));
        } else {
            ui.label(intro);
        }
        ui.add_space(8.0);
        for &item in Item::ALL.iter().filter(|item| item.page() == page) {
            self.setting_ui(ui, item);
            ui.add_space(4.0);
        }
    }

    /// Every setting the search finds, each under a link to its page.
    fn settings_results_ui(&mut self, ui: &mut egui::Ui, query: &str) {
        let found: Vec<Item> = Item::ALL
            .iter()
            .copied()
            .filter(|item| item.matches(query))
            .collect();
        if found.is_empty() {
            ui.weak(format!("No settings match “{query}”."));
        }
        for item in found {
            ui.add_space(4.0);
            let page = item.page();
            if ui
                .link(egui::RichText::new(format!("{} ›", PAGES[page].0)).small())
                .on_hover_text("Show the whole page")
                .clicked()
            {
                self.settings_section = page;
                self.settings_search.clear();
            }
            self.setting_ui(ui, item);
        }
    }

    fn setting_ui(&mut self, ui: &mut egui::Ui, item: Item) {
        match item {
            Item::Shortcuts => self.shortcut_settings_ui(ui),
            Item::Panels => {
                ui.checkbox(&mut self.show_panels, "Show workspace panels");
            }
            Item::PanelLayout => {
                if !self.docked() && ui.button("Restore automatic panel layout").clicked() {
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
            Item::Interface => self.interface_settings_ui(ui),
            Item::Palettes => {
                if self.docked() {
                    ui.strong("Palettes");
                    ui.weak(
                        "Drag a palette's tab to move it, onto an edge to split, or off to \
                         float it. Window opens closed palettes again.",
                    );
                    ui.checkbox(&mut self.ui_layout.docks.locked, "Lock palettes")
                        .on_hover_text("No dragging, closing or floating: safe for a pen");
                    if ui
                        .button(format!(
                            "Reset to the {} arrangement",
                            self.interface.layout.name()
                        ))
                        .clicked()
                    {
                        self.reset_palettes();
                    }
                } else {
                    ui.weak("Movable palettes come with the Slate, Float and Atelier styles.");
                }
            }
            Item::Theme => {
                ui.menu_button("Style and colors", |ui| self.theme_menu(ui));
            }
            Item::Extensions => {
                if ui.button("Manage extensions…").clicked() {
                    self.extension_manager = true;
                }
            }
            Item::QuickMenu => {
                ui.add_space(4.0);
                ui.strong("Pen quick menu");
                self.quick_menu_settings_ui(ui);
                ui.add_space(4.0);
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
            page::INTERFACE => {
                self.interface = Default::default();
                self.show_panels = true;
                self.tool_panel_override = None;
                self.inspector_override = None;
                self.ui_layout.docks.reset_all();
            }
            page::DRAWING => {
                self.input.native_requested = true;
                self.timelapse.prefs.record_new = false;
                self.guides_locked = true;
                self.snapping = true;
                self.quick_menu_prefs = Default::default();
            }
            page::SHORTCUTS => {
                self.apply_shortcuts(Default::default());
                self.shortcut_editor.message = "Default shortcuts restored".into();
            }
            page::FILES => {
                self.tabs.max = crate::tabs::DEFAULT_MAX.max(self.tabs.len());
                self.locations = Default::default();
            }
            page::DISPLAY => self.performance.open = false,
            _ => return,
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
        // Every page has something on it.
        for (page, (name, ..)) in PAGES.iter().enumerate() {
            assert!(Item::ALL.iter().any(|i| i.page() == page), "{name}");
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
        assert_eq!(find("atelier"), vec![Item::Interface]);
        assert_eq!(find("dock"), vec![Item::Palettes]);
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
        app.reset_settings_page(page::DRAWING);
        assert!(app.snapping && !app.show_panels && app.performance.open);
        app.interface = crate::ui::style::Interface::preset(crate::ui::style::Layout::Float);
        app.reset_settings_page(page::INTERFACE);
        assert!(app.show_panels && app.inspector_override.is_none());
        assert_eq!(app.interface, Default::default());
        app.reset_settings_page(page::DISPLAY);
        assert!(!app.performance.open);
        app.tabs.max = 3;
        app.locations.beside_project = false;
        app.reset_settings_page(page::FILES);
        assert!(app.locations.beside_project);
        assert_eq!(app.tabs.max, crate::tabs::DEFAULT_MAX);
    }
}
