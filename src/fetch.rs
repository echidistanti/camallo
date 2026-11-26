//! What yt-dlp says about an address the user added: one video, or a list of them.
use crate::job::{Cand, valid_url};
use crate::probe::{Info, parse_info, pick_thumb, run_json};
use serde_json::Value;
use std::path::Path;

/// Most videos taken from one address; longer lists are cut here.
pub const MAX_ENTRIES: usize = 200;

#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub url: String,
    pub title: String,
    pub duration: Option<u64>,
    pub thumb: Option<String>,
    /// Present when yt-dlp already gave the formats, so no second request is needed.
    pub info: Option<Info>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Fetched {
    Video(Info),
    List { title: String, entries: Vec<Entry> },
}

fn entry_of(e: &Value) -> Option<Entry> {
    // A video that is no longer available comes as null.
    let url = ["webpage_url", "url"]
        .iter()
        .filter_map(|k| e[k].as_str())
        .find_map(valid_url)?;
    Some(Entry {
        url,
        title: e["title"].as_str().unwrap_or_default().trim().to_string(),
        duration: e["duration"].as_f64().map(|d| d as u64),
        thumb: pick_thumb(e),
        info: e["formats"]
            .as_array()
            .filter(|f| !f.is_empty())
            .map(|_| parse_info(e)),
    })
}

/// Videos of a list in order; a list inside a list (the tabs of a channel) is opened too.
fn collect(v: &Value, out: &mut Vec<Entry>) {
    for e in v["entries"].as_array().into_iter().flatten() {
        if out.len() >= MAX_ENTRIES {
            return;
        }
        if e["entries"].is_array() {
            collect(e, out);
        } else if let Some(x) = entry_of(e) {
            out.push(x);
        }
    }
}

/// Reads the output of `yt-dlp -J --flat-playlist`.
pub fn parse(v: &Value) -> Result<Fetched, String> {
    if v["_type"].as_str() != Some("playlist") && !v["entries"].is_array() {
        return Ok(Fetched::Video(parse_info(v)));
    }
    let mut entries = vec![];
    collect(v, &mut entries);
    if entries.is_empty() {
        return Err("The list has no videos yt-dlp can read".into());
    }
    let title = v["title"]
        .as_str()
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .unwrap_or("Playlist")
        .to_string();
    Ok(Fetched::List { title, entries })
}

/// Asks yt-dlp about an address: a video comes back whole, a list as its entries only.
pub fn run(yt: &Path, c: &Cand) -> Result<Fetched, String> {
    let end = MAX_ENTRIES.to_string();
    parse(&run_json(
        yt,
        c,
        &["--flat-playlist", "--playlist-end", &end],
    )?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_video_is_not_a_list() {
        let v = json!({"title": "Solo", "duration": 61.0, "formats": [{"vcodec": "avc1", "acodec": "aac", "height": 720}]});
        match parse(&v).unwrap() {
            Fetched::Video(i) => assert_eq!((i.title.as_str(), i.duration), ("Solo", Some(61))),
            other => panic!("expected a video, got {other:?}"),
        }
    }

    #[test]
    fn flat_playlist_gives_entries_in_order() {
        let v = json!({
            "_type": "playlist", "title": " Corso di media ",
            "entries": [
                {"_type": "url", "url": "https://www.youtube.com/watch?v=aaa", "title": "Uno", "duration": 60.0,
                 "thumbnails": [{"url": "https://i/a.jpg", "width": 480}]},
                null,
                {"url": "javascript:alert(1)", "title": "Cattivo"},
                {"_type": "url", "url": "https://www.youtube.com/watch?v=bbb", "title": "Due"}
            ]
        });
        let Fetched::List { title, entries } = parse(&v).unwrap() else {
            panic!("expected a list")
        };
        assert_eq!(title, "Corso di media");
        let urls: Vec<&str> = entries.iter().map(|e| e.url.as_str()).collect();
        assert_eq!(
            urls,
            [
                "https://www.youtube.com/watch?v=aaa",
                "https://www.youtube.com/watch?v=bbb"
            ]
        );
        assert_eq!(
            (entries[0].title.as_str(), entries[0].duration),
            ("Uno", Some(60))
        );
        assert_eq!(entries[0].thumb.as_deref(), Some("https://i/a.jpg"));
        assert!(
            entries[0].info.is_none(),
            "a flat entry still needs its own request"
        );
    }

    #[test]
    fn entries_with_formats_need_no_second_request() {
        let v = json!({
            "_type": "playlist", "title": "Pagina",
            "entries": [{"webpage_url": "https://site.test/p#1", "url": "https://cdn/x.mp4", "title": "A",
                         "formats": [{"vcodec": "avc1", "acodec": "aac", "height": 480, "filesize": 10}]}]
        });
        let Fetched::List { entries, .. } = parse(&v).unwrap() else {
            panic!()
        };
        assert_eq!(
            entries[0].url, "https://site.test/p#1",
            "the page wins over the stream"
        );
        let info = entries[0].info.as_ref().expect("formats were there");
        assert_eq!(info.qualities.len(), 1);
    }

    #[test]
    fn lists_inside_lists_are_opened_and_capped() {
        let tab = |n: usize, p: &str| json!({"_type": "playlist", "entries": (0..n).map(|i| json!({"url": format!("https://v.test/{p}{i}"), "title": format!("{p}{i}")})).collect::<Vec<_>>()});
        let v = json!({"_type": "playlist", "title": "Canale", "entries": [tab(3, "a"), tab(MAX_ENTRIES, "b")]});
        let Fetched::List { entries, .. } = parse(&v).unwrap() else {
            panic!()
        };
        assert_eq!(entries.len(), MAX_ENTRIES);
        assert_eq!(
            (entries[0].title.as_str(), entries[3].title.as_str()),
            ("a0", "b0")
        );
    }

    #[test]
    fn an_empty_list_is_an_error_the_user_can_read() {
        let e = parse(&json!({"_type": "playlist", "entries": [null, {"title": "no address"}]}))
            .unwrap_err();
        assert!(e.contains("no videos"), "{e}");
        assert!(parse(&json!({"_type": "playlist", "entries": []})).is_err());
    }

    /// Runs the real yt-dlp against a local page with two videos. Set CAMALLO_YTDLP and CAMALLO_BASE
    /// (for example http://127.0.0.1:8765) and run with `cargo test -- --ignored real`.
    #[test]
    #[ignore]
    fn real_playlist_page_gives_one_entry_per_video_and_a_single_video_stays_single() {
        let (Ok(yt), Ok(base)) = (
            std::env::var("CAMALLO_YTDLP"),
            std::env::var("CAMALLO_BASE"),
        ) else {
            return;
        };
        let yt = std::path::PathBuf::from(yt);
        let cand = |u: String| Cand {
            url: u,
            referer: None,
            ua: None,
            title: None,
        };
        match run(&yt, &cand(format!("{base}/playlist.html"))).unwrap() {
            Fetched::List { title, entries } => {
                assert_eq!(title, "Corso di prova");
                assert_eq!(entries.len(), 2);
                assert!(
                    entries
                        .iter()
                        .all(|e| e.url.starts_with("http") && e.info.is_some()),
                    "{entries:?}"
                );
            }
            Fetched::Video(i) => panic!("a page with two videos is a list, got {i:?}"),
        }
        match run(&yt, &cand(format!("{base}/hls/master.m3u8"))).unwrap() {
            Fetched::Video(i) => assert_eq!(
                i.qualities.iter().map(|q| q.height).collect::<Vec<_>>(),
                [360, 180]
            ),
            Fetched::List { .. } => panic!("one manifest is one video"),
        }
        assert!(run(&yt, &cand(format!("{base}/missing.mp4"))).is_err());
    }
}
