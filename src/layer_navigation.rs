//! Session-only navigation: filtering never changes document order or selection.
use crate::{Dudel, document::Layer, egui};

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum Kind {
    #[default]
    All,
    Raster,
    Vector,
    Adjustment,
    Folder,
    Reference,
}

impl Kind {
    const ALL: [Self; 6] = [
        Self::All,
        Self::Raster,
        Self::Vector,
        Self::Adjustment,
        Self::Folder,
        Self::Reference,
    ];
    fn name(self) -> &'static str {
        match self {
            Self::All => "All types",
            Self::Raster => "Raster",
            Self::Vector => "Vector",
            Self::Adjustment => "Adjustment",
            Self::Folder => "Folders",
            Self::Reference => "References",
        }
    }
}

#[derive(Default)]
pub(crate) struct Navigation {
    pub query: String,
    pub kind: Kind,
    pub compact: bool,
    pub reveal_requested: bool,
    pub viewport: Option<[i32; 2]>,
}

impl Navigation {
    pub fn active(&self) -> bool {
        !self.query.trim().is_empty() || self.kind != Kind::All
    }
    fn name_matches(&self, name: &str) -> bool {
        let name = name.to_lowercase();
        self.query
            .split_whitespace()
            .all(|word| name.contains(&word.to_lowercase()))
    }
    pub fn matches(&self, layer: &Layer) -> bool {
        let kind = if layer.folder {
            Kind::Folder
        } else if layer.adjustment.is_some() {
            Kind::Adjustment
        } else if layer.vector.is_some() {
            Kind::Vector
        } else {
            Kind::Raster
        };
        (self.kind == Kind::All || self.kind == kind) && self.name_matches(&layer.name)
    }
    pub fn matches_reference(&self, name: &str) -> bool {
        matches!(self.kind, Kind::All | Kind::Reference) && self.name_matches(name)
    }
}

impl Dudel {
    pub(crate) fn layer_navigation_ui(&mut self, ui: &mut egui::Ui) {
        // Keep navigation to one compact row even in a short inspector.
        ui.scope(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(4.0, 2.0);
            ui.spacing_mut().button_padding = egui::vec2(4.0, 2.0);
            ui.spacing_mut().interact_size.y = 22.0;
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut self.layer_navigation.query)
                    .id_salt("layer search").hint_text("Search layers…").char_limit(256)
                    .desired_width((ui.available_width() - 112.0).max(30.0)))
                    .on_hover_text("Search layer names, including collapsed folders");
                let label = match self.layer_navigation.kind {
                    Kind::All => "Types",
                    Kind::Adjustment => "Adjust.",
                    Kind::Reference => "Refs",
                    kind => kind.name(),
                };
                ui.menu_button(label, |ui| {
                    for kind in Kind::ALL {
                        if ui.selectable_value(&mut self.layer_navigation.kind, kind, kind.name()).clicked() { ui.close(); }
                    }
                    ui.separator();
                    ui.checkbox(&mut self.layer_navigation.compact, "Compact rows");
                    if ui.add_enabled(self.layer_navigation.active(), egui::Button::new("Clear filters")).clicked() {
                        self.layer_navigation.query.clear();
                        self.layer_navigation.kind = Kind::All;
                        ui.close();
                    }
                });
                if ui.small_button("Reveal").on_hover_text("Reveal active: clear filters, expand parent folders and scroll to the active layer").clicked() {
                    self.reveal_active_layer();
                }
            });
            if self.layer_navigation.active() {
                let count = self.doc.layers.iter().filter(|l| self.layer_navigation.matches(l)).count()
                    + self.reference_layers.layers.iter().filter(|l| self.layer_navigation.matches_reference(&l.name)).count();
                if count == 0 { ui.small("No matching layers"); }
                else if ui.available_height() > 80.0 {
                    let noun = if count == 1 { "match" } else { "matches" };
                    ui.small(format!("{count} {noun} · clear filters to reorder"));
                }
            }
        });
    }

    pub(crate) fn reveal_active_layer(&mut self) {
        self.layer_navigation.query.clear();
        self.layer_navigation.kind = Kind::All;
        self.show_panels = true;
        self.inspector_override = Some(true);
        // Reference positions are attached to document folders as well.
        let index = self
            .reference_layers
            .active
            .and_then(|id| self.reference_layers.layers.iter().find(|b| b.id == id))
            .and_then(|b| self.reference_layers.location(b, &self.doc).2);
        let reference = self.reference_layers.active.is_some();
        let target = if reference {
            index.map(|i| self.doc.layers[i].id)
        } else {
            self.input_target()
        };
        let context = self
            .pending_layers
            .clone()
            .unwrap_or_else(|| crate::pending_layers::PendingLayers::from_document(&self.doc));
        let folders = target.map_or_else(Vec::new, |id| context.collapsed_ancestors(id, reference));
        for id in folders {
            self.set_canvas_property(id, crate::document_edits::Property::Collapsed(false));
        }
        self.layer_reveal = None;
        self.layer_navigation.reveal_requested = true;
    }
}
