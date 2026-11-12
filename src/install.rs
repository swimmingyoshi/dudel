//! Dudel as a set of files on disk: which files belong to it, how a new set
//! replaces the old one while Dudel runs, and, on Windows, installing the
//! portable copy for the current user (programs folder, Start menu,
//! `.dudel` files, Add or remove programs). Nothing here needs administrator
//! rights, and the portable copy keeps working as it is.
use std::path::{Path, PathBuf};

/// Every file of a package, written by the packaging scripts. Installing
/// and updating touch these files and no others, wherever Dudel was put.
pub(crate) const FILE_LIST: &str = "FILES.json";
/// New files wait here, inside Dudel's folder so they move in by renaming.
const STAGING: &str = ".dudel-update";
/// Replaced files move here; a running program cannot be deleted, only moved.
const RETIRED: &str = ".dudel-old";

#[derive(serde::Deserialize)]
struct FileList {
    files: Vec<String>,
}

/// A package path that stays inside the folder it is placed in.
pub(crate) fn safe_relative(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= 200
        && !path.starts_with(".dudel-")
        && !path.contains(|c: char| c.is_control() || r#"\:<>"|?*"#.contains(c))
        && path.split('/').all(|part| {
            !part.is_empty() && part != "." && part != ".." && !part.ends_with(['.', ' '])
        })
}

/// The files of the package in `folder`, or None when it has no list (a
/// development build, or a package older than the list).
pub(crate) fn listed_files(folder: &Path) -> Option<Vec<String>> {
    let text = std::fs::read_to_string(folder.join(FILE_LIST)).ok()?;
    let list: FileList = serde_json::from_str(text.trim_start_matches('\u{feff}')).ok()?;
    let mut files: Vec<String> = list
        .files
        .into_iter()
        .filter(|file| safe_relative(file))
        .collect();
    if !files.iter().any(|file| file == FILE_LIST) {
        files.push(FILE_LIST.to_owned());
    }
    files.sort();
    files.dedup();
    Some(files)
}

/// An empty folder inside `target` for the files about to be placed.
pub(crate) fn fresh_staging(target: &Path) -> Result<PathBuf, String> {
    let staging = target.join(STAGING);
    if staging.exists() {
        std::fs::remove_dir_all(&staging)
            .map_err(|e| format!("Could not clear {}: {e}", staging.display()))?;
    }
    std::fs::create_dir_all(&staging).map_err(|e| {
        format!(
            "Dudel's folder cannot be changed ({}): {e}",
            target.display()
        )
    })?;
    Ok(staging)
}

/// Move `files` from the staging folder into `target`, replacing what is
/// there, then retire the `previous` files the new set no longer has.
///
/// Files in use (the running program, the thumbnail library) are moved
/// aside instead of deleted, which Windows allows. If any file cannot be
/// placed, everything is put back and `target` is as it was.
pub(crate) fn place(target: &Path, files: &[String], previous: &[String]) -> Result<(), String> {
    let staging = target.join(STAGING);
    let retired = (0..1000)
        .map(|n| target.join(format!("{RETIRED}-{n}")))
        .find(|path| !path.exists())
        .ok_or("Too many leftover update folders")?;
    let step = |what: &str, path: &Path, error: std::io::Error| {
        format!("Could not {what} {}: {error}", path.display())
    };
    let parent = |path: &Path| -> Result<(), String> {
        let folder = path.parent().unwrap_or(target);
        std::fs::create_dir_all(folder).map_err(|e| step("create", folder, e))
    };
    // (file, whether an older copy was moved aside)
    let mut placed: Vec<(&String, bool)> = Vec::new();
    let result = files.iter().try_for_each(|file| {
        let (new, live, old) = (staging.join(file), target.join(file), retired.join(file));
        parent(&live)?;
        let replaced = live.exists();
        if replaced {
            parent(&old)?;
            std::fs::rename(&live, &old).map_err(|e| step("replace", &live, e))?;
        }
        placed.push((file, replaced));
        std::fs::rename(&new, &live).map_err(|e| step("place", &live, e))
    });
    if let Err(error) = result {
        for (file, replaced) in placed.into_iter().rev() {
            let live = target.join(file);
            let _ = std::fs::remove_file(&live);
            if replaced {
                let _ = std::fs::rename(retired.join(file), &live);
            }
        }
        let _ = std::fs::remove_dir_all(&staging);
        let _ = std::fs::remove_dir_all(&retired);
        return Err(error);
    }
    for file in previous {
        if !files.contains(file) && safe_relative(file) {
            let (live, old) = (target.join(file), retired.join(file));
            if live.is_file() && parent(&old).is_ok() {
                let _ = std::fs::rename(&live, &old);
            }
        }
    }
    let _ = std::fs::remove_dir_all(&staging);
    // Fails while the old program still runs; `sweep` clears it later.
    let _ = std::fs::remove_dir_all(&retired);
    Ok(())
}

/// Clear what earlier installs and updates left in `folder`.
pub(crate) fn sweep(folder: &Path) {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if (name == STAGING || name.starts_with(&format!("{RETIRED}-"))) && entry.path().is_dir() {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}

/// The folder of the running program, when it is a packaged copy that can
/// be replaced in place.
pub(crate) fn packaged_folder() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let folder = exe.parent()?;
    folder.join(FILE_LIST).is_file().then(|| folder.to_owned())
}

/// Housekeeping at an ordinary start: clear update leftovers and, after an
/// update of the installed copy, tell Windows the new version.
pub(crate) fn startup() {
    if let Some(folder) = packaged_folder() {
        sweep(&folder);
        #[cfg(windows)]
        windows_only::refresh(&folder);
    }
}

#[cfg(windows)]
pub(crate) use windows_only::{Options, install, running_installed, uninstall_command};

#[cfg(not(windows))]
pub(crate) fn running_installed() -> bool {
    false
}

#[cfg(windows)]
mod windows_only {
    use super::*;
    use windows::{
        Win32::{
            Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS},
            System::{
                Com::{
                    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance,
                    CoInitializeEx, CoUninitialize, IPersistFile, STGM_READ,
                },
                Registry::{
                    HKEY_CURRENT_USER, REG_DWORD, REG_SZ, RRF_RT_REG_SZ, RegDeleteKeyValueW,
                    RegDeleteTreeW, RegGetValueW, RegSetKeyValueW,
                },
            },
            UI::Shell::{IShellLinkW, SHCNE_ASSOCCHANGED, SHCNF_IDLIST, SHChangeNotify, ShellLink},
        },
        core::{HSTRING, Interface, PCWSTR},
    };

    const PROGRAM: &str = "dudel.exe";
    const THUMBNAILS: &str = "dudel_thumbnails.dll";
    const UNINSTALL: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall\Dudel";
    const EXTENSION: &str = r"Software\Classes\.dudel";
    const PROJECT_TYPE: &str = "Dudel.Project";
    const SHORTCUT: &str = "Dudel.lnk";
    /// Where the thumbnail library registers itself (shell/thumbnails).
    const THUMBNAIL_LIBRARY: &str =
        r"Software\Classes\CLSID\{7FD5A7C1-D502-47B2-A703-086E549231FF}\InprocServer32";

    /// What installing adds besides the files and the Start menu entry.
    #[derive(Clone, Copy)]
    pub(crate) struct Options {
        pub desktop: bool,
        pub open_projects: bool,
        pub thumbnails: bool,
    }

    impl Default for Options {
        fn default() -> Self {
            Self {
                desktop: true,
                open_projects: true,
                thumbnails: true,
            }
        }
    }

    #[derive(Debug, PartialEq)]
    enum Value {
        Text(String),
        Number(u32),
    }

    /// Where an installed Dudel lives: the current user's programs folder.
    pub(crate) fn target() -> Option<PathBuf> {
        directories::BaseDirs::new().map(|dirs| dirs.data_local_dir().join(r"Programs\Dudel"))
    }

    fn same_folder(a: &Path, b: &Path) -> bool {
        match (a.canonicalize(), b.canonicalize()) {
            (Ok(a), Ok(b)) => a == b,
            _ => false,
        }
    }

    fn program_folder() -> Option<PathBuf> {
        Some(std::env::current_exe().ok()?.parent()?.to_owned())
    }

    /// The folder Windows has on record for the installed Dudel.
    fn recorded() -> Option<PathBuf> {
        let folder = PathBuf::from(read(UNINSTALL, "InstallLocation")?);
        folder.join(PROGRAM).is_file().then_some(folder)
    }

    /// Whether the running program is the installed copy.
    pub(crate) fn running_installed() -> bool {
        match (recorded(), program_folder()) {
            (Some(recorded), Some(folder)) => same_folder(&recorded, &folder),
            _ => false,
        }
    }

    /// The registry entries of an installation in `folder`.
    fn entries(
        folder: &Path,
        version: &str,
        kilobytes: u32,
        open_projects: bool,
    ) -> Vec<(String, Option<&'static str>, Value)> {
        let program = folder.join(PROGRAM).display().to_string();
        let text = |value: &str| Value::Text(value.to_owned());
        let mut entries = vec![
            (UNINSTALL.to_owned(), Some("DisplayName"), text("Dudel")),
            (UNINSTALL.to_owned(), Some("DisplayVersion"), text(version)),
            (UNINSTALL.to_owned(), Some("Publisher"), text("Dudel")),
            (
                UNINSTALL.to_owned(),
                Some("InstallLocation"),
                text(&folder.display().to_string()),
            ),
            (UNINSTALL.to_owned(), Some("DisplayIcon"), text(&program)),
            (
                UNINSTALL.to_owned(),
                Some("UninstallString"),
                Value::Text(format!("\"{program}\" --uninstall")),
            ),
            (
                UNINSTALL.to_owned(),
                Some("QuietUninstallString"),
                Value::Text(format!("\"{program}\" --uninstall --quiet")),
            ),
            (
                UNINSTALL.to_owned(),
                Some("URLInfoAbout"),
                text(env!("CARGO_PKG_REPOSITORY")),
            ),
            (
                UNINSTALL.to_owned(),
                Some("EstimatedSize"),
                Value::Number(kilobytes),
            ),
            (UNINSTALL.to_owned(), Some("NoModify"), Value::Number(1)),
            (UNINSTALL.to_owned(), Some("NoRepair"), Value::Number(1)),
        ];
        if open_projects {
            let class = format!(r"Software\Classes\{PROJECT_TYPE}");
            entries.extend([
                (EXTENSION.to_owned(), None, text(PROJECT_TYPE)),
                (class.clone(), None, text("Dudel project")),
                (
                    format!(r"{class}\DefaultIcon"),
                    None,
                    Value::Text(format!("\"{program}\",0")),
                ),
                (
                    format!(r"{class}\shell\open\command"),
                    None,
                    Value::Text(format!("\"{program}\" \"%1\"")),
                ),
            ]);
        }
        entries
    }

    fn failed(what: &str, error: impl std::fmt::Display) -> String {
        format!("{what}: {error}")
    }

    fn write(key: &str, name: Option<&str>, value: &Value) -> Result<(), String> {
        let name = name.map(HSTRING::from);
        let name = name
            .as_ref()
            .map_or(PCWSTR::null(), |name| PCWSTR(name.as_ptr()));
        let (kind, data): (_, Vec<u8>) = match value {
            Value::Text(text) => (
                REG_SZ,
                text.encode_utf16()
                    .chain([0])
                    .flat_map(u16::to_le_bytes)
                    .collect(),
            ),
            Value::Number(number) => (REG_DWORD, number.to_le_bytes().to_vec()),
        };
        // SAFETY: the key, name and data outlive the call, and the length
        // given is the data's own.
        unsafe {
            RegSetKeyValueW(
                HKEY_CURRENT_USER,
                &HSTRING::from(key),
                name,
                kind.0,
                Some(data.as_ptr().cast()),
                data.len() as u32,
            )
        }
        .ok()
        .map_err(|e| failed("Windows registry", e.message()))
    }

    fn read(key: &str, name: &str) -> Option<String> {
        let mut buffer = vec![0u16; 2048];
        let mut bytes = (buffer.len() * 2) as u32;
        // SAFETY: the buffer holds `bytes` bytes for the whole call.
        let status = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                &HSTRING::from(key),
                &HSTRING::from(name),
                RRF_RT_REG_SZ,
                None,
                Some(buffer.as_mut_ptr().cast()),
                Some(&mut bytes),
            )
        };
        if status != ERROR_SUCCESS {
            return None;
        }
        let length = (bytes as usize / 2).min(buffer.len());
        let text = String::from_utf16_lossy(&buffer[..length]);
        Some(text.trim_end_matches('\0').to_owned())
    }

    fn delete_tree(key: &str) -> Result<(), String> {
        // SAFETY: the key name outlives the call.
        let status = unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, &HSTRING::from(key)) };
        if status == ERROR_SUCCESS || status == ERROR_FILE_NOT_FOUND {
            Ok(())
        } else {
            Err(failed("Windows registry", status.to_hresult().message()))
        }
    }

    /// Run `work` with the shell's link object.
    fn with_link<R>(
        work: impl FnOnce(&IShellLinkW, &IPersistFile) -> windows::core::Result<R>,
    ) -> Result<R, String> {
        // SAFETY: plain COM calls on this thread; the library is released
        // again only if this call was the one that initialized it.
        unsafe {
            let initialized = CoInitializeEx(None, COINIT_APARTMENTTHREADED).is_ok();
            let result = (|| {
                let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;
                let file: IPersistFile = link.cast()?;
                work(&link, &file)
            })();
            if initialized {
                CoUninitialize();
            }
            result.map_err(|e| failed("Shortcut", e.message()))
        }
    }

    fn create_shortcut(link: &Path, program: &Path) -> Result<(), String> {
        if let Some(folder) = link.parent() {
            std::fs::create_dir_all(folder).map_err(|e| failed("Shortcut", e))?;
        }
        with_link(|shell, file| unsafe {
            shell.SetPath(&HSTRING::from(program.as_os_str()))?;
            if let Some(folder) = program.parent() {
                shell.SetWorkingDirectory(&HSTRING::from(folder.as_os_str()))?;
            }
            shell.SetDescription(&HSTRING::from("Dudel drawing app"))?;
            file.Save(&HSTRING::from(link.as_os_str()), true)
        })
    }

    /// The program a shortcut starts.
    fn shortcut_program(link: &Path) -> Option<PathBuf> {
        with_link(|shell, file| unsafe {
            file.Load(&HSTRING::from(link.as_os_str()), STGM_READ)?;
            let mut buffer = [0u16; 1024];
            shell.GetPath(&mut buffer, std::ptr::null_mut(), 0)?;
            let length = buffer.iter().position(|&c| c == 0).unwrap_or(0);
            Ok(PathBuf::from(String::from_utf16_lossy(&buffer[..length])))
        })
        .ok()
    }

    fn shortcuts() -> [Option<PathBuf>; 2] {
        let menu = directories::BaseDirs::new().map(|dirs| {
            dirs.config_dir()
                .join(r"Microsoft\Windows\Start Menu\Programs")
                .join(SHORTCUT)
        });
        let desktop = directories::UserDirs::new()
            .and_then(|dirs| dirs.desktop_dir().map(|desktop| desktop.join(SHORTCUT)));
        [menu, desktop]
    }

    fn kilobytes(folder: &Path, files: &[String]) -> u32 {
        let bytes: u64 = files
            .iter()
            .filter_map(|file| std::fs::metadata(folder.join(file)).ok())
            .map(|metadata| metadata.len())
            .sum();
        (bytes / 1024).min(u64::from(u32::MAX)) as u32
    }

    fn files_changed() {
        // SAFETY: no item is passed, as this event requires.
        unsafe { SHChangeNotify(SHCNE_ASSOCCHANGED, SHCNF_IDLIST, None, None) };
    }

    /// Install the running portable copy for the current user and return
    /// the installed program. An existing installation is replaced, also
    /// while it runs.
    pub(crate) fn install(options: Options) -> Result<PathBuf, String> {
        let source = program_folder().ok_or("Dudel's own folder is unknown")?;
        let target = target().ok_or("The programs folder is unknown")?;
        if same_folder(&source, &target) {
            return Err("This is already the installed Dudel".into());
        }
        // A development build has no list: the program and its thumbnail
        // library are all there is.
        let files = listed_files(&source).unwrap_or_else(|| {
            [PROGRAM, THUMBNAILS]
                .into_iter()
                .filter(|file| source.join(file).is_file())
                .map(str::to_owned)
                .collect()
        });
        if !files.iter().any(|file| file == PROGRAM) {
            return Err(format!("{PROGRAM} is not part of this copy"));
        }
        let previous = listed_files(&target).unwrap_or_default();
        std::fs::create_dir_all(&target)
            .map_err(|e| format!("Could not create {}: {e}", target.display()))?;
        let staging = fresh_staging(&target)?;
        for file in &files {
            let (from, to) = (source.join(file), staging.join(file));
            if let Some(folder) = to.parent() {
                std::fs::create_dir_all(folder).map_err(|e| failed("Copying", e))?;
            }
            std::fs::copy(&from, &to)
                .map_err(|e| format!("Could not copy {}: {e}", from.display()))?;
        }
        place(&target, &files, &previous)?;
        let program = target.join(PROGRAM);
        let [menu, desktop] = shortcuts();
        create_shortcut(&menu.ok_or("The Start menu folder is unknown")?, &program)?;
        if options.desktop
            && let Some(desktop) = desktop
        {
            create_shortcut(&desktop, &program)?;
        }
        let size = kilobytes(&target, &files);
        for (key, name, value) in entries(
            &target,
            env!("CARGO_PKG_VERSION"),
            size,
            options.open_projects,
        ) {
            write(&key, name, &value)?;
        }
        let library = target.join(THUMBNAILS);
        if options.thumbnails && library.is_file() {
            crate::file_preview::register_thumbnails(&library, true)?;
        }
        files_changed();
        Ok(program)
    }

    /// After an update replaced the installed copy: the version and size
    /// Windows shows under Add or remove programs.
    pub(super) fn refresh(folder: &Path) {
        if !recorded().is_some_and(|recorded| same_folder(&recorded, folder))
            || read(UNINSTALL, "DisplayVersion").as_deref() == Some(env!("CARGO_PKG_VERSION"))
        {
            return;
        }
        let files = listed_files(folder).unwrap_or_default();
        for (key, name, value) in entries(
            folder,
            env!("CARGO_PKG_VERSION"),
            kilobytes(folder, &files),
            false,
        ) {
            let _ = write(&key, name, &value);
        }
    }

    /// `dudel.exe --uninstall`, which Windows runs from Add or remove
    /// programs: ask (unless `quiet`), then remove the installed copy and
    /// everything installing added. Projects, brushes and settings are kept.
    pub(crate) fn uninstall_command(quiet: bool) -> Result<(), String> {
        let folder = recorded().ok_or("Dudel is not installed for this user")?;
        if !program_folder().is_some_and(|own| same_folder(&own, &folder)) {
            return Err("Run the uninstall from the installed Dudel".into());
        }
        if !quiet {
            let answer = rfd::MessageDialog::new()
                .set_title("Remove Dudel")
                .set_description(
                    "Remove Dudel from this PC?

Your projects, brushes and settings are kept.",
                )
                .set_buttons(rfd::MessageButtons::YesNo)
                .show();
            if answer != rfd::MessageDialogResult::Yes {
                return Ok(());
            }
        }
        // Only what points at this installation is removed: previews or a
        // file type that another copy of Dudel registered stay.
        let library = folder.join(THUMBNAILS);
        if read(THUMBNAIL_LIBRARY, "").is_some_and(|path| Path::new(&path).starts_with(&folder)) {
            let _ = crate::file_preview::register_thumbnails(&library, false);
        }
        if read(EXTENSION, "").as_deref() == Some(PROJECT_TYPE) {
            // SAFETY: the key name outlives the call; no name is the default value.
            let _ = unsafe {
                RegDeleteKeyValueW(HKEY_CURRENT_USER, &HSTRING::from(EXTENSION), PCWSTR::null())
            };
        }
        delete_tree(&format!(r"Software\Classes\{PROJECT_TYPE}"))?;
        delete_tree(UNINSTALL)?;
        for link in shortcuts().into_iter().flatten() {
            if shortcut_program(&link).is_some_and(|program| program.starts_with(&folder)) {
                let _ = std::fs::remove_file(&link);
            }
        }
        files_changed();
        sweep(&folder);
        let files = listed_files(&folder).unwrap_or_else(|| vec![THUMBNAILS.to_owned()]);
        let mut folders = std::collections::BTreeSet::new();
        for file in files.iter().filter(|file| file.as_str() != PROGRAM) {
            let path = folder.join(file);
            let _ = std::fs::remove_file(&path);
            folders.extend(
                path.ancestors()
                    .skip(1)
                    .take_while(|parent| *parent != folder)
                    .map(Path::to_owned),
            );
        }
        // Deepest first; a folder holding anything that is not Dudel's stays.
        for path in folders.iter().rev() {
            let _ = std::fs::remove_dir(path);
        }
        remove_after_exit(&folder);
        if !quiet {
            rfd::MessageDialog::new()
                .set_title("Remove Dudel")
                .set_description("Dudel was removed from this PC.")
                .show();
        }
        Ok(())
    }

    /// A running program cannot delete itself: a hidden command prompt
    /// waits for this process to end, deletes the program, and removes its
    /// folder if nothing else is left in it.
    fn remove_after_exit(folder: &Path) {
        use std::os::windows::process::CommandExt;
        let program = folder.join(PROGRAM).display().to_string();
        let folder = folder.display().to_string();
        // The prompt would read these characters as its own syntax.
        if program.contains(['%', '"', '^', '!']) {
            return;
        }
        let system = std::env::var_os("SystemRoot").unwrap_or_else(|| r"C:\Windows".into());
        let script = format!(
            "/d /c for /l %i in (1,1,60) do (del /f /q \"{program}\" 2>nul & if not exist \"{program}\" (rmdir \"{folder}\" 2>nul & exit) else (ping -n 2 127.0.0.1 >nul))"
        );
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let _ = std::process::Command::new(Path::new(&system).join(r"System32\cmd.exe"))
            .raw_arg(script)
            .current_dir(std::env::temp_dir())
            .creation_flags(CREATE_NO_WINDOW)
            .spawn();
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn an_installation_is_recorded_for_the_current_user_only() {
            let folder = Path::new(r"C:\Users\artist\AppData\Local\Programs\Dudel");
            let entries = entries(folder, "1.2.3", 4096, true);
            let find = |key: &str, name: Option<&str>| {
                entries
                    .iter()
                    .find(|(k, n, _)| k.ends_with(key) && *n == name)
                    .map(|(_, _, value)| value)
            };
            assert_eq!(
                find("Uninstall\\Dudel", Some("UninstallString")),
                Some(&Value::Text(
                    r#""C:\Users\artist\AppData\Local\Programs\Dudel\dudel.exe" --uninstall"#
                        .into()
                ))
            );
            assert_eq!(
                find("Uninstall\\Dudel", Some("DisplayVersion")),
                Some(&Value::Text("1.2.3".into()))
            );
            assert_eq!(
                find(r"Dudel.Project\shell\open\command", None),
                Some(&Value::Text(
                    r#""C:\Users\artist\AppData\Local\Programs\Dudel\dudel.exe" "%1""#.into()
                ))
            );
            assert_eq!(
                find(".dudel", None),
                Some(&Value::Text("Dudel.Project".into()))
            );
            // Nothing outside the per-user hive, and no file type when declined.
            assert!(
                entries
                    .iter()
                    .all(|(key, ..)| key.starts_with("Software\\"))
            );
            let plain = super::entries(folder, "1.2.3", 4096, false);
            assert!(plain.iter().all(|(key, ..)| key == UNINSTALL));
        }

        #[test]
        fn a_shortcut_remembers_the_program_it_starts() {
            let folder = tempfile::tempdir().unwrap();
            let program = folder.path().join("dudel.exe");
            std::fs::write(&program, b"program").unwrap();
            let link = folder.path().join("menu").join(SHORTCUT);
            create_shortcut(&link, &program).unwrap();
            assert!(link.is_file());
            let found = shortcut_program(&link).unwrap();
            assert!(same_folder(
                found.parent().unwrap(),
                program.parent().unwrap()
            ));
            assert_eq!(found.file_name(), program.file_name());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(folder: &Path, file: &str, text: &str) {
        let path = folder.join(file);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    fn read(folder: &Path, file: &str) -> Option<String> {
        std::fs::read_to_string(folder.join(file)).ok()
    }

    fn names(files: &[&str]) -> Vec<String> {
        files.iter().map(|file| (*file).to_owned()).collect()
    }

    #[test]
    fn package_paths_cannot_leave_their_folder() {
        for good in ["dudel.exe", "docs/QUICK_START.md", "a/b/c.txt", "LICENSE"] {
            assert!(safe_relative(good), "{good}");
        }
        for bad in [
            "",
            "/dudel.exe",
            "../dudel.exe",
            "docs/../../x",
            "docs//x",
            r"docs\x",
            "C:/x",
            "x:stream",
            "docs/.",
            "trailing./x",
            ".dudel-update/x",
            "a\nb",
            "what?",
        ] {
            assert!(!safe_relative(bad), "{bad}");
        }
    }

    #[test]
    fn a_file_list_names_itself_and_drops_unsafe_paths() {
        let folder = tempfile::tempdir().unwrap();
        assert!(listed_files(folder.path()).is_none());
        write(
            folder.path(),
            FILE_LIST,
            "\u{feff}{\"files\":[\"dudel.exe\",\"../escape\",\"docs/a.md\",\"dudel.exe\"]}",
        );
        assert_eq!(
            listed_files(folder.path()).unwrap(),
            names(&[FILE_LIST, "docs/a.md", "dudel.exe"])
        );
    }

    #[test]
    fn new_files_replace_old_ones_and_dropped_files_leave() {
        let folder = tempfile::tempdir().unwrap();
        let target = folder.path();
        write(target, "dudel.exe", "old program");
        write(target, "docs/old.md", "old page");
        write(target, "docs/kept.md", "old kept");
        write(target, "my-notes.txt", "the artist's own file");
        let staging = fresh_staging(target).unwrap();
        write(&staging, "dudel.exe", "new program");
        write(&staging, "docs/kept.md", "new kept");
        write(&staging, "docs/new/page.md", "new page");
        place(
            target,
            &names(&["dudel.exe", "docs/kept.md", "docs/new/page.md"]),
            &names(&["dudel.exe", "docs/old.md", "docs/kept.md"]),
        )
        .unwrap();
        assert_eq!(read(target, "dudel.exe").unwrap(), "new program");
        assert_eq!(read(target, "docs/kept.md").unwrap(), "new kept");
        assert_eq!(read(target, "docs/new/page.md").unwrap(), "new page");
        assert!(read(target, "docs/old.md").is_none());
        // Files that were never Dudel's are left alone.
        assert_eq!(
            read(target, "my-notes.txt").unwrap(),
            "the artist's own file"
        );
        sweep(target);
        let left: Vec<_> = std::fs::read_dir(target)
            .unwrap()
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.starts_with(".dudel-"))
            .collect();
        assert!(left.is_empty(), "{left:?}");
    }

    #[test]
    fn a_failed_replacement_leaves_the_old_files_in_place() {
        let folder = tempfile::tempdir().unwrap();
        let target = folder.path();
        write(target, "dudel.exe", "old program");
        write(target, "docs/a.md", "old page");
        let staging = fresh_staging(target).unwrap();
        write(&staging, "dudel.exe", "new program");
        write(&staging, "docs/a.md", "new page");
        // The third file never arrived.
        let error = place(
            target,
            &names(&["dudel.exe", "docs/a.md", "missing.dll"]),
            &names(&["dudel.exe", "docs/a.md"]),
        )
        .unwrap_err();
        assert!(error.contains("missing.dll"), "{error}");
        assert_eq!(read(target, "dudel.exe").unwrap(), "old program");
        assert_eq!(read(target, "docs/a.md").unwrap(), "old page");
        assert!(!target.join("missing.dll").exists());
        assert!(!target.join(STAGING).exists());
    }
}
