//! Which Dudel this is. The packaging scripts set `DUDEL_PACKAGED` while
//! compiling; every other build is a development build, which keeps its
//! settings, recovery copies and extensions apart from the installed copy's.

/// Not built by the packaging scripts.
pub const DEVELOPMENT: bool = option_env!("DUDEL_PACKAGED").is_none();

/// The name eframe derives the settings folder from.
pub const SETTINGS: &str = if DEVELOPMENT {
    "Dudel \u{2014} Drawing Studio (Dev)"
} else {
    PACKAGED_SETTINGS
};
pub const PACKAGED_SETTINGS: &str = "Dudel \u{2014} Drawing Studio";

/// Recovery copies, extensions, problem reports and display settings.
pub fn project_dirs() -> Option<directories::ProjectDirs> {
    named_dirs(if DEVELOPMENT { "Dudel Dev" } else { "Dudel" })
}

/// The installed copy's folders, whichever build this is.
pub fn packaged_dirs() -> Option<directories::ProjectDirs> {
    named_dirs("Dudel")
}

fn named_dirs(application: &str) -> Option<directories::ProjectDirs> {
    directories::ProjectDirs::from("org", "Dudel", application)
}
