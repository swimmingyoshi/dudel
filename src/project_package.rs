//! Portable .zip project packages. A package holds the exact .dudel project
//! (lossless re-import), each raster layer and mask as a full-canvas PNG
//! inside folders that mirror the layer stack, reference images, a flattened
//! preview and a manifest. A package without `project.dudel` is rebuilt from
//! the manifest and layer PNGs, so layers edited elsewhere can come back.
use crate::{Dudel, document::Document, egui};
use serde::{Deserialize, Serialize};
use std::{
    io::{Read, Seek, Write},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

pub const PROJECT: &str = "project.dudel";
pub const MANIFEST: &str = "manifest.json";
pub const PREVIEW: &str = "preview.png";
const FORMAT: &str = "dudel-package";
const VERSION: u32 = 2;
/// Largest entry read from a package; bounds memory and zip bombs.
const MAX_ENTRY: u64 = 4 << 30;
const MAX_MANIFEST: u64 = 4 << 20;

#[derive(Serialize, Deserialize)]
pub struct Manifest {
    pub format: String,
    pub version: u32,
    pub app: String,
    pub name: String,
    pub width: usize,
    pub height: usize,
    /// Stack order, bottom layer first; folders precede nothing and contain
    /// the deeper entries directly below them.
    pub layers: Vec<LayerEntry>,
    #[serde(default)]
    pub references: Vec<String>,
    #[serde(default)]
    pub reference_boards: Vec<crate::reference_layers::ReferenceLayer>,
    #[serde(default)]
    pub guides: Option<Box<crate::guides::Guides>>,
}
#[derive(Serialize, Deserialize, Debug, PartialEq)]
pub struct LayerEntry {
    pub name: String,
    #[serde(default)]
    pub folder: bool,
    #[serde(default)]
    pub depth: u8,
    #[serde(default = "visible")]
    pub visible: bool,
    #[serde(default = "full")]
    pub opacity: f32,
    #[serde(default)]
    pub blend: crate::blend::BlendMode,
    #[serde(default)]
    pub clipped: bool,
    #[serde(default)]
    pub locked: bool,
    #[serde(default)]
    pub alpha_locked: bool,
    #[serde(default)]
    pub collapsed: bool,
    #[serde(default)]
    pub csp_byte_opacity: bool,
    #[serde(default = "visible")]
    pub mask_enabled: bool,
    #[serde(default = "visible")]
    pub mask_linked: bool,
    /// Full-canvas RGBA PNG; None for folders.
    #[serde(default)]
    pub image: Option<String>,
    /// Grayscale PNG of the layer mask, white revealing.
    #[serde(default)]
    pub mask: Option<String>,
}
fn visible() -> bool {
    true
}
fn full() -> f32 {
    1.0
}

/// A file or folder name safe on every platform.
fn clean(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c.is_control() || r#"<>:"/\|?*"#.contains(c) {
                '_'
            } else {
                c
            }
        })
        .take(60)
        .collect();
    let cleaned = cleaned.trim().trim_end_matches('.').trim().to_owned();
    if cleaned.is_empty() {
        "Untitled".into()
    } else {
        cleaned
    }
}

/// Zip directory for each stack entry: its enclosing folders, outermost first.
fn folder_paths(doc: &Document) -> Vec<String> {
    let mut paths = vec![String::new(); doc.layers.len()];
    // Walk top-down so each folder's path is known before its contents.
    let mut open: Vec<(u8, String)> = Vec::new();
    for index in (0..doc.layers.len()).rev() {
        let layer = &doc.layers[index];
        while open.last().is_some_and(|(depth, _)| *depth >= layer.depth) {
            open.pop();
        }
        let parent = open.last().map_or(String::new(), |(_, p)| p.clone());
        paths[index] = parent.clone();
        if layer.folder {
            open.push((
                layer.depth,
                format!("{parent}{:02} {}/", index + 1, clean(&layer.name)),
            ));
        }
    }
    paths
}

fn png(
    writer: &mut impl Write,
    rgba: &[u8],
    width: usize,
    height: usize,
    color: image::ExtendedColorType,
) -> Result<(), String> {
    use image::ImageEncoder;
    image::codecs::png::PngEncoder::new_with_quality(
        writer,
        image::codecs::png::CompressionType::Fast,
        image::codecs::png::FilterType::Adaptive,
    )
    .write_image(rgba, width as u32, height as u32, color)
    .map_err(|e| e.to_string())
}

fn destination(path: &Path) -> Result<(), String> {
    if !path
        .extension()
        .and_then(|s| s.to_str())
        .is_some_and(|s| s.eq_ignore_ascii_case("zip"))
    {
        return Err("Choose a .zip filename for the project package".into());
    }
    match std::fs::File::open(path) {
        Ok(mut file) => {
            // Never replace something that is not already a zip.
            if file.metadata().map_err(|e| e.to_string())?.len() != 0 {
                let mut signature = [0; 4];
                if file.read_exact(&mut signature).is_err()
                    || !matches!(&signature, b"PK\x03\x04" | b"PK\x05\x06")
                {
                    return Err("That file is not a zip package; choose a new filename".into());
                }
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.to_string()),
    }
    Ok(())
}

/// Write `doc` (with guides and references attached) as a package at `path`.
pub(crate) fn write(
    doc: &Document,
    name: &str,
    path: &Path,
    cancel: &AtomicBool,
) -> Result<(), String> {
    doc.validate()?;
    destination(path)?;
    let check = || crate::fill::check_cancel(Some(cancel));
    crate::storage::atomic_stream(path, |file| {
        let mut zip = zip::ZipWriter::new(file);
        let stored = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored)
            .large_file(true);
        let deflated = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        let zip_error = |e: zip::result::ZipError| e.to_string();
        zip.start_file(PROJECT, stored).map_err(zip_error)?;
        crate::tile_project::encode_cancelable(doc, &mut zip, Some(cancel))?;
        let folders = folder_paths(doc);
        let mut layers = Vec::with_capacity(doc.layers.len());
        for (index, layer) in doc.layers.iter().enumerate() {
            check()?;
            let stem = format!(
                "layers/{}{:02} {}",
                folders[index],
                index + 1,
                clean(&layer.name)
            );
            let mut entry = LayerEntry {
                name: layer.name.clone(),
                folder: layer.folder,
                depth: layer.depth,
                visible: layer.visible,
                opacity: layer.opacity,
                blend: layer.blend,
                clipped: layer.clipped,
                locked: layer.locked,
                alpha_locked: layer.alpha_locked,
                collapsed: layer.collapsed,
                csp_byte_opacity: layer.csp_byte_opacity,
                mask_enabled: layer.mask.as_ref().is_none_or(|m| m.enabled),
                mask_linked: layer.mask.as_ref().is_none_or(|m| m.linked),
                image: None,
                mask: None,
            };
            if !layer.folder {
                let image = format!("{stem}.png");
                zip.start_file(&image, stored).map_err(zip_error)?;
                let rgba = layer
                    .pixels
                    .region(doc.width, [0, 0, doc.width, doc.height]);
                png(
                    &mut zip,
                    &rgba,
                    doc.width,
                    doc.height,
                    image::ExtendedColorType::Rgba8,
                )?;
                entry.image = Some(image);
            }
            if let Some(mask) = &layer.mask {
                let image = if layer.folder {
                    format!(
                        "layers/{}{:02} {}/mask.png",
                        folders[index],
                        index + 1,
                        clean(&layer.name)
                    )
                } else {
                    format!("{stem} mask.png")
                };
                let mut gray = Vec::with_capacity(doc.width * doc.height);
                for y in 0..doc.height {
                    gray.extend((0..doc.width).map(|x| mask.coverage.at(x, y)));
                }
                check()?;
                zip.start_file(&image, stored).map_err(zip_error)?;
                png(
                    &mut zip,
                    &gray,
                    doc.width,
                    doc.height,
                    image::ExtendedColorType::L8,
                )?;
                entry.mask = Some(image);
            }
            layers.push(entry);
        }
        let mut references = Vec::new();
        for (board_index, board) in doc.references.iter().enumerate() {
            for (image_index, placement) in board.placements().enumerate() {
                check()?;
                let image = &placement.image;
                if image.width == 0 || image.height == 0 {
                    continue;
                }
                let path = format!(
                    "references/{:02} {}/{:02}.png",
                    board_index + 1,
                    clean(&board.name),
                    image_index + 1
                );
                zip.start_file(&path, stored).map_err(zip_error)?;
                png(
                    &mut zip,
                    &image.rgba,
                    image.width,
                    image.height,
                    image::ExtendedColorType::Rgba8,
                )?;
                references.push(path);
            }
        }
        check()?;
        let preview = doc.composite_bounds_checked([0, 0, doc.width, doc.height], true, check)?;
        zip.start_file(PREVIEW, stored).map_err(zip_error)?;
        png(
            &mut zip,
            &preview,
            doc.width,
            doc.height,
            image::ExtendedColorType::Rgba8,
        )?;
        let manifest = Manifest {
            format: FORMAT.into(),
            version: VERSION,
            app: format!("Dudel {}", env!("CARGO_PKG_VERSION")),
            name: name.into(),
            width: doc.width,
            height: doc.height,
            layers,
            references,
            reference_boards: doc.references.clone(),
            guides: doc.guides.clone(),
        };
        zip.start_file(MANIFEST, deflated).map_err(zip_error)?;
        serde_json::to_writer_pretty(&mut zip, &manifest).map_err(|e| e.to_string())?;
        check()?;
        zip.finish().map_err(zip_error)?;
        check()?;
        destination(path)
    })
}

fn open_entry<'a, R: Read + Seek>(
    archive: &'a mut zip::ZipArchive<R>,
    name: &str,
    limit: u64,
) -> Result<Option<impl Read + 'a>, String> {
    let entry = match archive.by_name(name) {
        Ok(entry) => entry,
        Err(zip::result::ZipError::FileNotFound) => return Ok(None),
        Err(e) => return Err(format!("{name}: {e}")),
    };
    if entry.size() > limit {
        return Err(format!("{name} is too large"));
    }
    Ok(Some(entry.take(limit)))
}

fn read_manifest<R: Read + Seek>(
    archive: &mut zip::ZipArchive<R>,
) -> Result<Option<Manifest>, String> {
    let Some(mut reader) = open_entry(archive, MANIFEST, MAX_MANIFEST)? else {
        return Ok(None);
    };
    let mut text = String::new();
    reader
        .read_to_string(&mut text)
        .map_err(|e| format!("Damaged manifest: {e}"))?;
    let manifest: Manifest =
        serde_json::from_str(&text).map_err(|e| format!("Damaged manifest: {e}"))?;
    if manifest.format != FORMAT || manifest.version > VERSION {
        return Err("This package was made by a newer Dudel or another program".into());
    }
    Ok(Some(manifest))
}

/// A free `<folder>/<name>.dudel`, adding a number when the name is taken.
fn unique_project(folder: &Path, name: &str) -> PathBuf {
    let name = clean(name);
    let mut candidate = folder.join(format!("{name}.dudel"));
    let mut number = 2;
    while candidate.exists() {
        candidate = folder.join(format!("{name} {number}.dudel"));
        number += 1;
    }
    candidate
}

/// Unpack `package` into a new .dudel in `folder` and return its path.
pub(crate) fn import(
    package: &Path,
    folder: &Path,
    cancel: &AtomicBool,
) -> Result<PathBuf, String> {
    let check = || crate::fill::check_cancel(Some(cancel));
    let file = std::fs::File::open(package).map_err(|e| e.to_string())?;
    let mut archive = zip::ZipArchive::new(std::io::BufReader::new(file))
        .map_err(|e| format!("Not a readable zip: {e}"))?;
    let manifest = read_manifest(&mut archive)?;
    let name = manifest
        .as_ref()
        .map(|m| m.name.clone())
        .filter(|n| !n.trim().is_empty())
        .or_else(|| {
            package
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
        })
        .unwrap_or_else(|| "Imported".into());
    std::fs::create_dir_all(folder)
        .map_err(|e| format!("Could not create {}: {e}", folder.display()))?;
    let target = unique_project(folder, &name);
    if let Some(mut reader) = open_entry(&mut archive, PROJECT, MAX_ENTRY)? {
        // Validate before publishing so a damaged package leaves nothing behind.
        let staged = tempfile::NamedTempFile::new_in(folder).map_err(|e| e.to_string())?;
        let mut writer = std::io::BufWriter::new(staged.as_file());
        std::io::copy(&mut reader, &mut writer).map_err(|e| format!("Damaged project: {e}"))?;
        writer.flush().map_err(|e| e.to_string())?;
        drop(writer);
        check()?;
        crate::storage::load_project(staged.path())?;
        staged.as_file().sync_all().map_err(|e| e.to_string())?;
        check()?;
        staged
            .persist_noclobber(&target)
            .map_err(|e| e.error.to_string())?;
        return Ok(target);
    }
    let Some(manifest) = manifest else {
        return Err("Not a Dudel package: it has neither project.dudel nor manifest.json".into());
    };
    let doc = rebuild(&mut archive, &manifest, &check)?;
    check()?;
    let staged = tempfile::NamedTempFile::new_in(folder).map_err(|e| e.to_string())?;
    {
        let mut writer = std::io::BufWriter::new(staged.as_file());
        crate::tile_project::encode_cancelable(&doc, &mut writer, Some(cancel))?;
        writer.flush().map_err(|e| e.to_string())?;
    }
    staged.as_file().sync_all().map_err(|e| e.to_string())?;
    check()?;
    staged
        .persist_noclobber(&target)
        .map_err(|e| e.error.to_string())?;
    Ok(target)
}

/// Assemble a document from the manifest's layer PNGs.
fn read_png<R: Read + Seek>(
    archive: &mut zip::ZipArchive<R>,
    name: &str,
    dimensions: Option<(usize, usize)>,
) -> Result<image::DynamicImage, String> {
    use image::ImageDecoder;
    // A full 100-megapixel layer of noise compresses to at most about 400 MiB.
    let Some(mut reader) = open_entry(archive, name, 512 << 20)? else {
        return Err(format!("Package is missing {name}"));
    };
    let mut bytes = Vec::new();
    reader
        .read_to_end(&mut bytes)
        .map_err(|e| format!("{name}: {e}"))?;
    let decoder = image::codecs::png::PngDecoder::new(std::io::Cursor::new(bytes))
        .map_err(|e| format!("{name}: {e}"))?;
    let (w, h) = decoder.dimensions();
    // Validate the header before allocating decoded pixels.
    if w == 0
        || h == 0
        || w as usize > crate::document::MAX_SIDE
        || h as usize > crate::document::MAX_SIDE
        || dimensions.is_some_and(|size| size != (w as usize, h as usize))
    {
        return Err(format!("Unsupported image dimensions for {name}"));
    }
    image::DynamicImage::from_decoder(decoder).map_err(|e| format!("{name}: {e}"))
}

fn rebuild<R: Read + Seek>(
    archive: &mut zip::ZipArchive<R>,
    manifest: &Manifest,
    check: &dyn Fn() -> Result<(), String>,
) -> Result<Document, String> {
    let (width, height) = (manifest.width, manifest.height);
    crate::canvas_size::validate_size(width, height)?;
    if manifest.layers.is_empty() || manifest.layers.len() > crate::document::MAX_ENTRIES {
        return Err("Unsupported layer count in package".into());
    }
    crate::reference_layers::validate(&manifest.reference_boards, false)?;
    let mut doc = Document::new(width, height);
    let mut layers = Vec::new();
    for entry in &manifest.layers {
        check()?;
        let mut layer = if entry.folder {
            crate::document::Layer::new_folder(entry.name.clone())
        } else {
            crate::document::Layer::raster(entry.name.clone(), width, height)
        };
        layer.depth = entry.depth;
        layer.visible = entry.visible;
        layer.opacity = entry.opacity.clamp(0.0, 1.0);
        layer.blend = entry.blend;
        layer.clipped = entry.clipped;
        layer.locked = entry.locked;
        layer.alpha_locked = entry.alpha_locked;
        layer.collapsed = entry.collapsed;
        layer.csp_byte_opacity = entry.csp_byte_opacity;
        if !entry.folder {
            let image = entry
                .image
                .as_deref()
                .ok_or("Raster layer has no image in package")?;
            let decoded = read_png(archive, image, Some((width, height)))?.into_rgba8();
            layer
                .pixels
                .writer()
                .write_region(width, [0, 0, width, height], decoded.as_raw());
        }
        if let Some(mask) = &entry.mask {
            doc.version = 2;
            check()?;
            let gray = read_png(archive, mask, Some((width, height)))?.into_luma8();
            layer.mask = Some(crate::layer_mask::LayerMask {
                id: crate::document::LayerId::new(),
                enabled: entry.mask_enabled,
                linked: entry.mask_linked,
                coverage: crate::layer_mask::Mask::from_coverage(
                    width,
                    height,
                    gray.as_raw(),
                    None,
                )?,
            });
        }
        layers.push(layer);
    }
    if layers.is_empty() {
        return Err("Package lists no layers".into());
    }
    doc.layers = layers;
    doc.active = doc.layers.iter().rposition(|l| !l.folder).unwrap_or(0);
    doc.guides = manifest.guides.clone();
    let expected = manifest
        .reference_boards
        .iter()
        .map(|b| b.images.len() + 1)
        .sum::<usize>();
    if expected != manifest.references.len() {
        return Err(
            "Reference metadata is missing or incomplete; keep project.dudel in this package"
                .into(),
        );
    }
    doc.references = manifest.reference_boards.clone();
    let mut paths = manifest.references.iter();
    let mut reference_bytes = 0usize;
    for board in &mut doc.references {
        for image in
            std::iter::once(&mut board.image).chain(board.images.iter_mut().map(|p| &mut p.image))
        {
            check()?;
            let name = paths.next().ok_or("Missing reference path")?;
            let decoded = read_png(archive, name, None)?.into_rgba8();
            reference_bytes = reference_bytes.saturating_add(decoded.as_raw().len());
            if reference_bytes > 512 * 1024 * 1024 {
                return Err("Reference images exceed the project memory limit".into());
            }
            *image = Arc::new(crate::reference_layers::Image {
                width: decoded.width() as usize,
                height: decoded.height() as usize,
                rgba: decoded.into_raw(),
            });
        }
    }
    doc.validate()?;
    Ok(doc)
}

pub(crate) struct ImportJob {
    worker: std::thread::JoinHandle<Result<PathBuf, String>>,
    cancel: Arc<AtomicBool>,
}

impl Dudel {
    pub(crate) fn export_package(&mut self) {
        if self.export_job.is_some() || self.queued_export.is_some() {
            self.status = "An export is already running".into();
            return;
        }
        let dialog = self.dialog_in(
            crate::locations::Kind::Packages,
            rfd::AsyncFileDialog::new()
                .add_filter("Dudel project package", &["zip"])
                .set_file_name(format!("{}.zip", clean(&self.project_name()))),
        );
        self.ask_save_file(dialog, |app, path| app.export_package_to(path));
    }
    pub(crate) fn project_name(&self) -> String {
        self.path
            .as_deref()
            .and_then(Path::file_stem)
            .map_or("Untitled".into(), |s| s.to_string_lossy().into_owned())
    }
    pub(crate) fn export_package_to(&mut self, path: PathBuf) {
        if let Err(error) = destination(&path) {
            self.status = format!("Package failed: {error}");
            return;
        }
        self.export_snapshot_or_queue(path, false, true);
    }
    /// Attach guides and references the way a save does, then write in the background.
    pub(crate) fn start_package_snapshot(&mut self, path: PathBuf, mut document: Document) {
        self.track_guide_changes();
        document.guides = Some(Box::new(self.guides_record(&document)));
        self.reference_layers.saved_positions(&self.doc);
        document.references = self.reference_layers.layers.clone();
        let name = self.project_name();
        self.start_export_worker(path, document, true, move |document, path, cancel| {
            write(document, &name, path, cancel)
        });
    }
    pub(crate) fn import_package(&mut self) {
        if self.package_import.is_some() {
            self.status = "A package import is already running".into();
            return;
        }
        let dialog = self.dialog_in(
            crate::locations::Kind::Packages,
            rfd::AsyncFileDialog::new().add_filter("Dudel project package", &["zip"]),
        );
        self.ask_open_file(dialog, Self::import_package_from);
    }
    pub(crate) fn import_package_from(&mut self, package: PathBuf) {
        if self.package_import.is_some() {
            return;
        }
        let folder = self.locations.get(crate::locations::Kind::Projects);
        let cancel = Arc::new(AtomicBool::new(false));
        let stop = cancel.clone();
        let worker = std::thread::Builder::new()
            .name("dudel-package-import".into())
            .spawn(move || import(&package, &folder, &stop));
        match worker {
            Ok(worker) => {
                self.package_import = Some(ImportJob { worker, cancel });
                self.status = "Unpacking project package…".into();
            }
            Err(error) => self.status = format!("Package import could not start: {error}"),
        }
    }
    pub(crate) fn poll_package_import(&mut self, ctx: &egui::Context) {
        let Some(job) = &self.package_import else {
            return;
        };
        if !job.worker.is_finished() {
            ctx.request_repaint_after(std::time::Duration::from_millis(50));
            egui::TopBottomPanel::top("package import progress").show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label("Unpacking project package…");
                    if ui.button("Cancel").clicked() {
                        job.cancel.store(true, Ordering::Relaxed);
                    }
                });
            });
            return;
        }
        let job = self.package_import.take().unwrap();
        let canceled = job.cancel.load(Ordering::Relaxed);
        match job.worker.join() {
            Ok(Ok(path)) => {
                self.status = format!("Package unpacked to {}", path.display());
                self.open_path(path);
            }
            Ok(Err(_)) if canceled => self.status = "Package import canceled".into(),
            Ok(Err(error)) => self.status = format!("Package import failed: {error}"),
            Err(_) => self.status = "Package import worker failed".into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn painted() -> Document {
        let mut doc = Document::new(40, 24);
        doc.layers[0].name = "Paper: base".into();
        doc.layers[0]
            .pixels
            .writer()
            .write_region(40, [2, 3, 6, 5], &[200, 10, 30, 255].repeat(8));
        doc.layers
            .push(crate::document::Layer::raster("Ink".into(), 40, 24));
        doc.layers[1].depth = 1;
        doc.layers[1].opacity = 0.5;
        doc.layers[1]
            .pixels
            .writer()
            .write_region(40, [10, 10, 12, 11], &[0, 0, 0, 255].repeat(2));
        doc.layers
            .push(crate::document::Layer::new_folder("Line/art".into()));
        doc.active = 1;
        doc.validate().unwrap();
        doc
    }
    fn names(path: &Path) -> Vec<String> {
        let mut archive = zip::ZipArchive::new(std::fs::File::open(path).unwrap()).unwrap();
        (0..archive.len())
            .map(|i| archive.by_index(i).unwrap().name().unwrap().into_owned())
            .collect()
    }
    #[test]
    fn package_round_trips_exactly_and_mirrors_folders_as_directories() {
        let dir = tempfile::tempdir().unwrap();
        let doc = painted();
        let package = dir.path().join("Study.zip");
        write(&doc, "Study", &package, &AtomicBool::new(false)).unwrap();
        let entries = names(&package);
        for expected in [
            PROJECT,
            "layers/01 Paper_ base.png",
            "layers/03 Line_art/02 Ink.png",
            PREVIEW,
            MANIFEST,
        ] {
            assert!(
                entries.iter().any(|e| e == expected),
                "{expected} in {entries:?}"
            );
        }
        let projects = dir.path().join("Projects");
        let opened = import(&package, &projects, &AtomicBool::new(false)).unwrap();
        assert_eq!(opened, projects.join("Study.dudel"));
        let restored = crate::storage::load_project(&opened).unwrap();
        assert_eq!(restored.layers.len(), 3);
        assert_eq!(restored.layers[1].opacity, 0.5);
        assert_eq!(
            restored.layers[0].pixels.rgba(3 * 40 + 2),
            [200, 10, 30, 255]
        );
        // A second import never overwrites the first.
        let again = import(&package, &projects, &AtomicBool::new(false)).unwrap();
        assert_eq!(again, projects.join("Study 2.dudel"));
    }
    #[test]
    fn package_without_project_is_rebuilt_from_layer_pngs() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = Dudel {
            doc: painted(),
            ..Default::default()
        };
        app.doc.version = 2;
        for (index, enabled, linked) in [(0, true, false), (2, false, true)] {
            let mut coverage = vec![255; 40 * 24];
            coverage[3 * 40 + 2] = 0;
            app.doc.layers[index].mask = Some(crate::layer_mask::LayerMask {
                id: crate::document::LayerId::new(),
                enabled,
                linked,
                coverage: crate::layer_mask::Mask::from_coverage(40, 24, &coverage, None).unwrap(),
            });
        }
        app.doc.layers[1].locked = true;
        app.doc.layers[1].alpha_locked = true;
        app.doc.layers[2].collapsed = true;
        app.insert_reference(
            "Hair".into(),
            crate::reference_layers::Image {
                width: 2,
                height: 2,
                rgba: [123, 45, 6, 255].repeat(4),
            },
        );
        let id = app.reference_layers.layers[0].id;
        app.insert_reference_image(
            id,
            "Details".into(),
            crate::reference_layers::Image {
                width: 1,
                height: 1,
                rgba: vec![4, 3, 2, 128],
            },
        )
        .unwrap();
        app.place_reference(id, 1, 1);
        app.reference_layers.saved_positions(&app.doc);
        let mut doc = app.doc.clone();
        doc.references = app.reference_layers.layers.clone();
        doc.guides = Some(Box::new(crate::guides::Guides::default()));
        let full = dir.path().join("full.zip");
        write(&doc, "Rebuilt", &full, &AtomicBool::new(false)).unwrap();
        // Copy every entry except the project, as a hand-made package would be.
        let mut source = zip::ZipArchive::new(std::fs::File::open(&full).unwrap()).unwrap();
        let partial = dir.path().join("layers only.zip");
        let mut out = zip::ZipWriter::new(std::fs::File::create(&partial).unwrap());
        for i in 0..source.len() {
            let entry = source.by_index(i).unwrap();
            if entry.name().unwrap() != PROJECT {
                out.raw_copy_file(entry).unwrap();
            }
        }
        out.finish().unwrap();
        let opened = import(&partial, dir.path(), &AtomicBool::new(false)).unwrap();
        let restored = crate::storage::load_project(&opened).unwrap();
        assert_eq!(restored.layers.len(), 3);
        assert!(restored.layers[2].folder && restored.layers[1].depth == 1);
        assert_eq!(restored.layers[1].pixels.rgba(10 * 40 + 10), [0, 0, 0, 255]);
        assert_eq!(restored.layers[1].opacity, 0.5);
        assert_eq!(restored.composite(), doc.composite());
        assert!(restored.layers[1].locked && restored.layers[1].alpha_locked);
        assert!(restored.layers[2].collapsed);
        for index in [0, 2] {
            let a = doc.layers[index].mask.as_ref().unwrap();
            let b = restored.layers[index].mask.as_ref().unwrap();
            assert_eq!((a.enabled, a.linked), (b.enabled, b.linked));
            assert!(a.coverage.equivalent(&b.coverage, None).unwrap());
        }
        assert_eq!(restored.guides, doc.guides);
        assert_eq!(
            serde_json::to_value(&restored.references).unwrap(),
            serde_json::to_value(&doc.references).unwrap()
        );
        assert_eq!(
            restored.references[0].image.rgba,
            doc.references[0].image.rgba
        );
        assert_eq!(
            restored.references[0].images[0].image.rgba,
            doc.references[0].images[0].image.rgba
        );
    }
    #[test]
    fn packages_refuse_other_files_damaged_input_and_cancellation() {
        let dir = tempfile::tempdir().unwrap();
        let doc = painted();
        let project = dir.path().join("keep.zip");
        std::fs::write(&project, b"DUDL not a zip").unwrap();
        assert!(write(&doc, "x", &project, &AtomicBool::new(false)).is_err());
        assert_eq!(std::fs::read(&project).unwrap(), b"DUDL not a zip");
        assert!(
            write(
                &doc,
                "x",
                &dir.path().join("x.png"),
                &AtomicBool::new(false)
            )
            .is_err()
        );
        let out = dir.path().join("Projects");
        assert!(import(&project, &out, &AtomicBool::new(false)).is_err());
        let canceled = dir.path().join("canceled.zip");
        assert!(write(&doc, "x", &canceled, &AtomicBool::new(true)).is_err());
        assert!(!canceled.exists());
        // An empty zip is not a package and leaves no project behind.
        let empty = dir.path().join("empty.zip");
        zip::ZipWriter::new(std::fs::File::create(&empty).unwrap())
            .finish()
            .unwrap();
        assert!(import(&empty, &out, &AtomicBool::new(false)).is_err());
        assert!(std::fs::read_dir(&out).map_or(true, |d| d.count() == 0));
    }
}
