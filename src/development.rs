//! A development build starts with a copy of the installed Dudel's settings
//! and extensions, then keeps its own.
use std::path::Path;

/// Before the first start of a development build. Best effort: without it
/// the build starts with defaults. Nothing of the installed copy changes.
pub(crate) fn seed(settings: &Path) {
    if !crate::identity::DEVELOPMENT || settings.exists() {
        return;
    }
    if let Some(from) = eframe::storage_dir(crate::identity::PACKAGED_SETTINGS) {
        copy(&from.join("app.ron"), settings);
    }
    if let (Some(from), Some(to)) = (
        crate::identity::packaged_dirs(),
        crate::identity::project_dirs(),
    ) {
        let (from, to) = (from.data_local_dir(), to.data_local_dir());
        copy(&from.join("display.json"), &to.join("display.json"));
        for entry in std::fs::read_dir(from.join("extensions"))
            .into_iter()
            .flatten()
            .flatten()
        {
            copy(
                &entry.path(),
                &to.join("extensions").join(entry.file_name()),
            );
        }
    }
}

/// Copy one file unless the destination has one.
fn copy(from: &Path, to: &Path) {
    if from.is_file()
        && !to.exists()
        && let Some(folder) = to.parent()
        && std::fs::create_dir_all(folder).is_ok()
    {
        let _ = std::fs::copy(from, to);
    }
}

/// What About says about where this build came from.
pub(crate) fn origin() -> String {
    if crate::identity::DEVELOPMENT {
        return format!(
            "Development build, commit {} or later edits",
            option_env!("DUDEL_REVISION").unwrap_or("unknown")
        );
    }
    // Not compiled in: a package is rebuilt at the commit that records its
    // hash and must come out the same.
    #[derive(serde::Deserialize)]
    struct Build {
        revision: String,
    }
    std::env::current_exe()
        .ok()
        .and_then(|exe| std::fs::read_to_string(exe.parent()?.join("BUILD.json")).ok())
        .and_then(|text| serde_json::from_str::<Build>(text.trim_start_matches('\u{feff}')).ok())
        .map(|build| {
            format!(
                "Commit {}",
                build.revision.chars().take(7).collect::<String>()
            )
        })
        .unwrap_or_else(|| "Packaged build".to_owned())
}

#[cfg(test)]
mod tests {
    #[test]
    fn copying_settings_never_replaces_the_development_builds_own() {
        let dir = tempfile::tempdir().unwrap();
        let (from, to) = (dir.path().join("a.ron"), dir.path().join("new/b.ron"));
        super::copy(&from, &to);
        assert!(!to.exists());
        std::fs::write(&from, "first").unwrap();
        super::copy(&from, &to);
        std::fs::write(&from, "second").unwrap();
        super::copy(&from, &to);
        assert_eq!(std::fs::read_to_string(&to).unwrap(), "first");
    }
}
