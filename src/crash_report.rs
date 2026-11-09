//! Problem reports. Every panic writes a short report under the app data
//! folder; when one escapes the window's event loop, the artist is told what
//! happened, where the report is and that recovery snapshots survive.
//!
//! The hook never decides whether a panic is fatal: background jobs and
//! `parallel::run` catch their own panics, sometimes on the UI thread. Only
//! `main` knows, because only it sees a panic that ended the window.
use std::{
    fmt::Write as _,
    path::{Path, PathBuf},
    sync::Mutex,
    time::{SystemTime, UNIX_EPOCH},
};

/// Reports kept; older ones are removed when a new one is written.
const KEEP: usize = 20;
pub(crate) const ISSUES: &str = concat!(env!("CARGO_PKG_REPOSITORY"), "/issues/new/choose");

/// The newest report written in this run.
static LAST: Mutex<Option<PathBuf>> = Mutex::new(None);

/// Where problem reports go: `%LOCALAPPDATA%\Dudel\...\problem-reports`.
pub(crate) fn folder() -> Option<PathBuf> {
    crate::locations::app_data().map(|data| data.join("problem-reports"))
}

/// Write a report for every panic, then run the default hook (stderr).
pub(crate) fn install() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if let Some(folder) = folder()
            && let Some(path) = write_report(&folder, &describe(info))
            && let Ok(mut last) = LAST.lock()
        {
            *last = Some(path);
        }
        previous(info);
    }));
}

fn describe(info: &std::panic::PanicHookInfo<'_>) -> String {
    let message = info
        .payload()
        .downcast_ref::<&str>()
        .map(|s| (*s).to_owned())
        .or_else(|| info.payload().downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "(no message)".into());
    let location = info.location().map_or_else(
        || "unknown".into(),
        |l| format!("{}:{}", l.file(), l.line()),
    );
    let thread = std::thread::current();
    let mut text = String::new();
    let _ = writeln!(text, "Dudel problem report");
    let _ = writeln!(text, "Version: {}", env!("CARGO_PKG_VERSION"));
    let _ = writeln!(
        text,
        "Platform: {} {}",
        std::env::consts::OS,
        std::env::consts::ARCH
    );
    let _ = writeln!(text, "Thread: {}", thread.name().unwrap_or("unnamed"));
    let _ = writeln!(text, "Message: {message}");
    let _ = writeln!(text, "Location: {location}");
    let _ = writeln!(
        text,
        "\nBacktrace:\n{}",
        std::backtrace::Backtrace::force_capture()
    );
    text
}

/// Save `text` as a new report and trim old ones. Never panics.
fn write_report(folder: &Path, text: &str) -> Option<PathBuf> {
    std::fs::create_dir_all(folder).ok()?;
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis());
    let path = folder.join(format!("dudel-problem-{stamp}.txt"));
    std::fs::write(&path, text).ok()?;
    trim(folder);
    Some(path)
}

fn trim(folder: &Path) {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return;
    };
    let mut reports: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("dudel-problem-") && n.ends_with(".txt"))
        })
        .collect();
    // Millisecond stamps of equal length sort in time order.
    reports.sort();
    let excess = reports.len().saturating_sub(KEEP);
    for old in &reports[..excess] {
        let _ = std::fs::remove_file(old);
    }
}

/// Tell the artist the window closed after a panic, and offer the report.
pub(crate) fn tell_artist() {
    let report = LAST.lock().ok().and_then(|last| last.clone());
    let location = report.as_ref().map_or_else(
        || "No report could be written.".to_owned(),
        |path| format!("A report was saved to:\n{}", path.display()),
    );
    let answer = rfd::MessageDialog::new()
        .set_title("Dudel closed unexpectedly")
        .set_level(rfd::MessageLevel::Error)
        .set_description(format!(
            "Something went wrong and Dudel had to close. Sorry about that.\n\n\
             Unsaved changes are kept in recovery snapshots: Dudel offers to \
             restore them the next time it starts.\n\n{location}\n\n\
             You can help by attaching the report to a problem report at\n{ISSUES}\n\n\
             Open the report folder now?"
        ))
        .set_buttons(rfd::MessageButtons::YesNo)
        .show();
    if answer == rfd::MessageDialogResult::Yes
        && let Some(folder) = report.as_deref().and_then(Path::parent)
    {
        let _ = crate::locations::reveal(folder);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_are_written_and_only_the_newest_are_kept() {
        let folder = tempfile::tempdir().unwrap();
        for index in 0..KEEP + 3 {
            let name = format!("dudel-problem-{:013}.txt", index);
            std::fs::write(folder.path().join(name), "old").unwrap();
        }
        std::fs::write(folder.path().join("keep-me.txt"), "other").unwrap();
        let path = write_report(folder.path(), "Message: test").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "Message: test");
        let left: Vec<_> = std::fs::read_dir(folder.path())
            .unwrap()
            .flatten()
            .map(|e| e.file_name().into_string().unwrap())
            .collect();
        assert_eq!(left.len(), KEEP + 1);
        assert!(left.contains(&"keep-me.txt".to_owned()));
        assert!(!left.contains(&format!("dudel-problem-{:013}.txt", 0)));
    }
}
