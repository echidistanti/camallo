use crate::job::quiet;
use sha2::{Digest, Sha256};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

pub struct Deps {
    pub yt: PathBuf,
    pub bin: PathBuf,
    /// yt-dlp was already there, so it may be out of date; see `update`.
    pub may_be_old: bool,
}

const YT_BASE: &str = "https://github.com/yt-dlp/yt-dlp/releases/latest/download/";

/// The file of the yt-dlp release that runs on this system, and the name it is kept under.
#[cfg(windows)]
const YT_ASSET: &str = "yt-dlp.exe";
#[cfg(not(windows))]
const YT_ASSET: &str = "yt-dlp_linux";

#[cfg(windows)]
const EXE: &str = ".exe";
#[cfg(not(windows))]
const EXE: &str = "";

#[cfg(windows)]
const FF_ZIP: &str = "https://www.gyan.dev/ffmpeg/builds/ffmpeg-release-essentials.zip";

/// Where the Linux builds of ffmpeg are kept. The `latest` release holds the current build of every
/// branch, with a `checksums.sha256` next to them.
#[cfg(not(windows))]
const FF_LINUX_BASE: &str =
    "https://github.com/FFmpeg-Builds/FFmpeg-Builds/releases/download/latest";

/// `%LOCALAPPDATA%\Camallo` on Windows, `~/.local/share/camallo` on Linux, where folders are lower case by habit.
pub fn data_dir() -> PathBuf {
    let name = if cfg!(windows) { "Camallo" } else { "camallo" };
    dirs::data_local_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join(name)
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
/// half a file under the real name (the next start would trust it, because it exists). On Linux the
/// file is made runnable: everything written here is a program.
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
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            f.set_permissions(std::fs::Permissions::from_mode(0o755))?;
        }
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
    let yt = bin.join(format!("yt-dlp{EXE}"));

    let may_be_old = yt.exists();
    if !may_be_old {
        status("Downloading yt-dlp...");
        let data = verified(
            &format!("{YT_BASE}{YT_ASSET}"),
            &format!("{YT_BASE}SHA2-256SUMS"),
            YT_ASSET,
        )?;
        write_atomic(&yt, |f| f.write_all(&data))?;
    }

    if !bin.join(format!("ffmpeg{EXE}")).exists() {
        download_ffmpeg(&bin, status)?;
    }
    Ok(Deps {
        yt,
        bin,
        may_be_old,
    })
}

/// Windows: the "essentials" build from gyan.dev, of which only ffmpeg.exe is kept.
#[cfg(windows)]
fn download_ffmpeg(bin: &Path, status: &mut dyn FnMut(&str)) -> Result<(), String> {
    status("Downloading ffmpeg (about 30 MB)...");
    let zip = verified(
        FF_ZIP,
        &format!("{FF_ZIP}.sha256"),
        "ffmpeg-release-essentials.zip",
    )?;
    let mut ar = zip::ZipArchive::new(std::io::Cursor::new(zip)).map_err(|e| e.to_string())?;
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
    Ok(())
}

/// Linux: the GPL build (static, with the encoders yt-dlp may ask for, MP3 among them) of the newest
/// release branch of FFmpeg-Builds. The system's own ffmpeg is not used: some distributions build theirs
/// without the MP3 encoder.
#[cfg(not(windows))]
fn download_ffmpeg(bin: &Path, status: &mut dyn FnMut(&str)) -> Result<(), String> {
    status("Looking for the current ffmpeg build...");
    let sums =
        String::from_utf8_lossy(&fetch(&format!("{FF_LINUX_BASE}/checksums.sha256"))?).into_owned();
    let (name, want) =
        pick_linux_build(&sums).ok_or("no Linux build in the list of ffmpeg builds")?;
    status("Downloading ffmpeg (about 60 MB)...");
    let archive = fetch(&format!("{FF_LINUX_BASE}/{name}"))?;
    if sha256_hex(&archive) != want {
        return Err(format!("checksum mismatch for {name}, file discarded"));
    }
    status("Unpacking ffmpeg (about half a minute)...");
    unpack_tar_xz(archive, bin)?;
    if !bin.join("ffmpeg").exists() {
        return Err("ffmpeg not found in archive".into());
    }
    Ok(())
}

/// Which Linux build to take from the list of checksums: the GPL build of the newest release branch,
/// `ffmpeg-n9.0-latest-linux64-gpl-9.0.tar.xz` and the like. The branches come and go, so the name is read
/// from the list; the build of the development branch is the fallback when no release branch is there.
/// Returns the file name and its SHA-256.
#[cfg(not(windows))]
fn pick_linux_build(sums: &str) -> Option<(String, String)> {
    let version = |name: &str| -> Option<(u32, u32)> {
        let rest = name.strip_prefix("ffmpeg-n")?.strip_suffix(".tar.xz")?;
        let (branch, tail) = rest.split_once("-latest-linux64-gpl-")?;
        if branch != tail {
            return None;
        }
        let (major, minor) = branch.split_once('.')?;
        Some((major.parse().ok()?, minor.parse().ok()?))
    };
    let lines: Vec<(&str, &str)> = sums
        .lines()
        .filter_map(|l| {
            let mut w = l.split_whitespace();
            Some((w.next()?, w.next()?.trim_start_matches('*')))
        })
        .collect();
    let best = lines
        .iter()
        .filter_map(|(h, n)| version(n).map(|v| (v, *h, *n)))
        .max_by_key(|(v, _, _)| *v);
    let (hash, name) = match best {
        Some((_, h, n)) => (h, n),
        None => lines
            .iter()
            .copied()
            .find(|(_, n)| *n == "ffmpeg-master-latest-linux64-gpl.tar.xz")?,
    };
    Some((name.to_string(), hash.to_lowercase()))
}

/// Takes `ffmpeg` and `ffprobe` out of a .tar.xz, whatever the folder inside it is called, and makes them
/// runnable. Stops reading as soon as it has both.
#[cfg(not(windows))]
fn unpack_tar_xz(archive: Vec<u8>, bin: &Path) -> Result<(), String> {
    const TOOLS: [&str; 2] = ["ffmpeg", "ffprobe"];
    let mut tar = tar::Archive::new(lzma_rust2::XzReader::new(
        std::io::Cursor::new(archive),
        true,
    ));
    let mut found = [false; 2];
    for entry in tar.entries().map_err(|e| e.to_string())? {
        let mut entry = entry.map_err(|e| e.to_string())?;
        if !entry.header().entry_type().is_file() {
            continue;
        }
        let path = entry.path().map_err(|e| e.to_string())?.into_owned();
        let in_bin = path.parent().is_some_and(|d| d.ends_with("bin"));
        let Some(i) = TOOLS
            .iter()
            .position(|t| in_bin && path.file_name().is_some_and(|f| f == *t))
        else {
            continue;
        };
        write_atomic(&bin.join(TOOLS[i]), |out| {
            std::io::copy(&mut entry, out).map(|_| ())
        })?;
        found[i] = true;
        if found == [true, true] {
            return Ok(());
        }
    }
    Err("ffmpeg or ffprobe was not in the archive".into())
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

    /// The first lines of the real list at FFmpeg-Builds, with a branch that is older and one that is only
    /// the development branch among them.
    #[cfg(not(windows))]
    const LIST: &str = "\
405ea9c76429c4b4ae777c6bc6748658b789e3bd6ff2541f52d54fd1cd2e605b  ffmpeg-master-latest-linux64-gpl-shared.tar.xz
231bf775a8800a6d148777b4be81a6f7e21b095d8f7e76b4fe02585806682cb6  ffmpeg-master-latest-linux64-gpl.tar.xz
83cee2d58418745840e991d6521e331f94c79cba138f5cf3dc623466958142ca  ffmpeg-master-latest-linux64-lgpl.tar.xz
6941b3ca1e60af55a5abd48bab758c29bd71d77389fea066e8ffd9a7bf37456f  ffmpeg-n8.1-latest-linux64-gpl-8.1.tar.xz
feb78e06d2d4d8e6e2944e67c6f5ec9d7cbe924559b77f3532ec1ffac13437c4  ffmpeg-n8.1-latest-linux64-gpl-shared-8.1.tar.xz
952E08967D724A632F9BD3D0CD765F771985831934100EACF92650713F61D54F  ffmpeg-n9.0-latest-linux64-gpl-9.0.tar.xz
f5a03389ec684b51fddb9f9c66e3e7e25437a3d013007a01af45eefcb16d9888  ffmpeg-n9.0-latest-linux64-gpl-shared-9.0.tar.xz
af5e4d9df419a247aa7d9a666bc5a4a10263d442998f5da235423ff8e687bbd5  ffmpeg-n9.0-latest-linux64-lgpl-9.0.tar.xz
1111111111111111111111111111111111111111111111111111111111111111  ffmpeg-n10.0-latest-win64-gpl-10.0.zip
";

    #[cfg(not(windows))]
    #[test]
    fn the_newest_gpl_release_branch_is_picked_from_the_list() {
        let (name, hash) = pick_linux_build(LIST).unwrap();
        assert_eq!(
            name, "ffmpeg-n9.0-latest-linux64-gpl-9.0.tar.xz",
            "not shared, not lgpl, not the older branch"
        );
        assert_eq!(
            hash, "952e08967d724a632f9bd3d0cd765f771985831934100eacf92650713f61d54f",
            "lower case"
        );
    }

    #[cfg(not(windows))]
    #[test]
    fn branch_numbers_compare_as_numbers_not_as_text() {
        let sums = "aa  ffmpeg-n9.0-latest-linux64-gpl-9.0.tar.xz\nbb  ffmpeg-n10.1-latest-linux64-gpl-10.1.tar.xz\ncc  ffmpeg-n10.0-latest-linux64-gpl-10.0.tar.xz\n";
        assert_eq!(
            pick_linux_build(sums).unwrap().0,
            "ffmpeg-n10.1-latest-linux64-gpl-10.1.tar.xz"
        );
    }

    #[cfg(not(windows))]
    #[test]
    fn without_a_release_branch_the_development_build_is_used() {
        let sums =
            "aa  ffmpeg-master-latest-linux64-gpl.tar.xz\nbb  ffmpeg-master-latest-win64-gpl.zip\n";
        assert_eq!(
            pick_linux_build(sums),
            Some((
                "ffmpeg-master-latest-linux64-gpl.tar.xz".into(),
                "aa".into()
            ))
        );
        assert_eq!(
            pick_linux_build("bb  ffmpeg-master-latest-win64-gpl.zip\n"),
            None
        );
        assert_eq!(pick_linux_build(""), None);
    }

    /// A tiny archive built like the real one: the two programs in `bin/`, next to a third program, a
    /// manual and a file with the same name somewhere else.
    #[cfg(unix)]
    #[test]
    fn the_two_programs_come_out_of_a_tar_xz_and_can_run() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("camallo-txz-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        unpack_tar_xz(
            include_bytes!("../tests/data/ffmpeg-like.tar.xz").to_vec(),
            &dir,
        )
        .unwrap();
        assert_eq!(
            std::fs::read(dir.join("ffmpeg")).unwrap(),
            b"not really ffmpeg\n",
            "the one in bin/, not the one in include/"
        );
        assert_eq!(
            std::fs::read(dir.join("ffprobe")).unwrap(),
            b"not really ffprobe\n"
        );
        let mut names: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        assert_eq!(
            names,
            ["ffmpeg", "ffprobe"],
            "nothing else is written, and no .part is left"
        );
        let mode = std::fs::metadata(dir.join("ffmpeg"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o111, 0o111, "runnable");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(not(windows))]
    #[test]
    fn an_archive_without_the_programs_is_an_error_and_writes_nothing() {
        let dir = std::env::temp_dir().join(format!("camallo-txz-none-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // Not an xz file at all, as a half download or an error page would be.
        assert!(unpack_tar_xz(b"<html>Not found</html>".to_vec(), &dir).is_err());
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Everything written is a program, so on Linux it must come out runnable.
    #[cfg(unix)]
    #[test]
    fn what_is_written_can_be_run() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("camallo-exec-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let target = dir.join("yt-dlp");
        write_atomic(&target, |f| f.write_all(b"#!/bin/sh\nexit 0\n")).unwrap();
        assert_eq!(
            std::fs::metadata(&target).unwrap().permissions().mode() & 0o111,
            0o111
        );
        assert!(Command::new(&target).status().unwrap().success());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Runs only when CAMALLO_TEST_TXZ names a real FFmpeg-Builds archive (about 60 MB).
    #[cfg(unix)]
    #[test]
    fn the_real_linux_archive_gives_both_programs() {
        let Some(file) = std::env::var_os("CAMALLO_TEST_TXZ") else {
            return;
        };
        let dir = std::env::temp_dir().join(format!("camallo-txz-real-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let t = std::time::Instant::now();
        unpack_tar_xz(std::fs::read(file).unwrap(), &dir).unwrap();
        eprintln!("unpacked in {:?}", t.elapsed());
        for tool in ["ffmpeg", "ffprobe"] {
            let data = std::fs::read(dir.join(tool)).unwrap();
            assert!(
                data.len() > 10_000_000,
                "{tool} is too small: {}",
                data.len()
            );
            assert_eq!(&data[..4], b"\x7fELF", "{tool} is not a Linux program");
        }
        let out = Command::new(dir.join("ffmpeg"))
            .arg("-encoders")
            .output()
            .unwrap();
        let list = String::from_utf8_lossy(&out.stdout);
        assert!(list.contains("libmp3lame"), "the build has no MP3 encoder");
        assert_eq!(
            std::fs::read_dir(&dir).unwrap().count(),
            2,
            "only the two programs are written"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Downloads the real programs (about 200 MB), so it runs only on request: `cargo test -- --ignored`,
    /// best with XDG_DATA_HOME pointing at an empty folder.
    #[test]
    #[ignore = "downloads yt-dlp and ffmpeg"]
    fn the_real_downloads_run() {
        let mut said = Vec::new();
        let d = ensure(&mut |s| said.push(s.to_string())).unwrap();
        eprintln!("{said:?}");
        assert!(!d.may_be_old, "the test wants a first run");
        let v = version(&d.yt).expect("yt-dlp answers");
        eprintln!("yt-dlp {v}");
        let ff = d.bin.join(format!("ffmpeg{EXE}"));
        let out = Command::new(&ff).arg("-encoders").output().unwrap();
        let list = String::from_utf8_lossy(&out.stdout);
        assert!(list.contains("libmp3lame"), "no MP3 encoder in this ffmpeg");
        assert!(d.bin.join(format!("ffprobe{EXE}")).exists() || cfg!(windows));
        // A second run finds everything and downloads nothing.
        let again = ensure(&mut |_| panic!("nothing should be downloaded again")).unwrap();
        assert!(again.may_be_old);
        eprintln!("{}", update(&d.yt).line());
    }
}
