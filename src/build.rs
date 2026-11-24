//! Names the commit a development build was made from, for About. Packaged
//! builds must not depend on it (see `development::origin`).
use std::process::Command;

fn git(arguments: &[&str]) -> Option<String> {
    let output = Command::new("git").args(arguments).output().ok()?;
    let text = String::from_utf8(output.stdout).ok()?;
    (output.status.success() && !text.trim().is_empty()).then(|| text.trim().to_owned())
}

fn main() {
    println!("cargo::rerun-if-changed=src/build.rs");
    println!("cargo::rerun-if-env-changed=DUDEL_PACKAGED");
    if std::env::var_os("DUDEL_PACKAGED").is_some() {
        return;
    }
    if let Some(log) = git(&["rev-parse", "--git-path", "logs/HEAD"])
        && std::path::Path::new(&log).is_file()
    {
        println!("cargo::rerun-if-changed={log}");
    }
    if let Some(revision) = git(&["rev-parse", "--short", "HEAD"]) {
        println!("cargo::rustc-env=DUDEL_REVISION={revision}");
    }
}
