//! Recognizing image files before they are decoded. The app's `image_paste`
//! turns them into layers.
use std::path::Path;

/// Formats Dudel can decode into a layer.
pub const EXTENSIONS: [&str; 6] = ["png", "jpg", "jpeg", "webp", "bmp", "gif"];

pub fn is_image(path: &Path) -> bool {
    extension(path).is_some_and(|e| EXTENSIONS.contains(&e.as_str()))
}
pub fn extension(path: &Path) -> Option<String> {
    path.extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
}

/// Why a file named as an image isn't one, from its first bytes: a browser
/// may save the page an image was on under the image's name.
pub fn not_an_image(path: &Path) -> Option<String> {
    use std::io::Read;
    let mut header = [0u8; 256];
    let count = std::fs::File::open(path)
        .and_then(|mut file| file.read(&mut header))
        .ok()?;
    let header = &header[..count];
    if count == 0 {
        return Some(format!("{} is empty", file_name(path)));
    }
    if image::guess_format(header).is_ok() {
        return None;
    }
    let text = String::from_utf8_lossy(header)
        .trim_start()
        .to_ascii_lowercase();
    Some(
        if text.starts_with("<!doctype html") || text.starts_with("<html") {
            format!(
                "{} is a web page saved under an image's name, not an image. Open the \
             image on its own in the browser, then save or copy it from there",
                file_name(path)
            )
        } else {
            format!(
                "{} isn't a PNG, JPEG, WebP, BMP or GIF image, whatever its name says",
                file_name(path)
            )
        },
    )
}

pub fn file_name(path: &Path) -> String {
    path.file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned()
}
