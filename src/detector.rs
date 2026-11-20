// Decides what the built-in browser does with the messages the injected script sends:
// when to show the Download button, what to tell the main window, and in which order the
// candidate addresses are tried.
use crate::job::Cand;
use serde_json::{json, Value};

pub const SHOW: &str = "window.__camallo&&window.__camallo.show()";
pub const HIDE: &str = "window.__camallo&&window.__camallo.hide()";
pub const SENT: &str = "window.__camallo&&window.__camallo.toast('Sent to Camallo')";

const MAX_TRIES: usize = 6;
/// A page can announce as many addresses as it likes; the detector keeps the first ones.
const MAX_SEEN: usize = 200;
/// Embeds whose address yt-dlp understands directly.
const KNOWN: [&str; 5] = [
    "dailymotion.com/video/",
    "youtube.com/embed/",
    "youtube-nocookie.com/embed/",
    "player.vimeo.com/video/",
    "streamable.com/e/",
];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    /// A player yt-dlp knows by name.
    Embed,
    /// The page the user is looking at.
    Page,
    /// HLS playlist.
    Hls,
    /// MPEG-DASH manifest.
    Dash,
    /// Microsoft Smooth Streaming manifest.
    Smooth,
    /// A plain video or audio file.
    File,
}

impl Kind {
    pub fn badge(self) -> &'static str {
        match self {
            Kind::Embed => "Embed",
            Kind::Page => "Page",
            Kind::Hls => "HLS",
            Kind::Dash => "DASH",
            Kind::Smooth => "Smooth",
            Kind::File => "File",
        }
    }
    /// Streams and files are not named by yt-dlp in a useful way, so the page title names them.
    pub fn named_by_page(self) -> bool {
        matches!(self, Kind::Hls | Kind::Dash | Kind::Smooth | Kind::File)
    }
}

pub fn kind_of(url: &str, page: bool) -> Kind {
    if page {
        return Kind::Page;
    }
    let u = url.to_lowercase();
    if KNOWN.iter().any(|k| u.contains(k)) {
        Kind::Embed
    } else if u.contains(".m3u8") {
        Kind::Hls
    } else if u.contains(".mpd") {
        Kind::Dash
    } else if u.contains(".ism/manifest") {
        Kind::Smooth
    } else {
        Kind::File
    }
}

pub enum Effect {
    /// A line for the main Camallo window (stdout of the browser process).
    Print(String),
    /// Script to run in the top frame of the page.
    Eval(&'static str),
}

struct Seen {
    cand: Cand,
    page: bool,
    /// Size in bytes when the network told it.
    size: Option<u64>,
}

#[derive(Default)]
pub struct Detector {
    seen: Vec<Seen>,
    page: Option<String>,
    title: String,
    /// User agent of the page, for media whose request headers were not reported.
    ua: String,
    shown: bool,
}

fn rank(s: &Seen) -> u8 {
    match kind_of(&s.cand.url, s.page) {
        Kind::Embed => 0,
        Kind::Page => 1,
        Kind::Hls | Kind::Dash | Kind::Smooth => 2,
        Kind::File => 3,
    }
}

/// Same address without the part after '#', which never reaches the server.
fn norm(u: &str) -> &str {
    u.split('#').next().unwrap_or(u)
}

impl Detector {
    /// Best way first: known embed, then the page itself (yt-dlp may know the site),
    /// then streaming manifests, then plain files. Arrival order breaks ties.
    pub fn ranked(&self) -> Vec<Cand> {
        let mut v: Vec<&Seen> = self.seen.iter().collect();
        v.sort_by_key(|s| rank(s));
        v.into_iter().take(MAX_TRIES).map(|s| s.cand.clone()).collect()
    }

    fn line(&self, kind: &str, c: &Cand, size: Option<u64>) -> String {
        json!({"kind": kind, "url": c.url, "referer": c.referer, "ua": c.ua, "title": self.title, "size": size}).to_string()
    }

    pub fn on_message(&mut self, raw: &str) -> Vec<Effect> {
        let Ok(v) = serde_json::from_str::<Value>(raw) else { return vec![] };
        let text = |k: &str| v[k].as_str().filter(|x| !x.is_empty()).map(String::from);
        let cand = || text("url").map(|url| Cand { url, referer: text("referer"), ua: text("ua"), title: None });
        match v["kind"].as_str() {
            Some("page") => {
                let Some(c) = cand() else { return vec![] };
                if self.page.as_deref() == Some(c.url.as_str()) {
                    return vec![];
                }
                // Pages that change address without reloading start their video a moment before the
                // script notices the new address; what was requested from the new address is kept.
                let kept: Vec<Seen> = self
                    .seen
                    .drain(..)
                    .filter(|s| !s.page && s.cand.referer.as_deref() == Some(c.url.as_str()))
                    .collect();
                self.page = Some(c.url.clone());
                self.title = text("title").unwrap_or_default();
                self.ua = text("ua").unwrap_or_default();
                self.shown = !kept.is_empty();
                let mut e = vec![Effect::Print(self.line("page", &c, None)), Effect::Eval(HIDE)];
                self.seen.push(Seen { cand: c, page: true, size: None });
                for k in kept {
                    e.push(Effect::Print(self.line("media", &k.cand, k.size)));
                    self.seen.push(k);
                }
                if self.shown {
                    e.push(Effect::Eval(SHOW));
                }
                e
            }
            // Many sites set the title after the page loaded.
            Some("title") => {
                let t = text("title").unwrap_or_default();
                if t == self.title {
                    return vec![];
                }
                self.title = t;
                vec![Effect::Print(json!({"kind": "title", "title": self.title}).to_string())]
            }
            Some("media") => {
                let Some(mut c) = cand() else { return vec![] };
                if self.seen.len() >= MAX_SEEN || self.seen.iter().any(|s| norm(&s.cand.url) == norm(&c.url)) {
                    return vec![];
                }
                // The network does not always report the request headers; the page's own stand in.
                if c.referer.is_none() {
                    c.referer = self.page.clone();
                }
                if c.ua.is_none() && !self.ua.is_empty() {
                    c.ua = Some(self.ua.clone());
                }
                let size = v["size"].as_u64();
                let line = self.line("media", &c, size);
                self.seen.push(Seen { cand: c, page: false, size });
                let mut e = vec![Effect::Print(line)];
                if !self.shown {
                    self.shown = true;
                    e.push(Effect::Eval(SHOW));
                }
                e
            }
            Some("play") if !self.shown => {
                self.shown = true;
                vec![Effect::Print(json!({"kind": "play"}).to_string()), Effect::Eval(SHOW)]
            }
            Some("download") => {
                let list = self.ranked();
                if list.is_empty() {
                    return vec![];
                }
                let c: Vec<Value> = list.iter().map(|c| json!({"url": c.url, "referer": c.referer, "ua": c.ua})).collect();
                let line = json!({"kind": "download", "title": self.title, "candidates": c}).to_string();
                vec![Effect::Print(line), Effect::Eval(SENT)]
            }
            _ => vec![],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(kind: &str, url: &str) -> String {
        json!({"kind": kind, "url": url, "referer": "https://site.test/p", "ua": "UA"}).to_string()
    }
    fn page(url: &str, title: &str) -> String {
        json!({"kind": "page", "url": url, "referer": url, "ua": "UA", "title": title}).to_string()
    }
    fn shows(e: &[Effect]) -> usize {
        e.iter().filter(|x| matches!(x, Effect::Eval(js) if *js == SHOW)).count()
    }
    fn urls(d: &Detector) -> Vec<String> {
        d.ranked().into_iter().map(|c| c.url).collect()
    }
    fn printed(e: &[Effect]) -> Vec<Value> {
        e.iter()
            .filter_map(|x| if let Effect::Print(l) = x { serde_json::from_str(l).ok() } else { None })
            .collect()
    }

    #[test]
    fn kinds_are_told_apart() {
        assert_eq!(kind_of("https://a.com/v/master.m3u8?t=1", false), Kind::Hls);
        assert_eq!(kind_of("https://a.com/x/manifest.MPD", false), Kind::Dash);
        assert_eq!(kind_of("https://a.com/v.ism/manifest", false), Kind::Smooth);
        assert_eq!(kind_of("https://a.com/clip.mp4", false), Kind::File);
        assert_eq!(kind_of("https://www.dailymotion.com/video/x8abc12", false), Kind::Embed);
        assert_eq!(kind_of("https://a.com/watch.m3u8", true), Kind::Page);
        assert!(Kind::Hls.named_by_page() && Kind::File.named_by_page());
        assert!(!Kind::Page.named_by_page() && !Kind::Embed.named_by_page());
    }

    #[test]
    fn first_play_is_announced_once() {
        let mut d = Detector::default();
        d.on_message(&page("https://site.test/p", "T"));
        let e = d.on_message(&json!({"kind": "play"}).to_string());
        assert_eq!(printed(&e)[0]["kind"], "play");
        assert_eq!(shows(&e), 1);
        assert!(d.on_message(&json!({"kind": "play"}).to_string()).is_empty());
        // A stream found first already announced itself: no extra line for the play.
        let mut d = Detector::default();
        d.on_message(&page("https://site.test/p", "T"));
        d.on_message(&msg("media", "https://a.com/v.m3u8"));
        assert!(d.on_message(&json!({"kind": "play"}).to_string()).is_empty());
    }

    #[test]
    fn network_media_borrow_the_pages_headers_and_pass_the_size() {
        let mut d = Detector::default();
        d.on_message(&page("https://site.test/p", "T"));
        // What the browser's network layer reports when the request headers are unknown.
        let e = d.on_message(&json!({"kind": "media", "url": "https://cdn.test/v.mp4", "size": 5_000_000}).to_string());
        let l = &printed(&e)[0];
        assert_eq!(l["referer"], "https://site.test/p");
        assert_eq!(l["ua"], "UA");
        assert_eq!(l["size"], 5_000_000);
        let c = &d.ranked()[1];
        assert_eq!((c.referer.as_deref(), c.ua.as_deref()), (Some("https://site.test/p"), Some("UA")));
    }

    #[test]
    fn video_requested_just_before_the_new_address_is_noticed_is_kept() {
        let mut d = Detector::default();
        d.on_message(&page("https://site.test/1", "One"));
        d.on_message(&msg("media", "https://a.com/old.mp4"));
        // The player of the next page starts loading; its request already carries the new address.
        let early = json!({"kind": "media", "url": "https://a.com/next.m3u8", "referer": "https://site.test/2", "ua": "UA"}).to_string();
        d.on_message(&early);
        let e = d.on_message(&page("https://site.test/2", "Two"));
        let kinds: Vec<String> = printed(&e).iter().map(|l| l["kind"].as_str().unwrap().to_string()).collect();
        assert_eq!(kinds, ["page", "media"]);
        assert_eq!(shows(&e), 1);
        assert_eq!(urls(&d), ["https://site.test/2", "https://a.com/next.m3u8"]);
    }

    #[test]
    fn a_page_cannot_make_the_detector_grow_without_end() {
        let mut d = Detector::default();
        d.on_message(&page("https://site.test/p", "T"));
        for i in 0..1000 {
            d.on_message(&msg("media", &format!("https://a.com/{i}.mp4")));
        }
        assert_eq!(d.seen.len(), MAX_SEEN);
        // The ranking still hands out at most MAX_TRIES ways.
        assert_eq!(d.ranked().len(), MAX_TRIES);
    }

    #[test]
    fn button_appears_once_on_play_or_media() {
        let mut d = Detector::default();
        d.on_message(&page("https://site.test/p", "T"));
        assert_eq!(shows(&d.on_message(&msg("play", "https://site.test/p"))), 1);
        assert_eq!(shows(&d.on_message(&msg("play", "https://site.test/p"))), 0);
        assert_eq!(shows(&d.on_message(&msg("media", "https://a.com/v.mp4"))), 0);

        let mut d = Detector::default();
        d.on_message(&page("https://site.test/p", "T"));
        assert_eq!(shows(&d.on_message(&msg("media", "https://a.com/v.m3u8"))), 1);
    }

    #[test]
    fn ranking_known_then_page_then_manifest_then_file() {
        let mut d = Detector::default();
        d.on_message(&page("https://site.test/p", "T"));
        d.on_message(&msg("media", "https://a.com/clip.mp4"));
        d.on_message(&msg("media", "https://a.com/master.m3u8?t=1"));
        d.on_message(&msg("media", "https://www.dailymotion.com/video/x8abc12"));
        assert_eq!(
            urls(&d),
            [
                "https://www.dailymotion.com/video/x8abc12",
                "https://site.test/p",
                "https://a.com/master.m3u8?t=1",
                "https://a.com/clip.mp4",
            ]
        );
    }

    #[test]
    fn new_page_resets_and_hides() {
        let mut d = Detector::default();
        d.on_message(&page("https://site.test/1", "One"));
        d.on_message(&msg("media", "https://a.com/old.mp4"));
        let e = d.on_message(&page("https://site.test/2", "Two"));
        assert!(e.iter().any(|x| matches!(x, Effect::Eval(js) if *js == HIDE)));
        assert_eq!(urls(&d), ["https://site.test/2"]);
        assert!(d.on_message(&page("https://site.test/2", "Two")).is_empty());
    }

    #[test]
    fn media_lines_carry_the_page_title_and_it_can_change() {
        let mut d = Detector::default();
        let e = d.on_message(&page("https://site.test/p", ""));
        assert_eq!(printed(&e)[0]["title"], "");
        // The site sets its title late; later media get the new one.
        let e = d.on_message(&json!({"kind": "title", "title": "Lezione 3"}).to_string());
        assert_eq!(printed(&e)[0]["kind"], "title");
        assert!(d.on_message(&json!({"kind": "title", "title": "Lezione 3"}).to_string()).is_empty());
        let e = d.on_message(&msg("media", "https://a.com/v.m3u8"));
        let l = &printed(&e)[0];
        assert_eq!((l["kind"].as_str(), l["title"].as_str()), (Some("media"), Some("Lezione 3")));
        let e = d.on_message(&json!({"kind": "download"}).to_string());
        assert_eq!(printed(&e)[0]["title"], "Lezione 3");
    }

    #[test]
    fn fragment_does_not_make_a_new_candidate() {
        let mut d = Detector::default();
        d.on_message(&page("https://site.test/p", "T"));
        d.on_message(&msg("media", "https://a.com/v.mp4"));
        assert!(d.on_message(&msg("media", "https://a.com/v.mp4#t=5")).is_empty());
        assert_eq!(d.ranked().len(), 2);
    }

    #[test]
    fn download_line_is_capped_and_deduplicated() {
        let mut d = Detector::default();
        d.on_message(&page("https://site.test/p", "T"));
        for i in 0..10 {
            d.on_message(&msg("media", &format!("https://a.com/v{i}.mp4")));
        }
        d.on_message(&msg("media", "https://a.com/v1.mp4"));
        assert_eq!(d.ranked().len(), MAX_TRIES);
        let e = d.on_message(&json!({"kind": "download"}).to_string());
        let Effect::Print(line) = &e[0] else { panic!("expected a line") };
        let v: Value = serde_json::from_str(line).unwrap();
        assert_eq!(v["kind"], "download");
        assert_eq!(v["candidates"].as_array().unwrap().len(), MAX_TRIES);
        assert_eq!(v["candidates"][0]["url"], "https://site.test/p");
    }
}
