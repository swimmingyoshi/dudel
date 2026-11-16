//! Real child termination at atomic-publication boundaries, using disposable
//! native/recovery payloads. These tests do not certify power-loss durability.
use super::*;
use std::{
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

const MARKER: &[u8] = b"Dudel disposable storage interruption fixture v1";

struct RunningChild(Child);
impl Drop for RunningChild {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn fixture(masked: bool, revised: bool) -> Document {
    let mut doc = Document::new(65, 33);
    doc.layers[0].name = if revised { "After" } else { "Before" }.into();
    doc.layers[0].pixels.writer().rgba_mut(0).copy_from_slice(&[
        17,
        83,
        if revised { 91 } else { 201 },
        0,
    ]);
    doc.layers[0]
        .pixels
        .writer()
        .rgba_mut(65 * 33 - 1)
        .copy_from_slice(&[91, 28, 147, 191]);
    doc.add_layer();
    doc.layers[1].opacity = 0.4;
    doc.layers[1].clipped = true;
    if masked {
        doc.version = 2;
        doc.layers[0].mask = Some(crate::layer_mask::LayerMask {
            id: crate::document::LayerId::new(),
            enabled: true,
            linked: false,
            coverage: crate::layer_mask::Mask::from_coverage(
                65,
                33,
                &vec![if revised { 73 } else { 128 }; 65 * 33],
                None,
            )
            .unwrap(),
        });
    }
    doc.validate().unwrap();
    doc
}

fn bytes(doc: &Document, recovery: bool, directory: &Path) -> Vec<u8> {
    let path = directory.join("encode.bin");
    if recovery {
        save_recovery(
            &path,
            &Recovery {
                document: doc.clone(),
                original_path: Some(directory.join("original.dudel")),
            },
        )
        .unwrap();
    } else {
        save_project(&path, doc).unwrap();
    }
    std::fs::read(path).unwrap()
}

#[test]
fn publication_child() {
    let Ok(directory) = std::env::var("DUDEL_STORAGE_CHILD_ROOT") else {
        return;
    };
    let directory = PathBuf::from(directory);
    assert_eq!(
        std::fs::read(directory.join("fixture-marker")).unwrap(),
        MARKER
    );
    let phase = std::env::var("DUDEL_STORAGE_CHILD_PHASE").unwrap();
    let payload = std::fs::read(directory.join("next-payload")).unwrap();
    let pause = || {
        std::fs::write(directory.join("reached"), phase.as_bytes()).unwrap();
        // Parent terminates this specific child; a failed parent still cannot
        // leave it running indefinitely. No user app/process is targeted.
        std::thread::sleep(Duration::from_secs(60));
        panic!("Parent did not terminate interruption child");
    };
    atomic_stream_observed(
        &directory.join("destination"),
        |file| {
            let middle = payload.len() / 2;
            file.write_all(&payload[..middle])
                .map_err(|e| e.to_string())?;
            if phase == "partial" {
                pause();
            }
            file.write_all(&payload[middle..])
                .map_err(|e| e.to_string())
        },
        |stage| {
            if phase == format!("{stage:?}") {
                pause();
            }
            Ok(())
        },
    )
    .unwrap();
    panic!("Requested phase was not reached");
}

#[test]
fn killed_writers_preserve_complete_native_and_recovery_generations() {
    for masked in [false, true] {
        for recovery in [false, true] {
            for phase in ["partial", "Written", "Synced", "Published"] {
                let directory = tempfile::tempdir().unwrap();
                std::fs::write(directory.path().join("fixture-marker"), MARKER).unwrap();
                let original = fixture(masked, false);
                let replacement = fixture(masked, true);
                let old = bytes(&original, recovery, directory.path());
                let next = bytes(&replacement, recovery, directory.path());
                let destination = directory.path().join("destination");
                std::fs::write(&destination, &old).unwrap();
                std::fs::write(directory.path().join("next-payload"), &next).unwrap();
                let mut child = RunningChild(
                    Command::new(std::env::current_exe().unwrap())
                        .args([
                            "--exact",
                            "storage::interruption_tests::publication_child",
                            "--nocapture",
                        ])
                        .env("DUDEL_STORAGE_CHILD_ROOT", directory.path())
                        .env("DUDEL_STORAGE_CHILD_PHASE", phase)
                        .stdout(Stdio::null())
                        .stderr(Stdio::null())
                        .spawn()
                        .unwrap(),
                );
                let deadline = Instant::now() + Duration::from_secs(20);
                while std::fs::read(directory.path().join("reached"))
                    .ok()
                    .as_deref()
                    != Some(phase.as_bytes())
                {
                    assert!(
                        child.0.try_wait().unwrap().is_none(),
                        "Child exited before {phase}"
                    );
                    assert!(Instant::now() < deadline, "Child never reached {phase}");
                    std::thread::sleep(Duration::from_millis(5));
                }
                child.0.kill().unwrap();
                assert!(!child.0.wait().unwrap().success());
                let expected_bytes = if phase == "Published" { &next } else { &old };
                assert_eq!(
                    &std::fs::read(&destination).unwrap(),
                    expected_bytes,
                    "masked={masked} recovery={recovery} phase={phase}"
                );
                let actual = if recovery {
                    read_recovery(&destination).unwrap().document
                } else {
                    load_project(&destination).unwrap()
                };
                let expected = if phase == "Published" {
                    &replacement
                } else {
                    &original
                };
                assert_eq!(
                    serde_json::to_vec(&actual).unwrap(),
                    serde_json::to_vec(expected).unwrap()
                );
            }
        }
    }
}

#[test]
fn failed_writer_and_sync_boundary_do_not_replace_existing_artwork() {
    for phase in [None, Some(CommitStage::Written), Some(CommitStage::Synced)] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("original.dudel");
        save_project(&path, &fixture(true, false)).unwrap();
        let original = std::fs::read(&path).unwrap();
        let result = atomic_stream_observed(
            &path,
            |file| {
                file.write_all(b"incomplete output")
                    .map_err(|e| e.to_string())?;
                if phase.is_none() {
                    return Err("Simulated storage-full write failure".into());
                }
                Ok(())
            },
            |current| {
                if Some(current) == phase {
                    Err("Injected pre-publication failure".into())
                } else {
                    Ok(())
                }
            },
        );
        assert!(result.is_err());
        assert_eq!(std::fs::read(&path).unwrap(), original);
        assert_eq!(
            std::fs::read_dir(directory.path()).unwrap().count(),
            1,
            "Failed staging file leaked"
        );
        load_project(&path).unwrap();
    }
}

#[cfg(windows)]
#[test]
fn windows_read_only_destination_rejects_replacement_and_preserves_artwork() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("protected.dudel");
    save_project(&path, &fixture(true, false)).unwrap();
    let original = std::fs::read(&path).unwrap();
    let original_permissions = std::fs::metadata(&path).unwrap().permissions();
    let mut protected = original_permissions.clone();
    protected.set_readonly(true);
    std::fs::set_permissions(&path, protected).unwrap();
    let result = save_project(&path, &fixture(true, true));
    let after = std::fs::read(&path);
    // Restore before asserting, so even a regression leaves a removable fixture.
    std::fs::set_permissions(&path, original_permissions).unwrap();
    assert!(result.is_err());
    assert_eq!(after.unwrap(), original);
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
    load_project(&path).unwrap();
}

#[test]
fn native_cancellation_fences_preserve_old_bytes_and_late_publication_is_successful() {
    use std::sync::atomic::{AtomicBool, Ordering};
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("cancel.dudel");
    let old_doc = fixture(true, false);
    let next_doc = fixture(true, true);
    save_project(&path, &old_doc).unwrap();
    let old = std::fs::read(&path).unwrap();
    let cancel = AtomicBool::new(true);
    assert_eq!(
        save_project_cancelable(&path, &next_doc, &cancel),
        Err(SAVE_CANCELED.into())
    );
    assert_eq!(std::fs::read(&path).unwrap(), old);
    cancel.store(false, Ordering::Relaxed);
    // Simulate a request arriving while the noninterruptible file flush runs:
    // the same pre-publication fence must see it after sync_all returns.
    let result = atomic_stream_observed(
        &path,
        |file| crate::tile_project::encode_cancelable(&next_doc, file, Some(&cancel)),
        |stage| {
            if stage == CommitStage::Written {
                cancel.store(true, Ordering::Relaxed);
            }
            if stage == CommitStage::Synced {
                check_save_cancel(Some(&cancel))
            } else {
                Ok(())
            }
        },
    );
    assert_eq!(result, Err(SAVE_CANCELED.into()));
    assert_eq!(std::fs::read(&path).unwrap(), old);
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
    cancel.store(false, Ordering::Relaxed);
    atomic_stream_observed(
        &path,
        |file| crate::tile_project::encode_cancelable(&next_doc, file, Some(&cancel)),
        |stage| {
            if stage == CommitStage::Synced {
                check_save_cancel(Some(&cancel))?;
            }
            if stage == CommitStage::Published {
                cancel.store(true, Ordering::Relaxed);
            }
            Ok(())
        },
    )
    .unwrap();
    assert!(cancel.load(Ordering::Relaxed));
    assert_eq!(
        serde_json::to_vec(&load_project(&path).unwrap()).unwrap(),
        serde_json::to_vec(&next_doc).unwrap()
    );
}
