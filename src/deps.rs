use crate::job::quiet;
use sha2::{Digest, Sha256};
use std::io::Cursor;
use std::path::PathBuf;
use std::process::Command;

pub struct Deps {
    pub yt: PathBuf,
    pub bin: PathBuf,
}

const YT_BASE: &str = "https://github.com/yt-dlp/yt-dlp/releases/latest/download/";
const FF_ZIP: &str = "https://www.gyan.dev/ffmpeg/builds/ffmpeg-release-essentials.zip";

pub fn data_dir() -> PathBuf {
    dirs::data_local_dir().unwrap_or_else(std::env::temp_dir).join("Camallo")
}

fn fetch(url: &str) -> Result<Vec<u8>, String> {
    let mut r = ureq::get(url).call().map_err(|e| format!("{url}: {e}"))?;
    r.body_mut().with_config().limit(400 << 20).read_to_vec().map_err(|e| e.to_string())
}

pub fn sha256_hex(data: &[u8]) -> String {
    Sha256::digest(data).iter().map(|b| format!("{b:02x}")).collect()
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

/// First run: download yt-dlp and ffmpeg (checksum-verified). Later runs: let yt-dlp self-update.
pub fn ensure(status: &mut dyn FnMut(&str)) -> Result<Deps, String> {
    let bin = data_dir().join("bin");
    std::fs::create_dir_all(&bin).map_err(|e| e.to_string())?;
    let yt = bin.join("yt-dlp.exe");

    if yt.exists() {
        status("Checking for yt-dlp updates...");
        let _ = quiet(&mut Command::new(&yt).arg("-U")).output();
    } else {
        status("Downloading yt-dlp...");
        let data = verified(&format!("{YT_BASE}yt-dlp.exe"), &format!("{YT_BASE}SHA2-256SUMS"), "yt-dlp.exe")?;
        std::fs::write(&yt, data).map_err(|e| e.to_string())?;
    }

    if !bin.join("ffmpeg.exe").exists() {
        status("Downloading ffmpeg (about 30 MB)...");
        let zip = verified(FF_ZIP, &format!("{FF_ZIP}.sha256"), "ffmpeg-release-essentials.zip")?;
        let mut ar = zip::ZipArchive::new(Cursor::new(zip)).map_err(|e| e.to_string())?;
        for i in 0..ar.len() {
            let mut f = ar.by_index(i).map_err(|e| e.to_string())?;
            if f.name().is_ok_and(|n| n.ends_with("/bin/ffmpeg.exe")) {
                let mut out = std::fs::File::create(bin.join("ffmpeg.exe")).map_err(|e| e.to_string())?;
                std::io::copy(&mut f, &mut out).map_err(|e| e.to_string())?;
                break;
            }
        }
        if !bin.join("ffmpeg.exe").exists() {
            return Err("ffmpeg.exe not found in archive".into());
        }
    }
    Ok(Deps { yt, bin })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_known_vector() {
        assert_eq!(sha256_hex(b"abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    }

    #[test]
    fn hash_parsing() {
        assert_eq!(expected_hash("AbC123\n", "x.zip").as_deref(), Some("abc123"));
        let sums = "aaa  other.exe\nbbb *yt-dlp.exe\n";
        assert_eq!(expected_hash(sums, "yt-dlp.exe").as_deref(), Some("bbb"));
        assert_eq!(expected_hash(sums, "missing.exe"), None);
    }
}
