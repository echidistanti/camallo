//! The window of the built-in browser on Linux, as a program of its own: `camallo-browser <address> <profile folder>`.
//! The main program starts it next to itself. It is separate so that Camallo starts on a system without
//! WebKitGTK, and says what to install when the browser is asked for.
//!
//! It shares the detector with the main program rather than copying it.

#[cfg(target_os = "linux")]
#[allow(dead_code)]
#[path = "../cand.rs"]
mod cand;
#[cfg(target_os = "linux")]
#[allow(dead_code)]
#[path = "../detector.rs"]
mod detector;
#[cfg(target_os = "linux")]
#[allow(dead_code)]
#[path = "../icon.rs"]
mod icon;
#[cfg(target_os = "linux")]
#[allow(dead_code)]
#[path = "../net.rs"]
mod net;
#[cfg(target_os = "linux")]
#[path = "../webkit.rs"]
mod webkit;

#[cfg(target_os = "linux")]
fn main() {
    let mut args = std::env::args().skip(1);
    let url = args
        .next()
        .unwrap_or_else(|| "https://duckduckgo.com".into());
    let profile = args
        .next()
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("camallo-webview"));
    webkit::run(url, profile);
}

/// On Windows the browser is the main program itself, started with `--browser`.
#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!(
        "camallo-browser is the Linux browser; on this system run camallo --browser <address>."
    );
}
