//! One way to fetch a video. Kept apart so the detector, which the browser process shares with the
//! main program, does not drag the download code along.

/// One way to fetch a video: an address, the headers the page used and, for plain streams, a file name.
#[derive(Clone, Debug, PartialEq)]
pub struct Cand {
    pub url: String,
    pub referer: Option<String>,
    pub ua: Option<String>,
    pub title: Option<String>,
}
