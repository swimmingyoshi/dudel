//! Looking for a newer Dudel and replacing this copy with it.
//!
//! Dudel connects to the internet only when the artist asks it to: nothing
//! here runs until they choose Help > Check for updates. The check reads the
//! public release list; installing downloads that release's package, checks
//! it against the checksum published with it, and swaps the files in place.
use std::io::{Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

const REPOSITORY: &str = env!("CARGO_PKG_REPOSITORY");
/// The end of this platform's package name, after `dudel-VERSION-`.
#[cfg(windows)]
const PACKAGE: &str = "windows-x64.zip";
#[cfg(not(windows))]
const PACKAGE: &str = "linux-x64.tar.gz";
const MAX_LISTING: u64 = 8 << 20;
const MAX_PACKAGE: u64 = 512 << 20;
const MAX_UNPACKED: u64 = 2 << 30;
const MAX_FILES: usize = 20_000;

pub(crate) type Version = [u32; 3];

/// `1.2.3` or `v1.2.3`; anything else is not a release of ours.
pub(crate) fn version(text: &str) -> Option<Version> {
    let mut parts = text.strip_prefix('v').unwrap_or(text).split('.');
    let mut number = || parts.next()?.parse::<u32>().ok();
    let version = [number()?, number()?, number()?];
    parts.next().is_none().then_some(version)
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Package {
    pub url: String,
    pub size: u64,
    /// The checksum GitHub computed for the file.
    pub digest: Option<[u8; 32]>,
    /// The `.sha256` file published beside the package.
    pub checksum_url: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Release {
    pub version: String,
    pub page: String,
    pub notes: String,
    /// The package for this platform, when the release has one.
    pub package: Option<Package>,
}

#[derive(serde::Deserialize)]
struct Listed {
    tag_name: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    html_url: String,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    assets: Vec<Asset>,
}

#[derive(serde::Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
    #[serde(default)]
    size: u64,
    #[serde(default)]
    digest: Option<String>,
}

fn listing_url() -> String {
    format!(
        "{}/releases?per_page=30",
        REPOSITORY.replacen("https://github.com/", "https://api.github.com/repos/", 1)
    )
}

/// The newest release after `current` in GitHub's release list, if any.
/// Pre-releases count: every Dudel release so far is one.
pub(crate) fn newest(listing: &str, current: &str) -> Result<Option<Release>, String> {
    let current = version(current).ok_or("This copy has no release version")?;
    let listed: Vec<Listed> = serde_json::from_str(listing)
        .map_err(|e| format!("The release list could not be read: {e}"))?;
    let downloads = format!("{REPOSITORY}/releases/download/");
    let Some((found, release)) = listed
        .into_iter()
        .filter(|release| !release.draft)
        .filter_map(|release| Some((version(&release.tag_name)?, release)))
        .filter(|(found, _)| *found > current)
        .max_by_key(|(found, _)| *found)
    else {
        return Ok(None);
    };
    let number = format!("{}.{}.{}", found[0], found[1], found[2]);
    let name = format!("dudel-{number}-{PACKAGE}");
    // Only files of this repository's own releases are ever downloaded.
    let ours = |asset: &&Asset| asset.browser_download_url.starts_with(&downloads);
    let checksum_url = release
        .assets
        .iter()
        .filter(ours)
        .find(|asset| asset.name == format!("{name}.sha256"))
        .map(|asset| asset.browser_download_url.clone());
    let package = release
        .assets
        .iter()
        .filter(ours)
        .find(|asset| asset.name == name)
        .map(|asset| Package {
            url: asset.browser_download_url.clone(),
            size: asset.size,
            digest: asset
                .digest
                .as_deref()
                .and_then(|digest| digest.strip_prefix("sha256:"))
                .and_then(crate::sha256::parse_hex),
            checksum_url,
        });
    Ok(Some(Release {
        page: if release.html_url.starts_with(REPOSITORY) {
            release.html_url
        } else {
            crate::welcome::RELEASES.to_owned()
        },
        notes: release.body.unwrap_or_default(),
        version: number,
        package,
    }))
}

/// Ask GitHub for a release newer than this copy.
pub(crate) fn check() -> Result<Option<Release>, String> {
    let mut listing = Vec::new();
    fetch(&listing_url(), MAX_LISTING, |bytes| {
        listing.extend_from_slice(bytes);
        Ok(())
    })?;
    newest(
        &String::from_utf8_lossy(&listing),
        env!("CARGO_PKG_VERSION"),
    )
}

/// Whether this copy can replace itself: a packaged Windows copy in a
/// folder it may write to. Other copies are sent to the release page.
pub(crate) fn can_install() -> bool {
    cfg!(windows) && crate::install::packaged_folder().is_some()
}

/// Download `release`, check it and replace the files in `folder` with it.
/// `received` counts the downloaded bytes for the progress bar.
pub(crate) fn download_and_install(
    release: &Release,
    folder: &Path,
    received: &AtomicU64,
) -> Result<(), String> {
    let package = release
        .package
        .as_ref()
        .ok_or("This release has no package for this system")?;
    let downloads = crate::locations::app_data()
        .ok_or("Dudel's data folder is unavailable")?
        .join("updates");
    std::fs::create_dir_all(&downloads)
        .map_err(|e| format!("Could not create {}: {e}", downloads.display()))?;
    let archive = downloads.join("update.zip");
    let result = (|| {
        let digest = download(package, &archive, received)?;
        let mut expected: Vec<[u8; 32]> = package.digest.into_iter().collect();
        if let Some(url) = &package.checksum_url {
            let mut text = Vec::new();
            fetch(url, 4096, |bytes| {
                text.extend_from_slice(bytes);
                Ok(())
            })?;
            let text = String::from_utf8_lossy(&text);
            let listed = text.split_whitespace().next().unwrap_or_default();
            expected.push(
                crate::sha256::parse_hex(listed)
                    .ok_or("The published checksum could not be read")?,
            );
        }
        if expected.is_empty() {
            return Err("This release has no checksum to check the download against".to_owned());
        }
        if expected.iter().any(|expected| *expected != digest) {
            return Err(
                "The download does not match its published checksum and was not installed"
                    .to_owned(),
            );
        }
        unpack(&archive, &release.version, folder)
    })();
    let _ = std::fs::remove_file(&archive);
    result
}

fn download(package: &Package, archive: &Path, received: &AtomicU64) -> Result<[u8; 32], String> {
    let failed = |e: std::io::Error| format!("Could not save the download: {e}");
    let mut file = std::io::BufWriter::new(std::fs::File::create(archive).map_err(failed)?);
    let mut hasher = crate::sha256::Sha256::new();
    received.store(0, Ordering::Relaxed);
    fetch(&package.url, MAX_PACKAGE, |bytes| {
        hasher.update(bytes);
        received.fetch_add(bytes.len() as u64, Ordering::Relaxed);
        file.write_all(bytes).map_err(failed)
    })?;
    file.flush().map_err(failed)?;
    let size = received.load(Ordering::Relaxed);
    if package.size != 0 && size != package.size {
        return Err(format!(
            "The download is incomplete ({size} of {} bytes)",
            package.size
        ));
    }
    Ok(hasher.finish())
}

/// Replace the package in `folder` with the one in `archive`, which must
/// be Dudel `version` in the layout the packaging script writes: one top
/// folder holding the program, its file list and the rest.
pub(crate) fn unpack(archive: &Path, version: &str, folder: &Path) -> Result<(), String> {
    use crate::install::{FILE_LIST, fresh_staging, listed_files, place, safe_relative};
    let damaged = |e: &dyn std::fmt::Display| format!("The update package is damaged: {e}");
    let file = std::fs::File::open(archive).map_err(|e| damaged(&e))?;
    let mut archive =
        zip::ZipArchive::new(std::io::BufReader::new(file)).map_err(|e| damaged(&e))?;
    if archive.len() > MAX_FILES {
        return Err(damaged(&"too many files"));
    }
    let previous = listed_files(folder).unwrap_or_default();
    let staging = fresh_staging(folder)?;
    let result = (|| {
        let mut files = Vec::new();
        let mut top: Option<String> = None;
        let mut total = 0u64;
        for index in 0..archive.len() {
            let mut entry = archive.by_index(index).map_err(|e| damaged(&e))?;
            if !entry.is_file() {
                continue;
            }
            let name = entry.name().map_err(|e| damaged(&e))?.into_owned();
            let (first, path) = name
                .split_once('/')
                .ok_or_else(|| damaged(&"unexpected layout"))?;
            if top.get_or_insert_with(|| first.to_owned()) != first || !safe_relative(path) {
                return Err(damaged(&format!("unexpected file {name}")));
            }
            total += entry.size();
            if total > MAX_UNPACKED {
                return Err(damaged(&"too large"));
            }
            let target = staging.join(path);
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent).map_err(|e| damaged(&e))?;
            }
            let mut output = std::fs::File::create(&target).map_err(|e| damaged(&e))?;
            // The declared size bounds the total; this bounds a lying entry.
            let copied = std::io::copy(&mut (&mut entry).take(MAX_UNPACKED), &mut output)
                .map_err(|e| damaged(&e))?;
            if copied != entry.size() {
                return Err(damaged(&format!("{name} has the wrong size")));
            }
            files.push(path.to_owned());
        }
        files.sort();
        // The package must be the release it was published as, and whole.
        #[derive(serde::Deserialize)]
        struct Build {
            version: String,
        }
        let build = std::fs::read_to_string(staging.join("BUILD.json"))
            .ok()
            .and_then(|text| {
                serde_json::from_str::<Build>(text.trim_start_matches('\u{feff}')).ok()
            })
            .ok_or_else(|| damaged(&"no build record"))?;
        if build.version != version {
            return Err(damaged(&format!(
                "it holds version {}, not {version}",
                build.version
            )));
        }
        let program = std::env::current_exe()
            .ok()
            .and_then(|exe| {
                exe.file_name()
                    .map(|name| name.to_string_lossy().into_owned())
            })
            .unwrap_or_default();
        if listed_files(&staging).as_ref() != Some(&files)
            || !files.contains(&program)
            || !files.iter().any(|file| file == FILE_LIST)
        {
            return Err(damaged(&"its file list does not match its files"));
        }
        Ok(files)
    })();
    match result {
        Ok(files) => place(folder, &files, &previous),
        Err(error) => {
            let _ = std::fs::remove_dir_all(&staging);
            Err(error)
        }
    }
}

/// HTTPS GET through Windows' own HTTP stack, which follows the system's
/// proxy settings and never downgrades a redirect to plain HTTP.
#[cfg(windows)]
fn fetch(
    url: &str,
    limit: u64,
    mut sink: impl FnMut(&[u8]) -> Result<(), String>,
) -> Result<(), String> {
    use std::ffi::c_void;
    use windows::{
        Win32::Networking::WinHttp::{
            WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, WINHTTP_FLAG_SECURE, WINHTTP_QUERY_FLAG_NUMBER,
            WINHTTP_QUERY_STATUS_CODE, WinHttpCloseHandle, WinHttpConnect, WinHttpOpen,
            WinHttpOpenRequest, WinHttpQueryHeaders, WinHttpReadData, WinHttpReceiveResponse,
            WinHttpSendRequest, WinHttpSetTimeouts,
        },
        core::{HSTRING, PCWSTR, w},
    };
    struct Handle(*mut c_void);
    impl Drop for Handle {
        fn drop(&mut self) {
            if !self.0.is_null() {
                // SAFETY: the handle came from WinHTTP and is closed once.
                let _ = unsafe { WinHttpCloseHandle(self.0) };
            }
        }
    }
    let rest = url
        .strip_prefix("https://")
        .ok_or("Only secure addresses are used")?;
    let (host, path) = rest.split_once('/').unwrap_or((rest, ""));
    let unreachable = |e: windows::core::Error| {
        format!(
            "Could not reach {host}: {}",
            e.message().trim_end_matches(['\r', '\n'])
        )
    };
    let opened = |handle: *mut c_void| {
        if handle.is_null() {
            Err(unreachable(windows::core::Error::from_win32()))
        } else {
            Ok(Handle(handle))
        }
    };
    // SAFETY: every pointer passed outlives its call; handles are checked
    // before use and closed by `Handle`, children before their parents.
    unsafe {
        let session = opened(WinHttpOpen(
            &HSTRING::from(concat!("Dudel/", env!("CARGO_PKG_VERSION"))),
            WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
            PCWSTR::null(),
            PCWSTR::null(),
            0,
        ))?;
        WinHttpSetTimeouts(session.0, 15_000, 15_000, 30_000, 30_000).map_err(unreachable)?;
        let connection = opened(WinHttpConnect(session.0, &HSTRING::from(host), 443, 0))?;
        let request = opened(WinHttpOpenRequest(
            connection.0,
            w!("GET"),
            &HSTRING::from(format!("/{path}")),
            PCWSTR::null(),
            PCWSTR::null(),
            std::ptr::null(),
            WINHTTP_FLAG_SECURE,
        ))?;
        WinHttpSendRequest(request.0, None, None, 0, 0, 0).map_err(unreachable)?;
        WinHttpReceiveResponse(request.0, std::ptr::null_mut()).map_err(unreachable)?;
        let mut status = 0u32;
        let mut size = size_of::<u32>() as u32;
        WinHttpQueryHeaders(
            request.0,
            WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            PCWSTR::null(),
            Some((&raw mut status).cast()),
            &mut size,
            std::ptr::null_mut(),
        )
        .map_err(unreachable)?;
        if status != 200 {
            return Err(answer(host, status));
        }
        let mut buffer = vec![0u8; 64 * 1024];
        let mut total = 0u64;
        loop {
            let mut read = 0u32;
            WinHttpReadData(
                request.0,
                buffer.as_mut_ptr().cast(),
                buffer.len() as u32,
                &mut read,
            )
            .map_err(unreachable)?;
            if read == 0 {
                return Ok(());
            }
            total += u64::from(read);
            if total > limit {
                return Err(format!("{host} sent more than expected"));
            }
            sink(&buffer[..read as usize])?;
        }
    }
}

/// HTTPS GET with the system's `curl`, which every common Linux desktop has.
#[cfg(not(windows))]
fn fetch(
    url: &str,
    limit: u64,
    mut sink: impl FnMut(&[u8]) -> Result<(), String>,
) -> Result<(), String> {
    let host = url
        .strip_prefix("https://")
        .ok_or("Only secure addresses are used")?
        .split('/')
        .next()
        .unwrap_or_default();
    let mut child = std::process::Command::new("curl")
        .args(["--fail", "--silent", "--show-error", "--location"])
        .args(["--proto", "=https", "--proto-redir", "=https"])
        .args(["--connect-timeout", "15", "--user-agent"])
        .arg(concat!("Dudel/", env!("CARGO_PKG_VERSION")))
        .arg(url)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| format!("Checking needs the curl program: {e}"))?;
    let mut output = child.stdout.take().ok_or("curl gave no output")?;
    let mut buffer = vec![0u8; 64 * 1024];
    let mut total = 0u64;
    let result = loop {
        match output.read(&mut buffer) {
            Ok(0) => break Ok(()),
            Ok(read) => {
                total += read as u64;
                if total > limit {
                    break Err(format!("{host} sent more than expected"));
                }
                if let Err(error) = sink(&buffer[..read]) {
                    break Err(error);
                }
            }
            Err(error) => break Err(format!("Could not reach {host}: {error}")),
        }
    };
    if result.is_err() {
        let _ = child.kill();
    }
    let status = child.wait().map_err(|e| e.to_string())?;
    result?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("Could not reach {host}"))
    }
}

#[cfg_attr(not(windows), allow(dead_code))]
fn answer(host: &str, status: u32) -> String {
    match status {
        403 | 429 => format!("{host} is not answering right now (too many requests). Try later."),
        404 => format!("{host} has no such release"),
        _ => format!("{host} answered with error {status}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tag, whether it is a draft, and its assets as (name, address).
    type Listing<'a> = (&'a str, bool, &'a [(&'a str, &'a str)]);

    fn listing(releases: &[Listing]) -> String {
        let releases: Vec<serde_json::Value> = releases
            .iter()
            .map(|(tag, draft, assets)| {
                serde_json::json!({
                    "tag_name": tag,
                    "draft": draft,
                    "prerelease": true,
                    "html_url": format!("{REPOSITORY}/releases/tag/{tag}"),
                    "body": format!("Notes for {tag}"),
                    "assets": assets.iter().map(|(name, url)| serde_json::json!({
                        "name": name,
                        "browser_download_url": url,
                        "size": 1234,
                        "digest": format!("sha256:{}", "ab".repeat(32)),
                    })).collect::<Vec<_>>(),
                })
            })
            .collect();
        serde_json::to_string(&releases).unwrap()
    }

    #[test]
    fn versions_compare_by_number_not_by_text() {
        assert_eq!(version("v0.4.0"), Some([0, 4, 0]));
        assert_eq!(version("1.10.2"), Some([1, 10, 2]));
        assert!(version("0.10.0") > version("0.9.9"));
        for other in ["", "v1", "1.2", "1.2.3.4", "1.2.x", "nightly", "v1.2.3-rc1"] {
            assert_eq!(version(other), None, "{other}");
        }
    }

    #[test]
    fn the_newest_published_release_is_offered_with_this_systems_package() {
        let name = format!("dudel-0.10.0-{PACKAGE}");
        let url = format!("{REPOSITORY}/releases/download/v0.10.0/{name}");
        let sidecar = format!("{url}.sha256");
        let listing = listing(&[
            ("v0.9.0", false, &[]),
            (
                "v0.10.0",
                false,
                &[
                    ("dudel-0.10.0-other-system.zip", "https://example.com/x"),
                    (&name, &url),
                    (&format!("{name}.sha256"), &sidecar),
                ],
            ),
            // Not published yet, and not a release of ours.
            ("v0.11.0", true, &[]),
            ("experiment", false, &[]),
        ]);
        let release = newest(&listing, "0.4.0").unwrap().unwrap();
        assert_eq!(release.version, "0.10.0");
        assert_eq!(release.notes, "Notes for v0.10.0");
        assert!(release.page.ends_with("/releases/tag/v0.10.0"));
        let package = release.package.unwrap();
        assert_eq!(package.url, url);
        assert_eq!(package.size, 1234);
        assert_eq!(package.digest, Some([0xab; 32]));
        assert_eq!(package.checksum_url, Some(sidecar));
        // Nothing newer: no offer.
        assert_eq!(newest(&listing, "0.10.0").unwrap(), None);
        assert_eq!(newest(&listing, "1.0.0").unwrap(), None);
        assert!(newest("not json", "0.4.0").is_err());
    }

    #[test]
    fn a_package_hosted_anywhere_else_is_never_offered() {
        let name = format!("dudel-0.5.0-{PACKAGE}");
        let listing = listing(&[(
            "v0.5.0",
            false,
            &[(
                &name,
                "https://example.com/releases/download/v0.5.0/dudel.zip",
            )],
        )]);
        let release = newest(&listing, "0.4.0").unwrap().unwrap();
        assert_eq!(release.version, "0.5.0");
        assert_eq!(release.package, None);
    }

    fn package(path: &Path, version: &str, files: &[(&str, &str)], list: &[&str]) {
        use zip::write::SimpleFileOptions;
        let mut writer = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
        let top = format!("dudel-{version}-{}", PACKAGE.trim_end_matches(".zip"));
        let build = format!("{{\"version\":\"{version}\"}}");
        let list = serde_json::json!({ "files": list }).to_string();
        let own = [
            ("BUILD.json", build.as_str()),
            ("FILES.json", list.as_str()),
        ];
        for (name, text) in files.iter().chain(&own) {
            let name = if name.starts_with("..") {
                (*name).to_owned()
            } else {
                format!("{top}/{name}")
            };
            writer
                .start_file(name, SimpleFileOptions::default())
                .unwrap();
            writer.write_all(text.as_bytes()).unwrap();
        }
        writer.finish().unwrap();
    }

    fn program() -> String {
        std::env::current_exe()
            .unwrap()
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned()
    }

    #[test]
    fn a_package_replaces_the_installed_files_and_keeps_the_artists_own() {
        let root = tempfile::tempdir().unwrap();
        let folder = root.path().join("Dudel");
        std::fs::create_dir_all(folder.join("docs")).unwrap();
        let program = program();
        std::fs::write(folder.join(&program), "old").unwrap();
        std::fs::write(folder.join("docs/gone.md"), "old").unwrap();
        std::fs::write(folder.join("mine.txt"), "mine").unwrap();
        let old = serde_json::json!({ "files": [program, "docs/gone.md", "FILES.json"] });
        std::fs::write(folder.join("FILES.json"), old.to_string()).unwrap();
        let archive = root.path().join("update.zip");
        let files = [(program.as_str(), "new"), ("docs/guide.md", "guide")];
        let list = [
            program.as_str(),
            "docs/guide.md",
            "BUILD.json",
            "FILES.json",
        ];
        package(&archive, "9.9.9", &files, &list);
        unpack(&archive, "9.9.9", &folder).unwrap();
        let read = |file: &str| std::fs::read_to_string(folder.join(file)).ok();
        assert_eq!(read(&program).as_deref(), Some("new"));
        assert_eq!(read("docs/guide.md").as_deref(), Some("guide"));
        assert_eq!(read("docs/gone.md"), None);
        assert_eq!(read("mine.txt").as_deref(), Some("mine"));
        assert!(read("FILES.json").unwrap().contains("docs/guide.md"));
    }

    #[test]
    fn a_wrong_or_tampered_package_changes_nothing() {
        let root = tempfile::tempdir().unwrap();
        let folder = root.path().join("Dudel");
        std::fs::create_dir_all(&folder).unwrap();
        let program = program();
        std::fs::write(folder.join(&program), "old").unwrap();
        let archive = root.path().join("update.zip");
        let good = [program.as_str(), "BUILD.json", "FILES.json"];
        let attempt = |version: &str, files: &[(&str, &str)], list: &[&str]| {
            let _ = std::fs::remove_file(&archive);
            package(&archive, version, files, list);
            let error = unpack(&archive, "9.9.9", &folder).unwrap_err();
            assert_eq!(
                std::fs::read_to_string(folder.join(&program)).unwrap(),
                "old"
            );
            let left: Vec<_> = std::fs::read_dir(&folder).unwrap().flatten().collect();
            assert_eq!(left.len(), 1, "{left:?}");
            assert!(!root.path().join("escaped.txt").exists());
            error
        };
        // Another version than the release named.
        let error = attempt("9.9.8", &[(&program, "new")], &good);
        assert!(error.contains("9.9.8"), "{error}");
        // A file the list does not name, and a listed file that is missing.
        attempt("9.9.9", &[(&program, "new"), ("extra.dll", "x")], &good);
        attempt("9.9.9", &[], &good);
        // A path that would leave the folder.
        attempt(
            "9.9.9",
            &[(&program, "new"), ("../escaped.txt", "x")],
            &good,
        );
        // Not an archive at all.
        std::fs::write(&archive, "not a zip").unwrap();
        assert!(unpack(&archive, "9.9.9", &folder).is_err());
    }

    /// Talks to GitHub; run by hand with `--ignored`.
    #[test]
    #[ignore]
    fn the_live_release_list_answers() {
        let mut listing = Vec::new();
        fetch(&listing_url(), MAX_LISTING, |bytes| {
            listing.extend_from_slice(bytes);
            Ok(())
        })
        .unwrap();
        let text = String::from_utf8_lossy(&listing);
        let release = newest(&text, "0.0.1")
            .unwrap()
            .expect("a published release");
        println!("newest: {} {:?}", release.version, release.package);
        assert!(fetch("http://example.com/", 10, |_| Ok(())).is_err());
        // The whole download path against a real package. 0.4.0 predates
        // the file list, so it is fetched and checked but never installed.
        let root = tempfile::tempdir().unwrap();
        let received = AtomicU64::new(0);
        let error = download_and_install(&release, root.path(), &received).unwrap_err();
        println!("{} bytes, then: {error}", received.load(Ordering::Relaxed));
        assert_eq!(
            received.load(Ordering::Relaxed),
            release.package.as_ref().unwrap().size
        );
        assert!(error.contains("file list"), "{error}");
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
        // A changed checksum stops the install before anything is unpacked.
        let mut tampered = release.clone();
        tampered.package.as_mut().unwrap().digest = Some([0; 32]);
        let error = download_and_install(&tampered, root.path(), &received).unwrap_err();
        assert!(error.contains("checksum"), "{error}");
    }
}
