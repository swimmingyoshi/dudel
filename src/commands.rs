//! Shared command identities, labels, defaults, availability and execution.
use crate::{Dudel, egui, tools::Tool, vector_pick::LineDrag};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Action {
    Search,
    Merge,
    Ungroup,
    CloseTab,
    ZoomIn,
    ZoomOut,
    SwapColors,
    SmallerBrush,
    LargerBrush,
    RotateLeft,
    RotateRight,
    ResetRotation,
    New,
    Open,
    Save,
    SaveAs,
    ExportPng(bool),
    ExportAgain,
    ExportPsd,
    ExportLive2d,
    ExportSvg,
    Undo,
    Redo,
    NewLayer,
    NewVector,
    NewText,
    NewFolder,
    Duplicate,
    Group,
    Effects,
    Material,
    SelectAll,
    Deselect,
    InvertSelection,
    Transform,
    FlipView,
    FlipArtwork,
    Fit,
    ActualSize,
    Transparency,
    Panels,
    RevealLayer,
    Settings,
    Shortcuts,
    BrushEditor,
    SmoothLines,
    RecolorLines,
    DeleteLines,
    DeselectLines,
    LineDrag(LineDrag),
    Tool(Tool),
}

pub(crate) struct Command {
    pub id: &'static str,
    pub label: &'static str,
    pub defaults: &'static str,
    pub aliases: &'static str,
    pub action: Action,
}

macro_rules! command {
    ($id:literal, $label:literal, $shortcut:literal, $aliases:literal, $action:expr) => {
        Command {
            id: $id,
            label: $label,
            defaults: $shortcut,
            aliases: $aliases,
            action: $action,
        }
    };
}

pub(crate) const COMMANDS: &[Command] = &[
    command!(
        "file.new",
        "New canvas…",
        "Ctrl+N",
        "file document create",
        Action::New
    ),
    command!(
        "file.open",
        "Open…",
        "Ctrl+O",
        "file document project",
        Action::Open
    ),
    command!(
        "file.save",
        "Save",
        "Ctrl+S",
        "file document project",
        Action::Save
    ),
    command!(
        "file.save_as",
        "Save as…",
        "Ctrl+Shift+S",
        "file document project copy",
        Action::SaveAs
    ),
    command!(
        "export.png",
        "Export image…",
        "Ctrl+Shift+E",
        "file image png jpeg jpg webp bmp gif opaque background quality",
        Action::ExportPng(false)
    ),
    command!(
        "export.transparent_png",
        "Export image with transparency…",
        "",
        "file image alpha no background",
        Action::ExportPng(true)
    ),
    command!(
        "export.again",
        "Export again",
        "Ctrl+Shift+R",
        "file image repeat same last overwrite",
        Action::ExportAgain
    ),
    command!(
        "export.psd",
        "Export Photoshop document…",
        "",
        "file psd editable layers",
        Action::ExportPsd
    ),
    command!(
        "export.live2d",
        "Export PSD for Live2D…",
        "",
        "file photoshop effects as layers",
        Action::ExportLive2d
    ),
    command!(
        "export.svg",
        "Export vector lines as SVG…",
        "",
        "file vector paths illustrator inkscape",
        Action::ExportSvg
    ),
    command!("edit.undo", "Undo", "Ctrl+Z", "edit revert", Action::Undo),
    command!(
        "edit.redo",
        "Redo",
        "Ctrl+Shift+Z;Ctrl+Y",
        "edit repeat",
        Action::Redo
    ),
    command!(
        "layer.new",
        "New raster layer",
        "Ctrl+Shift+N",
        "add paint pixels",
        Action::NewLayer
    ),
    command!(
        "layer.new_vector",
        "New vector layer",
        "",
        "add editable lines strokes",
        Action::NewVector
    ),
    command!(
        "layer.new_text",
        "New text layer",
        "",
        "add type words lettering font",
        Action::NewText
    ),
    command!(
        "layer.new_folder",
        "New folder",
        "",
        "layer organize",
        Action::NewFolder
    ),
    command!(
        "layer.duplicate",
        "Duplicate layer",
        "Ctrl+J",
        "copy",
        Action::Duplicate
    ),
    command!(
        "layer.group",
        "Put layer in new folder",
        "Ctrl+G",
        "group organize",
        Action::Group
    ),
    command!(
        "layer.effects",
        "Layer effects…",
        "",
        "style shadow glow outline satin",
        Action::Effects
    ),
    command!(
        "layer.material",
        "Layer material…",
        "",
        "watercolor dry media pastel charcoal thick paint impasto oil relief light",
        Action::Material
    ),
    command!(
        "selection.all",
        "Select all",
        "Ctrl+A",
        "selection",
        Action::SelectAll
    ),
    command!(
        "selection.none",
        "Deselect",
        "Ctrl+D",
        "selection clear remove cancel",
        Action::Deselect
    ),
    command!(
        "selection.invert",
        "Invert selection",
        "Ctrl+Shift+I",
        "selection reverse",
        Action::InvertSelection
    ),
    command!(
        "edit.transform",
        "Free transform / apply transform",
        "Ctrl+T",
        "edit scale rotate resize",
        Action::Transform
    ),
    command!(
        "view.flip",
        "Flip view horizontally",
        "H",
        "mirror canvas display",
        Action::FlipView
    ),
    command!(
        "layer.flip",
        "Flip artwork horizontally",
        "",
        "mirror layer selection pixels",
        Action::FlipArtwork
    ),
    command!(
        "view.fit",
        "Fit canvas in view",
        "Ctrl+0",
        "zoom reset",
        Action::Fit
    ),
    command!(
        "view.actual_size",
        "Actual size",
        "Ctrl+1",
        "zoom 100 percent",
        Action::ActualSize
    ),
    command!(
        "view.transparency",
        "Toggle transparency checkerboard",
        "Ctrl+Shift+T",
        "view alpha background",
        Action::Transparency
    ),
    command!(
        "view.panels",
        "Show / hide panels",
        "Tab",
        "workspace distraction free",
        Action::Panels
    ),
    command!(
        "layer.reveal",
        "Reveal active layer",
        "",
        "find selected layer search filters folders",
        Action::RevealLayer
    ),
    command!(
        "app.settings",
        "Settings…",
        "",
        "preferences options",
        Action::Settings
    ),
    command!(
        "app.shortcuts",
        "Keyboard shortcuts",
        "",
        "help keys bindings",
        Action::Shortcuts
    ),
    command!(
        "brush.editor",
        "Brush Editor…",
        "",
        "brush settings preset pressure",
        Action::BrushEditor
    ),
    command!(
        "lines.move",
        "Drag vector lines to move them",
        "",
        "pointer pick line mode",
        Action::LineDrag(LineDrag::Move)
    ),
    command!(
        "lines.reshape",
        "Drag vector lines to reshape them",
        "",
        "pointer pick line bend mode",
        Action::LineDrag(LineDrag::Reshape)
    ),
    command!(
        "lines.smooth",
        "Smooth picked lines",
        "",
        "vector pointer wobble simplify",
        Action::SmoothLines
    ),
    command!(
        "lines.recolor",
        "Recolor picked lines",
        "",
        "vector pointer paint color",
        Action::RecolorLines
    ),
    command!(
        "lines.delete",
        "Delete picked lines",
        "",
        "vector pointer remove erase",
        Action::DeleteLines
    ),
    command!(
        "lines.deselect",
        "Deselect lines",
        "",
        "vector pointer pick clear",
        Action::DeselectLines
    ),
    command!(
        "tool.brush",
        "Brush tool",
        "B",
        "draw paint pen pencil",
        Action::Tool(Tool::Brush)
    ),
    command!(
        "tool.eraser",
        "Eraser tool",
        "E",
        "erase remove",
        Action::Tool(Tool::Eraser)
    ),
    command!(
        "tool.fill",
        "Fill tool",
        "G",
        "bucket flood",
        Action::Tool(Tool::Fill)
    ),
    command!(
        "tool.picker",
        "Color picker tool",
        "I",
        "eyedropper sample",
        Action::Tool(Tool::Picker)
    ),
    command!(
        "tool.select",
        "Selection tool",
        "M",
        "select rectangle wand",
        Action::Tool(Tool::Select)
    ),
    command!(
        "tool.lasso",
        "Lasso tool",
        "L",
        "shape fill erase",
        Action::Tool(Tool::Lasso)
    ),
    command!(
        "tool.move",
        "Move tool",
        "V",
        "position layer selection",
        Action::Tool(Tool::Move)
    ),
    command!(
        "tool.text",
        "Text tool",
        "T",
        "type words lettering font",
        Action::Tool(Tool::Text)
    ),
    command!(
        "tool.pointer",
        "Pointer tool",
        "O",
        "vector line pick edit guide",
        Action::Tool(Tool::Pointer)
    ),
    command!(
        "app.search",
        "Find a command…",
        "Ctrl+K;Ctrl+Shift+P",
        "commands palette",
        Action::Search
    ),
    command!(
        "layer.merge",
        "Merge down / merge folder",
        "Ctrl+E",
        "flatten",
        Action::Merge
    ),
    command!(
        "layer.ungroup",
        "Ungroup folder",
        "Ctrl+Shift+G",
        "organize",
        Action::Ungroup
    ),
    command!(
        "file.close_tab",
        "Close project tab",
        "Ctrl+W",
        "document",
        Action::CloseTab
    ),
    command!(
        "view.zoom_in",
        "Zoom in",
        "Ctrl+Equals;Ctrl+Plus;Ctrl+Shift+Plus",
        "magnify",
        Action::ZoomIn
    ),
    command!(
        "view.zoom_out",
        "Zoom out",
        "Ctrl+Minus",
        "reduce",
        Action::ZoomOut
    ),
    command!(
        "paint.swap_colors",
        "Swap paint colors",
        "X",
        "foreground background mask",
        Action::SwapColors
    ),
    command!(
        "brush.smaller",
        "Decrease brush size",
        "OpenBracket",
        "eraser",
        Action::SmallerBrush
    ),
    command!(
        "brush.larger",
        "Increase brush size",
        "CloseBracket",
        "eraser",
        Action::LargerBrush
    ),
    command!(
        "view.rotate_left",
        "Rotate view left 15°",
        "4",
        "canvas",
        Action::RotateLeft
    ),
    command!(
        "view.rotate_right",
        "Rotate view right 15°",
        "6",
        "canvas",
        Action::RotateRight
    ),
    command!(
        "view.reset_rotation",
        "Reset rotation",
        "5",
        "canvas",
        Action::ResetRotation
    ),
];

impl Dudel {
    pub(crate) fn command_hint(&self, action: Action, description: &str) -> String {
        let binding = self.shortcuts.label(action);
        if binding.is_empty() {
            description.into()
        } else {
            format!("{description} · {binding}")
        }
    }
    pub(crate) fn command_menu_item(&mut self, ui: &mut egui::Ui, action: Action) {
        let command = get(action);
        let reason = self.command_disabled(action);
        let response = ui.add_enabled(
            reason.is_none(),
            egui::Button::new(command.label)
                .shortcut_text(self.shortcuts.label(action))
                .selected(match action {
                    Action::Panels => self.show_panels,
                    Action::Transparency => self.show_transparency,
                    Action::FlipView => self.mirror,
                    _ => false,
                }),
        );
        if response.clicked() {
            ui.close();
            self.execute_command(action);
        }
        if let Some(reason) = reason {
            response.on_disabled_hover_text(reason);
        }
    }
    pub(crate) fn command_disabled(&self, action: Action) -> Option<&'static str> {
        match action {
            Action::Merge
                if !self.input_is_folder()
                    && !self.render_queue.busy()
                    && self.input_layer().is_none_or(|i| {
                        i == 0
                            || self.doc.layers[i - 1].folder
                            || self.doc.layers[i - 1].depth != self.doc.layers[i].depth
                    }) =>
            {
                Some("No layer below to merge with")
            }
            Action::Ungroup if !self.input_is_folder() => Some("Select a folder first"),
            Action::Undo if !self.history.can_undo() && !self.render_queue.busy() => {
                Some("Nothing to undo")
            }
            Action::Redo if !self.history.can_redo() && !self.render_queue.busy() => {
                Some("Nothing to redo")
            }
            Action::NewLayer | Action::NewVector | Action::NewText if !self.doc.has_room(1, 1) => {
                Some("Layer limit reached")
            }
            Action::NewFolder | Action::Group if !self.doc.has_room(0, 1) => {
                Some("Layer limit reached")
            }
            Action::Effects | Action::Material | Action::Duplicate | Action::Group
                if self.input_target().is_none() =>
            {
                Some("Select a layer first")
            }
            Action::Deselect | Action::InvertSelection
                if !self.render_queue.busy()
                    && self.doc.selection.is_none()
                    && self.doc.selection_mask.is_none() =>
            {
                Some("No selection")
            }
            Action::SmoothLines
            | Action::RecolorLines
            | Action::DeleteLines
            | Action::DeselectLines
                if self.picked_lines().is_empty() =>
            {
                Some("Pick vector lines with the Pointer first")
            }
            Action::ExportSvg if !self.doc.layers.iter().any(|l| l.vector.is_some()) => {
                Some("No vector layers to export")
            }
            Action::LineDrag(_) if self.vector_target().is_none() => {
                Some("Select a vector layer first")
            }
            _ => None,
        }
    }

    pub(crate) fn execute_command(&mut self, action: Action) {
        if let Some(reason) = self.command_disabled(action) {
            self.status = reason.into();
            return;
        }
        match action {
            Action::Search => self.open_command_search(),
            Action::Merge => self.merge_down(),
            Action::Ungroup => self.ungroup_folder(),
            Action::CloseTab => self.close_tab(self.tabs.active),
            Action::ZoomIn => self.step_zoom(1.25),
            Action::ZoomOut => self.step_zoom(0.8),
            Action::SwapColors => self.swap_paint_colors(),
            Action::SmallerBrush => self.resize_brush(0.85),
            Action::LargerBrush => self.resize_brush(1.0 / 0.85),
            Action::RotateLeft => self.rotate_view(-15f32.to_radians()),
            Action::RotateRight => self.rotate_view(15f32.to_radians()),
            Action::ResetRotation => self.reset_rotation(),
            Action::New => self.open_new_canvas(),
            Action::Open => self.open(),
            Action::Save => self.save(false),
            Action::SaveAs => self.save(true),
            Action::ExportPng(transparent) => self.open_export_window(transparent),
            Action::ExportAgain => self.export_again(),
            Action::ExportPsd => self.export_psd(crate::psd_export::Effects::Editable),
            Action::ExportLive2d => self.export_psd(crate::psd_export::Effects::AsLayers),
            Action::ExportSvg => self.export_svg(),
            Action::Undo => self.undo(),
            Action::Redo => self.redo(),
            Action::NewLayer => self.new_layer(),
            Action::NewVector => self.create_vector_layer(),
            Action::NewText => {
                self.select_tool(Tool::Text);
                self.status = "Click the canvas to type, or drag a box".into();
            }
            Action::NewFolder => self.new_folder(),
            Action::Duplicate => self.duplicate_layer(),
            Action::Group => self.group_into_folder(),
            Action::Effects => self.effects_window_open = true,
            Action::Material => self.open_material_dialog(),
            Action::SelectAll => self.select_all(),
            Action::Deselect => self.set_selection(None),
            Action::InvertSelection => self.invert_selection(),
            Action::Transform => self.toggle_free_transform(),
            Action::FlipView => self.toggle_flip(),
            Action::FlipArtwork => {
                self.transform_active(crate::transform::Transform::FlipHorizontal)
            }
            Action::Fit => self.fit_view(),
            Action::ActualSize => self.actual_size(),
            Action::Transparency => self.set_show_transparency(!self.show_transparency),
            Action::Panels => self.show_panels = !self.show_panels,
            Action::RevealLayer => self.reveal_active_layer(),
            Action::Settings => self.settings_open = true,
            Action::Shortcuts => self.shortcuts_window = true,
            Action::BrushEditor => self.brush_studio = true,
            Action::SmoothLines => self.smooth_picked_lines(),
            Action::RecolorLines => self.recolor_picked_lines(),
            Action::DeleteLines => self.delete_picked_lines(),
            Action::DeselectLines => self.deselect_lines(),
            Action::LineDrag(drag) => {
                self.select_tool(Tool::Pointer);
                self.line_drag = drag;
            }
            Action::Tool(tool) => self.select_tool(tool),
        }
    }
}

pub(crate) fn get(action: Action) -> &'static Command {
    COMMANDS
        .iter()
        .find(|c| c.action == action)
        .expect("registered command")
}
