//! eframe writes only a private session file. Validated snapshots are published
//! atomically to the compatible RON store while holding a cooperating-writer lock.
use crate::preferences::{self, Preferences};
use std::{
    collections::HashMap,
    fs::{File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{Arc, Condvar, Mutex},
    time::{Duration, Instant},
};
const LIMIT: u64 = 384 * 1024 * 1024;
#[derive(Clone, Default)]
pub(crate) struct Status {
    pub saving: bool,
    pub error: Option<String>,
}
#[derive(Default)]
struct State {
    wanted: Option<String>,
    stopping: bool,
    status: Status,
}
#[derive(Clone)]
pub(crate) struct Handle(Arc<(Mutex<State>, Condvar)>);
impl Handle {
    pub(crate) fn request(&self, source: String) {
        let mut state = self.0.0.lock().unwrap();
        state.wanted = Some(source);
        state.status.saving = true;
        self.0.1.notify_one();
    }
    pub(crate) fn status(&self) -> Status {
        self.0.0.lock().unwrap().status.clone()
    }
}
#[derive(Clone)]
pub(crate) struct Issue {
    pub path: PathBuf,
    pub reason: String,
}
pub(crate) struct Session {
    _directory: tempfile::TempDir,
    pub runtime: PathBuf,
    pub issue: Option<Issue>,
    pub handle: Handle,
    worker: Option<std::thread::JoinHandle<()>>,
}
fn raw(path: &Path) -> Result<Vec<u8>, String> {
    let file = File::open(path).map_err(|e| e.to_string())?;
    if file.metadata().map_err(|e| e.to_string())?.len() > LIMIT {
        return Err("Settings container exceeds the size limit".into());
    }
    let mut bytes = Vec::new();
    file.take(LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > LIMIT {
        return Err("Settings container exceeds the size limit".into());
    }
    Ok(bytes)
}
pub(crate) fn preferences_from_container(bytes: &[u8]) -> Result<String, String> {
    let values: HashMap<String, String> = ron::de::from_bytes(bytes).map_err(|e| e.to_string())?;
    let source = values
        .get(preferences::KEY)
        .ok_or("The settings container has no app preferences")?;
    Preferences::decode(source)?;
    Ok(source.clone())
}
fn valid_container(bytes: &[u8]) -> Result<(), String> {
    let values: HashMap<String, String> = ron::de::from_bytes(bytes).map_err(|e| e.to_string())?;
    if let Some(source) = values.get(preferences::KEY) {
        Preferences::decode(source)?;
    }
    Ok(())
}
fn copy_snapshot(path: &Path, directory: &Path) -> Result<tempfile::NamedTempFile, String> {
    let mut source = File::open(path).map_err(|e| e.to_string())?;
    let mut file = tempfile::NamedTempFile::new_in(directory).map_err(|e| e.to_string())?;
    std::io::copy(&mut source, file.as_file_mut()).map_err(|e| e.to_string())?;
    file.as_file().sync_all().map_err(|e| e.to_string())?;
    Ok(file)
}
pub(crate) fn equal_files(a: &Path, b: &Path) -> Result<bool, String> {
    let mut a = File::open(a).map_err(|e| e.to_string())?;
    let mut b = File::open(b).map_err(|e| e.to_string())?;
    if a.metadata().map_err(|e| e.to_string())?.len()
        != b.metadata().map_err(|e| e.to_string())?.len()
    {
        return Ok(false);
    }
    let (mut x, mut y) = ([0; 65536], [0; 65536]);
    loop {
        let n = a.read(&mut x).map_err(|e| e.to_string())?;
        b.read_exact(&mut y[..n]).map_err(|e| e.to_string())?;
        if x[..n] != y[..n] {
            return Ok(false);
        }
        if n == 0 {
            return Ok(b.read(&mut y[..1]).map_err(|e| e.to_string())? == 0);
        }
    }
}
fn lock_writer(path: &Path) -> Result<File, String> {
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(0);
    }
    let file = options.open(path).map_err(|e| {
        format!("Settings writer is unavailable (another window or file permissions): {e}")
    })?;
    #[cfg(not(windows))]
    file.try_lock().map_err(|e| e.to_string())?;
    Ok(file)
}
struct Publisher {
    canonical: PathBuf,
    previous: PathBuf,
    directory: PathBuf,
    baseline: Option<tempfile::NamedTempFile>,
    unreadable: bool,
    lock: Option<File>,
    archived: Option<PathBuf>,
}
impl Publisher {
    fn unchanged(&self) -> Result<(), String> {
        if self.unreadable {
            return Err(
                "The original settings could not be read. Restart after restoring file access"
                    .into(),
            );
        }
        let same = if let Some(before) = &self.baseline {
            equal_files(&self.canonical, before.path())?
        } else {
            matches!(File::open(&self.canonical), Err(e) if e.kind() == std::io::ErrorKind::NotFound)
        };
        if same {
            Ok(())
        } else {
            Err(
                "Settings changed outside this session. Restart to reload them before saving"
                    .into(),
            )
        }
    }
    fn publish(&mut self, bytes: &[u8]) -> Result<(), String> {
        valid_container(bytes)?;
        let parent = self
            .canonical
            .parent()
            .ok_or("Settings directory is missing")?;
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        if self.lock.is_none() {
            self.lock = Some(lock_writer(
                &self.canonical.with_extension("ron.dudel-lock"),
            )?);
        }
        self.unchanged()?;
        if let Some(before) = &self.baseline
            && raw(before.path()).is_ok_and(|old| old == bytes)
        {
            return Ok(());
        }
        let mut next =
            tempfile::NamedTempFile::new_in(&self.directory).map_err(|e| e.to_string())?;
        next.write_all(bytes).map_err(|e| e.to_string())?;
        next.as_file().sync_all().map_err(|e| e.to_string())?;
        let mut staged = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
        staged.write_all(bytes).map_err(|e| e.to_string())?;
        staged.as_file().sync_all().map_err(|e| e.to_string())?;
        // An unreadable primary never replaces an existing valid previous version.
        if let Some(before) = &self.baseline
            && raw(before.path())
                .ok()
                .and_then(|b| valid_container(&b).ok())
                .is_some()
        {
            match raw(&self.previous){
                Ok(old) if valid_container(&old).is_err()=>{
                    let archived_matches=self.archived.as_ref().is_some_and(|p|equal_files(p,&self.previous).unwrap_or(false));
                    if !archived_matches {
                        let mut archive=tempfile::Builder::new().prefix("dudel-settings-preserved-").suffix(".ron").tempfile_in(parent).map_err(|e|e.to_string())?;
                        archive.write_all(&old).map_err(|e|e.to_string())?;archive.as_file().sync_all().map_err(|e|e.to_string())?;
                        self.archived=Some(archive.keep().map_err(|e|e.error.to_string())?.1);
                    }
                },
                Err(_) if !matches!(std::fs::symlink_metadata(&self.previous),Err(e) if e.kind()==std::io::ErrorKind::NotFound)=>return Err("The previous settings backup could not be read; original settings were preserved".into()),
                _=>{},
            }
            let old = raw(before.path())?;
            crate::storage::atomic_write(&self.previous, &old)?;
        }
        self.unchanged()?;
        staged
            .persist(&self.canonical)
            .map_err(|e| e.error.to_string())?;
        self.baseline = Some(next);
        Ok(())
    }
}
impl Session {
    pub(crate) fn open(canonical: PathBuf) -> Result<Self, String> {
        let directory = tempfile::tempdir().map_err(|e| e.to_string())?;
        let runtime = directory.path().join("app.ron");
        let previous = canonical.with_extension("ron.previous");
        let mut issue = None;
        let mut baseline = None;
        let mut unreadable = false;
        match File::open(&canonical) {
            Ok(_) => match copy_snapshot(&canonical, directory.path()) {
                Ok(file) => {
                    match raw(file.path()).and_then(|b| {
                        valid_container(&b)?;
                        Ok(b)
                    }) {
                        Ok(bytes) => std::fs::write(&runtime, bytes).map_err(|e| e.to_string())?,
                        Err(reason) => {
                            issue = Some(Issue {
                                path: canonical.clone(),
                                reason,
                            })
                        }
                    }
                    baseline = Some(file);
                }
                Err(reason) => {
                    unreadable = true;
                    issue = Some(Issue {
                        path: canonical.clone(),
                        reason,
                    });
                }
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => {
                unreadable = true;
                issue = Some(Issue {
                    path: canonical.clone(),
                    reason: e.to_string(),
                });
            }
        }
        if issue.is_some()
            && let Ok(bytes) = raw(&previous)
            && preferences_from_container(&bytes).is_ok()
        {
            std::fs::write(&runtime, bytes).map_err(|e| e.to_string())?;
            issue
                .as_mut()
                .unwrap()
                .reason
                .push_str(". The previous saved version is available in this session");
        }
        let handle = Handle(Arc::new((Mutex::new(State::default()), Condvar::new())));
        let shared = handle.clone();
        let runtime_worker = runtime.clone();
        let mut publisher = Publisher {
            canonical,
            previous,
            directory: directory.path().to_owned(),
            baseline,
            unreadable,
            lock: None,
            archived: None,
        };
        let worker=std::thread::Builder::new().name("durable-settings".into()).spawn(move||{
            loop {
                let mut state=shared.0.0.lock().unwrap();
                while state.wanted.is_none() && !state.stopping{state=shared.0.1.wait(state).unwrap();}
                let Some(mut wanted)=state.wanted.take() else{break;};drop(state);
                let started=Instant::now();
                let result=loop {
                    if let Some(newer)=shared.0.0.lock().unwrap().wanted.take(){wanted=newer;}
                    if let Ok(bytes)=raw(&runtime_worker) && preferences_from_container(&bytes).is_ok_and(|s|s==wanted){break publisher.publish(&bytes);}
                    if started.elapsed()>Duration::from_secs(5){break Err("The settings snapshot was not written completely. Retry saving settings".into());}
                    std::thread::sleep(Duration::from_millis(20));
                };
                let mut state=shared.0.0.lock().unwrap();
                state.status.saving=state.wanted.is_some();
                state.status.error=result.err();
            }
        }).map_err(|e|e.to_string())?;
        Ok(Self {
            _directory: directory,
            runtime,
            issue,
            handle,
            worker: Some(worker),
        })
    }
    pub(crate) fn finish(&mut self) -> Result<(), String> {
        {
            let mut state = self.handle.0.0.lock().unwrap();
            state.stopping = true;
            self.handle.0.1.notify_one();
        }
        if let Some(worker) = self.worker.take() {
            worker
                .join()
                .map_err(|_| "Settings worker failed".to_owned())?;
        }
        match self.handle.status().error {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.finish();
    }
}

impl crate::Dudel {
    pub(crate) fn settings_store_banner(&self, ctx: &crate::egui::Context) -> bool {
        let Some(handle) = &self.settings_store else {
            return false;
        };
        let status = handle.status();
        if status.saving {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
        let mut retry = false;
        if let Some(error) = status.error {
            crate::egui::TopBottomPanel::top("settings save failure").show(ctx, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.label(
                        crate::egui::RichText::new("Settings could not be saved")
                            .color(ui.visuals().warn_fg_color),
                    )
                    .on_hover_text(&error);
                    retry = ui
                        .add_enabled(
                            !status.saving,
                            crate::egui::Button::new("Retry saving settings"),
                        )
                        .clicked();
                });
            });
        }
        retry
    }
}

/// Separate-process native checks, restricted to an explicitly marked fixture folder.
pub(crate) fn probe(root: &Path, phase: &str) -> Result<(), String> {
    const MARKER: &[u8] = b"Dudel isolated settings fixture v1";
    const DAMAGED: &[u8] = b"{\"partial settings container";
    if !matches!(
        phase,
        "first" | "reopen" | "corrupt" | "repair" | "failure" | "retry"
    ) {
        return Err("Unknown settings probe phase".into());
    }
    if phase == "first" {
        std::fs::create_dir(root).map_err(|e| e.to_string())?;
        std::fs::write(root.join("fixture-marker"), MARKER).map_err(|e| e.to_string())?;
    } else if std::fs::read(root.join("fixture-marker")).map_err(|e| e.to_string())? != MARKER {
        return Err("Settings probe requires its isolated fixture marker".into());
    }
    let image = root.join(format!("{phase}.png"));
    let report = root.join(format!("{phase}.json"));
    if image.exists() || report.exists() {
        return Err("Probe outputs already exist".into());
    }
    let canonical = root.join("app.ron");
    let previous = canonical.with_extension("ron.previous");
    if phase == "failure" {
        preferences_from_container(&raw(&canonical)?)?;
        std::fs::rename(&previous, root.join("previous-before-failure.ron"))
            .map_err(|e| e.to_string())?;
        std::fs::create_dir(&previous).map_err(|e| e.to_string())?;
    }
    if phase == "retry" {
        std::fs::remove_dir(&previous).map_err(|e| e.to_string())?;
    }
    if phase == "corrupt" {
        preferences_from_container(&raw(&canonical)?)?;
        preferences_from_container(&raw(&previous)?)?;
        std::fs::write(&canonical, DAMAGED).map_err(|e| e.to_string())?;
    }
    let mut session = Session::open(canonical.clone())?;
    let original = raw(&canonical).ok();
    if phase == "failure" {
        let mut values: HashMap<String, String> =
            ron::de::from_bytes(&raw(&session.runtime)?).map_err(|e| e.to_string())?;
        let mut app = crate::Dudel::default();
        app.restore_preferences_source(values.get(preferences::KEY).unwrap());
        app.brush.opacity = 0.63;
        let source = app.encode_preferences()?;
        values.insert(preferences::KEY.into(), source.clone());
        std::fs::write(
            &session.runtime,
            ron::ser::to_string(&values).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        session.handle.request(source);
        let started = Instant::now();
        while session.handle.status().saving {
            if started.elapsed() > Duration::from_secs(8) {
                return Err("Failure probe timed out".into());
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        if session.handle.status().error.is_none() {
            return Err("Blocked settings write unexpectedly succeeded".into());
        }
    }
    let handle = session.handle.clone();
    let issue = session.issue.clone();
    if matches!(phase, "corrupt" | "repair") != issue.is_some() {
        return Err("Unexpected startup recovery state".into());
    }
    let phase_owned = phase.to_owned();
    let screenshot = image.clone();
    let folder = root.to_owned();
    let options = eframe::NativeOptions {
        persistence_path: Some(session.runtime.clone()),
        persist_window: false,
        viewport: crate::egui::ViewportBuilder::default()
            .with_inner_size([1280.0, 800.0])
            .with_visible(false),
        ..Default::default()
    };
    eframe::run_native(
        "Dudel isolated settings probe",
        options,
        Box::new(move |cc| {
            let mut app = crate::Dudel::for_capture(screenshot);
            if let Some(source) = cc.storage.and_then(|s| s.get_string(preferences::KEY)) {
                app.restore_preferences_source(&source);
            }
            if phase_owned != "first"
                && (app.brush.name != "Persistent settings fixture"
                    || app.color != [31, 97, 211]
                    || app.brush.size != 47.0)
            {
                return Err(std::io::Error::other(
                    "Native restart did not restore fixture preferences",
                )
                .into());
            }
            app.settings_store = Some(handle);
            if let Some(issue) = issue {
                app.protect_settings_container(issue.path, issue.reason);
            }
            if phase_owned == "first" {
                app.brush.name = "Persistent settings fixture".into();
                app.brush.size = 47.0;
                app.color = [31, 97, 211];
                app.hsv = crate::color_wheel::rgb_to_hsv(app.color);
            }
            if phase_owned == "reopen" {
                app.brush.opacity = 0.37;
            }
            if phase_owned == "retry" {
                app.brush.opacity = 0.63;
            }
            if phase_owned == "repair" {
                app.backup_preferences_to(cc.storage, &folder.join("corrupt-original.ron"))
                    .map_err(std::io::Error::other)?;
                app.use_current_preferences(cc.storage)
                    .map_err(std::io::Error::other)?;
            }
            Ok(Box::new(app))
        }),
    )
    .map_err(|e| e.to_string())?;
    let saved = session.finish();
    if phase == "failure" {
        if saved.is_ok() || Some(raw(&canonical)?) != original {
            return Err("Failed write did not preserve original settings".into());
        }
    } else {
        saved?;
    }
    if !image.is_file() {
        return Err("Native screenshot was not produced".into());
    }
    if phase == "corrupt" {
        if raw(&canonical)? != DAMAGED {
            return Err("Damaged primary was overwritten without backup approval".into());
        }
    } else {
        let source = preferences_from_container(&raw(&canonical)?)?;
        let mut restored = crate::Dudel::default();
        restored.restore_preferences_source(&source);
        if restored.brush.name != "Persistent settings fixture"
            || restored.brush.size != 47.0
            || restored.color != [31, 97, 211]
        {
            return Err("Committed preferences differ from fixture".into());
        }
        if phase == "reopen" && restored.brush.opacity != 0.37 {
            return Err("Second launch changes were not committed".into());
        }
        if phase == "retry" && restored.brush.opacity != 0.63 {
            return Err("Retry did not save changed settings".into());
        }
        if phase == "repair" && raw(&root.join("corrupt-original.ron"))? != DAMAGED {
            return Err("Damaged original backup differs".into());
        }
    }
    let json = serde_json::json!({"phase":phase,"passed":true,"isolated":true,"native_eframe_save_and_restart":true});
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(report)
        .map_err(|e| e.to_string())?;
    file.write_all(serde_json::to_string_pretty(&json).unwrap().as_bytes())
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn source(size: f32) -> String {
        let mut app = crate::Dudel::default();
        app.brush.size = size;
        app.encode_preferences().unwrap()
    }
    fn container(size: f32) -> Vec<u8> {
        ron::ser::to_string(&std::collections::BTreeMap::from([
            (preferences::KEY.to_owned(), source(size)),
            ("unknown-key".into(), "preserved".into()),
        ]))
        .unwrap()
        .into_bytes()
    }
    fn publish(session: &Session, size: f32) {
        std::fs::write(&session.runtime, container(size)).unwrap();
        session.handle.request(source(size));
    }
    fn wait(session: &Session) {
        let start = Instant::now();
        while session.handle.status().saving {
            assert!(start.elapsed() < Duration::from_secs(8));
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    #[test]
    fn compatible_migration_restart_and_distinct_previous() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("app.ron");
        let first = container(21.0);
        std::fs::write(&path, &first).unwrap();
        let mut session = Session::open(path.clone()).unwrap();
        assert!(session.issue.is_none());
        assert_eq!(raw(&session.runtime).unwrap(), first);
        publish(&session, 43.0);
        wait(&session);
        assert!(session.handle.status().error.is_none());
        publish(&session, 43.0);
        session.finish().unwrap();
        assert_eq!(raw(&path.with_extension("ron.previous")).unwrap(), first);
        assert_eq!(raw(&path).unwrap(), container(43.0));
        let mut reopen = Session::open(path).unwrap();
        assert_eq!(raw(&reopen.runtime).unwrap(), container(43.0));
        reopen.finish().unwrap();
    }
    #[test]
    fn partial_private_write_never_replaces_original() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("app.ron");
        std::fs::write(&path, container(21.0)).unwrap();
        let mut session = Session::open(path.clone()).unwrap();
        std::fs::write(&session.runtime, b"{partial").unwrap();
        session.handle.request(source(43.0));
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(raw(&path).unwrap(), container(21.0));
        std::fs::write(&session.runtime, container(43.0)).unwrap();
        session.finish().unwrap();
        assert_eq!(raw(&path).unwrap(), container(43.0));
    }
    #[test]
    fn corrupt_primary_uses_previous_and_requires_exact_backup() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("app.ron");
        let damaged = b"\xffpartial";
        std::fs::write(&path, damaged).unwrap();
        std::fs::write(path.with_extension("ron.previous"), container(21.0)).unwrap();
        let mut session = Session::open(path.clone()).unwrap();
        assert!(session.issue.is_some());
        assert_eq!(raw(&session.runtime).unwrap(), container(21.0));
        let mut app = crate::Dudel::default();
        app.protect_settings_container(path.clone(), "Damaged".into());
        assert!(app.use_current_preferences(None).is_err());
        let backup = dir.path().join("backup.ron");
        app.backup_preferences_to(None, &backup).unwrap();
        assert_eq!(raw(&backup).unwrap(), damaged);
        std::fs::write(&backup, b"changed").unwrap();
        assert!(app.use_current_preferences(None).is_err());
        std::fs::write(&backup, damaged).unwrap();
        app.use_current_preferences(None).unwrap();
        publish(&session, 43.0);
        session.finish().unwrap();
        assert_eq!(
            raw(&path.with_extension("ron.previous")).unwrap(),
            container(21.0)
        );
    }
    #[test]
    fn external_change_is_preserved() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("app.ron");
        std::fs::write(&path, container(21.0)).unwrap();
        let mut session = Session::open(path.clone()).unwrap();
        std::fs::write(&path, container(55.0)).unwrap();
        publish(&session, 43.0);
        assert!(session.finish().unwrap_err().contains("outside"));
        assert_eq!(raw(&path).unwrap(), container(55.0));
    }
    #[test]
    fn competing_writer_cannot_overwrite_and_retry_succeeds_after_release() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("app.ron");
        std::fs::write(&path, container(21.0)).unwrap();
        let mut first = Session::open(path.clone()).unwrap();
        let mut second = Session::open(path.clone()).unwrap();
        publish(&first, 21.0);
        wait(&first);
        publish(&second, 43.0);
        wait(&second);
        assert!(second.handle.status().error.is_some());
        assert_eq!(raw(&path).unwrap(), container(21.0));
        first.finish().unwrap();
        publish(&second, 43.0);
        second.finish().unwrap();
        assert_eq!(raw(&path).unwrap(), container(43.0));
    }
    #[test]
    fn failed_backup_write_preserves_primary_and_retries() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("app.ron");
        std::fs::write(&path, container(21.0)).unwrap();
        let previous = path.with_extension("ron.previous");
        std::fs::create_dir(&previous).unwrap();
        let mut session = Session::open(path.clone()).unwrap();
        publish(&session, 43.0);
        wait(&session);
        assert!(session.handle.status().error.is_some());
        assert_eq!(raw(&path).unwrap(), container(21.0));
        std::fs::remove_dir(previous).unwrap();
        publish(&session, 43.0);
        session.finish().unwrap();
        assert_eq!(raw(&path).unwrap(), container(43.0));
    }
    #[test]
    fn corrupt_previous_is_archived_before_rotation() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("app.ron");
        std::fs::write(&path, container(21.0)).unwrap();
        std::fs::write(path.with_extension("ron.previous"), b"damaged previous").unwrap();
        let mut session = Session::open(path.clone()).unwrap();
        publish(&session, 43.0);
        session.finish().unwrap();
        let archived: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(Result::unwrap)
            .filter(|e| {
                e.file_name()
                    .to_string_lossy()
                    .starts_with("dudel-settings-preserved-")
            })
            .collect();
        assert_eq!(archived.len(), 1);
        assert_eq!(raw(&archived[0].path()).unwrap(), b"damaged previous");
        assert_eq!(
            raw(&path.with_extension("ron.previous")).unwrap(),
            container(21.0)
        );
    }
    #[test]
    fn future_or_invalid_app_preferences_are_not_accepted_as_valid_containers() {
        assert!(valid_container(b"{partial").is_err());
        let bytes =
            ron::ser::to_string(&HashMap::from([(preferences::KEY, "{\"version\":999}")])).unwrap();
        assert!(valid_container(bytes.as_bytes()).is_err());
        assert!(valid_container(b"{\"widgets\":\"legacy\"}").is_ok());
        assert!(preferences_from_container(b"{\"widgets\":\"legacy\"}").is_err());
    }
}
