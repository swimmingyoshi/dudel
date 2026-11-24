//! Exporting an animation: an animated GIF, numbered PNG frames, a sprite
//! sheet, or an MP4 video. Each frame is the document as the
//! canvas shows it while playing (`animation::frame_view`), without the
//! onion skin, composited like Export image and optionally scaled up by a
//! whole number for pixel art. Runs as a background export job.

use crate::animation::{self, Animation, Frame, Playback};
use crate::document::Document;
use crate::{Dudel, egui};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) enum Format {
    #[default]
    Gif,
    /// One PNG per frame, numbered after the chosen name.
    PngFrames,
    /// Every frame in one PNG, left to right and top to bottom.
    SpriteSheet,
    Mp4,
}
impl Format {
    pub(crate) const ALL: [Self; 4] = [Self::Gif, Self::PngFrames, Self::SpriteSheet, Self::Mp4];
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Gif => "Animated GIF",
            Self::PngFrames => "PNG frames",
            Self::SpriteSheet => "Sprite sheet (PNG)",
            Self::Mp4 => "MP4 video",
        }
    }
    fn extension(self) -> &'static str {
        match self {
            Self::Gif => "gif",
            Self::Mp4 => "mp4",
            Self::PngFrames | Self::SpriteSheet => "png",
        }
    }
    fn hint(self) -> &'static str {
        match self {
            Self::Gif => {
                "Plays in browsers and chats. At most 256 colors per frame; a pixel is \
                 fully see-through or not at all. Holds become frame delays."
            }
            Self::PngFrames => {
                "One PNG per frame, numbered after the name you choose (walk_01.png, \
                 walk_02.png…), for game engines and other apps. Holds are not repeated."
            }
            Self::SpriteSheet => {
                "Every frame in one PNG, in rows of the chosen number of columns, for game \
                 engines. Holds are not repeated."
            }
            Self::Mp4 => {
                "A video for sharing. Holds repeat \
                 frames; see-through areas become white."
            }
        }
    }
}

/// How an animation is exported.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct Settings {
    pub(crate) format: Format,
    /// Each pixel as a square this many pixels wide (1 to 16).
    pub(crate) scale: u8,
    /// Keep see-through areas (GIF and PNG only).
    pub(crate) transparent: bool,
    /// Sprite sheet columns; 0 puts every frame in one row.
    pub(crate) columns: u16,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            format: Format::Gif,
            scale: 1,
            transparent: true,
            columns: 0,
        }
    }
}

/// The Export animation window.
#[derive(Default)]
pub(crate) struct Window {
    pub(crate) open: bool,
    pub(crate) settings: Settings,
}

/// The frames' pictures, straight RGBA, scaled up by `scale`.
fn pictures(
    doc: &Document,
    animation: &Animation,
    frames: &[Frame],
    settings: Settings,
    cancel: &AtomicBool,
) -> Result<(Vec<Vec<u8>>, [usize; 2]), String> {
    let size = crate::export_jobs::scaled_size(doc.width, doc.height, settings.scale)?;
    let transparent = settings.transparent && settings.format != Format::Mp4;
    let check = || crate::fill::check_cancel(Some(cancel));
    let mut out = Vec::with_capacity(frames.len());
    for index in 0..frames.len() {
        check()?;
        let view = animation::frame_view(doc, animation, frames, Some(index), false);
        let rgba =
            view.composite_bounds_checked([0, 0, doc.width, doc.height], transparent, check)?;
        out.push(crate::export_jobs::upscaled(
            rgba,
            doc.width,
            doc.height,
            settings.scale,
        ));
    }
    Ok((out, size))
}

/// The frames in playing order for one pass: Ping-pong comes back without
/// repeating its ends.
fn sequence(frames: &[Frame], animation: &Animation) -> Vec<usize> {
    let mut order: Vec<usize> = (0..frames.len()).collect();
    if animation.playback == Playback::PingPong && frames.len() > 2 {
        order.extend((1..frames.len() - 1).rev());
    }
    order
}

/// Hundredths of a second each entry of `order` shows for, rounded so the
/// total keeps time.
fn delays(frames: &[Frame], order: &[usize], fps: u8) -> Vec<u16> {
    let mut elapsed = 0u64;
    let mut shown = 0u64;
    order
        .iter()
        .map(|&index| {
            elapsed += u64::from(frames[index].hold);
            let until = elapsed * 100 / u64::from(fps.max(1));
            let delay = (until - shown).max(2);
            shown += delay;
            delay.min(u64::from(u16::MAX)) as u16
        })
        .collect()
}

/// Maps a straight RGBA pixel to its palette index.
type PaletteIndex = Box<dyn Fn(&[u8]) -> u8>;

/// One GIF frame: an exact palette when it has at most 255 colors, else a
/// NeuQuant one; index 255 is see-through.
fn gif_frame(rgba: &[u8], [width, height]: [usize; 2], delay: u16) -> gif::Frame<'static> {
    let mut exact = std::collections::HashMap::new();
    for pixel in rgba.chunks_exact(4).filter(|p| p[3] >= 128) {
        let next = exact.len();
        exact.entry([pixel[0], pixel[1], pixel[2]]).or_insert(next);
        if exact.len() > 255 {
            break;
        }
    }
    let opaque: Vec<u8> = rgba
        .chunks_exact(4)
        .filter(|p| p[3] >= 128)
        .flatten()
        .copied()
        .collect();
    let (palette, index): (Vec<u8>, PaletteIndex) = if exact.len() <= 255 {
        let mut palette = vec![0; 256 * 3];
        for (color, i) in &exact {
            palette[i * 3..i * 3 + 3].copy_from_slice(color);
        }
        (
            palette,
            Box::new(move |p: &[u8]| exact[&[p[0], p[1], p[2]]] as u8),
        )
    } else {
        let quant = color_quant::NeuQuant::new(10, 255, &opaque);
        let mut palette = quant.color_map_rgb();
        palette.resize(256 * 3, 0);
        (palette, Box::new(move |p: &[u8]| quant.index_of(p) as u8))
    };
    let buffer: Vec<u8> = rgba
        .chunks_exact(4)
        .map(|p| if p[3] >= 128 { index(p) } else { 255 })
        .collect();
    gif::Frame {
        width: width as u16,
        height: height as u16,
        buffer: buffer.into(),
        palette: Some(palette),
        transparent: Some(255),
        dispose: gif::DisposalMethod::Background,
        delay,
        ..Default::default()
    }
}

fn write_gif(
    path: &Path,
    pictures: &[Vec<u8>],
    size: [usize; 2],
    frames: &[Frame],
    animation: &Animation,
    cancel: &AtomicBool,
) -> Result<(), String> {
    if size[0] > usize::from(u16::MAX) || size[1] > usize::from(u16::MAX) {
        return Err("GIF pictures are at most 65,535 pixels wide".into());
    }
    crate::timelapse::gif_destination(path)?;
    let order = sequence(frames, animation);
    let delays = delays(frames, &order, animation.fps);
    crate::storage::atomic_stream(path, |file| {
        let mut encoder = gif::Encoder::new(file, size[0] as u16, size[1] as u16, &[])
            .map_err(|e| e.to_string())?;
        encoder
            .set_repeat(if animation.playback == Playback::Once {
                gif::Repeat::Finite(0)
            } else {
                gif::Repeat::Infinite
            })
            .map_err(|e| e.to_string())?;
        for (&index, &delay) in order.iter().zip(&delays) {
            crate::fill::check_cancel(Some(cancel))?;
            encoder
                .write_frame(&gif_frame(&pictures[index], size, delay))
                .map_err(|e| e.to_string())?;
        }
        encoder.into_inner().map_err(|e| e.to_string())?;
        Ok(())
    })?;
    crate::timelapse::gif_destination(path)
}

fn png(path: &Path, rgba: &[u8], [width, height]: [usize; 2]) -> Result<(), String> {
    crate::export_jobs::destination(path)?;
    crate::storage::atomic_stream(path, |file| {
        let mut encoder = png::Encoder::new(file, width as u32, height as u32);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
        writer.write_image_data(rgba).map_err(|e| e.to_string())?;
        writer.finish().map_err(|e| e.to_string())
    })
}

/// `walk.png` becomes `walk_01.png`, `walk_02.png`… with enough digits for
/// every frame.
pub(crate) fn numbered(path: &Path, index: usize, count: usize) -> PathBuf {
    let stem = path
        .file_stem()
        .map_or("Animation".into(), |s| s.to_string_lossy().into_owned());
    let digits = count.to_string().len().max(2);
    path.with_file_name(format!("{stem}_{:0digits$}.png", index + 1))
}

/// Every frame in one picture: `columns` per row (0 for one row).
pub(crate) fn sheet(
    pictures: &[Vec<u8>],
    [width, height]: [usize; 2],
    columns: u16,
) -> (Vec<u8>, [usize; 2]) {
    let count = pictures.len().max(1);
    let columns = if columns == 0 {
        count
    } else {
        usize::from(columns).min(count)
    };
    let rows = count.div_ceil(columns);
    let size = [width * columns, height * rows];
    let mut out = vec![0; size[0] * size[1] * 4];
    for (index, picture) in pictures.iter().enumerate() {
        let (column, row) = (index % columns, index / columns);
        for y in 0..height {
            let from = &picture[y * width * 4..(y + 1) * width * 4];
            let at = ((row * height + y) * size[0] + column * width) * 4;
            out[at..at + width * 4].copy_from_slice(from);
        }
    }
    (out, size)
}

fn write_mp4(
    path: &Path,
    pictures: &[Vec<u8>],
    size: [usize; 2],
    frames: &[Frame],
    animation: &Animation,
    cancel: &AtomicBool,
) -> Result<(), String> {
    crate::video::write(path, size, animation.fps.into(), cancel, |sink| {
        for index in sequence(frames, animation) {
            for _ in 0..frames[index].hold {
                sink(&pictures[index])?;
            }
        }
        Ok(())
    })
}

/// Write the animation in `doc` to `path` as `settings` ask.
pub(crate) fn write(
    doc: &Document,
    path: &Path,
    settings: Settings,
    cancel: &AtomicBool,
) -> Result<(), String> {
    doc.validate()?;
    let animation = doc.animation.ok_or("The project is not an animation")?;
    let frames = animation::frames(doc, &animation);
    if frames.is_empty() {
        return Err("The animation has no frames".into());
    }
    let (pictures, size) = pictures(doc, &animation, &frames, settings, cancel)?;
    match settings.format {
        Format::Gif => write_gif(path, &pictures, size, &frames, &animation, cancel),
        Format::Mp4 => write_mp4(path, &pictures, size, &frames, &animation, cancel),
        Format::SpriteSheet => {
            let (rgba, size) = sheet(&pictures, size, settings.columns);
            crate::export_jobs::scaled_size(size[0], size[1], 1)?;
            png(path, &rgba, size)
        }
        Format::PngFrames => {
            // Check every destination before writing any.
            for index in 0..pictures.len() {
                crate::export_jobs::destination(&numbered(path, index, pictures.len()))?;
            }
            for (index, rgba) in pictures.iter().enumerate() {
                crate::fill::check_cancel(Some(cancel))?;
                png(&numbered(path, index, pictures.len()), rgba, size)?;
            }
            Ok(())
        }
    }
}

impl Dudel {
    /// The Export animation window, while it is open.
    pub(crate) fn animation_export_window(&mut self, ctx: &egui::Context) {
        if !self.animation_export.open {
            return;
        }
        let Some((_, frames)) = self.animation_frames() else {
            self.animation_export.open = false;
            return;
        };
        let mut open = true;
        let mut export = false;
        let mut cancel = false;
        let mut settings = self.animation_export.settings;
        egui::Window::new("Export animation")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .pivot(egui::Align2::CENTER_CENTER)
            .default_pos(ctx.content_rect().center())
            .show(ctx, |ui| {
                egui::ComboBox::from_label("Format")
                    .selected_text(settings.format.name())
                    .show_ui(ui, |ui| {
                        for format in Format::ALL {
                            ui.selectable_value(&mut settings.format, format, format.name());
                        }
                    });
                ui.label(settings.format.hint());
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.label("Scale");
                    ui.add(
                        egui::DragValue::new(&mut settings.scale)
                            .range(1..=16)
                            .suffix("×"),
                    )
                    .on_hover_text("Each pixel as a sharp square this many pixels wide");
                });
                if settings.format != Format::Mp4 {
                    ui.checkbox(&mut settings.transparent, "Keep transparency");
                }
                if settings.format == Format::SpriteSheet {
                    ui.horizontal(|ui| {
                        ui.label("Columns");
                        ui.add(egui::DragValue::new(&mut settings.columns).range(0..=256))
                            .on_hover_text("Frames per row; 0 puts every frame in one row");
                    });
                }
                let scale = usize::from(settings.scale.max(1));
                ui.weak(format!(
                    "{} frames, {} × {} px each",
                    frames.len(),
                    self.doc.width * scale,
                    self.doc.height * scale
                ));
                let too_large = crate::export_jobs::scaled_size(
                    self.doc.width,
                    self.doc.height,
                    settings.scale,
                )
                .err();
                if let Some(error) = &too_large {
                    ui.colored_label(ui.visuals().error_fg_color, error);
                }
                crate::dialogs::dialog_buttons(ui, |ui| {
                    export = ui
                        .add_enabled(
                            too_large.is_none() && !frames.is_empty(),
                            crate::workspace::primary_button(ui, "Export…"),
                        )
                        .clicked();
                    cancel = ui.button("Cancel").clicked();
                });
            });
        settings.scale = settings.scale.clamp(1, 16);
        self.animation_export.settings = settings;
        if export {
            self.animation_export.open = false;
            let name = self
                .path
                .as_deref()
                .and_then(|p| p.file_stem())
                .map_or("Animation".into(), |s| s.to_string_lossy().into_owned());
            let format = settings.format;
            let dialog = self.dialog_in(
                crate::locations::Kind::Exports,
                rfd::AsyncFileDialog::new()
                    .add_filter(format.name(), &[format.extension()])
                    .set_file_name(format!("{name}.{}", format.extension())),
            );
            self.ask_save_file(dialog, move |app, path| {
                app.export_snapshot_or_queue(
                    path,
                    crate::export_jobs::ExportKind::Animation(settings),
                );
            });
        } else if cancel || !open {
            self.animation_export.open = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Layer;

    fn walk() -> Document {
        let mut doc = Document::new(3, 2);
        doc.layers = (0..3)
            .map(|i| {
                let mut layer = Layer::raster(format!("F{i}"), 3, 2);
                layer
                    .pixels
                    .writer()
                    .rgba_mut(i)
                    .copy_from_slice(&[200, 10 * i as u8, 40, 255]);
                layer
            })
            .collect();
        doc.layers[1].hold = 2;
        doc.animation = Some(Animation {
            fps: 10,
            ..Default::default()
        });
        doc
    }

    #[test]
    fn gif_frames_follow_holds_and_keep_transparency_and_pixels() {
        let dir = tempfile::tempdir().unwrap();
        let doc = walk();
        let path = dir.path().join("walk.gif");
        let settings = Settings {
            scale: 2,
            ..Default::default()
        };
        write(&doc, &path, settings, &AtomicBool::new(false)).unwrap();
        let decoder = image::codecs::gif::GifDecoder::new(std::io::BufReader::new(
            std::fs::File::open(&path).unwrap(),
        ))
        .unwrap();
        use image::AnimationDecoder;
        let frames = decoder.into_frames().collect_frames().unwrap();
        assert_eq!(frames.len(), 3);
        let delays: Vec<u32> = frames
            .iter()
            .map(|f| {
                let (n, d) = f.delay().numer_denom_ms();
                n / d
            })
            .collect();
        assert_eq!(delays, [100, 200, 100]);
        let first = frames[0].buffer();
        assert_eq!(first.dimensions(), (6, 4));
        assert_eq!(first.get_pixel(0, 0).0, [200, 0, 40, 255]);
        assert_eq!(first.get_pixel(1, 1).0, [200, 0, 40, 255], "scaled up 2×");
        assert_eq!(
            first.get_pixel(2, 0).0[3],
            0,
            "see-through stays see-through"
        );
        assert_eq!(frames[1].buffer().get_pixel(2, 0).0, [200, 10, 40, 255]);
        assert_eq!(
            frames[1].buffer().get_pixel(0, 0).0[3],
            0,
            "the previous frame is gone"
        );
    }

    #[test]
    fn png_frames_and_sprite_sheets_hold_every_frame() {
        let dir = tempfile::tempdir().unwrap();
        let doc = walk();
        let path = dir.path().join("walk.png");
        let cancel = AtomicBool::new(false);
        let frames = Settings {
            format: Format::PngFrames,
            ..Default::default()
        };
        write(&doc, &path, frames, &cancel).unwrap();
        for index in 0..3 {
            let file = dir.path().join(format!("walk_{:02}.png", index + 1));
            let image = image::open(&file).unwrap().to_rgba8();
            assert_eq!(
                image.get_pixel(index as u32, 0).0,
                [200, 10 * index as u8, 40, 255]
            );
        }
        let sheet_path = dir.path().join("sheet.png");
        let settings = Settings {
            format: Format::SpriteSheet,
            columns: 2,
            ..Default::default()
        };
        write(&doc, &sheet_path, settings, &cancel).unwrap();
        let sheet = image::open(&sheet_path).unwrap().to_rgba8();
        assert_eq!(sheet.dimensions(), (6, 4), "two columns, two rows");
        assert_eq!(sheet.get_pixel(0, 0).0, [200, 0, 40, 255]);
        assert_eq!(sheet.get_pixel(3 + 1, 0).0, [200, 10, 40, 255]);
        assert_eq!(sheet.get_pixel(2, 2).0, [200, 20, 40, 255]);
        assert_eq!(sheet.get_pixel(3, 2).0[3], 0, "the empty cell");
    }

    #[test]
    fn ping_pong_returns_without_repeating_its_ends_and_delays_keep_time() {
        let doc = walk();
        let animation = Animation {
            playback: Playback::PingPong,
            fps: 12,
            ..Default::default()
        };
        let frames = animation::frames(&doc, &animation);
        let order = sequence(&frames, &animation);
        assert_eq!(order, [0, 1, 2, 1]);
        let delays = delays(&frames, &order, 12);
        // Holds 1, 2, 1, 2 at 12 fps: 6 periods = 50 hundredths... per
        // second 100, so the total is exactly 6 / 12 s.
        assert_eq!(delays.iter().map(|&d| u32::from(d)).sum::<u32>(), 50);
        assert_eq!(
            numbered(Path::new("a/walk.png"), 9, 120),
            Path::new("a/walk_010.png")
        );
    }

    #[test]
    #[ignore = "requires ffprobe on PATH"]
    fn mp4_export_repeats_frames_for_their_holds() {
        let dir = tempfile::tempdir().unwrap();
        let mut doc = walk();
        // Video needs some size; scale the 3 × 2 walk up.
        doc.animation = Some(Animation {
            fps: 10,
            ..Default::default()
        });
        let path = dir.path().join("walk.mp4");
        let settings = Settings {
            format: Format::Mp4,
            scale: 16,
            ..Default::default()
        };
        write(&doc, &path, settings, &AtomicBool::new(false)).unwrap();
        let probe = std::process::Command::new("ffprobe")
            .args([
                "-v",
                "error",
                "-count_frames",
                "-select_streams",
                "v:0",
                "-show_entries",
                "stream=nb_read_frames,width,height",
                "-of",
                "csv=p=0",
            ])
            .arg(&path)
            .output()
            .unwrap();
        let text = String::from_utf8_lossy(&probe.stdout);
        // 48 × 32 pixels, which Windows' encoder needs doubled; holds
        // 1 + 2 + 1 = 4 video frames.
        let expected = if cfg!(windows) { "96,64,4" } else { "48,32,4" };
        assert_eq!(text.trim(), expected);
    }
}
