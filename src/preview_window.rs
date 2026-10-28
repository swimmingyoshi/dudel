//! A second OS window showing only the artwork, for another monitor, an
//! audience or capture software such as OBS. It can leave out reference
//! layers and chosen layers. When nothing is left out it draws the main
//! canvas tiles; otherwise it keeps its own tiles, updated only from the same
//! damage as the main canvas.
use crate::{Dudel, canvas_textures::CanvasTextures, document::LayerId, egui, view::View};
use egui::{Color32, Rect};
use std::collections::HashSet;

#[derive(Clone, Copy, PartialEq, Eq, Default, Debug, serde::Serialize, serde::Deserialize)]
pub enum Framing {
    /// The whole canvas, upright, centered in the window.
    #[default]
    Fit,
    /// The main window's zoom, pan, rotation and mirroring, scaled to fit.
    FollowView,
}
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug, serde::Serialize, serde::Deserialize)]
pub enum Background {
    #[default]
    Theme,
    Black,
    White,
    /// Pure green for chroma keying in capture software.
    Green,
}
impl Background {
    const ALL: [Background; 4] = [Self::Theme, Self::Black, Self::White, Self::Green];
    fn name(self) -> &'static str {
        match self {
            Self::Theme => "Workspace",
            Self::Black => "Black",
            Self::White => "White",
            Self::Green => "Chroma green",
        }
    }
}

/// Choices kept in preferences.
#[derive(Clone, PartialEq, Debug, serde::Serialize, serde::Deserialize)]
pub struct Settings {
    #[serde(default)]
    pub framing: Framing,
    #[serde(default)]
    pub background: Background,
    #[serde(default = "yes")]
    pub hide_references: bool,
    #[serde(default)]
    pub show_pointer: bool,
    #[serde(default)]
    pub always_on_top: bool,
}
fn yes() -> bool {
    true
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            framing: Framing::Fit,
            background: Background::Theme,
            hide_references: true,
            show_pointer: false,
            always_on_top: false,
        }
    }
}

#[derive(Default)]
pub struct Preview {
    pub open: bool,
    pub settings: Settings,
    /// Layers left out of the preview for this session.
    pub hidden: HashSet<LayerId>,
    /// Own tiles; only allocated while something is left out.
    textures: Option<CanvasTextures>,
    /// Damage since the preview's own tiles last updated.
    damage: crate::render_region::Damage,
    full: bool,
    /// What the own tiles last showed: document revision and hidden layers.
    shown: Option<(u64, Vec<LayerId>, bool)>,
    /// Pen position over the main canvas, in document pixels.
    pub pointer: Option<egui::Pos2>,
    fullscreen: bool,
    /// UI capture mode: where to save a screenshot of this window.
    pub capture: Option<std::path::PathBuf>,
    capture_started: Option<std::time::Instant>,
}
impl Preview {
    pub fn toggle_hidden(&mut self, id: LayerId) {
        if !self.hidden.remove(&id) {
            self.hidden.insert(id);
        }
    }
    /// Record main-canvas damage so own tiles can follow it later.
    pub(crate) fn note_damage(&mut self, full: bool, damage: &crate::render_region::Damage) {
        if self.textures.is_some() {
            self.full |= full;
            self.damage.merge(damage.clone());
        }
    }
    /// Forget tiles, for example when another project becomes active.
    pub(crate) fn reset(&mut self) {
        self.textures = None;
        self.shown = None;
        self.damage = Default::default();
        self.full = true;
    }
}

impl Dudel {
    /// Independent artwork or large-canvas sampling requires its own display plan.
    fn preview_needs_own_tiles(&self) -> bool {
        let positioned = self
            .reference_layers
            .layers
            .iter()
            .any(|b| b.position.is_some());
        self.doc.width * self.doc.height >= 2048 * 2048
            || (self.preview.settings.hide_references && positioned)
            || self
                .doc
                .layers
                .iter()
                .any(|l| l.visible && self.preview.hidden.contains(&l.id))
    }
    /// The document as the preview shows it.
    fn preview_document(&mut self) -> crate::document::Document {
        let mut doc = if self.preview.settings.hide_references {
            self.doc.clone()
        } else {
            self.reference_layers
                .display_document(&self.doc)
                .unwrap_or_else(|| self.doc.clone())
        };
        for layer in &mut doc.layers {
            if self.preview.hidden.contains(&layer.id) {
                layer.visible = false;
            }
        }
        doc
    }
    fn update_preview_tiles(&mut self, ctx: &egui::Context, area: Rect, view: View) {
        let mut hidden: Vec<LayerId> = self.preview.hidden.iter().copied().collect();
        hidden.sort_unstable_by_key(|id| format!("{id:?}"));
        let key = (self.revision, hidden, self.preview.settings.hide_references);
        let textures = self.preview.textures.get_or_insert_with(|| {
            self.preview.full = true;
            CanvasTextures::default()
        });
        let alpha = self.show_transparency;
        let mut full = std::mem::take(&mut self.preview.full);
        full |= textures.set_transparent(alpha);
        textures.set_checker(self.show_transparency);
        textures.set_viewport(area, view);
        textures.sync_resolution_plan(ctx, &self.doc);
        let damage = self.preview.damage.take();
        if !full
            && damage.is_empty()
            && !textures.busy()
            && self.preview.shown.as_ref() == Some(&key)
        {
            return;
        }
        let doc = self.preview_document();
        let options = egui::TextureOptions {
            magnification: egui::TextureFilter::Linear,
            minification: egui::TextureFilter::Linear,
            wrap_mode: egui::TextureWrapMode::ClampToEdge,
            mipmap_mode: Some(egui::TextureFilter::Linear),
        };
        let textures = self.preview.textures.as_mut().unwrap();
        textures.update_damage(ctx, &doc, full, damage, options, &mut self.performance);
        if let Some(error) = textures.take_error() {
            self.status = format!("Preview: {error}");
        }
        self.preview.shown = Some(key);
    }
    fn preview_view(&self, area: Rect) -> View {
        let half = egui::vec2(self.doc.width as f32, self.doc.height as f32) / 2.0;
        let main = self.canvas_area;
        if self.preview.settings.framing == Framing::FollowView
            && self.zoom > 0.0
            && main.width() > 1.0
            && main.height() > 1.0
        {
            let scale = (area.width() / main.width()).min(area.height() / main.height());
            return View {
                origin: area.center() + self.pan * scale,
                zoom: self.zoom * scale,
                rotation: self.rotation,
                mirror: self.mirror,
                half,
            };
        }
        let zoom = ((area.width() - 16.0) / (half.x * 2.0))
            .min((area.height() - 16.0) / (half.y * 2.0))
            .max(crate::view::MIN_ZOOM);
        View {
            origin: area.center(),
            zoom,
            rotation: 0.0,
            mirror: false,
            half,
        }
    }
    fn preview_shapes(&mut self, ctx: &egui::Context, area: Rect) -> Vec<egui::Shape> {
        let view = self.preview_view(area);
        let own = self.preview_needs_own_tiles();
        if own {
            self.update_preview_tiles(ctx, area, view);
        } else if self.preview.textures.is_some() {
            // Release the second set of tiles as soon as it is not needed.
            self.preview.reset();
        }
        let refs = !self.preview.settings.hide_references;
        let mut shapes = Vec::new();
        match &self.preview.textures {
            Some(textures) if own => {
                shapes.extend(textures.backdrop_shapes(area, view));
                if refs {
                    shapes.extend(self.reference_layers.shapes(view, false));
                }
                shapes.extend(textures.tile_shapes(area, view));
            }
            _ => {
                shapes.extend(self.texture.backdrop_shapes(area, view));
                if refs {
                    shapes.extend(self.reference_layers.shapes(view, false));
                }
                shapes.extend(self.texture.tile_shapes(area, view));
            }
        }
        if refs {
            shapes.extend(self.reference_layers.shapes(view, true));
            shapes.extend(
                self.reference_margin_shapes_with(view, |id| self.preview.hidden.contains(&id)),
            );
        }
        if self.preview.settings.show_pointer
            && let Some(point) = self.preview.pointer
        {
            let at = view.to_screen(point);
            let radius = (self.brush.size * view.zoom / 2.0).clamp(3.0, 400.0);
            shapes.push(egui::Shape::circle_stroke(
                at,
                radius,
                egui::Stroke::new(3.0_f32, Color32::from_black_alpha(140)),
            ));
            shapes.push(egui::Shape::circle_stroke(
                at,
                radius,
                egui::Stroke::new(1.5_f32, Color32::WHITE),
            ));
        }
        shapes
    }
    /// Track the pen over the main canvas for the preview's pointer.
    pub(crate) fn note_preview_pointer(&mut self, ctx: &egui::Context) {
        if !self.preview.open || !self.preview.settings.show_pointer {
            self.preview.pointer = None;
            return;
        }
        let area = self.canvas_area;
        let view = self.view(area);
        self.preview.pointer = ctx
            .pointer_hover_pos()
            .filter(|p| area.contains(*p))
            .map(|p| view.to_doc(p));
    }
    pub(crate) fn preview_window(&mut self, ctx: &egui::Context) {
        if !self.preview.open {
            if self.preview.textures.is_some() {
                self.preview.reset();
            }
            return;
        }
        let id = egui::ViewportId::from_hash_of("dudel artwork preview");
        let title = format!("{} — Dudel preview", self.project_name());
        let builder = egui::ViewportBuilder::default()
            .with_title(title)
            .with_inner_size([960.0, 600.0])
            .with_min_inner_size([200.0, 150.0])
            .with_window_level(if self.preview.settings.always_on_top {
                egui::WindowLevel::AlwaysOnTop
            } else {
                egui::WindowLevel::Normal
            });
        ctx.show_viewport_immediate(id, builder, |ctx, _class| {
            if ctx.input(|i| i.viewport().close_requested()) {
                self.preview.open = false;
                return;
            }
            let toggle_fullscreen = ctx.input(|i| {
                i.key_pressed(egui::Key::F11)
                    || (self.preview.fullscreen && i.key_pressed(egui::Key::Escape))
            });
            let background = match self.preview.settings.background {
                Background::Theme => crate::color_wheel::color32(self.theme.canvas_background),
                Background::Black => Color32::BLACK,
                Background::White => Color32::WHITE,
                Background::Green => Color32::from_rgb(0, 255, 0),
            };
            let mut fullscreen_clicked = false;
            egui::CentralPanel::default()
                .frame(egui::Frame::NONE.fill(background))
                .show(ctx, |ui| {
                    let area = ui.max_rect();
                    let response = ui.allocate_rect(area, egui::Sense::click());
                    let shapes = self.preview_shapes(ctx, area);
                    ui.painter_at(area).extend(shapes);
                    if response.double_clicked() {
                        fullscreen_clicked = true;
                    }
                    response.context_menu(|ui| {
                        self.preview_menu(ui);
                    });
                });
            self.capture_preview(ctx);
            if toggle_fullscreen || fullscreen_clicked {
                self.preview.fullscreen = !self.preview.fullscreen;
                ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(self.preview.fullscreen));
            }
        });
    }
    /// UI capture mode keeps the preview up for a few seconds so an
    /// external screen grab can record it (eframe cannot screenshot
    /// immediate viewports), then lets the main capture finish.
    fn capture_preview(&mut self, ctx: &egui::Context) {
        if self.preview.capture.is_none() {
            return;
        }
        let started = *self
            .preview
            .capture_started
            .get_or_insert_with(std::time::Instant::now);
        if started.elapsed() >= std::time::Duration::from_secs(4) {
            self.preview.capture = None;
        }
        ctx.request_repaint();
    }
    /// Options shown on right-click in the preview and under View.
    pub(crate) fn preview_menu(&mut self, ui: &mut egui::Ui) {
        ui.set_min_width(220.0);
        let settings = &mut self.preview.settings;
        ui.label(egui::RichText::new("Framing").small().weak());
        ui.radio_value(&mut settings.framing, Framing::Fit, "Fit whole canvas");
        ui.radio_value(
            &mut settings.framing,
            Framing::FollowView,
            "Follow main view",
        );
        ui.separator();
        ui.label(egui::RichText::new("Background").small().weak());
        for background in Background::ALL {
            ui.radio_value(&mut settings.background, background, background.name());
        }
        ui.separator();
        ui.checkbox(&mut settings.hide_references, "Hide reference layers");
        ui.checkbox(&mut settings.show_pointer, "Show pen pointer");
        ui.checkbox(&mut settings.always_on_top, "Always on top");
        ui.menu_button("Hide layers", |ui| {
            egui::ScrollArea::vertical()
                .max_height(360.0)
                .show(ui, |ui| {
                    for index in (0..self.doc.layers.len()).rev() {
                        let layer = &self.doc.layers[index];
                        let mut shown = !self.preview.hidden.contains(&layer.id);
                        let name = format!("{}{}", "   ".repeat(layer.depth as usize), layer.name);
                        if ui.checkbox(&mut shown, name).changed() {
                            self.preview.toggle_hidden(layer.id);
                        }
                    }
                });
            if !self.preview.hidden.is_empty() && ui.button("Show all layers").clicked() {
                self.preview.hidden.clear();
            }
        });
        ui.separator();
        ui.label(
            egui::RichText::new("Double-click or F11 for full screen")
                .small()
                .weak(),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reference_board_margins_and_folder_exclusions_match_the_preview() {
        let ctx = egui::Context::default();
        let mut app = Dudel {
            doc: crate::document::Document::new(8, 8),
            ..Default::default()
        };
        app.doc
            .layers
            .push(crate::document::Layer::new_folder("References".into()));
        app.insert_reference(
            "Hair".into(),
            crate::reference_layers::Image {
                width: 2,
                height: 2,
                rgba: [255, 0, 0, 255].repeat(4),
            },
        );
        let id = app.reference_layers.layers[0].id;
        app.place_reference(id, 1, 1);
        app.reference_layers.layers[0].center = [0.0, 1.0];
        app.reference_layers.layers[0].scale = 1.0;
        let area = Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(200.0, 200.0));
        let _ = ctx.run(Default::default(), |ctx| {
            app.prepare_reference_layers(ctx);
            let texture = app.reference_layers.texture(id).unwrap().id();
            let contains_reference = |shapes: &[egui::Shape]| {
                shapes
                    .iter()
                    .any(|s| matches!(s, egui::Shape::Mesh(mesh) if mesh.texture_id == texture))
            };
            assert!(!contains_reference(&app.preview_shapes(ctx, area)));
            app.preview.settings.hide_references = false;
            assert!(contains_reference(&app.preview_shapes(ctx, area)));
            let folder = app.doc.layers[1].id;
            app.preview.hidden.insert(folder);
            assert!(!contains_reference(&app.preview_shapes(ctx, area)));
            let doc = app.preview_document();
            assert!(
                doc.composite_bounds_checked([0, 0, 8, 8], true, || Ok(()))
                    .unwrap()
                    .iter()
                    .all(|v| *v == 0)
            );
            assert!(app.doc.layers[1].visible);
            assert!(app.reference_layers.layers[0].visible);
        });
    }
    #[test]
    fn preview_uses_main_tiles_until_something_is_left_out_and_hides_chosen_layers() {
        let mut app = Dudel {
            doc: crate::document::Document::new(32, 32),
            ..Default::default()
        };
        app.preview.open = true;
        assert!(!app.preview_needs_own_tiles());
        let id = app.doc.layers[0].id;
        app.preview.toggle_hidden(id);
        assert!(app.preview_needs_own_tiles());
        assert!(!app.preview_document().layers[0].visible);
        // The artwork itself is untouched.
        assert!(app.doc.layers[0].visible);
        // A layer already hidden in the artwork needs no second composite.
        app.doc.layers[0].visible = false;
        assert!(!app.preview_needs_own_tiles());
        app.preview.toggle_hidden(id);
        assert!(app.preview.hidden.is_empty());
        // A large preview cannot inherit the main view's offscreen resolution.
        app.doc = crate::document::Document::new(2048, 2048);
        assert!(app.preview_needs_own_tiles());
    }
    #[test]
    fn fit_framing_centers_the_whole_canvas_and_follow_scales_the_main_view() {
        let mut app = Dudel {
            doc: crate::document::Document::new(200, 100),
            ..Default::default()
        };
        let area = Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(416.0, 416.0));
        let fit = app.preview_view(area);
        assert_eq!(fit.zoom, 2.0);
        assert_eq!(fit.to_screen(egui::pos2(0.0, 0.0)), egui::pos2(8.0, 108.0));
        app.preview.settings.framing = Framing::FollowView;
        app.canvas_area = Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(832.0, 832.0));
        app.zoom = 3.0;
        app.pan = egui::vec2(10.0, 0.0);
        app.rotation = 0.5;
        let follow = app.preview_view(area);
        assert_eq!(follow.zoom, 1.5);
        assert_eq!(follow.origin, egui::pos2(213.0, 208.0));
        assert_eq!(follow.rotation, 0.5);
    }
}
