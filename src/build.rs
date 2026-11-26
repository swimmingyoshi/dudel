//! Gives the Windows program its icon and names the commit a development build was made from, for About. Packaged
//! builds must not depend on it (see `development::origin`).
use std::process::Command;

fn git(arguments: &[&str]) -> Option<String> {
    let output = Command::new("git").args(arguments).output().ok()?;
    let text = String::from_utf8(output.stdout).ok()?;
    (output.status.success() && !text.trim().is_empty()).then(|| text.trim().to_owned())
}

/// A compiled resource file holding every size of an `.ico`, so that no
/// resource compiler is needed. Explorer shows the first icon group.
fn icon_resource(ico: &[u8]) -> Vec<u8> {
    fn resource(out: &mut Vec<u8>, kind: u16, id: u16, data: &[u8]) {
        out.extend((data.len() as u32).to_le_bytes());
        out.extend(32u32.to_le_bytes());
        // Type, name, data version, memory flags and language (US English).
        for half in [0xffff, kind, 0xffff, id, 0, 0, 0, 0x0409] {
            out.extend(half.to_le_bytes());
        }
        out.extend([0; 8]);
        out.extend(data);
        out.resize(out.len().next_multiple_of(4), 0);
    }
    const ICON: u16 = 3;
    const ICON_GROUP: u16 = 14;
    let number = |at: usize| u32::from_le_bytes(ico[at..at + 4].try_into().unwrap()) as usize;
    let count = u16::from_le_bytes([ico[4], ico[5]]);
    let mut out = Vec::new();
    // An empty entry marks the file as 32-bit resources.
    resource(&mut out, 0, 0, &[]);
    let mut group = ico[..6].to_vec();
    for id in 1..=count {
        let entry = 6 + 16 * (usize::from(id) - 1);
        let (length, start) = (number(entry + 8), number(entry + 12));
        resource(&mut out, ICON, id, &ico[start..start + length]);
        group.extend(&ico[entry..entry + 12]);
        group.extend(id.to_le_bytes());
    }
    resource(&mut out, ICON_GROUP, 1, &group);
    out
}

fn embed_icon() {
    println!("cargo::rerun-if-changed=src/app_icon/dudel.ico");
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() != Ok("msvc") {
        return;
    }
    let ico = std::fs::read("src/app_icon/dudel.ico").expect("the application icon");
    let out = std::env::var_os("OUT_DIR").expect("OUT_DIR");
    let path = std::path::Path::new(&out).join("icon.res");
    std::fs::write(&path, icon_resource(&ico)).expect("write the icon resource");
    println!("cargo::rustc-link-arg-bins={}", path.display());
}

fn main() {
    println!("cargo::rerun-if-changed=src/build.rs");
    embed_icon();
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
