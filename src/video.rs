//! MP4 (H.264) encoding for timelapses and animations. Windows' own encoder
//! does it there; FFmpeg takes over where that is missing or refuses the
//! video, and on other systems.
use std::path::Path;
use std::sync::atomic::AtomicBool;

/// `DUDEL_VIDEO_ENCODER=ffmpeg` skips Windows' encoder, for comparing.
#[cfg(windows)]
fn ffmpeg_asked_for() -> bool {
    std::env::var_os("DUDEL_VIDEO_ENCODER").is_some_and(|name| name == "ffmpeg")
}

/// Takes one opaque RGBA frame of the video.
pub(crate) type Sink<'a> = dyn FnMut(&[u8]) -> Result<(), String> + 'a;

/// Write a video of the frames `feed` hands to its sink, all `size` pixels,
/// to `path`. Nothing at `path` changes unless the whole video is written.
pub(crate) fn write(
    path: &Path,
    size: [usize; 2],
    fps: u32,
    cancel: &AtomicBool,
    feed: impl FnOnce(&mut Sink) -> Result<(), String>,
) -> Result<(), String> {
    crate::timelapse::mp4_destination(path)?;
    crate::storage::atomic_stream(path, |file| {
        encode(file, size, fps, cancel, feed)?;
        crate::fill::check_cancel(Some(cancel))?;
        crate::timelapse::mp4_destination(path)
    })
}

#[cfg(windows)]
fn encode(
    file: &mut std::fs::File,
    size: [usize; 2],
    fps: u32,
    cancel: &AtomicBool,
    feed: impl FnOnce(&mut Sink) -> Result<(), String>,
) -> Result<(), String> {
    let refused = if ffmpeg_asked_for() {
        "not used".to_owned()
    } else {
        match media_foundation::Encoder::start(size, fps) {
            Ok(mut encoder) => {
                feed(&mut |frame| {
                    crate::fill::check_cancel(Some(cancel))?;
                    encoder.frame(frame)
                })?;
                crate::fill::check_cancel(Some(cancel))?;
                return encoder.finish(file);
            }
            Err(error) => error,
        }
    };
    // Nothing has been fed yet, so FFmpeg can start from the beginning.
    ffmpeg(file, size, fps, cancel, feed).map_err(|error| match error {
        Failure::Missing => format!(
            "Windows could not encode this video ({refused}). Try another size, install \
             Microsoft's Media Feature Pack if this is an N edition of Windows, or install FFmpeg."
        ),
        Failure::Other(error) => error,
    })
}

#[cfg(not(windows))]
fn encode(
    file: &mut std::fs::File,
    size: [usize; 2],
    fps: u32,
    cancel: &AtomicBool,
    feed: impl FnOnce(&mut Sink) -> Result<(), String>,
) -> Result<(), String> {
    ffmpeg(file, size, fps, cancel, feed).map_err(|error| match error {
        Failure::Missing => {
            "Could not start FFmpeg. Install FFmpeg with libx264 and add it to PATH.".to_owned()
        }
        Failure::Other(error) => error,
    })
}

enum Failure {
    /// FFmpeg is not installed.
    Missing,
    Other(String),
}

fn ffmpeg(
    file: &std::fs::File,
    size: [usize; 2],
    fps: u32,
    cancel: &AtomicBool,
    feed: impl FnOnce(&mut Sink) -> Result<(), String>,
) -> Result<(), Failure> {
    use std::io::{Read, Seek, Write};
    use std::process::{Command, Stdio};
    let other = |e: std::io::Error| Failure::Other(e.to_string());
    let mut errors = tempfile::tempfile().map_err(other)?;
    let mut command = Command::new("ffmpeg");
    command
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-nostdin",
            "-f",
            "rawvideo",
            "-pixel_format",
            "rgba",
            "-video_size",
            &format!("{}x{}", size[0], size[1]),
            "-framerate",
            &fps.to_string(),
            "-i",
            "pipe:0",
            "-an",
            "-vf",
            "pad=ceil(iw/2)*2:ceil(ih/2)*2:0:0:color=white",
            "-c:v",
            "libx264",
            "-preset",
            "medium",
            "-crf",
            "18",
            "-pix_fmt",
            "yuv420p",
            "-movflags",
            "+frag_keyframe+empty_moov",
            "-f",
            "mp4",
            "pipe:1",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::from(file.try_clone().map_err(other)?))
        .stderr(Stdio::from(errors.try_clone().map_err(other)?));
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let mut child = command.spawn().map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => Failure::Missing,
        _ => Failure::Other(format!("Could not start FFmpeg: {e}")),
    })?;
    let mut input = child
        .stdin
        .take()
        .ok_or_else(|| Failure::Other("FFmpeg input unavailable".into()))?;
    let written = feed(&mut |frame| {
        crate::fill::check_cancel(Some(cancel))?;
        input.write_all(frame).map_err(|e| e.to_string())
    });
    drop(input);
    if written.is_err() {
        let _ = child.kill();
    }
    let status = loop {
        if crate::fill::check_cancel(Some(cancel)).is_err() {
            let _ = child.kill();
        }
        if let Some(status) = child.try_wait().map_err(other)? {
            break status;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    };
    crate::fill::check_cancel(Some(cancel)).map_err(Failure::Other)?;
    errors.rewind().map_err(other)?;
    let mut message = String::new();
    errors
        .take(8192)
        .read_to_string(&mut message)
        .map_err(other)?;
    // A frame refused because FFmpeg gave up is explained by its message.
    if !message.trim().is_empty() && !status.success() {
        return Err(Failure::Other(format!("FFmpeg failed: {message}")));
    }
    written.map_err(Failure::Other)?;
    if !status.success() {
        return Err(Failure::Other("FFmpeg failed".into()));
    }
    Ok(())
}

/// Convert an RGBA frame, each pixel enlarged to `steps` squared, to NV12 in
/// video-range BT.709. `width` and `height` are the even output size; what
/// the frame does not cover is white.
#[cfg(any(windows, test))]
fn nv12(
    rgba: &[u8],
    size: [usize; 2],
    steps: usize,
    [width, height]: [usize; 2],
    out: &mut Vec<u8>,
) {
    out.resize(width * height * 3 / 2, 0);
    let (luma, chroma) = out.split_at_mut(width * height);
    let pixel = |x: usize, y: usize| -> [i32; 3] {
        let (x, y) = (x / steps, y / steps);
        if x >= size[0] || y >= size[1] {
            return [255; 3];
        }
        let at = (y * size[0] + x) * 4;
        [0, 1, 2].map(|c| i32::from(rgba[at + c]))
    };
    // Coefficients in 1/65536; rows of the chroma ones sum to zero, so gray
    // stays exactly neutral.
    for (y, row) in luma.chunks_exact_mut(width).enumerate() {
        for (x, value) in row.iter_mut().enumerate() {
            let [r, g, b] = pixel(x, y);
            *value = (16 + ((11966 * r + 40254 * g + 4064 * b + 32768) >> 16)) as u8;
        }
    }
    for (y, row) in chroma.chunks_exact_mut(width).enumerate() {
        for (x, pair) in row.chunks_exact_mut(2).enumerate() {
            let mut sum = [0i32; 3];
            for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                let p = pixel(x * 2 + dx, y * 2 + dy);
                for c in 0..3 {
                    sum[c] += p[c];
                }
            }
            let [r, g, b] = sum;
            let round = (128 << 18) + (1 << 17);
            pair[0] = ((-6596 * r - 22188 * g + 28784 * b + round) >> 18) as u8;
            pair[1] = ((28784 * r - 26145 * g - 2639 * b + round) >> 18) as u8;
        }
    }
}

/// Windows' H.264 encoder. Its libraries are loaded on use, not linked: they
/// are missing from N editions of Windows, where Dudel must still start.
#[cfg(windows)]
mod media_foundation {
    use std::ffi::c_void;
    use windows::{
        Win32::{
            Media::MediaFoundation::*,
            System::{
                Com::{COINIT_MULTITHREADED, CoInitializeEx, CoUninitialize},
                LibraryLoader::{GetProcAddress, LOAD_LIBRARY_SEARCH_SYSTEM32, LoadLibraryExW},
            },
        },
        core::{GUID, HRESULT, HSTRING, Interface, PCSTR, PCWSTR, s, w},
    };

    type Create = unsafe extern "system" fn(*mut *mut c_void) -> HRESULT;

    #[derive(Clone, Copy)]
    struct Api {
        startup: unsafe extern "system" fn(u32, u32) -> HRESULT,
        shutdown: unsafe extern "system" fn() -> HRESULT,
        media_type: Create,
        sample: Create,
        buffer: unsafe extern "system" fn(u32, *mut *mut c_void) -> HRESULT,
        attributes: unsafe extern "system" fn(*mut *mut c_void, u32) -> HRESULT,
        writer: unsafe extern "system" fn(
            PCWSTR,
            *mut c_void,
            *mut c_void,
            *mut *mut c_void,
        ) -> HRESULT,
    }

    /// The functions, or None where Windows has no Media Foundation. The
    /// libraries stay loaded.
    fn api() -> Option<Api> {
        static API: std::sync::OnceLock<Option<Api>> = std::sync::OnceLock::new();
        // SAFETY: each function pointer is given the signature documented for
        // the export it was looked up by.
        *API.get_or_init(|| unsafe {
            let load = |name: PCWSTR| LoadLibraryExW(name, None, LOAD_LIBRARY_SEARCH_SYSTEM32).ok();
            let (platform, writer) = (load(w!("mfplat.dll"))?, load(w!("mfreadwrite.dll"))?);
            let find = |name: PCSTR| GetProcAddress(platform, name);
            Some(Api {
                startup: std::mem::transmute::<
                    unsafe extern "system" fn() -> isize,
                    unsafe extern "system" fn(u32, u32) -> HRESULT,
                >(find(s!("MFStartup"))?),
                shutdown: std::mem::transmute::<
                    unsafe extern "system" fn() -> isize,
                    unsafe extern "system" fn() -> HRESULT,
                >(find(s!("MFShutdown"))?),
                media_type: std::mem::transmute::<unsafe extern "system" fn() -> isize, Create>(
                    find(s!("MFCreateMediaType"))?,
                ),
                sample: std::mem::transmute::<unsafe extern "system" fn() -> isize, Create>(find(
                    s!("MFCreateSample"),
                )?),
                buffer: std::mem::transmute::<
                    unsafe extern "system" fn() -> isize,
                    unsafe extern "system" fn(u32, *mut *mut c_void) -> HRESULT,
                >(find(s!("MFCreateMemoryBuffer"))?),
                attributes: std::mem::transmute::<
                    unsafe extern "system" fn() -> isize,
                    unsafe extern "system" fn(*mut *mut c_void, u32) -> HRESULT,
                >(find(s!("MFCreateAttributes"))?),
                writer: std::mem::transmute::<
                    unsafe extern "system" fn() -> isize,
                    unsafe extern "system" fn(
                        PCWSTR,
                        *mut c_void,
                        *mut c_void,
                        *mut *mut c_void,
                    ) -> HRESULT,
                >(GetProcAddress(writer, s!("MFCreateSinkWriterFromURL"))?),
            })
        })
    }

    /// Run a function that returns an object through its last argument.
    unsafe fn object<T: Interface>(
        call: impl FnOnce(*mut *mut c_void) -> HRESULT,
    ) -> windows::core::Result<T> {
        let mut raw = std::ptr::null_mut();
        call(&mut raw).ok()?;
        // SAFETY: a successful call stored one reference to a T.
        unsafe { windows::core::Type::from_abi(raw) }
    }

    /// Media Foundation, started on this thread.
    struct Started {
        api: Api,
        com: bool,
    }
    impl Started {
        fn new() -> Result<Self, String> {
            let api = api().ok_or("Media Foundation is not installed")?;
            // SAFETY: balanced in drop; a thread already in another
            // apartment is used as it is.
            unsafe {
                let com = CoInitializeEx(None, COINIT_MULTITHREADED).is_ok();
                if let Err(error) = (api.startup)(MF_VERSION, 0).ok() {
                    if com {
                        CoUninitialize();
                    }
                    return Err(error.message());
                }
                Ok(Self { api, com })
            }
        }
    }
    impl Drop for Started {
        fn drop(&mut self) {
            // SAFETY: balances `new`, on the same thread: `Encoder` holds COM
            // pointers, which keep it from being sent to another one.
            unsafe {
                let _ = (self.api.shutdown)();
                if self.com {
                    CoUninitialize();
                }
            }
        }
    }

    /// The smallest side the encoder is given; smaller videos are enlarged.
    const SMALLEST: usize = 64;

    /// Fields are dropped in order: the writer, then the file it has open,
    /// then Media Foundation.
    pub(super) struct Encoder {
        writer: IMFSinkWriter,
        stream: u32,
        path: std::path::PathBuf,
        _folder: tempfile::TempDir,
        started: Started,
        size: [usize; 2],
        steps: usize,
        coded: [usize; 2],
        fps: u32,
        frames: u64,
        picture: Vec<u8>,
    }

    impl Encoder {
        pub(super) fn start(size: [usize; 2], fps: u32) -> Result<Self, String> {
            if size.contains(&0) || fps == 0 {
                return Err("empty video".into());
            }
            // The encoder takes no tiny videos. Whole pixels, an even number
            // of steps so half-size color samples stay inside each square.
            let shortest = size[0].min(size[1]);
            let steps = if shortest >= SMALLEST {
                1
            } else {
                SMALLEST.div_ceil(shortest).next_multiple_of(2)
            };
            let coded = size.map(|side| (side * steps).next_multiple_of(2));
            // H.264 level 5.2, the most this encoder takes.
            if coded[0].max(coded[1]) > 4096 || coded[0] * coded[1] > 4096 * 2304 {
                return Err("too large for its encoder".into());
            }
            let folder = tempfile::tempdir().map_err(|e| e.to_string())?;
            let path = folder.path().join("video.mp4");
            let started = Started::new()?;
            let (writer, stream) = Self::open(&started.api, &path, coded, fps)
                .map_err(|e: windows::core::Error| e.message())?;
            Ok(Self {
                writer,
                stream,
                path,
                _folder: folder,
                started,
                size,
                steps,
                coded,
                fps,
                frames: 0,
                picture: Vec::new(),
            })
        }

        fn open(
            api: &Api,
            path: &std::path::Path,
            [width, height]: [usize; 2],
            fps: u32,
        ) -> windows::core::Result<(IMFSinkWriter, u32)> {
            // SAFETY: plain calls with live arguments.
            unsafe {
                let media_type = |subtype: &GUID| -> windows::core::Result<IMFMediaType> {
                    let kind: IMFMediaType = object(|out| (api.media_type)(out))?;
                    kind.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
                    kind.SetGUID(&MF_MT_SUBTYPE, subtype)?;
                    kind.SetUINT64(&MF_MT_FRAME_SIZE, (width as u64) << 32 | height as u64)?;
                    kind.SetUINT64(&MF_MT_FRAME_RATE, u64::from(fps) << 32 | 1)?;
                    kind.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, 1 << 32 | 1)?;
                    let progressive = MFVideoInterlace_Progressive.0 as u32;
                    kind.SetUINT32(&MF_MT_INTERLACE_MODE, progressive)?;
                    kind.SetUINT32(&MF_MT_VIDEO_PRIMARIES, MFVideoPrimaries_BT709.0 as u32)?;
                    kind.SetUINT32(&MF_MT_TRANSFER_FUNCTION, MFVideoTransFunc_709.0 as u32)?;
                    kind.SetUINT32(&MF_MT_YUV_MATRIX, MFVideoTransferMatrix_BT709.0 as u32)?;
                    kind.SetUINT32(&MF_MT_VIDEO_NOMINAL_RANGE, MFNominalRange_16_235.0 as u32)?;
                    Ok(kind)
                };
                let output = media_type(&MFVideoFormat_H264)?;
                output.SetUINT32(&MF_MT_MPEG2_PROFILE, eAVEncH264VProfile_High.0 as u32)?;
                // Required, and only a hint: the quality below decides.
                let bits = (width * height * fps as usize / 5).clamp(1_000_000, 50_000_000);
                output.SetUINT32(&MF_MT_AVG_BITRATE, bits as u32)?;
                let input = media_type(&MFVideoFormat_NV12)?;
                input.SetUINT32(&MF_MT_DEFAULT_STRIDE, width as u32)?;
                let encoding: IMFAttributes = object(|out| (api.attributes)(out, 3))?;
                encoding.SetUINT32(
                    &CODECAPI_AVEncCommonRateControlMode,
                    eAVEncCommonRateControlMode_Quality.0 as u32,
                )?;
                encoding.SetUINT32(&CODECAPI_AVEncCommonQuality, quality())?;
                // Reordered frames would start the video one frame late.
                encoding.SetUINT32(&CODECAPI_AVEncMPVDefaultBPictureCount, 0)?;
                let url = HSTRING::from(path.as_os_str());
                let none = std::ptr::null_mut();
                let writer: IMFSinkWriter =
                    object(|out| (api.writer)(PCWSTR(url.as_ptr()), none, none, out))?;
                let stream = writer.AddStream(&output)?;
                writer.SetInputMediaType(stream, &input, &encoding)?;
                writer.BeginWriting()?;
                Ok((writer, stream))
            }
        }

        pub(super) fn frame(&mut self, rgba: &[u8]) -> Result<(), String> {
            if rgba.len() != self.size[0] * self.size[1] * 4 {
                return Err("A video frame has the wrong size".into());
            }
            super::nv12(rgba, self.size, self.steps, self.coded, &mut self.picture);
            self.submit()
                .map_err(|e| format!("Windows could not encode the video: {}", e.message()))
        }

        fn submit(&mut self) -> windows::core::Result<()> {
            let api = &self.started.api;
            // Rounded from the frame number, so times never drift.
            let time = |frame: u64| (frame * 10_000_000 / u64::from(self.fps)) as i64;
            let length = self.picture.len() as u32;
            // SAFETY: the buffer was made `length` bytes long and is locked
            // while the picture is copied into it.
            unsafe {
                let buffer: IMFMediaBuffer = object(|out| (api.buffer)(length, out))?;
                let mut data = std::ptr::null_mut();
                buffer.Lock(&mut data, None, None)?;
                std::ptr::copy_nonoverlapping(self.picture.as_ptr(), data, self.picture.len());
                buffer.Unlock()?;
                buffer.SetCurrentLength(length)?;
                let sample: IMFSample = object(|out| (api.sample)(out))?;
                sample.AddBuffer(&buffer)?;
                sample.SetSampleTime(time(self.frames))?;
                sample.SetSampleDuration(time(self.frames + 1) - time(self.frames))?;
                self.writer.WriteSample(self.stream, &sample)?;
            }
            self.frames += 1;
            Ok(())
        }

        pub(super) fn finish(self, file: &mut std::fs::File) -> Result<(), String> {
            // SAFETY: called once, after the last frame.
            unsafe { self.writer.Finalize() }
                .map_err(|e| format!("Windows could not finish the video: {}", e.message()))?;
            let mut video = std::fs::File::open(&self.path).map_err(|e| e.to_string())?;
            std::io::copy(&mut video, file).map_err(|e| e.to_string())?;
            Ok(())
        }
    }

    /// 0 to 100. Within 1.7 dB of x264 at CRF 18 on real recordings, with
    /// files 1.5 to 1.9 times as large (measured 2026-11-23).
    fn quality() -> u32 {
        #[cfg(test)]
        if let Ok(value) = std::env::var("DUDEL_VIDEO_QUALITY") {
            return value.parse().unwrap();
        }
        80
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colors_become_video_range_bt709_and_the_rest_is_white() {
        let mut out = Vec::new();
        // Black, white / red, gray, each a 2 × 2 square.
        let rgba = [
            [0, 0, 0, 255],
            [255, 255, 255, 255],
            [255, 0, 0, 255],
            [128, 128, 128, 255],
        ]
        .concat();
        nv12(&rgba, [2, 2], 2, [6, 4], &mut out);
        assert_eq!(out[..6], [16, 16, 235, 235, 235, 235]);
        assert_eq!(out[12..18], [63, 63, 126, 126, 235, 235]);
        assert_eq!(out[24..30], [128, 128, 128, 128, 128, 128]);
        assert_eq!(out[30..36], [102, 240, 128, 128, 128, 128]);
    }

    /// Every size an export can ask for either encodes or is refused before
    /// the first frame. Skipped where Windows has no encoder.
    #[cfg(windows)]
    #[test]
    fn windows_encodes_small_odd_and_large_videos() {
        use media_foundation::Encoder;
        if let Err(error) = Encoder::start([64, 64], 10) {
            eprintln!("skipped: {error}");
            return;
        }
        for size in [
            [1, 1],
            [16, 16],
            [48, 32],
            [65, 67],
            [71, 300],
            [1279, 719],
            [2304, 64],
            [4096, 2304],
        ] {
            let mut encoder = Encoder::start(size, 30).unwrap_or_else(|e| panic!("{size:?}: {e}"));
            let frame = vec![90; size[0] * size[1] * 4];
            for _ in 0..3 {
                encoder.frame(&frame).unwrap();
            }
            assert!(encoder.frame(&frame[4..]).is_err(), "a short frame");
            let mut file = tempfile::tempfile().unwrap();
            encoder
                .finish(&mut file)
                .unwrap_or_else(|e| panic!("{size:?}: {e}"));
            assert!(file.metadata().unwrap().len() > 500, "{size:?}");
        }
        for size in [[0, 5], [5000, 100], [4096, 4096], [1, 2000]] {
            assert!(Encoder::start(size, 30).is_err(), "{size:?}");
        }
    }

    #[test]
    fn a_canceled_or_failed_video_leaves_the_destination_alone() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("video.mp4");
        let cancel = AtomicBool::new(false);
        let result = write(&path, [64, 64], 10, &cancel, |sink| {
            sink(&[200; 64 * 64 * 4])?;
            Err("stop".into())
        });
        assert!(result.is_err() && !path.exists());
        let result = write(&path, [64, 64], 10, &cancel, |sink| {
            sink(&[200; 64 * 64 * 4])?;
            cancel.store(true, std::sync::atomic::Ordering::Relaxed);
            Ok(())
        });
        assert!(result.is_err() && !path.exists());
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    /// `DUDEL_VIDEO_FRAMES=folder DUDEL_VIDEO_OUT=x.mp4 cargo test --profile
    /// fast video_sample -- --ignored --nocapture`: encode a folder of equal
    /// PNGs, enlarged to about 1280, to compare encoders and qualities.
    #[test]
    #[ignore]
    fn video_sample() {
        let folder = std::env::var("DUDEL_VIDEO_FRAMES").unwrap();
        let out = std::env::var("DUDEL_VIDEO_OUT").unwrap();
        let mut paths: Vec<_> = std::fs::read_dir(folder)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.extension().is_some_and(|e| e == "png"))
            .collect();
        paths.sort();
        let (w, h) = image::image_dimensions(&paths[0]).unwrap();
        let steps = ((1280 / w.max(h)) / 2 * 2).max(1);
        let size = [(w * steps) as usize, (h * steps) as usize];
        let started = std::time::Instant::now();
        write(Path::new(&out), size, 30, &AtomicBool::new(false), |sink| {
            for path in &paths {
                let frame = image::open(path).unwrap().to_rgba8();
                let frame = image::imageops::resize(
                    &frame,
                    w * steps,
                    h * steps,
                    image::imageops::FilterType::Nearest,
                );
                sink(frame.as_raw())?;
            }
            Ok(())
        })
        .unwrap();
        println!(
            "{} frames {size:?}: {} bytes in {:?}",
            paths.len(),
            std::fs::metadata(&out).unwrap().len(),
            started.elapsed()
        );
    }
}
