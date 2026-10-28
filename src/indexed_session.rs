//! Opt-in editor session. One owner moves the locked journal between workers.
use super::*;
use crate::{
    document::Document,
    indexed_document::{self, Keys},
};

pub(crate) struct Session {
    journal: Journal,
    keys: Keys,
    path: PathBuf,
    last_stats: StreamStats,
}
fn target(path: &Path) -> Result<PathBuf> {
    if !path
        .extension()
        .and_then(|s| s.to_str())
        .is_some_and(|s| s.eq_ignore_ascii_case("didx"))
    {
        return Err("Experimental indexed saves require a .didx filename".into());
    }
    if path.exists() {
        let metadata = io(std::fs::symlink_metadata(path))?;
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            return Err("Indexed saves require a regular file path".into());
        }
        return io(std::fs::canonicalize(path));
    }
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    Ok(
        io(std::fs::canonicalize(parent))?
            .join(path.file_name().ok_or("Missing indexed filename")?),
    )
}
fn canceled(cancel: &impl Fn() -> bool) -> Result<()> {
    if cancel() {
        Err("Indexed save canceled before publication".into())
    } else {
        Ok(())
    }
}
impl Session {
    pub(crate) fn descriptor_bytes(&self) -> usize {
        self.journal.raster_proofs.descriptor_bytes() + self.keys.descriptor_bytes()
    }
    pub(crate) fn save(
        previous: Option<Self>,
        path: &Path,
        doc: &Document,
        cancel: &impl Fn() -> bool,
    ) -> Result<Self> {
        Self::save_with_hook(previous, path, doc, cancel, |_| Ok(()))
    }
    fn save_with_hook(
        previous: Option<Self>,
        path: &Path,
        doc: &Document,
        cancel: &impl Fn() -> bool,
        mut checkpoint: impl FnMut(Phase) -> Result<()>,
    ) -> Result<Self> {
        canceled(cancel)?;
        doc.validate()?;
        if doc.version != 1 || doc.layers.iter().any(|l| l.mask.is_some()) {
            return Err(
                "Indexed projects do not yet store layer masks; use a regular Dudel project".into(),
            );
        }
        let path = target(path)?;
        let mut session = if let Some(session) = previous.filter(|s| s.path == path) {
            same_destination(&path, identity(&session.journal.file)?)?;
            session
        } else if path.exists() {
            let mut journal = Journal::open(&path, true)?;
            same_destination(&path, identity(&journal.file)?)?;
            let inventory = journal.index.entries.keys().copied().collect::<Vec<_>>();
            let keys = indexed_document::replacement_keys(
                &inventory,
                &mut |key| {
                    journal
                        .read(key)?
                        .ok_or("Missing reopened indexed record".into())
                },
                cancel,
            )?;
            Self {
                journal,
                keys,
                path: path.clone(),
                last_stats: StreamStats::default(),
            }
        } else {
            let lease = acquire_lease(&path, true)?;
            // Recheck after acquiring the stable pathname lease. Never clobber a
            // destination that appeared while preparing a first-save staging file.
            if path.exists() {
                return Err("Indexed destination appeared during creation".into());
            }
            let temporary =
                tempfile::NamedTempFile::new_in(path.parent().ok_or("Missing indexed parent")?)
                    .map_err(|e| e.to_string())?;
            let (file, temporary_path) = temporary.into_parts();
            // TempPath must be dropped after the data handle, including unwinding.
            let mut journal = Journal::initialize(file)?;
            journal.failed_stage = Some(temporary_path);
            journal.path_lease = lease;
            Self {
                journal,
                keys: Keys::default(),
                path: path.clone(),
                last_stats: StreamStats::default(),
            }
        };
        let (keys, stats) =
            session
                .journal
                .replace_document(doc, &session.keys, cancel, &mut checkpoint)?;
        if session.journal.failed_stage.is_some() {
            // The committed staging file is still private: cancellation here can
            // discard it. Once persisted, completion is reported even if a late
            // cancellation request arrives; publication cannot be undone.
            canceled(cancel)?;
            session.journal.validate_document(cancel)?;
            canceled(cancel)?;
            let temporary_path = session.journal.failed_stage.take().unwrap();
            if let Err(error) = temporary_path.persist_noclobber(&path) {
                session.journal.failed_stage = Some(error.path);
                return Err(error.error.to_string());
            }
        }
        session.keys = keys;
        session.last_stats = stats;
        Ok(session)
    }
}
pub(crate) fn load_document(path: &Path, cancel: &impl Fn() -> bool) -> Result<Document> {
    canceled(cancel)?;
    let mut journal = Journal::open(path, false)?;
    let inventory = journal.index.entries.keys().copied().collect::<Vec<_>>();
    indexed_document::decode(
        &inventory,
        &mut |key| journal.read(key)?.ok_or("Missing indexed record".into()),
        cancel,
    )
    .map(|(doc, _)| doc)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn new_save_cancellation_and_failure_publish_no_partial_destination_or_temp_file() {
        for phase in [
            Phase::PayloadWritten,
            Phase::PayloadSynced,
            Phase::RootWritten,
            Phase::RootSynced,
        ] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("new.didx");
            let doc = indexed_document::fixture();
            assert!(
                Session::save_with_hook(None, &path, &doc, &|| false, |at| if at == phase {
                    Err("injected".into())
                } else {
                    Ok(())
                })
                .is_err()
            );
            assert!(!path.exists());
            assert_eq!(
                std::fs::read_dir(dir.path()).unwrap().count(),
                1,
                "only persistent lease remains"
            );
        }
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cancel.didx");
        let cancel = std::cell::Cell::new(false);
        assert!(
            Session::save_with_hook(
                None,
                &path,
                &indexed_document::fixture(),
                &|| cancel.get(),
                |phase| {
                    if phase == Phase::RootSynced {
                        cancel.set(true);
                    }
                    Ok(())
                }
            )
            .is_err()
        );
        assert!(!path.exists());
    }
    #[test]
    fn save_as_reopen_and_foreign_files_preserve_artwork_and_monotonic_keys() {
        let dir = tempfile::tempdir().unwrap();
        let first = dir.path().join("first.didx");
        let second = dir.path().join("second.didx");
        let mut doc = indexed_document::fixture();
        let mut session = Session::save(None, &first, &doc, &|| false).unwrap();
        session.journal.file.seek(SeekFrom::Start(0)).unwrap();
        let mut original = Vec::new();
        session.journal.file.read_to_end(&mut original).unwrap();
        doc.layers[0]
            .pixels
            .writer()
            .rgba_mut(0)
            .copy_from_slice(&[15, 27, 80, 99]);
        let mut session = Session::save(Some(session), &second, &doc, &|| false).unwrap();
        assert_eq!(std::fs::read(&first).unwrap(), original);
        assert!(
            load_document(&second, &|| false).is_err(),
            "live writer retains exclusive locks"
        );
        let previous_manifest: serde_json::Value =
            serde_json::from_slice(&session.journal.read(0).unwrap().unwrap()).unwrap();
        let previous_next = previous_manifest["next_layer_key"].as_u64().unwrap();
        drop(session);
        assert!(
            indexed_document::equivalent(&doc, &load_document(&second, &|| false).unwrap())
                .unwrap()
        );
        let session = Session::save(None, &second, &doc, &|| false).unwrap();
        assert_eq!(session.last_stats.raster_records_reused_without_read, 0);
        assert_eq!(session.journal.index.generation, 2);
        let mut session = session;
        let manifest: serde_json::Value =
            serde_json::from_slice(&session.journal.read(0).unwrap().unwrap()).unwrap();
        assert!(
            manifest["layer_keys"]
                .as_array()
                .unwrap()
                .iter()
                .all(|key| key.as_u64().unwrap() >= previous_next)
        );
        drop(session);
        for bytes in [
            b"CSFCHUNK artwork".as_slice(),
            b"DUDL old project".as_slice(),
            b"".as_slice(),
        ] {
            let foreign = dir.path().join("foreign.didx");
            std::fs::write(&foreign, bytes).unwrap();
            assert!(Session::save(None, &foreign, &doc, &|| false).is_err());
            assert_eq!(std::fs::read(&foreign).unwrap(), bytes);
        }
    }
}
