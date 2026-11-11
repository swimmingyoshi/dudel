//! Raster clipboard transactions. OS writes finish before cut changes artwork.
use crate::{Dudel, document::Document, selection::Selection};

pub(crate) struct Raster {
    pub width: usize,
    pub height: usize,
    pub rgba: Vec<u8>,
}
pub(crate) enum Action {
    Copy {
        target: crate::document::LayerId,
        cut: bool,
    },
    Paste {
        target: crate::document::LayerId,
        id: crate::document::LayerId,
        source: PasteSource,
        /// Canvas point to center the image on, such as a drop position.
        at: Option<[f32; 2]>,
    },
    CopyMask {
        target: crate::document::LayerId,
        mask: crate::document::LayerId,
        cut: bool,
    },
    PasteMask {
        target: crate::document::LayerId,
        mask: crate::document::LayerId,
    },
}
/// Where a pasted layer's image comes from.
pub(crate) enum PasteSource {
    Clipboard,
    File(std::path::PathBuf),
}
/// A pasted image as read, before it is decoded or placed.
pub(crate) enum PasteInput {
    Raster(Raster),
    File(std::path::PathBuf),
}
pub(crate) struct PasteJob {
    pub source: Document,
    revision: u64,
    ticket: u64,
    status: &'static str,
    worker: std::thread::JoinHandle<Result<Option<Document>, String>>,
}
struct PreparedCopy {
    raster: Raster,
    document: Option<Document>,
    status: &'static str,
}
pub(crate) struct CopyJob {
    pub source: Document,
    revision: u64,
    ticket: u64,
    worker: std::thread::JoinHandle<Result<PreparedCopy, String>>,
}
pub(crate) struct ClipboardSettlement {
    pub cursor: crate::document::Settlement,
    ticket: u64,
    status: &'static str,
}
#[cfg(test)]
fn prepare_reserved_paste(
    source: &Document,
    raster: &Raster,
    target: crate::document::LayerId,
    id: crate::document::LayerId,
) -> Result<Document, String> {
    prepare_reserved_paste_at(source, raster, target, id, None, "Pasted image")
}
fn prepare_reserved_paste_at(
    source: &Document,
    raster: &Raster,
    target: crate::document::LayerId,
    id: crate::document::LayerId,
    at: Option<[f32; 2]>,
    name: &str,
) -> Result<Document, String> {
    let mut source = source.clone();
    source.active = crate::document_edits::Edit::resolve(
        &crate::document_edits::Target::Identity(target),
        &source,
    )?;
    let mut updated = prepare_paste_at(&source, raster, at, name)?;
    updated.layers[updated.active].id = id;
    updated.validate()?;
    Ok(updated)
}
trait ClipboardIo {
    fn read(&mut self) -> Result<Raster, String>;
    fn write(&mut self, raster: &Raster) -> Result<(), String>;
}
impl ClipboardIo for arboard::Clipboard {
    fn read(&mut self) -> Result<Raster, String> {
        let image = self.get_image().map_err(|error| error.to_string())?;
        Ok(Raster {
            width: image.width,
            height: image.height,
            rgba: image.bytes.into_owned(),
        })
    }
    fn write(&mut self, raster: &Raster) -> Result<(), String> {
        self.set_image(arboard::ImageData {
            width: raster.width,
            height: raster.height,
            bytes: std::borrow::Cow::Borrowed(&raster.rgba),
        })
        .map_err(|error| error.to_string())
    }
}
type ClipboardResult = Result<(Option<Document>, &'static str), String>;
#[cfg(test)]
#[path = "mask_clipboard_tests.rs"]
mod mask_target_tests;
#[cfg(test)]
mod layer_mask_clipboard_tests {
    use super::*;
    fn source() -> Document {
        let mut doc = Document::new(4, 1);
        doc.version = 2;
        doc.layers[0].opacity = 0.1;
        doc.layers[0].blend = crate::blend::BlendMode::Multiply;
        doc.layers[0].pixels = vec![
            10, 20, 30, 255, 30, 50, 70, 129, 83, 97, 101, 0, 110, 130, 150, 128,
        ]
        .into();
        doc.layers[0].mask = Some(crate::layer_mask::LayerMask {
            id: crate::document::LayerId::new(),
            enabled: true,
            linked: true,
            coverage: crate::layer_mask::Mask::from_coverage(4, 1, &[0, 128, 255, 255], None)
                .unwrap(),
        });
        let selected =
            crate::selection_mask::Mask::from_coverage(4, 1, vec![255, 128, 255, 0]).unwrap();
        doc.selection = selected.bounds();
        doc.selection_mask = Some(selected);
        doc
    }
    #[test]
    fn copy_combines_layer_and_selection_coverage_without_layer_style_and_disabled_masks_copy_raw()
    {
        let mut doc = source();
        let frozen = doc.clone();
        let raster = capture(&doc).unwrap();
        assert_eq!([raster.width, raster.height], [3, 1]);
        assert_eq!(raster.rgba, [0, 0, 0, 0, 30, 50, 70, 33, 83, 97, 101, 0]);
        assert!(crate::document::same_snapshot(&doc, &frozen));
        doc.layers[0].mask.as_mut().unwrap().enabled = false;
        doc.selection = None;
        doc.selection_mask = None;
        assert_eq!(
            capture(&doc).unwrap().rgba,
            doc.layers[0].pixels.as_slice().to_vec()
        );
    }
    #[test]
    fn cut_writes_before_changing_raster_keeps_mask_and_paste_has_baked_coverage() {
        let doc = source();
        let frozen = doc.clone();
        assert!(copy_transaction(&doc, true, |_| Err("clipboard busy".into())).is_err());
        assert!(crate::document::same_snapshot(&doc, &frozen));
        let mut copied = None;
        let cut = copy_transaction(&doc, true, |raster| {
            copied = Some(Raster {
                width: raster.width,
                height: raster.height,
                rgba: raster.rgba.clone(),
            });
            Ok(())
        })
        .unwrap()
        .unwrap();
        assert_eq!(cut.layers[0].pixels.rgba(0), [0; 4]);
        assert_eq!(cut.layers[0].pixels.rgba(1), [30, 50, 70, 64]);
        assert_eq!(cut.layers[0].pixels.rgba(3), [110, 130, 150, 128]);
        assert!(crate::layer_mask::same_snapshot(
            &cut.layers[0].mask,
            &doc.layers[0].mask
        ));
        let raster = copied.unwrap();
        let pasted = prepare_paste(&doc, &raster).unwrap();
        assert!(pasted.layers[pasted.active].mask.is_none());
        assert!(crate::layer_mask::same_snapshot(
            &pasted.layers[0].mask,
            &doc.layers[0].mask
        ));
        assert_eq!(
            pasted.layers[pasted.active].pixels.rgba(1),
            [30, 50, 70, 33]
        );
        let mut history = crate::document::History::new();
        let mut actual = doc.clone();
        history.checkpoint(&actual);
        actual = cut;
        history.settle(&actual);
        assert!(history.undo(&mut actual));
        // Pixel-patch undo restores bytes, not the original pixel allocation.
        assert_eq!(actual.layers[0].pixels, doc.layers[0].pixels);
        assert!(crate::layer_mask::same_snapshot(
            &actual.layers[0].mask,
            &doc.layers[0].mask
        ));
        assert_eq!(actual.selection_mask, doc.selection_mask);
    }
}
#[cfg(test)]
fn execute_action(doc: &Document, action: Action, io: &mut impl ClipboardIo) -> ClipboardResult {
    match action {
        Action::Copy { target, cut } => {
            let prepared = prepare_reserved_copy(doc, target, cut)?;
            io.write(&prepared.raster)?;
            Ok((prepared.document, prepared.status))
        }
        Action::Paste { target, id, .. } => {
            let raster = io.read()?;
            let updated = prepare_reserved_paste(doc, &raster, target, id)?;
            Ok((Some(updated), "Pasted as a new layer"))
        }
        Action::CopyMask { target, mask, cut } => {
            let prepared = prepare_mask_copy(doc, target, mask, cut)?;
            io.write(&prepared.raster)?;
            Ok((prepared.document, prepared.status))
        }
        Action::PasteMask { target, mask } => Ok((
            crate::mask_clipboard::paste(doc, &io.read()?, target, mask)?,
            "Pasted into mask",
        )),
    }
}

impl Raster {
    pub fn validate(&self) -> Result<(), String> {
        if raster_byte_len(self.width, self.height)? != self.rgba.len() {
            return Err(
                "Clipboard image exceeds supported dimensions or has invalid RGBA data".into(),
            );
        }
        Ok(())
    }
}
fn raster_byte_len(width: usize, height: usize) -> Result<usize, String> {
    let pixels = width
        .checked_mul(height)
        .ok_or("Clipboard dimensions overflow")?;
    if width == 0
        || height == 0
        || width > crate::document::MAX_SIDE
        || height > crate::document::MAX_SIDE
        || pixels > crate::document::MAX_PIXELS
    {
        return Err("Clipboard image exceeds supported dimensions or has invalid RGBA data".into());
    }
    pixels
        .checked_mul(4)
        .ok_or("Clipboard dimensions overflow".into())
}
fn bounds(doc: &Document) -> Selection {
    doc.selection.unwrap_or(Selection {
        left: 0,
        top: 0,
        right: doc.width,
        bottom: doc.height,
    })
}
fn capture(doc: &Document) -> Result<Raster, String> {
    doc.validate()?;
    if doc.selection_mask.is_some() && doc.selection.is_none() {
        return Err("Selection is empty".into());
    }
    let layer = &doc.layers[doc.active];
    if layer.folder {
        return Err("Select a raster layer to copy".into());
    }
    let area = bounds(doc);
    raster_byte_len(area.right - area.left, area.bottom - area.top)?;
    let mut rgba = layer
        .pixels
        .region(doc.width, [area.left, area.top, area.right, area.bottom]);
    if let Some(mask) = layer.mask.as_ref().filter(|m| m.enabled) {
        let width = area.right - area.left;
        for (i, pixel) in rgba.chunks_exact_mut(4).enumerate() {
            let coverage = mask
                .coverage
                .at(area.left + i % width, area.top + i / width);
            if coverage == 0 {
                pixel.fill(0);
            } else {
                pixel[3] = ((u16::from(pixel[3]) * u16::from(coverage) + 127) / 255) as u8;
            }
        }
    }
    if let Some(mask) = &doc.selection_mask {
        let width = area.right - area.left;
        for (index, pixel) in rgba.chunks_exact_mut(4).enumerate() {
            let coverage = mask.at(area.left + index % width, area.top + index / width);
            if coverage == 0 {
                pixel.fill(0);
            } else {
                pixel[3] = ((u16::from(pixel[3]) * u16::from(coverage) + 127) / 255) as u8;
            }
        }
    }
    Ok(Raster {
        width: area.right - area.left,
        height: area.bottom - area.top,
        rgba,
    })
}
fn copy_transaction(
    doc: &Document,
    cut: bool,
    write: impl FnOnce(&Raster) -> Result<(), String>,
) -> Result<Option<Document>, String> {
    let (raster, updated) = prepare_copy(doc, cut)?;
    write(&raster)?;
    Ok(updated)
}
fn prepare_reserved_copy(
    doc: &Document,
    target: crate::document::LayerId,
    cut: bool,
) -> Result<PreparedCopy, String> {
    let mut source = doc.clone();
    source.active = crate::document_edits::Edit::resolve(
        &crate::document_edits::Target::Identity(target),
        &source,
    )?;
    let (raster, mut document) = prepare_copy(&source, cut)?;
    if let Some(updated) = &mut document {
        updated.active = doc.active;
    }
    Ok(PreparedCopy {
        raster,
        document,
        status: if cut {
            "Cut to clipboard"
        } else {
            "Copied to clipboard"
        },
    })
}
fn prepare_mask_copy(
    doc: &Document,
    target: crate::document::LayerId,
    mask: crate::document::LayerId,
    cut: bool,
) -> Result<PreparedCopy, String> {
    let (raster, document) = crate::mask_clipboard::copy(doc, target, mask, cut)?;
    Ok(PreparedCopy {
        raster,
        document,
        status: if cut {
            "Cut mask to clipboard"
        } else {
            "Copied mask to clipboard"
        },
    })
}
fn prepare_copy(doc: &Document, cut: bool) -> Result<(Raster, Option<Document>), String> {
    let raster = capture(doc)?;
    let mut updated = None;
    if cut {
        let layer = &doc.layers[doc.active];
        if layer.locked || layer.alpha_locked {
            return Err("Unlock pixels and transparency before cutting".into());
        }
        let area = bounds(doc);
        let mut candidate = doc.clone();
        let rect = [area.left, area.top, area.right, area.bottom];
        let changed = if let Some(mask) = &doc.selection_mask {
            let mut changed = false;
            let mut pixels = layer.pixels.clone();
            {
                let mut writer = pixels.writer();
                for y in area.top..area.bottom {
                    for x in area.left..area.right {
                        let coverage = mask.at(x, y);
                        if coverage == 0 {
                            continue;
                        }
                        let before = layer.pixels.rgba(y * doc.width + x);
                        let mut after = before;
                        if coverage == 255 {
                            after = [0; 4];
                        } else {
                            after[3] = ((u16::from(before[3]) * (255 - u16::from(coverage)) + 127)
                                / 255) as u8;
                        }
                        if after != before {
                            writer.rgba_mut(y * doc.width + x).copy_from_slice(&after);
                            changed = true;
                        }
                    }
                }
                if changed {
                    writer.reclaim_region(doc.width, rect);
                }
            }
            if changed {
                candidate.layers[candidate.active].pixels = pixels;
            }
            changed
        } else {
            candidate.layers[candidate.active]
                .pixels
                .clear_region(candidate.width, rect)
        };
        if changed {
            updated = Some(candidate);
        }
    }
    Ok((raster, updated))
}
fn prepare_paste(doc: &Document, raster: &Raster) -> Result<Document, String> {
    prepare_paste_at(doc, raster, None, "Pasted image")
}
/// A new layer above the active one holding `raster`, scaled down to fit the
/// canvas and centered on `at`, or else on the selection or canvas.
fn prepare_paste_at(
    doc: &Document,
    raster: &Raster,
    at: Option<[f32; 2]>,
    name: &str,
) -> Result<Document, String> {
    doc.validate()?;
    crate::image_paste::validate_incoming(raster)?;
    if !doc.has_room(1, 1) {
        return Err("Layer limit reached".into());
    }
    let fitted;
    let raster = if raster.width > doc.width || raster.height > doc.height {
        fitted = crate::image_paste::fit(
            Raster {
                width: raster.width,
                height: raster.height,
                rgba: raster.rgba.clone(),
            },
            doc.width,
            doc.height,
        )?;
        &fitted
    } else {
        raster
    };
    let area = bounds(doc);
    let center = at.unwrap_or([
        (area.left + area.right) as f32 / 2.0,
        (area.top + area.bottom) as f32 / 2.0,
    ]);
    let [left, top] = crate::image_paste::placement(
        [raster.width, raster.height],
        [doc.width, doc.height],
        center,
    );
    let region = Selection {
        left,
        top,
        right: left + raster.width,
        bottom: top + raster.height,
    };
    let mut updated = doc.clone();
    updated.insert_layer_above()?;
    updated.layers[updated.active].name = name.chars().take(128).collect();
    updated.layers[updated.active].pixels.writer().write_region(
        updated.width,
        [region.left, region.top, region.right, region.bottom],
        &raster.rgba,
    );
    updated.selection = Some(region);
    updated.selection_mask = None;
    updated.validate()?;
    Ok(updated)
}
impl Dudel {
    pub(crate) fn clipboard_paste_shortcut(&mut self, ctx: &crate::egui::Context) -> bool {
        // egui-winit swallows Ctrl+V without a Paste event for image-only
        // clipboard content. Windows messages retain even a fast key tap.
        #[cfg(windows)]
        let pressed = self
            .clipboard_shortcuts
            .as_mut()
            .is_some_and(|backend| backend.take_paste());
        #[cfg(not(windows))]
        let pressed = false;
        ctx.input(|input| pressed && input.focused)
    }
    pub(crate) fn clipboard_backend(&mut self) -> Result<&mut arboard::Clipboard, String> {
        if self.clipboard_backend.is_none() {
            self.clipboard_backend =
                Some(arboard::Clipboard::new().map_err(|error| error.to_string())?);
        }
        Ok(self.clipboard_backend.as_mut().unwrap())
    }
    fn queue_clipboard(&mut self, action: Action) -> Result<u64, String> {
        if self.pending_clipboard.len() >= 16 {
            return Err("Too many pending clipboard actions".into());
        }
        self.finish_layer_rename(true);
        self.close_input_gestures();
        let ticket = self.render_queue.barrier()?;
        self.pending_clipboard.insert(ticket, action);
        self.status = "Clipboard action queued after preceding artwork".into();
        Ok(ticket)
    }
    fn commit_clipboard_document(&mut self, document: Document) {
        self.history.checkpoint(&self.doc);
        self.doc = document;
        self.history.settle(&self.doc);
        self.changed();
    }
    pub(crate) fn copy_canvas(&mut self, cut: bool) {
        if let crate::paint_target::Target::Mask(mask) = self.input_stroke_target() {
            let result = (|| {
                let target = self
                    .input_target()
                    .ok_or("Selected layer no longer exists")?;
                if !self
                    .input_properties()
                    .and_then(|p| p.mask)
                    .is_some_and(|m| m.id == mask)
                {
                    return Err("Clipboard mask was removed or replaced".to_string());
                }
                if cut
                    && let Some(error) =
                        self.stroke_target_blocker(crate::paint_target::Target::Mask(mask), false)
                {
                    return Err(error.to_string());
                }
                self.queue_clipboard(Action::CopyMask { target, mask, cut })?;
                Ok(())
            })();
            if let Err(error) = result {
                self.status = format!("Clipboard: {error}");
            }
            return;
        }
        let result = self
            .input_target()
            .ok_or("Selected layer no longer exists".to_string())
            .and_then(|target| self.queue_clipboard(Action::Copy { target, cut }));
        if let Err(error) = result {
            self.status = format!("Clipboard: {error}");
        }
    }
    pub(crate) fn paste_canvas(&mut self) {
        self.paste_canvas_with_files(Self::clipboard_image_files);
    }
    /// Keep clipboard discovery injectable so transaction tests never read the
    /// user's system clipboard or depend on files copied by another process.
    pub(crate) fn paste_canvas_with_files(
        &mut self,
        files: impl FnOnce(&mut Self) -> Vec<std::path::PathBuf>,
    ) {
        if self.pending_clipboard.len() >= 16 {
            self.status = "Clipboard: Too many pending clipboard actions".into();
            return;
        }
        if let crate::paint_target::Target::Mask(mask) = self.input_stroke_target() {
            let result = (|| {
                let target = self
                    .input_target()
                    .ok_or("Selected layer no longer exists")?;
                if let Some(error) =
                    self.stroke_target_blocker(crate::paint_target::Target::Mask(mask), false)
                {
                    return Err(error.to_string());
                }
                self.queue_clipboard(Action::PasteMask { target, mask })?;
                Ok(())
            })();
            if let Err(error) = result {
                self.status = format!("Clipboard: {error}");
            }
            return;
        }
        // Files copied in a file manager paste as one layer each.
        let files = files(self);
        if !files.is_empty() {
            self.paste_image_files(files, None);
            return;
        }
        if let Err(error) = self.queue_paste_layer(PasteSource::Clipboard, None) {
            self.status = format!("Clipboard: {error}");
        }
    }
    /// Reserve a new layer above the current one and queue filling it from
    /// `source` after the artwork ahead of it.
    pub(crate) fn queue_paste_layer(
        &mut self,
        source: PasteSource,
        at: Option<[f32; 2]>,
    ) -> Result<(), String> {
        self.finish_layer_rename(true);
        self.close_input_gestures();
        let target = self
            .input_target()
            .ok_or("Selected layer no longer exists")?;
        let id = crate::document::LayerId::new();
        let context = self
            .pending_layers
            .clone()
            .unwrap_or_else(|| crate::pending_layers::PendingLayers::from_document(&self.doc));
        let projected = context.with_new_layer(target, id)?;
        self.queue_clipboard(Action::Paste {
            target,
            id,
            source,
            at,
        })?;
        self.pending_layers = Some(projected);
        self.logical_layer = Some(id);
        Ok(())
    }
    pub(crate) fn complete_clipboard(&mut self, ticket: u64) {
        let Some(action) = self.pending_clipboard.remove(&ticket) else {
            self.status = "Unrecognized clipboard barrier".into();
            let _ = self.render_queue.release_barrier(ticket);
            return;
        };
        if let Action::Paste {
            target,
            id,
            source,
            at,
        } = action
        {
            let input = match source {
                PasteSource::File(path) => Ok(PasteInput::File(path)),
                PasteSource::Clipboard => self
                    .clipboard_backend()
                    .and_then(|io| io.read())
                    .map_err(|_| "The clipboard holds no image".to_string())
                    .and_then(|raster| {
                        crate::image_paste::validate_incoming(&raster)?;
                        Ok(PasteInput::Raster(raster))
                    }),
            };
            match input {
                Ok(input) => self.start_paste_job(ticket, input, target, id, at),
                Err(error) => self.finish_clipboard_barrier(ticket, Err(error)),
            }
            return;
        }
        if let Action::PasteMask { target, mask } = action {
            let raster = self
                .clipboard_backend()
                .and_then(|io| io.read())
                .and_then(|r| {
                    r.validate()?;
                    Ok(r)
                });
            match raster {
                Ok(raster) => self.start_mask_paste_job(ticket, raster, target, mask),
                Err(error) => self.finish_clipboard_barrier(ticket, Err(error)),
            };
            return;
        }
        if let Action::CopyMask { target, mask, cut } = action {
            self.start_copy_target_job(
                ticket,
                target,
                crate::paint_target::Target::Mask(mask),
                cut,
            );
            return;
        }
        let Action::Copy { target, cut } = action else {
            unreachable!()
        };
        self.start_copy_job(ticket, target, cut);
    }
    fn start_copy_job(&mut self, ticket: u64, target: crate::document::LayerId, cut: bool) {
        self.start_copy_target_job(ticket, target, crate::paint_target::Target::Raster, cut);
    }
    fn start_copy_target_job(
        &mut self,
        ticket: u64,
        target: crate::document::LayerId,
        paint_target: crate::paint_target::Target,
        cut: bool,
    ) {
        let source = self.doc.clone();
        let input = source.clone();
        self.copy_job = Some(CopyJob {
            source,
            revision: self.revision,
            ticket,
            worker: std::thread::spawn(move || match paint_target {
                crate::paint_target::Target::Raster => prepare_reserved_copy(&input, target, cut),
                crate::paint_target::Target::Mask(mask) => {
                    prepare_mask_copy(&input, target, mask, cut)
                }
            }),
        });
        self.status = "Preparing clipboard pixels".into();
    }
    fn take_prepared_copy(&mut self, wait: bool) -> Option<(u64, Result<PreparedCopy, String>)> {
        if self
            .copy_job
            .as_ref()
            .is_none_or(|job| !wait && !job.worker.is_finished())
        {
            return None;
        }
        let job = self.copy_job.take().unwrap();
        let result = job
            .worker
            .join()
            .unwrap_or_else(|_| Err("Copy worker failed".into()));
        let result = if self.revision != job.revision
            || !crate::document::same_snapshot(&job.source, &self.doc)
        {
            Err("Document changed while preparing clipboard pixels".into())
        } else {
            result
        };
        Some((job.ticket, result))
    }
    pub(crate) fn finish_copy_job(&mut self, wait: bool) {
        let Some((ticket, prepared)) = self.take_prepared_copy(wait) else {
            return;
        };
        let result = prepared.and_then(|prepared| {
            self.clipboard_backend()?.write(&prepared.raster)?;
            Ok((prepared.document, prepared.status))
        });
        self.finish_clipboard_barrier(ticket, result);
    }
    fn finish_copy_with_io(&mut self, wait: bool, io: &mut impl ClipboardIo) {
        let Some((ticket, prepared)) = self.take_prepared_copy(wait) else {
            return;
        };
        let result = prepared.and_then(|prepared| {
            io.write(&prepared.raster)?;
            Ok((prepared.document, prepared.status))
        });
        self.finish_clipboard_barrier(ticket, result);
    }
    fn start_paste_job(
        &mut self,
        ticket: u64,
        image: PasteInput,
        target: crate::document::LayerId,
        id: crate::document::LayerId,
        at: Option<[f32; 2]>,
    ) {
        let source = self.doc.clone();
        let input = source.clone();
        // Decoding and scaling run off the UI thread.
        let worker = std::thread::spawn(move || {
            let (raster, name) = match image {
                PasteInput::Raster(raster) => (raster, "Pasted image".to_owned()),
                PasteInput::File(path) => {
                    let name = path
                        .file_stem()
                        .map_or("Pasted image".into(), |s| s.to_string_lossy().into_owned());
                    (crate::image_paste::decode(&path)?, name)
                }
            };
            prepare_reserved_paste_at(&input, &raster, target, id, at, &name).map(Some)
        });
        self.paste_job = Some(PasteJob {
            source,
            revision: self.revision,
            ticket,
            status: "Pasted as a new layer",
            worker,
        });
        self.status = "Preparing pasted image".into();
    }
    fn start_mask_paste_job(
        &mut self,
        ticket: u64,
        raster: Raster,
        target: crate::document::LayerId,
        mask: crate::document::LayerId,
    ) {
        let source = self.doc.clone();
        let input = source.clone();
        self.paste_job = Some(PasteJob {
            source,
            revision: self.revision,
            ticket,
            status: "Pasted into mask",
            worker: std::thread::spawn(move || {
                crate::mask_clipboard::paste(&input, &raster, target, mask)
            }),
        });
        self.status = "Preparing pasted mask coverage".into();
    }
    /// Isolated native capture input, never reads or writes the OS clipboard.
    pub(crate) fn capture_mask_paste(&mut self, raster: Raster) -> Result<(), String> {
        if self.capture.is_none() {
            return Err("Mask paste fixture requires capture mode".into());
        }
        raster.validate()?;
        let crate::paint_target::Target::Mask(mask) = self.input_stroke_target() else {
            return Err("Select a mask for the paste fixture".into());
        };
        let target = self
            .input_target()
            .ok_or("Mask paste target no longer exists")?;
        self.close_input_gestures();
        self.flush_render_queue();
        self.track_guide_changes();
        let ticket = self.render_queue.barrier()?;
        let progress = self
            .render_queue
            .advance(&mut self.doc, &mut self.history, 1)?;
        if progress.barrier != Some(ticket) {
            return Err("Mask paste fixture barrier was not reached".into());
        }
        self.start_mask_paste_job(ticket, raster, target, mask);
        Ok(())
    }
    fn finish_paste_job(&mut self, wait: bool) {
        if self
            .paste_job
            .as_ref()
            .is_none_or(|job| !wait && !job.worker.is_finished())
        {
            return;
        }
        let job = self.paste_job.take().unwrap();
        let result = job
            .worker
            .join()
            .unwrap_or_else(|_| Err("Paste worker failed".into()));
        let result = if self.revision != job.revision
            || !crate::document::same_snapshot(&job.source, &self.doc)
        {
            Err("Document changed while preparing pasted image".into())
        } else {
            result.map(|document| {
                let status = if document.is_some() {
                    job.status
                } else {
                    "Paste made no changes"
                };
                (document, status)
            })
        };
        self.finish_clipboard_barrier(job.ticket, result);
    }
    pub(crate) fn wait_paste(&mut self) {
        self.finish_paste_job(true);
    }
    pub(crate) fn poll_paste(&mut self, ctx: &crate::egui::Context) {
        self.finish_copy_job(false);
        self.finish_paste_job(false);
        self.advance_clipboard_settlement(
            4096,
            std::time::Instant::now() + std::time::Duration::from_millis(2),
        );
        if self.clipboard_settlement.is_some() {
            ctx.request_repaint();
        }
        if self.paste_job.is_some() || self.copy_job.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(8));
        }
    }
    fn finish_clipboard_barrier(&mut self, ticket: u64, result: ClipboardResult) {
        self.status = match result {
            Ok((document, status)) => {
                if let Some(document) = document {
                    self.history.begin_gesture_checkpoint(&self.doc);
                    self.doc = document;
                    self.changed();
                    if let Some(mut cursor) = self.history.begin_settle(&self.doc) {
                        cursor.prefer_smaller_full_snapshot();
                        self.clipboard_settlement = Some(ClipboardSettlement {
                            cursor,
                            ticket,
                            status,
                        });
                        self.status = status.into();
                        return;
                    }
                    self.history.settle(&self.doc);
                }
                status.into()
            }
            Err(error) => format!("Clipboard: {error}"),
        };
        self.release_clipboard_barrier(ticket);
    }
    fn release_clipboard_barrier(&mut self, ticket: u64) {
        if let Err(error) = self.render_queue.release_barrier(ticket) {
            self.status = error;
        }
        if !self.render_queue.busy() {
            self.logical_layer = None;
            self.pending_layers = None;
        }
    }
    fn advance_clipboard_settlement(&mut self, max_work: usize, deadline: std::time::Instant) {
        for _ in 0..max_work {
            if std::time::Instant::now() >= deadline {
                break;
            }
            let Some(settlement) = &mut self.clipboard_settlement else {
                break;
            };
            match settlement.cursor.step(&mut self.history, &self.doc, 1) {
                Ok(step) if !step.complete => continue,
                Ok(_) => {
                    let settlement = self.clipboard_settlement.take().unwrap();
                    self.status = settlement.status.into();
                    self.release_clipboard_barrier(settlement.ticket);
                }
                Err(error) => {
                    let settlement = self.clipboard_settlement.take().unwrap();
                    settlement.cursor.retain_full(&mut self.history, &self.doc);
                    self.status = format!("Clipboard history: {error}");
                    self.release_clipboard_barrier(settlement.ticket);
                }
            }
            break;
        }
    }
    pub(crate) fn finish_clipboard_settlement(&mut self) {
        while self.clipboard_settlement.is_some() {
            self.advance_clipboard_settlement(
                4096,
                std::time::Instant::now() + std::time::Duration::from_secs(1),
            );
        }
    }
}

pub(crate) fn benchmark(path: &std::path::Path) -> Result<(), String> {
    struct MemoryClipboard(Option<Raster>);
    impl ClipboardIo for MemoryClipboard {
        fn read(&mut self) -> Result<Raster, String> {
            let raster = self.0.as_ref().ok_or("Headless clipboard empty")?;
            Ok(Raster {
                width: raster.width,
                height: raster.height,
                rgba: raster.rgba.clone(),
            })
        }
        fn write(&mut self, raster: &Raster) -> Result<(), String> {
            self.0 = Some(Raster {
                width: raster.width,
                height: raster.height,
                rgba: raster.rgba.clone(),
            });
            Ok(())
        }
    }
    let mut cases = Vec::new();
    for size in [512, 2048] {
        for storage in ["sparse", "dense-flat", "dense-tiled"] {
            let mut doc = Document::new(size, size);
            if storage != "sparse" {
                let bytes = [17, 83, 201, 128].repeat(size * size);
                doc.layers[0].pixels = if storage == "dense-flat" {
                    bytes.into()
                } else {
                    crate::pixels::Pixels::read_rgba(&mut &bytes[..], size, size)
                        .map_err(|error| error.to_string())?
                };
            }
            doc.layers[0]
                .pixels
                .writer()
                .rgba_mut(0)
                .copy_from_slice(&[9, 11, 13, 0]);
            let original = doc.clone();
            let started = std::time::Instant::now();
            let raster = capture(&doc)?;
            let copy_ms = started.elapsed().as_secs_f64() * 1000.0;
            let started = std::time::Instant::now();
            let cut =
                copy_transaction(&doc, true, |_| Ok(()))?.ok_or("Cut probe produced no change")?;
            let cut_ms = started.elapsed().as_secs_f64() * 1000.0;
            let mut app = Dudel {
                doc,
                ..Default::default()
            };
            app.commit_clipboard_document(cut);
            let cut_pixels = app.doc.layers[0].pixels.clone();
            let started = std::time::Instant::now();
            let pasted = prepare_paste(&app.doc, &raster)?;
            let paste_prepare_ms = started.elapsed().as_secs_f64() * 1000.0;
            app.commit_clipboard_document(pasted);
            let pasted_pixels = app.doc.layers[app.doc.active].pixels.capacity();
            if app.doc.layers[app.doc.active].pixels != original.layers[0].pixels
                || app.doc.layers[0].pixels != cut_pixels
                || raster.rgba[..4] != [9, 11, 13, 0]
            {
                return Err("Clipboard probe changed RGBA or existing artwork".into());
            }
            let retained_history_bytes = app.history.retained_bytes();
            if !app.history.undo(&mut app.doc)
                || app.doc.layers.len() != 1
                || !app.history.undo(&mut app.doc)
                || app.doc.layers[0].pixels != original.layers[0].pixels
                || !app.history.redo(&mut app.doc)
                || !app.history.redo(&mut app.doc)
                || app.doc.layers[app.doc.active].pixels != original.layers[0].pixels
            {
                return Err("Clipboard probe undo/redo failed".into());
            }
            let mut queued = Dudel {
                doc: original.clone(),
                ..Default::default()
            };
            let admission = std::time::Instant::now();
            queued.copy_canvas(true);
            queued.paste_canvas_with_files(|_| Vec::new());
            let queued_admission_ms = admission.elapsed().as_secs_f64() * 1000.0;
            if queued.pending_clipboard.len() != 2
                || queued.doc.layers.len() != 1
                || queued.doc.layers[0].pixels != original.layers[0].pixels
            {
                return Err("Clipboard admission changed artwork or lost an action".into());
            }
            let reserved = queued.input_target().ok_or("Paste identity missing")?;
            let ticket = queued.render_queue.snapshot()?;
            let mut io = MemoryClipboard(None);
            let mut queued_max_owner_call_ms = 0.0f64;
            let mut queued_calls = 0;
            let mut paste_publish_max_owner_call_ms = 0.0f64;
            let mut copy_publish_max_owner_call_ms = 0.0f64;
            let mut settlement_peak_bytes = 0;
            let drain_started = std::time::Instant::now();
            let mut snapshot_checked = false;
            while queued.render_queue.busy() {
                let started = std::time::Instant::now();
                let pending_paste = queued.paste_job.is_some();
                let pending_copy = queued.copy_job.is_some();
                if let Some(settlement) = &queued.clipboard_settlement {
                    settlement_peak_bytes =
                        settlement_peak_bytes.max(settlement.cursor.peak_payload_capacity_bytes());
                }
                queued.finish_copy_with_io(false, &mut io);
                queued.finish_paste_job(false);
                queued.advance_clipboard_settlement(
                    4096,
                    started + std::time::Duration::from_millis(2),
                );
                if let Some(settlement) = &queued.clipboard_settlement {
                    settlement_peak_bytes =
                        settlement_peak_bytes.max(settlement.cursor.peak_payload_capacity_bytes());
                }
                let published = pending_paste && queued.paste_job.is_none();
                let copied = pending_copy && queued.copy_job.is_none();
                let progress = if queued.render_queue.needs_work() {
                    queued.render_queue.advance_until(
                        &mut queued.doc,
                        &mut queued.history,
                        32,
                        started + std::time::Duration::from_millis(2),
                    )?
                } else {
                    crate::render_queue::Progress::default()
                };
                if let Some(barrier) = progress.barrier {
                    let action = queued
                        .pending_clipboard
                        .remove(&barrier)
                        .ok_or("Clipboard action missing")?;
                    match action {
                        Action::Paste { target, id, .. } => queued.start_paste_job(
                            barrier,
                            PasteInput::Raster(io.read()?),
                            target,
                            id,
                            None,
                        ),
                        Action::Copy { target, cut } => queued.start_copy_job(barrier, target, cut),
                        Action::CopyMask { target, mask, cut } => queued.start_copy_target_job(
                            barrier,
                            target,
                            crate::paint_target::Target::Mask(mask),
                            cut,
                        ),
                        Action::PasteMask { target, mask } => {
                            queued.start_mask_paste_job(barrier, io.read()?, target, mask)
                        }
                    }
                }
                if published {
                    paste_publish_max_owner_call_ms = paste_publish_max_owner_call_ms
                        .max(started.elapsed().as_secs_f64() * 1000.0);
                }
                if copied {
                    copy_publish_max_owner_call_ms = copy_publish_max_owner_call_ms
                        .max(started.elapsed().as_secs_f64() * 1000.0);
                }
                queued_max_owner_call_ms =
                    queued_max_owner_call_ms.max(started.elapsed().as_secs_f64() * 1000.0);
                queued_calls += 1;
                if queued_calls > 10000 {
                    return Err("Queued clipboard probe did not drain".into());
                }
                if let Some((actual, snapshot)) = progress.snapshot {
                    if actual != ticket
                        || snapshot.layers.len() != 2
                        || snapshot.layers[1].id != reserved
                        || snapshot.layers[1].pixels != original.layers[0].pixels
                    {
                        return Err("Queued clipboard snapshot lost identity or pixels".into());
                    }
                    snapshot_checked = true;
                }
                if queued.paste_job.is_some() || queued.copy_job.is_some() {
                    std::thread::sleep(std::time::Duration::from_millis(1));
                }
            }
            let queued_wall_ms = drain_started.elapsed().as_secs_f64() * 1000.0;
            if !snapshot_checked || !queued.pending_clipboard.is_empty() {
                return Err("Queued clipboard actions did not complete".into());
            }
            cases.push(serde_json::json!({"size":size, "storage":storage, "copy_ms":copy_ms,
                "cut_ms":cut_ms, "paste_prepare_ms":paste_prepare_ms,
                "raster_capacity_bytes":raster.rgba.capacity(), "pasted_pixel_capacity_bytes":pasted_pixels,
                "retained_history_bytes":retained_history_bytes, "rgba_and_undo_redo_verified":true,
                "queued_admission_ms":queued_admission_ms, "queued_max_owner_call_ms":queued_max_owner_call_ms,
                "queued_calls":queued_calls, "queued_identity_and_snapshot_verified":true,
                "clipboard_settlement_sampled_peak_bytes":settlement_peak_bytes,
                "copy_publish_max_owner_call_ms":copy_publish_max_owner_call_ms,
                "paste_publish_max_owner_call_ms":paste_publish_max_owner_call_ms, "queued_wall_ms":queued_wall_ms}));
        }
    }
    crate::storage::atomic_write(path, &serde_json::to_vec_pretty(&serde_json::json!({
        "version":5, "paste_scope":"Paste raster-to-sparse-layer preparation uses one background worker while its barrier holds. UI polling joins only finished workers, checks revision and exact source snapshot, then publishes atomically; synchronous document fences wait for that worker before replacement. Queued wall includes 1 ms worker polling and final verification. Paste publication max owner timing covers ready-worker poll/publication and that tick queue advance, excluding worker execution; clipboard read/decoder cost and history accounting remain UI work. Copy/cut extraction and candidate clearing use a background worker; owner publication validates revision and exact source before OS write, and changes artwork only after a successful write. Legacy fences finish both workers. Frozen paste source is registered; worker output/scratch is not included in the common source estimate.", "queue_scope":"Admission queues bounded action descriptors without finishing preceding work. Production barriers serialize source capture, mock clipboard IO and commit before following commands. Queued owner-call timing includes advance, ready-worker publication, mock IO and history commit, excludes worker raster preparation and full pixel verification; OS clipboard IO is absent. Copy publication timing includes mock sink raster cloning and that tick queue advance. Cut keeps its full checkpoint and holds the barrier during row-budgeted history compaction, choosing retained snapshot cost when smaller. The 2 ms allowance is checked between settlement units; OS IO and final history accounting remain synchronous and can overrun. Copy geometry is capped before allocating the raster. Frozen copy/paste sources are registered; raster and candidate worker outputs/scratch are not included in the common source estimate. Settlement payload capacity is sampled before/after polling and excludes a cursor completed within that call. Reserved paste identity and following snapshot are verified; failed actions release barriers. Not tablet/UI latency or total process memory.", "scope":"Supported 512/2048-square full-canvas raster clipboard core; sparse hidden-RGB fixture and dense flat/tiled RGBA. One repetition. Copy measures extraction; cut includes extraction and candidate pixel clearing with a successful no-op clipboard sink; paste measures sparse layer preparation. OS clipboard IO, input fence, history commit, UI/GPU, pixel checks and undo/redo are outside these timers. RGBA alpha and hidden RGB plus existing layer preservation checked exactly; both transactions undo/redo. Capacity fields are separate allocations/retained-history conventions and cannot be summed as process peak. Clipboard decoder temporary allocations, allocator overhead and RSS excluded. No host clipboard is read or modified.", "cases":cases
    })).map_err(|error| error.to_string())?)
}

#[cfg(test)]
mod tests {
    #[test]
    fn csp_cut_keeps_source_mode_and_external_raster_paste_keeps_raw_bytes() {
        let mut doc = crate::document::Document::new(2, 1);
        doc.layers[0].csp_byte_opacity = true;
        doc.layers[0].opacity = 156. / 256.;
        doc.layers[0].pixels = vec![10, 20, 30, 1, 40, 50, 60, 128].into();
        let captured = super::capture(&doc).unwrap();
        assert_eq!(captured.rgba, [10, 20, 30, 1, 40, 50, 60, 128]);
        let (_, cut) = super::prepare_copy(&doc, true).unwrap();
        let cut = cut.unwrap();
        assert!(cut.layers[0].csp_byte_opacity);
        assert_eq!(cut.layers[0].opacity, 156. / 256.);
        let pasted = super::prepare_paste(&doc, &captured).unwrap();
        assert!(pasted.layers[0].csp_byte_opacity);
        assert!(!pasted.layers[pasted.active].csp_byte_opacity);
        assert_eq!(pasted.layers[pasted.active].opacity, 1.0);
        assert_eq!(
            pasted.layers[pasted.active].pixels.as_slice(),
            [10, 20, 30, 1, 40, 50, 60, 128]
        );
    }
    use super::*;
    #[test]
    fn masked_copy_cut_preserves_holes_fractional_alpha_hidden_rgb_and_failed_write() {
        let mut doc = Document::new(517, 259);
        let mut coverage = vec![0; 517 * 259];
        coverage[255 * 517 + 255] = 255;
        coverage[255 * 517 + 257] = 128;
        coverage[256 * 517 + 255] = 255;
        let mask = crate::selection_mask::Mask::from_coverage(517, 259, coverage).unwrap();
        doc.selection = mask.bounds();
        doc.selection_mask = Some(mask);
        for (x, y, pixel) in [
            (255, 255, [17, 83, 201, 255]),
            (256, 255, [251, 7, 11, 255]),
            (257, 255, [31, 61, 97, 255]),
            (255, 256, [9, 11, 13, 0]),
        ] {
            doc.layers[0]
                .pixels
                .writer()
                .rgba_mut(y * 517 + x)
                .copy_from_slice(&pixel);
        }
        let before = doc.clone();
        assert!(copy_transaction(&doc, true, |_| Err("Clipboard busy".into())).is_err());
        assert!(crate::document::same_snapshot(&doc, &before));
        let mut copied = None;
        let cut = copy_transaction(&doc, true, |raster| {
            assert_eq!((raster.width, raster.height), (3, 2));
            copied = Some(raster.rgba.clone());
            Ok(())
        })
        .unwrap()
        .unwrap();
        assert_eq!(
            copied.unwrap(),
            [
                17, 83, 201, 255, 0, 0, 0, 0, 31, 61, 97, 128, 9, 11, 13, 0, 0, 0, 0, 0, 0, 0, 0, 0
            ]
        );
        assert_eq!(cut.layers[0].pixels.rgba(255 * 517 + 255), [0; 4]);
        assert_eq!(
            cut.layers[0].pixels.rgba(255 * 517 + 256),
            [251, 7, 11, 255]
        );
        assert_eq!(
            cut.layers[0].pixels.rgba(255 * 517 + 257),
            [31, 61, 97, 127]
        );
        assert_eq!(cut.layers[0].pixels.rgba(256 * 517 + 255), [0; 4]);
        let mut history = crate::document::History::new();
        history.checkpoint(&doc);
        doc = cut;
        history.settle(&doc);
        let after = doc.clone();
        assert!(history.undo(&mut doc));
        assert_eq!(doc.layers[0].pixels, before.layers[0].pixels);
        assert_eq!(doc.selection_mask, before.selection_mask);
        assert!(history.redo(&mut doc));
        assert_eq!(doc.layers[0].pixels, after.layers[0].pixels);
    }
    #[test]
    fn paste_replaces_mask_with_image_extent_and_undo_restores_shape() {
        let mut doc = Document::new(8, 8);
        let mask = crate::selection_mask::Mask::from_coverage(8, 8, vec![128; 64]).unwrap();
        doc.selection = mask.bounds();
        doc.selection_mask = Some(mask.clone());
        let raster = Raster {
            width: 2,
            height: 2,
            rgba: [17, 83, 201, 128].repeat(4),
        };
        let pasted = prepare_paste(&doc, &raster).unwrap();
        assert!(pasted.selection_mask.is_none());
        assert_eq!(
            pasted.layers[pasted.active].pixels.rgba(3 * 8 + 3),
            [17, 83, 201, 128]
        );
        let mut history = crate::document::History::new();
        history.checkpoint(&doc);
        doc = pasted;
        history.settle(&doc);
        assert!(history.undo(&mut doc));
        assert_eq!(doc.selection_mask, Some(mask));
        assert_eq!(doc.layers.len(), 1);
        let empty = crate::selection_mask::Mask::from_coverage(8, 8, vec![0; 64]).unwrap();
        doc.selection = empty.bounds();
        doc.selection_mask = Some(empty);
        assert!(
            copy_transaction(&doc, true, |_| panic!("Empty mask reached clipboard sink")).is_err()
        );
    }
    fn reach_paste_barrier(
        app: &mut Dudel,
    ) -> (u64, crate::document::LayerId, crate::document::LayerId) {
        app.paste_canvas_with_files(|_| Vec::new());
        let progress = app
            .render_queue
            .advance(&mut app.doc, &mut app.history, 1)
            .unwrap();
        let ticket = progress.barrier.unwrap();
        let Action::Paste { target, id, .. } = app.pending_clipboard.remove(&ticket).unwrap()
        else {
            panic!("Expected paste");
        };
        (ticket, target, id)
    }
    fn await_paste(app: &mut Dudel) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        let ctx = crate::egui::Context::default();
        while app.paste_job.is_some() {
            assert!(std::time::Instant::now() < deadline);
            app.poll_paste(&ctx);
            std::thread::yield_now();
        }
    }
    #[test]
    fn background_paste_holds_following_snapshot_until_atomic_publication() {
        let mut app = Dudel {
            doc: Document::new(517, 259),
            ..Default::default()
        };
        let original = app.doc.clone();
        let (ticket, target, id) = reach_paste_barrier(&mut app);
        let bytes = [9, 11, 13, 0, 17, 83, 201, 128].repeat(257 * 2);
        app.start_paste_job(
            ticket,
            crate::clipboard::PasteInput::Raster(Raster {
                width: 257,
                height: 4,
                rgba: bytes.clone(),
            }),
            target,
            id,
            None,
        );
        let snapshot = app.render_queue.snapshot().unwrap();
        let blocked = app
            .render_queue
            .advance(&mut app.doc, &mut app.history, 32)
            .unwrap();
        assert!(blocked.snapshot.is_none());
        assert_eq!(app.doc.layers.len(), 1);
        assert_eq!(app.doc.layers[0].pixels, original.layers[0].pixels);
        await_paste(&mut app);
        assert_eq!(app.doc.layers.len(), 2);
        assert_eq!(app.doc.layers[1].id, id);
        let selection = app.doc.selection.unwrap();
        assert_eq!(
            app.doc.layers[1].pixels.region(
                app.doc.width,
                [
                    selection.left,
                    selection.top,
                    selection.right,
                    selection.bottom
                ]
            ),
            bytes
        );
        let progress = app
            .render_queue
            .advance(&mut app.doc, &mut app.history, 1)
            .unwrap();
        assert_eq!(progress.snapshot.unwrap().0, snapshot);
        assert!(app.history.undo(&mut app.doc));
        assert_eq!(app.doc.layers.len(), 1);
        assert!(app.history.redo(&mut app.doc));
        assert_eq!(app.doc.layers[1].id, id);
    }
    #[test]
    fn invalid_and_stale_paste_results_release_barrier_without_replacing_artwork() {
        for stale in [false, true] {
            let mut app = Dudel {
                doc: Document::new(16, 16),
                ..Default::default()
            };
            let (ticket, target, id) = reach_paste_barrier(&mut app);
            // Oversized images now scale to fit, so the invalid case is a
            // payload whose bytes do not match its dimensions.
            let width = if stale { 1 } else { 3 };
            app.start_paste_job(
                ticket,
                crate::clipboard::PasteInput::Raster(Raster {
                    width,
                    height: 1,
                    rgba: [17, 83, 201, 128].to_vec(),
                }),
                target,
                id,
                None,
            );
            if stale {
                // Bypass revision bookkeeping to test the snapshot guard itself.
                app.doc.layers[0]
                    .pixels
                    .writer()
                    .rgba_mut(0)
                    .copy_from_slice(&[9, 11, 13, 0]);
            }
            let preserved = app.doc.layers[0].pixels.clone();
            await_paste(&mut app);
            assert_eq!(app.doc.layers.len(), 1);
            assert_eq!(app.doc.layers[0].pixels, preserved);
            assert!(app.status.starts_with("Clipboard:"));
            assert!(!app.render_queue.busy());
            assert!(!app.history.can_undo());
            assert!(app.pending_layers.is_none() && app.logical_layer.is_none());
        }
    }
    #[test]
    fn synchronous_document_fence_finishes_background_paste_before_later_changes() {
        let mut app = Dudel {
            doc: Document::new(16, 16),
            ..Default::default()
        };
        let (ticket, target, id) = reach_paste_barrier(&mut app);
        app.start_paste_job(
            ticket,
            crate::clipboard::PasteInput::Raster(Raster {
                width: 1,
                height: 1,
                rgba: vec![17, 83, 201, 128],
            }),
            target,
            id,
            None,
        );
        app.cancel_gestures();
        assert!(app.paste_job.is_none() && !app.render_queue.busy());
        assert_eq!(app.doc.layers.len(), 2);
        assert_eq!(app.doc.layers[1].id, id);
    }
    #[derive(Default)]
    struct MockClipboard {
        image: Option<Raster>,
        fail: bool,
    }
    impl ClipboardIo for MockClipboard {
        fn read(&mut self) -> Result<Raster, String> {
            if self.fail {
                return Err("clipboard unavailable".into());
            }
            let image = self.image.as_ref().ok_or("clipboard empty")?;
            Ok(Raster {
                width: image.width,
                height: image.height,
                rgba: image.rgba.clone(),
            })
        }
        fn write(&mut self, raster: &Raster) -> Result<(), String> {
            if self.fail {
                return Err("clipboard unavailable".into());
            }
            self.image = Some(Raster {
                width: raster.width,
                height: raster.height,
                rgba: raster.rgba.clone(),
            });
            Ok(())
        }
    }
    fn step(app: &mut Dudel, io: &mut MockClipboard) -> crate::render_queue::Progress {
        app.advance_clipboard_settlement(
            1,
            std::time::Instant::now() + std::time::Duration::from_secs(1),
        );
        if !app.render_queue.needs_work() {
            return Default::default();
        }
        let progress = app
            .render_queue
            .advance(&mut app.doc, &mut app.history, 1)
            .unwrap();
        if let Some(ticket) = progress.barrier {
            let action = app.pending_clipboard.remove(&ticket).unwrap();
            let result = execute_action(&app.doc, action, io);
            app.finish_clipboard_barrier(ticket, result);
        }
        progress
    }
    #[test]
    fn copy_rejects_unsupported_canvas_and_captures_maximum_canvas_selection() {
        let mut doc = Document::new(10001, 10000);
        let original = doc.clone();
        assert!(copy_transaction(&doc, true, |_| panic!("Oversized raster reached sink")).is_err());
        assert!(crate::document::same_snapshot(&doc, &original));
        doc = Document::new(4096, 1024);
        doc.selection = Some(Selection {
            left: 4090,
            top: 1016,
            right: 4096,
            bottom: 1024,
        });
        doc.layers[0]
            .pixels
            .writer()
            .rgba_mut(1023 * 4096 + 4095)
            .copy_from_slice(&[3, 5, 7, 0]);
        let updated = copy_transaction(&doc, true, |raster| {
            assert_eq!((raster.width, raster.height), (6, 8));
            assert_eq!(&raster.rgba[raster.rgba.len() - 4..], &[3, 5, 7, 0]);
            Ok(())
        })
        .unwrap()
        .unwrap();
        assert_eq!(updated.layers[0].pixels.rgba(1023 * 4096 + 4095), [0; 4]);
    }
    #[test]
    fn background_cut_waits_for_write_then_history_before_following_snapshot() {
        let mut app = Dudel {
            doc: Document::new(517, 259),
            ..Default::default()
        };
        app.doc.layers[0]
            .pixels
            .writer()
            .rgba_mut(256 * 517 + 256)
            .copy_from_slice(&[9, 11, 13, 0]);
        let original = app.doc.clone();
        app.copy_canvas(true);
        let snapshot = app.render_queue.snapshot().unwrap();
        let progress = app
            .render_queue
            .advance(&mut app.doc, &mut app.history, 1)
            .unwrap();
        app.complete_clipboard(progress.barrier.unwrap());
        assert!(app.copy_job.is_some());
        assert_eq!(app.doc.layers[0].pixels, original.layers[0].pixels);
        let held = app
            .render_queue
            .advance(&mut app.doc, &mut app.history, 32)
            .unwrap();
        assert!(held.snapshot.is_none());
        let mut io = MockClipboard::default();
        app.finish_copy_with_io(true, &mut io);
        assert!(app.copy_job.is_none() && app.clipboard_settlement.is_some());
        assert_eq!(
            &io.image.as_ref().unwrap().rgba[(256 * 517 + 256) * 4..][..4],
            &[9, 11, 13, 0]
        );
        app.finish_clipboard_settlement();
        let result = app
            .render_queue
            .advance(&mut app.doc, &mut app.history, 32)
            .unwrap();
        assert_eq!(result.snapshot.unwrap().0, snapshot);
        assert!(app.doc.layers[0].pixels.iter().all(|byte| *byte == 0));
        assert!(app.history.undo(&mut app.doc));
        assert_eq!(app.doc.layers[0].pixels, original.layers[0].pixels);
        assert!(app.history.redo(&mut app.doc));
        assert!(app.doc.layers[0].pixels.iter().all(|byte| *byte == 0));
    }
    #[test]
    fn background_copy_rejects_stale_revision_and_exact_source_before_clipboard_write() {
        for revision_change in [false, true] {
            let mut app = Dudel {
                doc: Document::new(8, 8),
                ..Default::default()
            };
            app.copy_canvas(true);
            let progress = app
                .render_queue
                .advance(&mut app.doc, &mut app.history, 1)
                .unwrap();
            app.complete_clipboard(progress.barrier.unwrap());
            if revision_change {
                app.revision += 1;
            } else {
                app.doc.layers[0]
                    .pixels
                    .writer()
                    .rgba_mut(0)
                    .copy_from_slice(&[3, 5, 7, 0]);
            }
            let current = app.doc.clone();
            let mut io = MockClipboard::default();
            app.finish_copy_with_io(true, &mut io);
            assert!(io.image.is_none());
            assert!(app.copy_job.is_none() && !app.render_queue.busy());
            assert!(app.clipboard_settlement.is_none());
            assert_eq!(app.doc.layers[0].pixels, current.layers[0].pixels);
            assert!(app.status.contains("Document changed"));
            assert!(!app.history.undo(&mut app.doc));
        }
    }
    #[test]
    fn background_cut_write_failure_preserves_pixels_selection_and_redo() {
        let mut app = Dudel {
            doc: Document::new(8, 8),
            ..Default::default()
        };
        app.history.checkpoint(&app.doc);
        app.doc.layers[0]
            .pixels
            .writer()
            .rgba_mut(0)
            .copy_from_slice(&[9, 11, 13, 0]);
        app.history.settle(&app.doc);
        assert!(app.history.undo(&mut app.doc));
        app.doc.selection = Some(Selection {
            left: 0,
            top: 0,
            right: 4,
            bottom: 4,
        });
        let original = app.doc.clone();
        app.copy_canvas(true);
        let progress = app
            .render_queue
            .advance(&mut app.doc, &mut app.history, 1)
            .unwrap();
        app.complete_clipboard(progress.barrier.unwrap());
        let mut io = MockClipboard {
            fail: true,
            ..Default::default()
        };
        app.finish_copy_with_io(true, &mut io);
        assert!(!app.render_queue.busy() && app.clipboard_settlement.is_none());
        assert!(crate::document::same_snapshot(&app.doc, &original));
        assert!(app.status.contains("clipboard unavailable"));
        assert!(app.history.redo(&mut app.doc));
        assert_eq!(app.doc.layers[0].pixels.rgba(0), [9, 11, 13, 0]);
    }
    #[test]
    fn cut_settlement_holds_barrier_and_avoids_canvas_sized_pixel_expansion() {
        let mut app = Dudel {
            doc: Document::new(517, 259),
            ..Default::default()
        };
        app.doc.layers[0]
            .pixels
            .writer()
            .rgba_mut(256 * 517 + 256)
            .copy_from_slice(&[9, 11, 13, 0]);
        let original = app.doc.clone();
        app.copy_canvas(true);
        let snapshot = app.render_queue.snapshot().unwrap();
        let mut io = MockClipboard::default();
        let first = step(&mut app, &mut io);
        assert!(first.barrier.is_some() && app.clipboard_settlement.is_some());
        assert!(app.history.has_pending_stroke());
        assert!(app.doc.layers[0].pixels.iter().all(|byte| *byte == 0));
        let held = app
            .render_queue
            .advance(&mut app.doc, &mut app.history, 32)
            .unwrap();
        assert!(held.snapshot.is_none());
        let mut peak = 0;
        let mut saved = None;
        for _ in 0..10000 {
            if !app.render_queue.busy() {
                break;
            }
            if let Some(settlement) = &app.clipboard_settlement {
                peak = peak.max(settlement.cursor.peak_payload_capacity_bytes());
            }
            if let Some(result) = step(&mut app, &mut io).snapshot {
                saved = Some(result);
            }
        }
        assert!(!app.render_queue.busy() && app.clipboard_settlement.is_none());
        assert_eq!(saved.unwrap().0, snapshot);
        assert!(peak < 300 * 1024);
        assert!(app.history.undo(&mut app.doc));
        assert_eq!(app.doc.layers[0].pixels, original.layers[0].pixels);
        assert!(app.history.redo(&mut app.doc));
        assert!(app.doc.layers[0].pixels.iter().all(|byte| *byte == 0));
    }
    #[test]
    fn clipboard_admission_limit_does_not_reserve_an_unqueued_paste_layer() {
        let mut app = Dudel {
            doc: Document::new(32, 32),
            ..Default::default()
        };
        let original = app.input_target().unwrap();
        for _ in 0..16 {
            app.copy_canvas(false);
        }
        assert_eq!(app.pending_clipboard.len(), 16);
        app.paste_canvas_with_files(|_| panic!("Rejected paste must not read the clipboard"));
        assert!(app.status.contains("Too many pending clipboard actions"));
        assert_eq!(app.input_target(), Some(original));
        assert_eq!(app.doc.layers.len(), 1);
        let mut io = MockClipboard::default();
        for _ in 0..1000 {
            if !app.render_queue.busy() {
                break;
            }
            step(&mut app, &mut io);
        }
        assert!(!app.render_queue.busy() && app.pending_clipboard.is_empty());
        assert!(!app.history.can_undo());
    }
    #[test]
    fn queued_cut_paste_and_following_stroke_use_reserved_layer_and_exact_undo() {
        let mut app = Dudel {
            doc: Document::new(64, 64),
            ..Default::default()
        };
        app.doc.layers[0]
            .pixels
            .writer()
            .rgba_mut(0)
            .copy_from_slice(&[9, 11, 13, 0]);
        app.doc.layers[0]
            .pixels
            .writer()
            .rgba_mut(32 * 64 + 32)
            .copy_from_slice(&[17, 83, 201, 128]);
        let original = app.doc.clone();
        app.copy_canvas(true);
        app.paste_canvas_with_files(|_| Vec::new());
        let reserved = app.input_target().unwrap();
        assert_ne!(reserved, original.layers[0].id);
        assert_eq!(app.doc.layers.len(), 1);
        assert_eq!(app.doc.layers[0].pixels, original.layers[0].pixels);
        let settings = std::sync::Arc::new(crate::stroke_buffer::Settings {
            target: crate::paint_target::Target::Raster,
            brush: crate::brush::Brush {
                size: 12.0,
                pressure_size: false,
                ..Default::default()
            },
            color: [201, 17, 83],
            secondary: [201, 17, 83],
            erase: false,
            layer: 0,
            symmetry: Default::default(),
        });
        let sample = crate::stroke::Sample {
            x: 12.0,
            y: 12.0,
            pressure: 1.0,
            ..Default::default()
        };
        let id = app
            .render_queue
            .begin_layer(settings.clone(), reserved)
            .unwrap();
        app.render_queue.dab(id, sample).unwrap();
        app.render_queue.end(id).unwrap();
        let snapshot_ticket = app.render_queue.snapshot().unwrap();
        let mut expected = original.clone();
        expected.insert_layer_above().unwrap();
        expected.layers[1].pixels = original.layers[0].pixels.clone();
        let mut painted_settings = (*settings).clone();
        painted_settings.layer = 1;
        let mut buffer = crate::stroke_buffer::StrokeBuffer::default();
        buffer.paint(&mut expected, sample, &painted_settings);
        let mut io = MockClipboard::default();
        let mut saved = None;
        for _ in 0..10000 {
            if !app.render_queue.busy() {
                break;
            }
            let progress = step(&mut app, &mut io);
            if let Some(snapshot) = progress.snapshot {
                saved = Some(snapshot);
            }
        }
        assert!(!app.render_queue.busy() && app.pending_clipboard.is_empty());
        assert_eq!(app.doc.layers[1].id, reserved);
        assert!(app.doc.layers[0].pixels.iter().all(|byte| *byte == 0));
        assert_eq!(app.doc.layers[1].pixels, expected.layers[1].pixels);
        let (ticket, snapshot) = saved.unwrap();
        assert_eq!(ticket, snapshot_ticket);
        assert_eq!(snapshot.layers[1].pixels, expected.layers[1].pixels);
        for _ in 0..3 {
            assert!(app.history.undo(&mut app.doc));
        }
        assert_eq!(app.doc.layers.len(), 1);
        assert_eq!(app.doc.layers[0].pixels, original.layers[0].pixels);
        for _ in 0..3 {
            assert!(app.history.redo(&mut app.doc));
        }
        assert_eq!(app.doc.layers[1].pixels, expected.layers[1].pixels);
    }
    #[test]
    fn clipboard_failure_releases_barrier_and_missing_paste_target_cannot_paint_old_layer() {
        let mut app = Dudel {
            doc: Document::new(64, 64),
            ..Default::default()
        };
        app.doc.layers[0]
            .pixels
            .writer()
            .rgba_mut(0)
            .copy_from_slice(&[9, 11, 13, 0]);
        let original = app.doc.layers[0].pixels.clone();
        app.copy_canvas(true);
        app.paste_canvas_with_files(|_| Vec::new());
        let reserved = app.input_target().unwrap();
        let settings = std::sync::Arc::new(crate::stroke_buffer::Settings {
            target: crate::paint_target::Target::Raster,
            brush: Default::default(),
            color: [201, 17, 83],
            secondary: [201, 17, 83],
            erase: false,
            layer: 0,
            symmetry: Default::default(),
        });
        let id = app.render_queue.begin_layer(settings, reserved).unwrap();
        app.render_queue
            .dab(
                id,
                crate::stroke::Sample {
                    x: 12.0,
                    y: 12.0,
                    pressure: 1.0,
                    ..Default::default()
                },
            )
            .unwrap();
        app.render_queue.end(id).unwrap();
        let mut io = MockClipboard {
            fail: true,
            ..Default::default()
        };
        for _ in 0..1000 {
            if !app.render_queue.busy() {
                break;
            }
            step(&mut app, &mut io);
        }
        assert!(!app.render_queue.busy());
        assert!(app.pending_clipboard.is_empty());
        assert_eq!(app.doc.layers.len(), 1);
        assert_eq!(app.doc.layers[0].pixels, original);
        assert!(!app.history.can_undo());
    }
    #[test]
    fn selected_cut_waits_for_success_preserves_hidden_rgba_and_undo_redo() {
        let mut doc = Document::new(517, 259);
        let selection = Selection {
            left: 255,
            top: 255,
            right: 258,
            bottom: 258,
        };
        doc.selection = Some(selection);
        let bytes = [9, 11, 13, 0, 17, 83, 201, 128, 201, 17, 83, 255].repeat(3);
        doc.layers[0]
            .pixels
            .writer()
            .write_region(doc.width, [255, 255, 258, 258], &bytes);
        doc.layers[0]
            .pixels
            .writer()
            .rgba_mut(0)
            .copy_from_slice(&[8, 7, 6, 255]);
        let original = doc.clone();
        assert!(copy_transaction(&doc, true, |_| Err("clipboard busy".into())).is_err());
        assert_eq!(doc.layers[0].pixels, original.layers[0].pixels);
        let mut copied = None;
        let updated = copy_transaction(&doc, true, |raster| {
            assert_eq!((raster.width, raster.height), (3, 3));
            assert_eq!(raster.rgba, bytes);
            copied = Some(Raster {
                width: raster.width,
                height: raster.height,
                rgba: raster.rgba.clone(),
            });
            Ok(())
        })
        .unwrap()
        .unwrap();
        let mut app = Dudel {
            doc,
            ..Default::default()
        };
        app.commit_clipboard_document(updated);
        assert_eq!(
            app.doc.layers[0].pixels.region(517, [255, 255, 258, 258]),
            vec![0; 36]
        );
        assert_eq!(app.doc.layers[0].pixels.rgba(0), [8, 7, 6, 255]);
        assert!(app.history.undo(&mut app.doc));
        assert_eq!(app.doc.layers[0].pixels, original.layers[0].pixels);
        assert_eq!(app.doc.selection, Some(selection));
        assert!(app.history.redo(&mut app.doc));
        let cut_pixels = app.doc.layers[0].pixels.clone();
        let pasted = prepare_paste(&app.doc, &copied.unwrap()).unwrap();
        app.commit_clipboard_document(pasted);
        assert_eq!(
            app.doc.layers[1].pixels.region(517, [255, 255, 258, 258]),
            bytes
        );
        assert_eq!(app.doc.layers[0].pixels, cut_pixels);
        assert!(app.history.undo(&mut app.doc));
        assert_eq!(app.doc.layers.len(), 1);
        assert!(app.history.redo(&mut app.doc));
        assert_eq!(app.doc.layers.len(), 2);
        assert_eq!(
            app.doc.layers[1].pixels.region(517, [255, 255, 258, 258]),
            bytes
        );
    }
    #[test]
    fn clipboard_locks_and_invalid_payloads_are_atomic() {
        let mut doc = Document::new(8, 8);
        doc.layers[0]
            .pixels
            .writer()
            .rgba_mut(0)
            .copy_from_slice(&[9, 11, 13, 0]);
        for alpha_lock in [false, true] {
            doc.layers[0].locked = !alpha_lock;
            doc.layers[0].alpha_locked = alpha_lock;
            assert!(
                copy_transaction(&doc, true, |_| panic!(
                    "locked cut must not write clipboard"
                ))
                .is_err()
            );
            assert!(copy_transaction(&doc, false, |_| Ok(())).unwrap().is_none());
        }
        for (width, height, rgba) in [(0, 1, vec![]), (usize::MAX, 2, vec![]), (2, 2, vec![0; 4])] {
            assert!(
                prepare_paste(
                    &doc,
                    &Raster {
                        width,
                        height,
                        rgba
                    }
                )
                .is_err()
            );
            assert_eq!(doc.layers.len(), 1);
            assert_eq!(doc.layers[0].pixels.rgba(0), [9, 11, 13, 0]);
        }
        for _ in 1..crate::document::MAX_RASTER_LAYERS {
            doc.add_layer();
        }
        assert!(
            prepare_paste(
                &doc,
                &Raster {
                    width: 1,
                    height: 1,
                    rgba: vec![17, 83, 201, 128]
                }
            )
            .is_err()
        );
        assert_eq!(doc.layers.len(), crate::document::MAX_RASTER_LAYERS);
    }
    #[test]
    fn paste_into_expanded_folder_preserves_stack_and_original_selection_on_undo() {
        let mut doc = Document::new(16, 16);
        doc.add_folder().unwrap();
        let active = doc.active;
        doc.layers[active].collapsed = false;
        doc.selection = Some(Selection {
            left: 14,
            top: 14,
            right: 16,
            bottom: 16,
        });
        let original = doc.clone();
        let pasted = prepare_paste(
            &doc,
            &Raster {
                width: 4,
                height: 4,
                rgba: [17, 83, 201, 128].repeat(16),
            },
        )
        .unwrap();
        pasted.validate().unwrap();
        assert_eq!(pasted.layers[pasted.active].depth, 1);
        assert_eq!(
            pasted.selection,
            Some(Selection {
                left: 12,
                top: 12,
                right: 16,
                bottom: 16
            })
        );
        let mut app = Dudel {
            doc,
            ..Default::default()
        };
        app.commit_clipboard_document(pasted);
        assert!(app.history.undo(&mut app.doc));
        assert_eq!(app.doc.active, original.active);
        assert_eq!(app.doc.selection, original.selection);
        assert_eq!(app.doc.layers.len(), original.layers.len());
    }
    #[test]
    fn clipboard_fence_includes_preceding_queued_artwork() {
        let mut app = Dudel {
            doc: Document::new(64, 64),
            ..Default::default()
        };
        let settings = std::sync::Arc::new(crate::stroke_buffer::Settings {
            target: crate::paint_target::Target::Raster,
            brush: crate::brush::Brush {
                size: 12.0,
                pressure_size: false,
                ..Default::default()
            },
            color: [17, 83, 201],
            secondary: [17, 83, 201],
            erase: false,
            layer: 0,
            symmetry: Default::default(),
        });
        let id = app.render_queue.begin(settings).unwrap();
        app.render_queue
            .dab(
                id,
                crate::stroke::Sample {
                    x: 32.0,
                    y: 32.0,
                    pressure: 1.0,
                    ..Default::default()
                },
            )
            .unwrap();
        app.render_queue.end(id).unwrap();
        assert!(app.render_queue.busy());
        app.copy_canvas(false);
        assert!(app.render_queue.busy() && app.pending_clipboard.len() == 1);
        assert_eq!(app.doc.layers[0].pixels.rgba(32 * 64 + 32), [0; 4]);
        let mut io = MockClipboard::default();
        for _ in 0..10000 {
            if !app.render_queue.busy() {
                break;
            }
            step(&mut app, &mut io);
        }
        assert!(!app.render_queue.busy());
        let copied = io.image.unwrap();
        assert!(copied.rgba.chunks_exact(4).any(|pixel| pixel[3] > 0));
        assert!(app.history.can_undo());
    }
}
