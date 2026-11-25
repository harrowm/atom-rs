//! Fetching the Atom Software Archive, so `atom-rs --archive` just works.
//!
//! The archive is published as a zip on GitHub
//! (<https://github.com/hoglet67/AtomSoftwareArchive>) laid out as the files
//! of an AtoMMC SD card. [`prepare`] keeps a copy in a cache folder:
//!
//! 1. ask GitHub for the newest release;
//! 2. if we already have that one, use it;
//! 3. otherwise download the zip, unpack it beside the old copy, and swap it in.
//!
//! If GitHub cannot be reached (no network, say) an existing copy is used
//! anyway, so it keeps working offline after the first run.

use std::fs;
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// GitHub's list of releases, newest first. (Its "latest" shortcut is no use:
/// it skips pre-releases, and the maintainers mark every recent release as one.)
const RELEASES_URL: &str =
    "https://api.github.com/repos/hoglet67/AtomSoftwareArchive/releases?per_page=5";
/// The biggest download we will accept (the archive is about 17 MB).
const MAX_DOWNLOAD_BYTES: u64 = 500 * 1024 * 1024;
/// Records which release the unpacked files came from.
const VERSION_FILE: &str = "version.txt";
/// The unpacked archive: this folder is the SD card.
const FILES_FOLDER: &str = "files";

/// A published release of the archive.
#[derive(Debug, PartialEq, Eq)]
pub struct Release {
    /// For example `V13_01`.
    pub tag: String,
    pub download_url: String,
}

/// Where the archive is kept: `~/.cache/atom-rs/archive`.
pub fn cache_folder() -> Result<PathBuf, String> {
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .ok_or("cannot find your home folder to keep the archive in")?;
    Ok(PathBuf::from(home)
        .join(".cache")
        .join("atom-rs")
        .join("archive"))
}

/// Make sure the latest archive is unpacked in `cache` and return the folder
/// to use as the SD card. `say` is told what is happening, for the user.
pub fn prepare(cache: &Path, say: &mut impl FnMut(&str)) -> Result<PathBuf, String> {
    let card = cache.join(FILES_FOLDER);
    let have = installed_version(cache).filter(|_| card.join("MENU").exists());

    say("Checking for the latest Atom Software Archive...");
    let latest = match latest_release() {
        Ok(release) => release,
        Err(problem) => {
            return match have {
                Some(tag) => {
                    say(&format!(
                        "Could not check for updates ({problem}); using {tag}."
                    ));
                    Ok(card)
                }
                None => Err(format!("cannot download the archive: {problem}")),
            };
        }
    };

    if have.as_deref() == Some(latest.tag.as_str()) {
        say(&format!("Archive {} is up to date.", latest.tag));
        return Ok(card);
    }

    match &have {
        Some(old) => say(&format!(
            "A newer archive is available ({old} -> {}).",
            latest.tag
        )),
        None => say(&format!("Downloading archive {}...", latest.tag)),
    }
    let zip = download(&latest.download_url)?;
    say(&format!("Unpacking {} MB...", zip.len() / 1_000_000));
    install(cache, &latest.tag, &zip)?;
    say("Archive ready.");
    Ok(card)
}

fn installed_version(cache: &Path) -> Option<String> {
    let text = fs::read_to_string(cache.join(VERSION_FILE)).ok()?;
    Some(text.trim().to_string()).filter(|tag| !tag.is_empty())
}

// ---- Talking to GitHub --------------------------------------------------------

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(120)))
        .user_agent("atom-rs") // GitHub's API insists on one
        .build()
        .into()
}

fn latest_release() -> Result<Release, String> {
    let mut response = agent()
        .get(RELEASES_URL)
        .header("Accept", "application/vnd.github+json")
        .call()
        .map_err(|e| e.to_string())?;
    let json = response
        .body_mut()
        .read_to_string()
        .map_err(|e| e.to_string())?;
    parse_release(&json)
}

fn download(url: &str) -> Result<Vec<u8>, String> {
    agent()
        .get(url)
        .call()
        .map_err(|e| format!("download failed: {e}"))?
        .body_mut()
        .with_config()
        .limit(MAX_DOWNLOAD_BYTES)
        .read_to_vec()
        .map_err(|e| format!("download failed: {e}"))
}

/// Read GitHub's list of releases: the tag of the newest, and the zip to
/// download. A release has the plain archive and some variants for other
/// hardware (`..._ECONET.zip`, `..._SDDOS2.zip`...); the plain one has the
/// shortest name.
fn parse_release(json: &str) -> Result<Release, String> {
    let reply: serde_json::Value =
        serde_json::from_str(json).map_err(|e| format!("unexpected reply from GitHub: {e}"))?;
    let newest = reply
        .as_array()
        .and_then(|releases| releases.iter().find(|r| r["draft"] != true))
        .ok_or("GitHub lists no releases")?;
    let tag = newest["tag_name"]
        .as_str()
        .ok_or("the release has no tag")?
        .to_string();
    let download_url = newest["assets"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|asset| asset["name"].as_str().is_some_and(|n| n.ends_with(".zip")))
        .min_by_key(|asset| asset["name"].as_str().map_or(0, str::len))
        .and_then(|asset| asset["browser_download_url"].as_str())
        .ok_or("the release has no zip file")?
        .to_string();
    Ok(Release { tag, download_url })
}

// ---- Unpacking ------------------------------------------------------------------

/// Unpack `zip` as the new contents of `cache/files` and record `tag`.
///
/// It is unpacked beside the old copy first, so a failure part-way (a full
/// disk, a corrupt download) leaves the old copy working.
fn install(cache: &Path, tag: &str, zip: &[u8]) -> Result<(), String> {
    let new = cache.join("files.new");
    let current = cache.join(FILES_FOLDER);
    let io = |e: std::io::Error| e.to_string();

    let _ = fs::remove_dir_all(&new);
    fs::create_dir_all(&new).map_err(io)?;
    if let Err(problem) = unzip(zip, &new) {
        let _ = fs::remove_dir_all(&new);
        return Err(problem);
    }
    let _ = fs::remove_dir_all(&current);
    fs::rename(&new, &current).map_err(io)?;
    fs::write(cache.join(VERSION_FILE), tag).map_err(io)
}

/// Unpack every file in `zip` below `into`. Names that would land outside it
/// (like `../../x`) are skipped.
fn unzip(zip: &[u8], into: &Path) -> Result<(), String> {
    let mut archive = zip::ZipArchive::new(Cursor::new(zip))
        .map_err(|e| format!("the download is not a valid zip: {e}"))?;
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).map_err(|e| e.to_string())?;
        let Some(relative) = entry.enclosed_name() else {
            continue;
        };
        let target = into.join(relative);
        if entry.is_dir() {
            fs::create_dir_all(&target).map_err(|e| e.to_string())?;
            continue;
        }
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let mut contents = Vec::new();
        entry
            .read_to_end(&mut contents)
            .map_err(|e| e.to_string())?;
        fs::write(&target, contents).map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// A scratch folder that is removed afterwards.
    struct TempFolder(PathBuf);

    impl TempFolder {
        fn new() -> TempFolder {
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let path = std::env::temp_dir().join(format!(
                "atom-rs-archive-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).unwrap();
            TempFolder(path)
        }
    }

    impl Drop for TempFolder {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    /// A zip holding the given (name, contents) files.
    fn make_zip(files: &[(&str, &str)]) -> Vec<u8> {
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default();
        for (name, contents) in files {
            writer.start_file(*name, options).unwrap();
            writer.write_all(contents.as_bytes()).unwrap();
        }
        writer.finish().unwrap().into_inner()
    }

    #[test]
    fn takes_the_newest_release_and_its_plain_zip() {
        let json = r#"[
            {"tag_name": "V14", "draft": false, "assets": [
                {"name": "Archive_V14_ECONET.zip", "browser_download_url": "https://example/econet.zip"},
                {"name": "notes.txt", "browser_download_url": "https://example/notes.txt"},
                {"name": "Archive_V14.zip", "browser_download_url": "https://example/plain.zip"},
                {"name": "Archive_V14_SDDOS2.zip", "browser_download_url": "https://example/sddos.zip"}
            ]},
            {"tag_name": "V13", "draft": false, "assets": [
                {"name": "Archive_V13.zip", "browser_download_url": "https://example/old.zip"}
            ]}
        ]"#;
        assert_eq!(
            parse_release(json).unwrap(),
            Release {
                tag: "V14".into(),
                download_url: "https://example/plain.zip".into()
            }
        );
    }

    #[test]
    fn drafts_are_skipped() {
        let json = r#"[
            {"tag_name": "V15", "draft": true, "assets": []},
            {"tag_name": "V14", "assets": [{"name": "A.zip", "browser_download_url": "https://example/a.zip"}]}
        ]"#;
        assert_eq!(parse_release(json).unwrap().tag, "V14");
    }

    #[test]
    fn a_release_without_a_zip_is_an_error() {
        assert!(parse_release(r#"[{"tag_name": "V1", "assets": []}]"#).is_err());
        assert!(parse_release(r#"[{"assets": []}]"#).is_err());
        assert!(parse_release("[]").is_err());
        assert!(parse_release("not json").is_err());
    }

    #[test]
    fn install_unpacks_the_files_and_records_the_version() {
        let cache = TempFolder::new();
        let zip = make_zip(&[("MENU", "menu"), ("ASA/GAME", "game")]);
        install(&cache.0, "V1", &zip).unwrap();
        assert_eq!(
            fs::read_to_string(cache.0.join("files/MENU")).unwrap(),
            "menu"
        );
        assert_eq!(
            fs::read_to_string(cache.0.join("files/ASA/GAME")).unwrap(),
            "game"
        );
        assert_eq!(installed_version(&cache.0).as_deref(), Some("V1"));
    }

    #[test]
    fn a_newer_install_replaces_the_old_files() {
        let cache = TempFolder::new();
        install(&cache.0, "V1", &make_zip(&[("MENU", "old"), ("GONE", "x")])).unwrap();
        install(&cache.0, "V2", &make_zip(&[("MENU", "new")])).unwrap();
        assert_eq!(
            fs::read_to_string(cache.0.join("files/MENU")).unwrap(),
            "new"
        );
        assert!(
            !cache.0.join("files/GONE").exists(),
            "files that left the archive go too"
        );
        assert_eq!(installed_version(&cache.0).as_deref(), Some("V2"));
    }

    #[test]
    fn a_bad_download_leaves_the_old_copy_working() {
        let cache = TempFolder::new();
        install(&cache.0, "V1", &make_zip(&[("MENU", "old")])).unwrap();
        assert!(install(&cache.0, "V2", b"this is not a zip").is_err());
        assert_eq!(
            fs::read_to_string(cache.0.join("files/MENU")).unwrap(),
            "old"
        );
        assert_eq!(installed_version(&cache.0).as_deref(), Some("V1"));
        assert!(
            !cache.0.join("files.new").exists(),
            "no half-unpacked leftovers"
        );
    }

    #[test]
    fn names_that_escape_the_folder_are_not_unpacked() {
        let cache = TempFolder::new();
        let zip = make_zip(&[("../escaped", "bad"), ("MENU", "ok")]);
        install(&cache.0, "V1", &zip).unwrap();
        assert!(!cache.0.join("escaped").exists());
        assert!(cache.0.join("files/MENU").exists());
    }
}
