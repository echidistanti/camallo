// User choices that survive a restart, and the list of finished downloads.
use crate::deps;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default, Debug)]
pub enum Container {
    #[default]
    Mp4,
    Mkv,
    Webm,
}

impl Container {
    pub const ALL: [Container; 3] = [Container::Mp4, Container::Mkv, Container::Webm];
    pub fn ext(self) -> &'static str {
        match self {
            Container::Mp4 => "mp4",
            Container::Mkv => "mkv",
            Container::Webm => "webm",
        }
    }
    /// For small spaces: the combo box and the capsule on a download.
    pub fn short(self) -> &'static str {
        match self {
            Container::Mp4 => "MP4",
            Container::Mkv => "MKV",
            Container::Webm => "WebM",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Container::Mp4 => "MP4",
            Container::Mkv => "MKV",
            Container::Webm => "WebM (re-encodes, slow)",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default, Debug)]
pub enum Quality {
    #[default]
    Best,
    P1080,
    P720,
    P480,
}

impl Quality {
    pub const ALL: [Quality; 4] = [Quality::Best, Quality::P1080, Quality::P720, Quality::P480];
    /// Tallest picture accepted, None for no limit.
    pub fn height(self) -> Option<u32> {
        match self {
            Quality::Best => None,
            Quality::P1080 => Some(1080),
            Quality::P720 => Some(720),
            Quality::P480 => Some(480),
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Quality::Best => "Highest",
            Quality::P1080 => "1080p",
            Quality::P720 => "720p",
            Quality::P480 => "480p",
        }
    }
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub container: Container,
    pub quality: Quality,
    pub audio_only: bool,
    pub parallel: usize,
    pub dir: PathBuf,
    /// Optional folder inside `dir` (for example one per site or per project).
    pub subdir: String,
    /// Start each video as soon as yt-dlp has read it, without waiting for Download.
    pub auto_start: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            container: Container::Mp4,
            quality: Quality::Best,
            audio_only: false,
            parallel: 2,
            dir: dirs::video_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join("Camallo"),
            subdir: String::new(),
            auto_start: false,
        }
    }
}

pub const MAX_PARALLEL: usize = 4;

fn settings_file() -> PathBuf {
    deps::data_dir().join("settings.json")
}

/// A single folder name: no separators, no traversal. Anything else is dropped.
pub fn clean_subdir(s: &str) -> String {
    let s: String = s
        .chars()
        .filter(|c| !"<>:\"/\\|?*".contains(*c) && !c.is_control())
        .collect();
    let s = s.trim().trim_matches('.').trim();
    s.chars().take(60).collect()
}

impl Settings {
    pub fn load() -> Settings {
        let mut s: Settings = std::fs::read_to_string(settings_file())
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default();
        s.parallel = s.parallel.clamp(1, MAX_PARALLEL);
        s.subdir = clean_subdir(&s.subdir);
        if s.dir.as_os_str().is_empty() {
            s.dir = Settings::default().dir;
        }
        s
    }

    pub fn save(&self) {
        let _ = std::fs::create_dir_all(deps::data_dir());
        if let Ok(t) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(settings_file(), t);
        }
    }

    /// Where files land: the chosen folder plus the optional sub-folder.
    pub fn out_dir(&self) -> PathBuf {
        let sub = clean_subdir(&self.subdir);
        if sub.is_empty() {
            self.dir.clone()
        } else {
            self.dir.join(sub)
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct HistoryItem {
    pub title: String,
    pub url: String,
    pub path: String,
    /// Seconds since the Unix epoch.
    pub when: u64,
}

const HISTORY_MAX: usize = 100;

fn history_file() -> PathBuf {
    deps::data_dir().join("history.json")
}

/// An address without what can act as a key: the part after `?` or `#` (signed links carry tokens there)
/// and any `user:password@`. The history is a plain file any program of the user can read.
pub fn plain_url(u: &str) -> String {
    let u = u.split(['?', '#']).next().unwrap_or(u);
    match u.split_once("://") {
        Some((scheme, rest)) => {
            let (auth, path) = rest.split_at(rest.find('/').unwrap_or(rest.len()));
            format!(
                "{scheme}://{}{path}",
                auth.rsplit('@').next().unwrap_or(auth)
            )
        }
        None => u.to_string(),
    }
}

/// Cleans the addresses of old entries, written by versions that kept them whole. True when something changed.
fn scrub(h: &mut [HistoryItem]) -> bool {
    let mut changed = false;
    for i in h {
        let p = plain_url(&i.url);
        if p != i.url {
            i.url = p;
            changed = true;
        }
    }
    changed
}

pub fn load_history() -> Vec<HistoryItem> {
    let mut h: Vec<HistoryItem> = std::fs::read_to_string(history_file())
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default();
    if scrub(&mut h) {
        save_history(&h);
    }
    h
}

pub fn save_history(h: &[HistoryItem]) {
    let _ = std::fs::create_dir_all(deps::data_dir());
    let start = h.len().saturating_sub(HISTORY_MAX);
    if let Ok(t) = serde_json::to_string(&h[start..]) {
        let _ = std::fs::write(history_file(), t);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subdir_is_a_single_clean_name() {
        assert_eq!(clean_subdir("../../etc"), "etc");
        assert_eq!(clean_subdir("a/b\\c"), "abc");
        assert_eq!(clean_subdir("  .. "), "");
        assert_eq!(clean_subdir("Corsi 2026"), "Corsi 2026");
    }

    #[test]
    fn history_keeps_no_keys_from_the_address() {
        assert_eq!(
            plain_url("https://cdn.test/v/master.m3u8?token=abc&exp=1#t=5"),
            "https://cdn.test/v/master.m3u8"
        );
        assert_eq!(
            plain_url("https://me:secret@site.test/a/b?x=1"),
            "https://site.test/a/b"
        );
        assert_eq!(plain_url("https://site.test"), "https://site.test");
        assert_eq!(plain_url("https://site.test?k=1"), "https://site.test");
        assert_eq!(plain_url("https://site.test/p@q"), "https://site.test/p@q");
        let item = |u: &str| HistoryItem {
            title: "t".into(),
            url: u.into(),
            path: String::new(),
            when: 0,
        };
        let mut h = vec![item("https://a.test/x?sig=1"), item("https://b.test/y")];
        assert!(scrub(&mut h));
        assert_eq!(
            (h[0].url.as_str(), h[1].url.as_str()),
            ("https://a.test/x", "https://b.test/y")
        );
        assert!(!scrub(&mut h));
    }

    #[test]
    fn out_dir_joins_subdir() {
        let mut s = Settings {
            dir: PathBuf::from("base"),
            ..Settings::default()
        };
        assert_eq!(s.out_dir(), PathBuf::from("base"));
        s.subdir = "x/y".into();
        assert_eq!(s.out_dir(), PathBuf::from("base").join("xy"));
    }

    #[test]
    fn settings_roundtrip_and_defaults_for_missing_fields() {
        let s: Settings = serde_json::from_str(r#"{"quality":"P720","parallel":3}"#).unwrap();
        assert_eq!(s.quality, Quality::P720);
        assert_eq!(s.parallel, 3);
        assert_eq!(s.container, Container::Mp4);
        let t = serde_json::to_string(&s).unwrap();
        let back: Settings = serde_json::from_str(&t).unwrap();
        assert_eq!(back.quality, Quality::P720);
    }
}
