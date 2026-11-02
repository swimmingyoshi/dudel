//! Serializable document operations, applied atomically by the queue/controller.
//! Layer indices and Active refer to the stack at execution. Identity targets are
//! session-local: serialized command streams must not be reused across processes
//! or treated as persistent project layer references.
use crate::{
    document::{Document, Layer},
    selection::Selection,
    transform::Transform,
};

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub enum Target {
    Active,
    Index(usize),
    Identity(crate::document::LayerId),
}
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub enum Property {
    Name(String),
    Visible(bool),
    Opacity(f32),
    Blend(crate::blend::BlendMode),
    Clipped(bool),
    Locked(bool),
    AlphaLocked(bool),
    Collapsed(bool),
    /// The layer's whole effect list.
    Effects(Vec<crate::layer_effects::LayerEffect>),
    /// What an adjustment layer does (it stays an adjustment layer).
    Adjustment(crate::adjustment::Adjustment),
}
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub enum Edit {
    MaskTransform {
        target: crate::document::LayerId,
        mask: crate::document::LayerId,
        operation: crate::mask_transform::Operation,
    },
    LayerMask {
        target: crate::document::LayerId,
        command: crate::layer_mask_edits::Command,
    },
    SelectLayer(usize),
    SelectTarget(Target),
    Selection(Option<Selection>),
    SelectionMask(Option<crate::selection_mask::Mask>),
    SelectionRectangle {
        rectangle: Selection,
        mode: crate::selection_mask::Combine,
    },
    SelectionPolygon {
        points: Vec<[f64; 2]>,
        mode: crate::selection_mask::Combine,
    },
    SelectionWand {
        point: [usize; 2],
        settings: crate::wand::Settings,
        mode: crate::selection_mask::Combine,
    },
    SelectionCoverage {
        target: crate::document::LayerId,
        paint_target: crate::paint_target::Target,
    },
    LassoPaint {
        points: Vec<[f64; 2]>,
        settings: crate::lasso_paint::Settings,
    },
    FeatherSelection(u8),
    InvertSelection,
    Property {
        target: Target,
        property: Property,
    },
    NewLayer,
    NewLayerWithId(crate::document::LayerId),
    /// A new adjustment layer above the target, with this identity.
    NewAdjustment {
        id: crate::document::LayerId,
        settings: crate::adjustment::Settings,
    },
    NewFolder,
    FolderTarget {
        target: crate::document::LayerId,
        folder: crate::document::LayerId,
        group: bool,
    },
    Group,
    Ungroup,
    UngroupTarget(crate::document::LayerId),
    Duplicate,
    DuplicateTarget {
        target: crate::document::LayerId,
        copies: Vec<crate::document::LayerId>,
    },
    Delete,
    DeleteTarget(crate::document::LayerId),
    Merge,
    MergeTarget {
        target: crate::document::LayerId,
        folder_result: Option<crate::document::LayerId>,
    },
    Step(bool),
    StepTarget {
        target: crate::document::LayerId,
        up: bool,
    },
    Reorder {
        from: usize,
        gap: usize,
        depth: u8,
    },
    /// Insert immediately below the captured destination entry, or at the top
    /// when None. Both entries are resolved in the execution-time stack.
    ReorderTarget {
        from: crate::document::LayerId,
        before: Option<crate::document::LayerId>,
        depth: u8,
    },
    Transform(Transform),
    MoveTarget {
        target: crate::document::LayerId,
        offset: [i64; 2],
    },
    TransformTarget {
        target: crate::document::LayerId,
        transform: Transform,
    },
    AffineTarget {
        target: crate::document::LayerId,
        matrix: crate::affine::Matrix,
        sampling: crate::affine::Sampling,
    },
    AffineSettingsTarget {
        target: crate::document::LayerId,
        /// Boxed: a mesh warp makes settings large, and every queued edit
        /// would otherwise carry that size.
        settings: Box<crate::affine::Settings>,
    },
    Rotate(u8),
    CanvasSize(crate::canvas_size::Settings),
    CropCanvas,
    FillTarget {
        target: crate::document::LayerId,
        source: crate::fill::Source,
        point: [usize; 2],
        color: [u8; 3],
        opacity: f32,
        tolerance: u8,
        #[serde(default)]
        options: crate::fill::Options,
    },
    Clear {
        whole_layer: bool,
    },
    ClearTarget {
        target: crate::document::LayerId,
        whole_layer: bool,
    },
}
#[derive(Clone, Copy, Debug, Default, serde::Serialize)]
pub struct Outcome {
    pub changed: bool,
    pub undoable: bool,
    pub redraw: bool,
}
impl Document {
    pub fn insert_layer_above(&mut self) -> Result<(), String> {
        if !self.has_room(1, 1) {
            return Err("Document layer capacity reached".into());
        }
        let active = &self.layers[self.active];
        let (index, depth) = if active.folder && !active.collapsed {
            (self.active, active.depth + 1)
        } else {
            (self.block(self.active).end, active.depth)
        };
        self.add_layer();
        let mut layer = self.layers.pop().unwrap();
        layer.depth = depth;
        self.layers.insert(index, layer);
        self.active = index;
        self.validate_structure()
    }
}
impl Edit {
    pub fn resolve(target: &Target, doc: &Document) -> Result<usize, String> {
        match target {
            Target::Active => Ok(doc.active),
            Target::Index(index) if *index < doc.layers.len() => Ok(*index),
            Target::Identity(id) => doc
                .layers
                .iter()
                .position(|layer| layer.id == *id)
                .ok_or_else(|| "Queued layer no longer exists".into()),
            _ => Err("Layer target is outside the stack".into()),
        }
    }
    pub fn payload_bytes(&self) -> usize {
        if matches!(
            self,
            Self::MaskTransform {
                operation: crate::mask_transform::Operation::Affine(_),
                ..
            }
        ) {
            return std::mem::size_of::<crate::affine::Settings>();
        }
        if let Self::SelectionPolygon { points, .. } | Self::LassoPaint { points, .. } = self {
            return points.capacity() * std::mem::size_of::<[f64; 2]>();
        }
        if let Self::SelectionMask(Some(mask)) = self {
            return mask.allocation().1;
        }
        if let Self::DuplicateTarget { copies, .. } = self {
            return copies.capacity() * std::mem::size_of::<crate::document::LayerId>();
        }
        if let Self::Property {
            property: Property::Name(name),
            ..
        } = self
        {
            name.capacity()
        } else {
            0
        }
    }
    pub fn validate_arguments(&self) -> Result<(), String> {
        match self {
            Self::MaskTransform { operation, .. } => operation.validate(),
            Self::LayerMask {
                command: crate::layer_mask_edits::Command::Fill { request, .. },
                ..
            } => request.validate(),
            Self::LayerMask { .. } => Ok(()),
            Self::CanvasSize(settings) => settings.validate(),
            Self::AffineTarget { matrix, .. } => matrix.validate(),
            Self::AffineSettingsTarget { settings, .. } => settings.validate(),
            Self::SelectionPolygon { points, .. } | Self::LassoPaint { points, .. }
                if !(3..=16_384).contains(&points.len())
                    || points
                        .iter()
                        .flatten()
                        .any(|v| !v.is_finite() || v.abs() > 1.0e9) =>
            {
                Err("Invalid or oversized lasso path".into())
            }
            Self::FeatherSelection(radius) if *radius > 64 => {
                Err("Selection feather radius exceeds 64 pixels".into())
            }
            Self::FillTarget { opacity, .. }
                if !opacity.is_finite() || !(0.0..=1.0).contains(opacity) =>
            {
                Err("Fill opacity must be finite and between zero and one".into())
            }
            Self::FillTarget { options, .. } => options.validate(),
            Self::LassoPaint { settings, .. }
                if !settings.opacity.is_finite() || !(0.0..=1.0).contains(&settings.opacity) =>
            {
                Err("Invalid lasso opacity".into())
            }
            Self::DuplicateTarget { copies, .. }
                if copies.is_empty() || copies.len() > crate::document::MAX_ENTRIES =>
            {
                Err("Invalid duplicate identity count".into())
            }
            Self::Property {
                property: Property::Name(name),
                ..
            } if name.len() > 4096 => Err("Layer name exceeds 4096 UTF-8 bytes".into()),
            Self::Property {
                property: Property::Opacity(value),
                ..
            } if !value.is_finite() || !(0.0..=1.0).contains(value) => {
                Err("Opacity must be finite and between zero and one".into())
            }
            Self::Property {
                property: Property::Adjustment(adjustment),
                ..
            } => adjustment.validate(),
            Self::NewAdjustment { settings, .. } => {
                crate::adjustment::Adjustment::try_from(settings.clone()).map(|_| ())
            }
            Self::Property {
                property: Property::Effects(effects),
                ..
            } => {
                if effects.len() > crate::document::MAX_EFFECTS {
                    return Err("Too many layer effects".into());
                }
                effects.iter().try_for_each(|e| e.validate())
            }
            _ => Ok(()),
        }
    }
    pub(crate) fn background_label(&self, masked: bool) -> Option<&'static str> {
        match self {
            Self::MaskTransform { .. } => Some("Mask transform"),
            Self::LayerMask { .. } => Some("Layer mask"),
            Self::Merge | Self::MergeTarget { .. } if masked => Some("Merge"),
            Self::CanvasSize(_) | Self::CropCanvas => Some("Canvas bounds"),
            Self::Rotate(turns) if turns % 4 != 0 => Some("Canvas bounds"),
            Self::LassoPaint { settings, .. } => Some(if settings.erase {
                "Lasso erase"
            } else {
                "Lasso fill"
            }),
            Self::FillTarget { .. } => Some("Fill"),
            Self::MoveTarget { .. } => Some("Move"),
            Self::AffineTarget { .. } | Self::AffineSettingsTarget { .. } => Some("Transform"),
            Self::SelectionRectangle { .. }
            | Self::SelectionPolygon { .. }
            | Self::SelectionWand { .. }
            | Self::SelectionCoverage { .. }
            | Self::FeatherSelection(_)
            | Self::InvertSelection => Some("Selection"),
            Self::Transform(_) | Self::TransformTarget { .. } if masked => Some("Transform"),
            _ => None,
        }
    }
    /// Work on a COW candidate. Errors and no-ops never mutate the source or
    /// create history; expensive raster operations are still synchronous.
    pub fn prepare(&self, source: &Document) -> Result<(Document, Outcome), String> {
        self.prepare_control(source, None)
    }
    pub(crate) fn prepare_control(
        &self,
        source: &Document,
        cancel: Option<&std::sync::atomic::AtomicBool>,
    ) -> Result<(Document, Outcome), String> {
        crate::fill::check_cancel(cancel)?;
        self.validate_arguments()?;
        source.validate()?;
        let mut doc = source.clone();
        let mut outcome = Outcome {
            changed: true,
            undoable: true,
            redraw: true,
        };
        match self {
            Self::MaskTransform {
                target,
                mask,
                operation,
            } => {
                if Self::resolve(&Target::Identity(*target), &doc)? != doc.active {
                    return Err("Captured mask transform target is no longer active".into());
                }
                outcome.changed = crate::mask_transform::apply(&mut doc, *mask, operation, cancel)?;
            }
            Self::LayerMask { target, command } => {
                let index = Self::resolve(&Target::Identity(*target), &doc)?;
                outcome.changed =
                    crate::layer_mask_edits::apply(&mut doc, index, *command, cancel)?;
            }
            Self::CanvasSize(settings) => {
                (doc, outcome.changed) = crate::canvas_size::resize(source, *settings, cancel)?;
            }
            Self::CropCanvas => {
                (doc, outcome.changed) = crate::canvas_size::crop(source, cancel)?;
            }
            Self::SelectionRectangle { rectangle, mode } => {
                let incoming = crate::selection_mask::Mask::rectangle_control(
                    doc.width, doc.height, *rectangle, cancel,
                )?;
                apply_selection(&mut doc, incoming, *mode, cancel)?;
                outcome.changed = doc.selection != source.selection
                    || doc.selection_mask != source.selection_mask;
                outcome.redraw = false;
            }
            Self::SelectionPolygon { points, mode } => {
                let incoming =
                    crate::selection_mask::Mask::polygon(doc.width, doc.height, points, cancel)?;
                apply_selection(&mut doc, incoming, *mode, cancel)?;
                outcome.changed = doc.selection != source.selection
                    || doc.selection_mask != source.selection_mask;
                outcome.redraw = false;
            }
            Self::LassoPaint { points, settings } => {
                Self::resolve(&Target::Identity(settings.target), &doc)?;
                outcome.changed = crate::lasso_paint::apply(&mut doc, points, *settings, cancel)?;
            }
            Self::SelectionWand {
                point,
                settings,
                mode,
            } => {
                let incoming = crate::wand::mask(&doc, *point, *settings, cancel)?;
                apply_selection(&mut doc, incoming, *mode, cancel)?;
                outcome.changed = doc.selection != source.selection
                    || doc.selection_mask != source.selection_mask;
                outcome.redraw = false;
            }
            Self::SelectionCoverage {
                target,
                paint_target,
            } => {
                let index = Self::resolve(&Target::Identity(*target), &doc)?;
                let layer = &doc.layers[index];
                if !paint_target.valid(layer, [doc.width, doc.height]) {
                    return Err("Selection coverage target was removed or replaced".into());
                }
                let mut coverage = Vec::with_capacity(doc.width * doc.height);
                for y in 0..doc.height {
                    crate::fill::check_cancel(cancel)?;
                    for x in 0..doc.width {
                        let value = match paint_target {
                            crate::paint_target::Target::Mask(_) => {
                                layer.mask.as_ref().unwrap().coverage.at(x, y)
                            }
                            crate::paint_target::Target::Raster => {
                                let alpha = layer.pixels.rgba(y * doc.width + x)[3];
                                let mask = layer
                                    .mask
                                    .as_ref()
                                    .filter(|m| m.enabled)
                                    .map_or(255, |m| m.coverage.at(x, y));
                                ((u16::from(alpha) * u16::from(mask) + 127) / 255) as u8
                            }
                        };
                        coverage.push(value);
                    }
                }
                let incoming = crate::selection_mask::Mask::from_coverage_control(
                    doc.width, doc.height, coverage, cancel,
                )?;
                apply_selection(
                    &mut doc,
                    incoming,
                    crate::selection_mask::Combine::Replace,
                    cancel,
                )?;
                outcome.changed = doc.selection != source.selection
                    || doc.selection_mask != source.selection_mask;
                outcome.redraw = false;
            }
            Self::FeatherSelection(radius) => {
                let mask = current_selection(&doc, cancel)?;
                let mask = mask.feather(usize::from(*radius), cancel)?;
                doc.selection = mask.bounds();
                doc.selection_mask = Some(mask);
                outcome.changed = doc.selection_mask != source.selection_mask;
                outcome.redraw = false;
            }
            Self::InvertSelection => {
                let mask = current_selection(&doc, cancel)?.inverted(cancel)?;
                doc.selection = mask.bounds();
                doc.selection_mask = doc.selection.map(|_| mask);
                outcome.changed = doc.selection != source.selection
                    || doc.selection_mask != source.selection_mask;
                outcome.redraw = false;
            }
            Self::SelectLayer(index) => {
                if *index >= doc.layers.len() {
                    return Err("Layer target is outside the stack".into());
                }
                outcome.changed = doc.active != *index;
                outcome.undoable = false;
                outcome.redraw = false;
                doc.active = *index;
            }
            Self::SelectTarget(target) => {
                let index = Self::resolve(target, &doc)?;
                outcome.changed = doc.active != index;
                outcome.undoable = false;
                outcome.redraw = false;
                doc.active = index;
            }
            Self::Selection(selection) => {
                outcome.changed = doc.selection != *selection || doc.selection_mask.is_some();
                doc.selection = *selection;
                doc.selection_mask = None;
                outcome.redraw = false;
            }
            Self::SelectionMask(mask) => {
                if mask
                    .as_ref()
                    .is_some_and(|m| m.dimensions() != [doc.width, doc.height])
                {
                    return Err("Selection mask dimensions differ from canvas".into());
                }
                let bounds = mask.as_ref().and_then(|m| m.bounds());
                outcome.changed = doc.selection_mask != *mask || doc.selection != bounds;
                doc.selection = bounds;
                doc.selection_mask = mask.clone();
                outcome.redraw = false;
            }
            Self::Property { target, property } => {
                let index = Self::resolve(target, &doc)?;
                let layer = doc
                    .layers
                    .get_mut(index)
                    .ok_or("Layer target is outside the stack")?;
                outcome.changed = apply_property(layer, property)?;
                outcome.redraw = !matches!(
                    property,
                    Property::Name(_)
                        | Property::Collapsed(_)
                        | Property::Locked(_)
                        | Property::AlphaLocked(_)
                );
                if outcome.changed
                    && matches!(property, Property::Collapsed(true))
                    && doc.block(index).contains(&doc.active)
                {
                    doc.active = index;
                }
            }
            Self::NewLayer => doc.insert_layer_above()?,
            Self::NewLayerWithId(id) => {
                if doc.layers.iter().any(|layer| layer.id == *id) {
                    return Err("Layer identity is already present".into());
                }
                doc.insert_layer_above()?;
                doc.layers[doc.active].id = *id;
            }
            Self::NewAdjustment { id, settings } => {
                if doc.layers.iter().any(|layer| layer.id == *id) {
                    return Err("Layer identity is already present".into());
                }
                let adjustment = crate::adjustment::Adjustment::try_from(settings.clone())?;
                // "Curves 2" for the second Curves layer.
                let kind = adjustment.name();
                let count = doc
                    .layers
                    .iter()
                    .filter(|l| l.adjustment.as_ref().is_some_and(|a| a.name() == kind))
                    .count();
                doc.insert_layer_above()?;
                let layer = &mut doc.layers[doc.active];
                layer.id = *id;
                layer.name = format!("{kind} {}", count + 1);
                layer.adjustment = Some(adjustment);
            }
            Self::NewFolder => doc.add_folder()?,
            Self::FolderTarget {
                target,
                folder,
                group,
            } => {
                if Self::resolve(&Target::Identity(*target), &doc)? != doc.active {
                    return Err("Captured folder target is no longer active".into());
                }
                if doc.layers.iter().any(|layer| layer.id == *folder) {
                    return Err("Folder identity is already present".into());
                }
                if *group {
                    doc.group_active()?;
                } else {
                    doc.add_folder()?;
                }
                doc.layers[doc.active].id = *folder;
            }
            Self::Group => doc.group_active()?,
            Self::Ungroup => doc.ungroup_active()?,
            Self::UngroupTarget(target) => {
                if Self::resolve(&Target::Identity(*target), &doc)? != doc.active {
                    return Err("Captured ungroup target is no longer active".into());
                }
                doc.ungroup_active()?;
            }
            Self::Duplicate => doc.duplicate_active()?,
            Self::DuplicateTarget { target, copies } => {
                if Self::resolve(&Target::Identity(*target), &doc)? != doc.active {
                    return Err("Captured duplicate target is no longer active".into());
                }
                let block = doc.block(doc.active);
                if copies.len() != block.len()
                    || copies.iter().enumerate().any(|(index, id)| {
                        copies[..index].contains(id)
                            || doc.layers.iter().any(|layer| layer.id == *id)
                    })
                {
                    return Err("Invalid duplicate identities".into());
                }
                let start = block.end;
                doc.duplicate_active()?;
                for (layer, id) in doc.layers[start..start + copies.len()]
                    .iter_mut()
                    .zip(copies)
                {
                    layer.id = *id;
                    // A reserved copy identity also names its independent mask
                    // slot, allowing later FIFO input to capture it in advance.
                    if let Some(mask) = &mut layer.mask {
                        mask.id = *id;
                    }
                }
            }
            Self::Delete => doc.delete_active()?,
            Self::DeleteTarget(target) => {
                if Self::resolve(&Target::Identity(*target), &doc)? != doc.active {
                    return Err("Captured delete target is no longer active".into());
                }
                doc.delete_active()?;
            }
            Self::Merge => {
                if doc.layers[doc.active].folder {
                    doc.merge_folder()?
                } else {
                    doc.merge_down()?
                }
            }
            Self::MergeTarget {
                target,
                folder_result,
            } => {
                if Self::resolve(&Target::Identity(*target), &doc)? != doc.active {
                    return Err("Captured merge target is no longer active".into());
                }
                match (doc.layers[doc.active].folder, folder_result) {
                    (true, Some(id)) if !doc.layers.iter().any(|layer| layer.id == *id) => {
                        doc.merge_folder()?;
                        doc.layers[doc.active].id = *id;
                    }
                    (false, None) => doc.merge_down()?,
                    _ => return Err("Merge result does not match the captured layer kind".into()),
                }
            }
            Self::Step(up) => {
                let old = doc.active;
                doc.step_active(*up)?;
                outcome.changed = old != doc.active;
            }
            Self::FillTarget {
                target,
                source: reference,
                point,
                color,
                opacity,
                tolerance,
                options,
            } => {
                if Self::resolve(&Target::Identity(*target), &doc)? != doc.active {
                    return Err("Captured fill target is no longer active".into());
                }
                outcome.changed = if *options == crate::fill::Options::default() && cancel.is_none()
                {
                    if *reference == crate::fill::Source::Active {
                        doc.fill(point[0], point[1], *color, *opacity, *tolerance)?
                    } else {
                        doc.fill_using(
                            point[0], point[1], *color, *opacity, *tolerance, *reference,
                        )?
                    }
                } else {
                    doc.fill_request(
                        crate::fill::Request {
                            point: *point,
                            color: *color,
                            opacity: *opacity,
                            tolerance: *tolerance,
                            source: *reference,
                            options: *options,
                        },
                        cancel,
                    )?
                };
            }
            Self::StepTarget { target, up } => {
                if Self::resolve(&Target::Identity(*target), &doc)? != doc.active {
                    return Err("Captured step target is no longer active".into());
                }
                let old = doc.active;
                doc.step_active(*up)?;
                outcome.changed = old != doc.active;
            }
            Self::Reorder { from, gap, depth } => {
                if *from >= doc.layers.len() {
                    return Err("Layer target is outside the stack".into());
                }
                if *gap > doc.layers.len() || *depth > crate::document::MAX_DEPTH {
                    return Err("Layer destination is outside the supported stack".into());
                }
                let block = doc.block(*from);
                outcome.changed = !((*gap == block.start || *gap == block.end)
                    && *depth == doc.layers[*from].depth);
                if outcome.changed {
                    doc.move_block(*from, *gap, *depth)?;
                }
            }
            Self::ReorderTarget {
                from,
                before,
                depth,
            } => {
                let from = Self::resolve(&Target::Identity(*from), &doc)?;
                let gap = if let Some(before) = before {
                    Self::resolve(&Target::Identity(*before), &doc)?
                } else {
                    doc.layers.len()
                };
                return Self::Reorder {
                    from,
                    gap,
                    depth: *depth,
                }
                .prepare(source);
            }
            Self::Transform(transform) => {
                outcome.changed = doc.transform_active_control(*transform, cancel)?
            }
            Self::MoveTarget { target, offset } => {
                if Self::resolve(&Target::Identity(*target), &doc)? != doc.active {
                    return Err("Captured Move target is no longer active".into());
                }
                if let Some(blocker) = doc.paint_blocker() {
                    return Err(blocker.into());
                }
                if doc.layers[doc.active].alpha_locked {
                    return Err("Turn off Lock transparency to move pixels".into());
                }
                outcome.changed = crate::move_preview::apply(&mut doc, *offset, cancel)?;
            }
            Self::AffineTarget {
                target,
                matrix,
                sampling,
            } => {
                if Self::resolve(&Target::Identity(*target), &doc)? != doc.active {
                    return Err("Captured affine target is no longer active".into());
                }
                outcome.changed = crate::affine::apply(&mut doc, *matrix, *sampling, cancel)?;
            }
            Self::AffineSettingsTarget { target, settings } => {
                if Self::resolve(&Target::Identity(*target), &doc)? != doc.active {
                    return Err("Captured affine target is no longer active".into());
                }
                outcome.changed = crate::affine::apply_settings(
                    &mut doc,
                    **settings,
                    crate::paint_target::Target::Raster,
                    cancel,
                )?;
            }
            Self::TransformTarget { target, transform } => {
                if Self::resolve(&Target::Identity(*target), &doc)? != doc.active {
                    return Err("Captured transform target is no longer active".into());
                }
                outcome.changed = doc.transform_active_control(*transform, cancel)?;
            }
            Self::Rotate(turns) => {
                outcome.changed = turns % 4 != 0;
                for _ in 0..turns % 4 {
                    doc.canvas_frame = doc.canvas_frame.rotated(doc.height)?.into();
                    doc.rotate_canvas_clockwise_control(cancel)?;
                }
                if let Some(guides) = &mut doc.guides {
                    guides.map_canvas(*source.canvas_frame, *doc.canvas_frame);
                }
            }
            Self::Clear { whole_layer } => {
                let selection = doc.selection;
                let mask = doc.selection_mask.clone();
                if *whole_layer {
                    doc.selection = None;
                    doc.selection_mask = None;
                }
                outcome.changed = doc.clear_selected()?;
                doc.selection = selection;
                doc.selection_mask = mask;
            }
            Self::ClearTarget {
                target,
                whole_layer,
            } => {
                if Self::resolve(&Target::Identity(*target), &doc)? != doc.active {
                    return Err("Captured clear target is no longer active".into());
                }
                return Self::Clear {
                    whole_layer: *whole_layer,
                }
                .prepare(source);
            }
        }
        crate::fill::check_cancel(cancel)?;
        doc.validate()?;
        Ok((doc, outcome))
    }
}
fn current_selection(
    doc: &Document,
    cancel: Option<&std::sync::atomic::AtomicBool>,
) -> Result<crate::selection_mask::Mask, String> {
    if let Some(mask) = &doc.selection_mask {
        return Ok(mask.clone());
    }
    let rectangle = doc.selection.ok_or("No selection to modify")?;
    crate::selection_mask::Mask::rectangle_control(doc.width, doc.height, rectangle, cancel)
}
fn apply_selection(
    doc: &mut Document,
    incoming: crate::selection_mask::Mask,
    mode: crate::selection_mask::Combine,
    cancel: Option<&std::sync::atomic::AtomicBool>,
) -> Result<(), String> {
    use crate::selection_mask::{Combine, Mask};
    let mask = if mode == Combine::Replace {
        incoming
    } else {
        let current = if doc.selection.is_some() || doc.selection_mask.is_some() {
            current_selection(doc, cancel)?
        } else {
            Mask::from_coverage_control(
                doc.width,
                doc.height,
                vec![0; doc.width * doc.height],
                cancel,
            )?
        };
        current.combine(&incoming, mode, cancel)?
    };
    doc.selection = mask.bounds();
    doc.selection_mask = Some(mask);
    Ok(())
}
fn apply_property(layer: &mut Layer, property: &Property) -> Result<bool, String> {
    macro_rules! set {
        ($field:ident, $value:expr) => {{
            let changed = layer.$field != $value;
            layer.$field = $value;
            changed
        }};
    }
    Ok(match property {
        Property::Name(name) => {
            let changed = layer.name != *name;
            layer.name.clone_from(name);
            changed
        }
        Property::Visible(value) => set!(visible, *value),
        Property::Opacity(value) => set!(opacity, *value),
        Property::Blend(value) => set!(blend, *value),
        Property::Clipped(value) => set!(clipped, *value),
        Property::Locked(value) => set!(locked, *value),
        Property::AlphaLocked(value) => set!(alpha_locked, *value),
        Property::Collapsed(value) => {
            if !layer.folder {
                return Err("Collapse applies only to folders".into());
            }
            set!(collapsed, *value)
        }
        Property::Effects(effects) => set!(effects, effects.clone()),
        Property::Adjustment(adjustment) => {
            if layer.adjustment.is_none() {
                return Err("Only adjustment layers have adjustment settings".into());
            }
            let changed = layer.adjustment.as_ref() != Some(adjustment);
            layer.adjustment = Some(adjustment.clone());
            changed
        }
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn captured_merge_checks_kind_identity_and_preserves_folder_style() {
        let mut source = Document::new(16, 16);
        source.layers[0].pixels[4..8].copy_from_slice(&[17, 83, 201, 255]);
        source.group_active().unwrap();
        let header = &mut source.layers[source.active];
        header.opacity = 0.4;
        header.visible = false;
        header.locked = true;
        header.alpha_locked = true;
        header.clipped = true;
        let target = header.id;
        let (context, reserved, folder_result) =
            crate::pending_layers::PendingLayers::from_document(&source)
                .with_merge(target)
                .unwrap();
        let (merged, _) = Edit::MergeTarget {
            target,
            folder_result,
        }
        .prepare(&source)
        .unwrap();
        let layer = &merged.layers[merged.active];
        assert_eq!(layer.id, reserved);
        assert!(!layer.folder && !layer.locked && !layer.alpha_locked);
        assert!(!layer.visible && layer.clipped);
        assert_eq!(layer.opacity, 0.4);
        assert_eq!(layer.name, source.layers[source.active].name);
        assert_eq!(
            context.paint_blocker(reserved),
            Some("Layer or its folder is hidden")
        );
        assert!(!context.alpha_locked(reserved));
        assert!(
            Edit::MergeTarget {
                target,
                folder_result: None
            }
            .prepare(&source)
            .is_err()
        );
        assert!(
            Edit::MergeTarget {
                target,
                folder_result: Some(target)
            }
            .prepare(&source)
            .is_err()
        );
        assert!(
            Edit::MergeTarget {
                target: source.layers[0].id,
                folder_result
            }
            .prepare(&source)
            .is_err()
        );
        assert_eq!(source.layers.len(), 2);
        assert_eq!(&source.layers[0].pixels[4..8], &[17, 83, 201, 255]);
    }
    use super::*;

    #[test]
    fn identities_follow_reorder_but_not_duplicate_or_reload() {
        let mut source = Document::new(16, 16);
        let original = source.layers[0].id;
        source.group_active().unwrap();
        let folder = source.layers[source.active].id;
        source.duplicate_active().unwrap();
        source.validate().unwrap();
        assert_ne!(source.layers[2].id, original);
        assert_ne!(source.layers[3].id, folder);
        let mut history = crate::document::History::new();
        history.checkpoint(&source);
        source.active = 1;
        source.step_active(true).unwrap();
        history.settle(&source);
        assert_eq!(source.layers[3].id, folder);
        assert!(history.undo(&mut source));
        assert_eq!(source.layers[1].id, folder);
        assert!(history.redo(&mut source));
        assert_eq!(source.layers[3].id, folder);
        let (renamed, _) = Edit::Property {
            target: Target::Identity(original),
            property: Property::Name("Captured child".into()),
        }
        .prepare(&source)
        .unwrap();
        assert_eq!(renamed.layers[2].name, "Captured child");
        assert_ne!(renamed.layers[0].name, "Captured child");
        let json = serde_json::to_vec(&source).unwrap();
        let loaded: Document = serde_json::from_slice(&json).unwrap();
        loaded.validate().unwrap();
        assert!(
            loaded
                .layers
                .iter()
                .all(|layer| !source.layers.iter().any(|old| old.id == layer.id))
        );
        assert!(
            Edit::SelectTarget(Target::Identity(original))
                .prepare(&loaded)
                .is_err()
        );
        assert_eq!(serde_json::to_vec(&loaded).unwrap(), json);
    }

    #[test]
    fn folder_commands_keep_structure_and_collapse_active_child() {
        let source = Document::new(16, 16);
        let (grouped, result) = Edit::Group.prepare(&source).unwrap();
        assert!(result.changed && result.undoable && result.redraw);
        let folder = grouped
            .layers
            .iter()
            .position(|layer| layer.folder)
            .unwrap();
        let (child, _) = Edit::SelectLayer(0).prepare(&grouped).unwrap();
        let (collapsed, result) = Edit::Property {
            target: Target::Index(folder),
            property: Property::Collapsed(true),
        }
        .prepare(&child)
        .unwrap();
        assert!(result.changed && result.undoable && !result.redraw);
        assert_eq!(collapsed.active, folder);
        assert_eq!(child.active, 0);
        assert!(!child.layers[folder].collapsed);
        let (ungrouped, _) = Edit::Ungroup.prepare(&collapsed).unwrap();
        assert_eq!(ungrouped.layers.len(), 1);
        ungrouped.validate().unwrap();
        assert!(
            Edit::Property {
                target: Target::Active,
                property: Property::Collapsed(true),
            }
            .prepare(&ungrouped)
            .is_err()
        );
    }

    #[test]
    fn serialized_commands_are_atomic_and_whole_clear_preserves_selection() {
        let mut source = Document::new(16, 16);
        source.layers[0].pixels[4..8].copy_from_slice(&[17, 83, 201, 255]);
        source.selection = Some(Selection {
            left: 8,
            top: 8,
            right: 10,
            bottom: 10,
        });
        let encoded = serde_json::to_vec(&Edit::Clear { whole_layer: true }).unwrap();
        let edit: Edit = serde_json::from_slice(&encoded).unwrap();
        let (cleared, result) = edit.prepare(&source).unwrap();
        assert!(result.changed && result.redraw && result.undoable);
        assert_eq!(cleared.selection, source.selection);
        assert_eq!(&cleared.layers[0].pixels[4..8], &[0; 4]);
        assert_eq!(&source.layers[0].pixels[4..8], &[17, 83, 201, 255]);
        let (untouched, result) = Edit::Clear { whole_layer: false }.prepare(&source).unwrap();
        assert!(!result.changed);
        assert_eq!(untouched.layers[0].pixels, source.layers[0].pixels);
        assert!(
            Edit::Reorder {
                from: 0,
                gap: 100,
                depth: 0
            }
            .prepare(&source)
            .is_err()
        );
        assert_eq!(source.layers.len(), 1);
        assert_eq!(source.active, 0);
    }
}
