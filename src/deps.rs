use crate::job::quiet;
use sha2::{Digest, Sha256};
use std::io::{Cursor, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

pub struct Deps {
    pub yt: PathBuf,
    pub bin: PathBuf,
    /// yt-dlp was already there, so it may be out of date; see `update`.
    pub may_be_old: bool,
}

const YT_BASE: &str = "https://github.com/yt-dlp/yt-dlp/releases/latest/download/";
const FF_ZIP: &str = "https://www.gyan.dev/ffmpeg/builds/ffmpeg-release-essentials.zip";

pub fn data_dir() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("Camallo")
}

fn fetch(url: &str) -> Result<Vec<u8>, String> {
    let mut r = ureq::get(url).call().map_err(|e| format!("{url}: {e}"))?;
    r.body_mut()
        .with_config()
        .limit(400 << 20)
        .read_to_vec()
        .map_err(|e| e.to_string())
}

pub fn sha256_hex(data: &[u8]) -> String {
    Sha256::digest(data)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Reads a hash out of a checksum file: either a bare hash or "hash  filename" lines.
pub fn expected_hash(sums: &str, name: &str) -> Option<String> {
    sums.lines().find_map(|l| {
        let t: Vec<&str> = l.split_whitespace().collect();
        match t.as_slice() {
            [h] => Some(h.to_lowercase()),
            [h, f] if f.trim_start_matches('*') == name => Some(h.to_lowercase()),
            _ => None,
        }
    })
}

fn verified(url: &str, sums_url: &str, name: &str) -> Result<Vec<u8>, String> {
    let data = fetch(url)?;
    let sums = String::from_utf8_lossy(&fetch(sums_url)?).into_owned();
    let want = expected_hash(&sums, name).ok_or("checksum not found")?;
    if sha256_hex(&data) != want {
        return Err(format!("checksum mismatch for {name}, file discarded"));
    }
    Ok(data)
}

/// Writes `path` through a `.part` file and a rename, so a closed app or a power cut never leaves
/// half a file under the real name (the next start would trust it, because it exists).
fn write_atomic(
    path: &Path,
    fill: impl FnOnce(&mut std::fs::File) -> std::io::Result<()>,
) -> Result<(), String> {
    let mut part = path.as_os_str().to_owned();
    part.push(".part");
    let part = PathBuf::from(part);
    let r: std::io::Result<()> = (|| {
        let mut f = std::fs::File::create(&part)?;
        fill(&mut f)?;
        f.sync_all()?;
        drop(f); // closed before the rename, which Windows requires
        std::fs::rename(&part, path)
    })();
    if let Err(e) = r {
        let _ = std::fs::remove_file(&part);
        return Err(format!("{}: {e}", path.display()));
    }
    Ok(())
}

/// First run: download yt-dlp and ffmpeg (checksum-verified).
pub fn ensure(status: &mut dyn FnMut(&str)) -> Result<Deps, String> {
    let bin = data_dir().join("bin");
    std::fs::create_dir_all(&bin).map_err(|e| e.to_string())?;
    let yt = bin.join("yt-dlp.exe");

    let may_be_old = yt.exists();
    if !may_be_old {
        status("Downloading yt-dlp...");
        let data = verified(
            &format!("{YT_BASE}yt-dlp.exe"),
            &format!("{YT_BASE}SHA2-256SUMS"),
            "yt-dlp.exe",
        )?;
        write_atomic(&yt, |f| f.write_all(&data))?;
    }

    if !bin.join("ffmpeg.exe").exists() {
        status("Downloading ffmpeg (about 30 MB)...");
        let zip = verified(
            FF_ZIP,
            &format!("{FF_ZIP}.sha256"),
            "ffmpeg-release-essentials.zip",
        )?;
        let mut ar = zip::ZipArchive::new(Cursor::new(zip)).map_err(|e| e.to_string())?;
        for i in 0..ar.len() {
            let mut f = ar.by_index(i).map_err(|e| e.to_string())?;
            if f.name().is_ok_and(|n| n.ends_with("/bin/ffmpeg.exe")) {
                write_atomic(&bin.join("ffmpeg.exe"), |out| {
                    std::io::copy(&mut f, out).map(|_| ())
                })?;
                break;
            }
        }
        if !bin.join("ffmpeg.exe").exists() {
            return Err("ffmpeg.exe not found in archive".into());
        }
    }
    Ok(Deps {
        yt,
        bin,
        may_be_old,
    })
}

/// How the start-up look at yt-dlp ended, for the line the window shows.
#[derive(Debug, Clone, PartialEq)]
pub enum Refresh {
    /// Just downloaded, so it is the latest.
    Fresh(String),
    /// Already the latest release.
    Current(String),
    Updated {
        from: Option<String>,
        to: String,
    },
    /// The check or the update did not work; the version it is still at, if it can be told, and why.
    Failed {
        version: Option<String>,
        why: String,
    },
}

impl Refresh {
    pub fn line(&self) -> String {
        match self {
            Refresh::Fresh(v) => format!("yt-dlp {v}, just downloaded"),
            Refresh::Current(v) => format!("yt-dlp {v}, up to date"),
            Refresh::Updated { from: Some(f), to } => format!("yt-dlp updated from {f} to {to}"),
            Refresh::Updated { from: None, to } => format!("yt-dlp updated to {to}"),
            Refresh::Failed {
                version: Some(v),
                why,
            } => format!("yt-dlp {v}, could not check for updates ({why})"),
            Refresh::Failed { version: None, why } => {
                format!("yt-dlp could not be checked ({why})")
            }
        }
    }
}

/// The version yt-dlp says it is ("2026.11.20"), or None if it does not answer.
pub fn version(yt: &Path) -> Option<String> {
    let out = quiet(Command::new(yt).arg("--version")).output().ok()?;
    let v = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (out.status.success() && !v.is_empty()).then_some(v)
}

/// Lets yt-dlp replace itself with the latest release, then asks which version it is now.
pub fn update(yt: &Path) -> Refresh {
    let before = version(yt);
    let ran = quiet(Command::new(yt).arg("-U"))
        .output()
        .map(|o| {
            (
                o.status.success(),
                format!(
                    "{}\n{}",
                    String::from_utf8_lossy(&o.stdout),
                    String::from_utf8_lossy(&o.stderr)
                ),
            )
        })
        .map_err(|e| e.to_string());
    judge(before, version(yt), ran)
}

/// What the versions before and after, and how `-U` ended, say about the update.
fn judge(
    before: Option<String>,
    after: Option<String>,
    ran: Result<(bool, String), String>,
) -> Refresh {
    match (ran, after) {
        (Ok((true, _)), Some(to)) if before.as_deref() == Some(to.as_str()) => Refresh::Current(to),
        (Ok((true, _)), Some(to)) => Refresh::Updated { from: before, to },
        (Ok((_, said)), version) => Refresh::Failed {
            version: version.or(before),
            why: why(&said),
        },
        (Err(e), version) => Refresh::Failed {
            version: version.or(before),
            why: e,
        },
    }
}

/// The reason in yt-dlp's words: its first ERROR line, else its last line, cut to fit the window.
fn why(said: &str) -> String {
    let lines: Vec<&str> = said
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    let line = lines
        .iter()
        .find(|l| l.starts_with("ERROR"))
        .or(lines.last())
        .map_or("no answer", |l| l);
    let line = line.trim_start_matches("ERROR:").trim();
    match line.char_indices().nth(70) {
        Some((i, _)) => format!("{}...", &line[..i]),
        None => line.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_known_vector() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn a_failed_write_leaves_neither_the_file_nor_a_part() {
        let dir = std::env::temp_dir().join(format!("camallo-atomic-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let target = dir.join("tool.exe");
        let e = write_atomic(&target, |f| {
            f.write_all(b"half of it")?;
            Err(std::io::Error::other("connection lost"))
        });
        assert!(e.unwrap_err().contains("connection lost"));
        assert!(
            !target.exists(),
            "a half file must never take the real name"
        );
        assert!(!dir.join("tool.exe.part").exists());
        // A later try works, and replaces what was there.
        write_atomic(&target, |f| f.write_all(b"v1")).unwrap();
        write_atomic(&target, |f| f.write_all(b"v2")).unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"v2");
        assert!(!dir.join("tool.exe.part").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn v(s: &str) -> Option<String> {
        Some(s.to_string())
    }

    #[test]
    fn the_update_is_told_from_the_versions_around_it() {
        let ok = || Ok((true, "yt-dlp is up to date".to_string()));
        assert_eq!(
            judge(v("2026.11.01"), v("2026.11.01"), ok()),
            Refresh::Current("2026.11.01".into())
        );
        assert_eq!(
            judge(v("2026.11.01"), v("2026.11.20"), ok()),
            Refresh::Updated {
                from: v("2026.11.01"),
                to: "2026.11.20".into()
            }
        );
        assert_eq!(
            judge(None, v("2026.11.20"), ok()),
            Refresh::Updated {
                from: None,
                to: "2026.11.20".into()
            }
        );
    }

    #[test]
    fn a_failed_update_keeps_the_version_and_says_why() {
        let bad = Ok((false, "Current version: stable@2026.11.01\nERROR: Unable to obtain version info (<urlopen error>)\n".to_string()));
        let Refresh::Failed { version, why } = judge(v("2026.11.01"), v("2026.11.01"), bad) else {
            panic!("not a failure")
        };
        assert_eq!(version.as_deref(), Some("2026.11.01"));
        assert_eq!(why, "Unable to obtain version info (<urlopen error>)");
        // An update that ended well but left no yt-dlp that answers is a failure, not a success.
        assert!(matches!(
            judge(v("2026.11.01"), None, Ok((true, String::new()))),
            Refresh::Failed {
                version: Some(_),
                ..
            }
        ));
        assert_eq!(
            judge(None, None, Err("not found".into())),
            Refresh::Failed {
                version: None,
                why: "not found".into()
            }
        );
    }

    #[test]
    fn the_reason_is_short_and_never_empty() {
        assert_eq!(why(""), "no answer");
        assert!(why(&"x".repeat(300)).chars().count() <= 73);
        assert_eq!(why("one\ntwo"), "two");
    }

    #[test]
    fn the_lines_the_window_shows() {
        assert_eq!(Refresh::Current("1".into()).line(), "yt-dlp 1, up to date");
        assert_eq!(
            Refresh::Fresh("1".into()).line(),
            "yt-dlp 1, just downloaded"
        );
        assert_eq!(
            Refresh::Updated {
                from: v("1"),
                to: "2".into()
            }
            .line(),
            "yt-dlp updated from 1 to 2"
        );
        assert_eq!(
            Refresh::Failed {
                version: v("1"),
                why: "offline".into()
            }
            .line(),
            "yt-dlp 1, could not check for updates (offline)"
        );
    }

    #[test]
    fn hash_parsing() {
        assert_eq!(
            expected_hash("AbC123\n", "x.zip").as_deref(),
            Some("abc123")
        );
        let sums = "aaa  other.exe\nbbb *yt-dlp.exe\n";
        assert_eq!(expected_hash(sums, "yt-dlp.exe").as_deref(), Some("bbb"));
        assert_eq!(expected_hash(sums, "missing.exe"), None);
    }
}
