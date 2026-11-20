// Asks yt-dlp what a video offers, so the user can pick a quality before downloading.
use crate::job::{quiet, Cand};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::Path;
use std::process::{Command, Stdio};

#[derive(Clone, Debug, PartialEq)]
pub struct Quality {
    pub height: u32,
    /// Rough size of video plus best audio, when the site tells.
    pub size: Option<u64>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Info {
    pub title: String,
    pub duration: Option<u64>,
    /// Tallest first.
    pub qualities: Vec<Quality>,
    pub has_audio: bool,
    pub thumbnail: Option<String>,
}

pub fn human_size(b: u64) -> String {
    const U: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut v = b as f64;
    let mut i = 0;
    while v >= 1024.0 && i < U.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 { format!("{b} B") } else { format!("{v:.1} {}", U[i]) }
}

pub fn human_duration(s: u64) -> String {
    let (h, m, s) = (s / 3600, s % 3600 / 60, s % 60);
    if h > 0 { format!("{h}:{m:02}:{s:02}") } else { format!("{m}:{s:02}") }
}

fn size_of(f: &Value) -> Option<u64> {
    f["filesize"].as_u64().or_else(|| f["filesize_approx"].as_f64().map(|x| x as u64))
}

/// Reads the output of `yt-dlp -J`.
/// A codec of "none" means the stream has no such track; null means the playlist did not say
/// (common in HLS), which is treated as present.
pub fn parse_info(v: &Value) -> Info {
    let absent = |f: &Value, k: &str| f[k].as_str() == Some("none");
    let formats: Vec<&Value> = v["formats"].as_array().map(|a| a.iter().collect()).unwrap_or_default();

    let audio_size = formats
        .iter()
        .filter(|f| absent(f, "vcodec") && !absent(f, "acodec"))
        .filter_map(|f| size_of(f))
        .max()
        .unwrap_or(0);
    let has_audio = formats.iter().any(|f| !absent(f, "acodec")) || (formats.is_empty() && !absent(v, "acodec"));

    // One entry per picture height; the biggest file at that height stands for it.
    // A picture without sound of its own is sized together with the best separate audio.
    let mut by_h: BTreeMap<u32, Option<u64>> = BTreeMap::new();
    let entry = |f: &Value| -> Option<(u32, Option<u64>)> {
        let h = f["height"].as_u64().filter(|h| *h > 0)? as u32;
        Some((h, size_of(f).map(|s| s + if absent(f, "acodec") { audio_size } else { 0 })))
    };
    for (h, s) in formats.iter().filter(|f| !absent(f, "vcodec")).filter_map(|f| entry(f)) {
        let e = by_h.entry(h).or_insert(None);
        *e = (*e).max(s);
    }
    if by_h.is_empty() && !absent(v, "vcodec") {
        if let Some((h, s)) = entry(v) {
            by_h.insert(h, s);
        }
    }
    let qualities = by_h.into_iter().rev().map(|(height, size)| Quality { height, size }).collect();

    Info {
        title: v["title"].as_str().unwrap_or_default().to_string(),
        duration: v["duration"].as_f64().map(|d| d as u64),
        qualities,
        has_audio,
        thumbnail: pick_thumb(v),
    }
}

/// A picture for the list: the widest one up to 700 px (small enough to be quick), else whatever yt-dlp calls "the" thumbnail.
pub fn pick_thumb(v: &Value) -> Option<String> {
    let usable = |u: &str| u.starts_with("http://") || u.starts_with("https://");
    let best = v["thumbnails"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|t| Some((t["width"].as_u64().unwrap_or(0), t["url"].as_str().filter(|u| usable(u))?)))
        .filter(|(w, _)| *w <= 700)
        .max_by_key(|(w, _)| *w)
        .map(|(_, u)| u.to_string());
    best.or_else(|| v["thumbnail"].as_str().filter(|u| usable(u)).map(String::from))
}

/// Runs `yt-dlp -J` on an address and returns what it printed, or the one line that says why it could not.
pub fn run_json(yt: &Path, c: &Cand, extra: &[&str]) -> Result<Value, String> {
    let mut cmd = Command::new(yt);
    quiet(&mut cmd)
        // The user's own yt-dlp configuration must not change what is read.
        .args(["-J", "--ignore-config", "--no-warnings", "--socket-timeout", "20"])
        .args(extra)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(r) = &c.referer {
        cmd.args(["--referer", r]);
    }
    if let Some(u) = &c.ua {
        cmd.args(["--add-header", &format!("User-Agent:{u}")]);
    }
    cmd.arg(&c.url);
    let out = cmd.output().map_err(|e| format!("cannot start yt-dlp: {e}"))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        let line = err.lines().rev().find(|l| l.starts_with("ERROR:")).unwrap_or("yt-dlp could not read this address");
        return Err(line.trim_start_matches("ERROR:").trim().chars().take(160).collect());
    }
    serde_json::from_slice(&out.stdout).map_err(|e| format!("unreadable answer: {e}"))
}

/// What one video offers.
pub fn probe(yt: &Path, c: &Cand) -> Result<Info, String> {
    let v = run_json(yt, c, &["--no-playlist"])?;
    if v["_type"].as_str() == Some("playlist") || v["entries"].is_array() {
        return Err("This address is a list of videos, not a single video".into());
    }
    Ok(parse_info(&v))
}

impl Info {
    /// Rough size of the file for a picture height (None: the tallest), when the site told.
    pub fn size_for(&self, height: Option<u32>) -> Option<u64> {
        match height {
            None => self.qualities.first()?.size,
            Some(h) => self.qualities.iter().find(|q| q.height == h)?.size,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[cfg(unix)]
    #[test]
    fn reading_an_address_ignores_the_users_yt_dlp_configuration() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("camallo-probe-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // A stand-in that answers with the arguments it got.
        let fake = dir.join("fake-yt");
        std::fs::write(&fake, "#!/bin/sh\nprintf '{\"args\":\"%s\"}' \"$*\"\n").unwrap();
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        let c = Cand { url: "https://example.com/v".into(), referer: None, ua: None, title: None };
        let v = run_json(&fake, &c, &[]).unwrap();
        assert!(v["args"].as_str().unwrap().contains("--ignore-config"), "{v}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn sizes_and_durations_read_well() {
        assert_eq!(human_size(512), "512 B");
        assert_eq!(human_size(1536), "1.5 KB");
        assert_eq!(human_size(5 * 1024 * 1024), "5.0 MB");
        assert_eq!(human_duration(75), "1:15");
        assert_eq!(human_duration(3725), "1:02:05");
    }

    #[test]
    fn heights_are_deduplicated_and_sorted_tallest_first() {
        let v = json!({
            "title": "Demo", "duration": 125.4,
            "formats": [
                {"vcodec": "avc1", "acodec": "none", "height": 360, "filesize": 1000},
                {"vcodec": "vp9",  "acodec": "none", "height": 720, "filesize": 5000},
                {"vcodec": "avc1", "acodec": "none", "height": 720, "filesize_approx": 7000.0},
                {"vcodec": "none", "acodec": "mp4a", "filesize": 400},
                {"vcodec": "none", "acodec": "none", "height": null}
            ]
        });
        let i = parse_info(&v);
        assert_eq!(i.title, "Demo");
        assert_eq!(i.duration, Some(125));
        assert!(i.has_audio);
        assert_eq!(
            i.qualities,
            [Quality { height: 720, size: Some(7400) }, Quality { height: 360, size: Some(1400) }]
        );
    }

    #[test]
    fn unknown_codecs_count_as_present() {
        // What yt-dlp prints for an HLS master playlist that declares no CODECS.
        let v = json!({
            "title": "master", "height": 360, "vcodec": null, "acodec": null,
            "formats": [
                {"format_id": "300", "height": 180, "vcodec": null, "acodec": null, "filesize": null},
                {"format_id": "900", "height": 360, "vcodec": null, "acodec": null, "filesize": null}
            ]
        });
        let i = parse_info(&v);
        assert_eq!(i.qualities, [Quality { height: 360, size: None }, Quality { height: 180, size: None }]);
        assert!(i.has_audio);
    }

    #[test]
    fn muxed_single_format_and_audio_only() {
        let v = json!({"title": "One", "height": 480, "acodec": "aac", "vcodec": "h264", "filesize": 900});
        let i = parse_info(&v);
        assert_eq!(i.qualities, [Quality { height: 480, size: Some(900) }]);
        assert!(i.has_audio);

        let v = json!({"title": "Song", "formats": [{"vcodec": "none", "acodec": "opus", "filesize": 10}]});
        let i = parse_info(&v);
        assert!(i.qualities.is_empty() && i.has_audio);
    }

    #[test]
    fn thumbnail_is_the_widest_up_to_700_px() {
        let v = json!({
            "thumbnails": [
                {"url": "https://i/small.jpg", "width": 168},
                {"url": "https://i/mid.jpg", "width": 480},
                {"url": "https://i/huge.jpg", "width": 1920},
                {"url": "ftp://i/odd.jpg", "width": 600}
            ],
            "thumbnail": "https://i/huge.jpg"
        });
        assert_eq!(pick_thumb(&v).as_deref(), Some("https://i/mid.jpg"));
        // Nothing small enough: the plain thumbnail field is used.
        let v = json!({"thumbnails": [{"url": "https://i/huge.jpg", "width": 1920}], "thumbnail": "https://i/t.webp"});
        assert_eq!(pick_thumb(&v).as_deref(), Some("https://i/t.webp"));
        assert_eq!(pick_thumb(&json!({"title": "x"})), None);
        assert_eq!(parse_info(&json!({"thumbnail": "https://i/t.jpg"})).thumbnail.as_deref(), Some("https://i/t.jpg"));
    }

    #[test]
    fn size_follows_the_chosen_height() {
        let q = |height, size| Quality { height, size };
        let i = Info { qualities: vec![q(1080, Some(300)), q(720, None), q(480, Some(80))], ..Info::default() };
        assert_eq!(i.size_for(None), Some(300));
        assert_eq!(i.size_for(Some(480)), Some(80));
        assert_eq!(i.size_for(Some(720)), None, "the site did not tell");
        assert_eq!(i.size_for(Some(240)), None, "no such height");
        assert_eq!(Info::default().size_for(None), None);
    }
}
