// Decides from a response's headers whether a request fetched a video, an audio file or a streaming
// manifest. The built-in browser feeds it every response WebView2 receives, the way the extension looks
// at every response its browser reports, so detection does not depend on what the page's script does.

/// What the browser knows about one response.
pub struct Resp<'a> {
    pub url: &'a str,
    pub status: i32,
    pub content_type: &'a str,
    pub content_length: Option<u64>,
    /// Raw Content-Range header, for partial responses.
    pub content_range: &'a str,
}

/// Smaller than this is a tracking clip or an icon, not something to save.
const MIN_BYTES: u64 = 64 * 1024;

const MANIFEST_TYPES: [&str; 6] = [
    "application/vnd.apple.mpegurl",
    "application/x-mpegurl",
    "audio/mpegurl",
    "audio/x-mpegurl",
    "application/dash+xml",
    "application/vnd.ms-sstr+xml",
];

const FILE_EXTS: [&str; 7] = ["mp4", "webm", "m4v", "mov", "mkv", "flv", "ogv"];

/// Path of the address without query and fragment, lower case.
fn path_of(url: &str) -> String {
    let end = url.find(['?', '#']).unwrap_or(url.len());
    url[..end].to_lowercase()
}

fn ext_of(url: &str) -> String {
    let p = path_of(url);
    let name = p.rsplit('/').next().unwrap_or("");
    name.rsplit_once('.')
        .map(|(_, e)| e.to_string())
        .unwrap_or_default()
}

/// A piece of a stream (an HLS or DASH chunk), never worth offering on its own.
pub fn is_segment(url: &str) -> bool {
    let p = path_of(url);
    if [".ts", ".m4s", ".m4f", ".m4a", ".aac", ".cmfv", ".cmfa"]
        .iter()
        .any(|e| p.ends_with(e))
    {
        return true;
    }
    let u = url.to_lowercase();
    ["seg", "segment", "chunk", "frag", "fragment"]
        .iter()
        .any(|w| {
            u.match_indices(w).any(|(i, _)| {
                let before_ok = i == 0 || matches!(u.as_bytes()[i - 1], b'/' | b'_' | b'.' | b'-');
                let rest = &u.as_bytes()[i + w.len()..];
                let rest = match rest.first() {
                    Some(b'-' | b'_' | b'.') => &rest[1..],
                    _ => rest,
                };
                before_ok && rest.first().is_some_and(u8::is_ascii_digit)
            })
        })
}

/// Total size of the file: from Content-Range when the answer is a slice, else from Content-Length.
pub fn total_size(r: &Resp) -> Option<u64> {
    let cr = r.content_range.trim();
    if let Some(total) = cr.rsplit('/').next().filter(|_| cr.contains('/')) {
        if let Ok(n) = total.trim().parse::<u64>() {
            return Some(n);
        }
    }
    r.content_length
}

/// The line the detector expects for a response that fetched media: the address, the headers the request
/// carried (a download needs the same ones) and the total size when it is known.
pub fn media_json(r: &Resp, referer: &str, ua: &str) -> String {
    serde_json::json!({"kind": "media", "url": r.url, "referer": referer, "ua": ua, "size": total_size(r)})
        .to_string()
}

pub fn is_media(r: &Resp) -> bool {
    if !(r.status == 200 || r.status == 206) {
        return false;
    }
    let u = r.url.to_lowercase();
    if !(u.starts_with("http://") || u.starts_with("https://")) || is_segment(r.url) {
        return false;
    }
    let ct = r
        .content_type
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_lowercase();
    let ext = ext_of(r.url);

    // Manifests: by type, or by name when the server calls them plain text.
    if MANIFEST_TYPES.contains(&ct.as_str())
        || ext == "m3u8"
        || ext == "mpd"
        || path_of(r.url).ends_with(".ism/manifest")
    {
        return true;
    }
    let size_ok = total_size(r).is_none_or(|n| n >= MIN_BYTES);
    let media_type = (ct.starts_with("video/") && ct != "video/mp2t" && ct != "video/iso.segment")
        || ct.starts_with("audio/");
    let untyped = ct.is_empty() || ct == "application/octet-stream" || ct == "binary/octet-stream";
    (media_type || (untyped && FILE_EXTS.contains(&ext.as_str()))) && size_ok
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r<'a>(url: &'a str, ct: &'a str, len: Option<u64>) -> Resp<'a> {
        Resp {
            url,
            status: 200,
            content_type: ct,
            content_length: len,
            content_range: "",
        }
    }

    #[test]
    fn the_line_for_the_detector_carries_address_headers_and_size() {
        let mut resp = r("https://a.com/v.mp4", "video/mp4", Some(10));
        resp.content_range = "bytes 0-9/5000000";
        let v: serde_json::Value =
            serde_json::from_str(&media_json(&resp, "https://a.com/page", "UA/1")).unwrap();
        assert_eq!(v["kind"], "media");
        assert_eq!(v["url"], "https://a.com/v.mp4");
        assert_eq!(v["referer"], "https://a.com/page");
        assert_eq!(v["ua"], "UA/1");
        assert_eq!(
            v["size"], 5_000_000,
            "the total from Content-Range, not the slice"
        );
    }

    #[test]
    fn segments_are_recognised() {
        for u in [
            "https://a.com/v/seg-12.ts",
            "https://a.com/v/chunk_3.m4s?x=1",
            "https://a.com/v/segment1.mp4",
            "https://a.com/v/video.frag.7",
            "https://a.com/v/init.cmfv",
            "https://a.com/audio/a.aac",
        ] {
            assert!(is_segment(u), "{u}");
        }
        for u in [
            "https://a.com/video.mp4",
            "https://a.com/master.m3u8",
            "https://a.com/segway.mp4",
            "https://a.com/movie2.webm",
        ] {
            assert!(!is_segment(u), "{u}");
        }
    }

    #[test]
    fn manifests_are_found_by_type_or_name() {
        assert!(is_media(&r(
            "https://a.com/api/play?id=1",
            "application/vnd.apple.mpegurl",
            None
        )));
        assert!(is_media(&r(
            "https://a.com/x",
            "application/dash+xml; charset=utf-8",
            Some(900)
        )));
        assert!(is_media(&r(
            "https://a.com/live/index.m3u8?t=1",
            "text/plain",
            Some(300)
        )));
        assert!(is_media(&r(
            "https://a.com/x/manifest.MPD",
            "application/xml",
            None
        )));
        assert!(is_media(&r(
            "https://a.com/v.ism/manifest",
            "text/xml",
            None
        )));
    }

    #[test]
    fn files_need_a_media_type_and_a_real_size() {
        assert!(is_media(&r(
            "https://a.com/stream?id=2",
            "video/mp4",
            Some(5_000_000)
        )));
        assert!(is_media(&r(
            "https://a.com/song",
            "audio/mpeg",
            Some(3_000_000)
        )));
        assert!(is_media(&r(
            "https://a.com/clip.mp4",
            "application/octet-stream",
            Some(2_000_000)
        )));
        assert!(is_media(&r("https://a.com/clip.webm", "", None))); // size unknown: still offered
        assert!(!is_media(&r(
            "https://a.com/pixel.mp4",
            "video/mp4",
            Some(900)
        )));
        assert!(!is_media(&r(
            "https://a.com/data.bin",
            "application/octet-stream",
            Some(5_000_000)
        )));
        assert!(!is_media(&r(
            "https://a.com/logo.png",
            "image/png",
            Some(5_000_000)
        )));
        assert!(!is_media(&r(
            "https://a.com/part1.bin",
            "video/mp2t",
            Some(5_000_000)
        )));
        assert!(!is_media(&r(
            "https://a.com/seg-12.ts",
            "video/mp2t",
            Some(5_000_000)
        )));
        assert!(!is_media(&r(
            "https://a.com/chunk_3.m4s",
            "video/mp4",
            Some(5_000_000)
        )));
    }

    #[test]
    fn only_successful_web_responses_count() {
        let mut x = r("https://a.com/v.mp4", "video/mp4", Some(5_000_000));
        x.status = 404;
        assert!(!is_media(&x));
        x.status = 206;
        assert!(is_media(&x));
        assert!(!is_media(&r(
            "blob:https://a.com/1234",
            "video/mp4",
            Some(5_000_000)
        )));
        assert!(!is_media(&r(
            "data:video/mp4;base64,AAAA",
            "video/mp4",
            Some(5_000_000)
        )));
    }

    #[test]
    fn slices_report_the_whole_file() {
        let x = Resp {
            url: "https://a.com/v.mp4",
            status: 206,
            content_type: "video/mp4",
            content_length: Some(1024),
            content_range: "bytes 0-1023/52428800",
        };
        assert_eq!(total_size(&x), Some(52_428_800));
        assert!(is_media(&x)); // the first 1 KB slice must not make it look tiny
        let y = Resp {
            content_range: "bytes 0-1023/*",
            ..x
        };
        assert_eq!(total_size(&y), Some(1024));
    }
}
