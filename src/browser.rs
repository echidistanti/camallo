// Built-in browser: a separate window (own process) that reports media URLs on stdout, one JSON per line.
//
// Media are found two ways. The injected script sees what the page asks for and what it plays. The network
// listener sees every response WebView2 receives, from any frame, and judges it by its headers, as the
// extension does with its browser's request events. Both feed the same detector.

use std::path::Path;
use std::process::Command;

const SCRIPT: &str = include_str!("detect.js");

/// What to tell the user when WebKitGTK is not on the system (Linux).
const WEBKIT_MISSING: &str = "The built-in browser needs WebKitGTK, which this system does not have. Install webkit2gtk4.1 (Fedora), libwebkit2gtk-4.1-0 (Ubuntu, Debian) or webkit2gtk-4.1 (Arch), then open the browser again. Everything else works without it.";

/// The command that opens the browser window on `url`, or what is missing. On Windows the browser is this
/// program started with `--browser`. On Linux it is `camallo-browser`, next to this program, which keeps
/// WebKitGTK out of the main program: Camallo starts on a system that does not have it.
pub fn command(url: &str, profile: &Path) -> Result<Command, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    #[cfg(windows)]
    {
        let _ = profile; // the browser finds its own folder, see `run`
        let mut c = Command::new(exe);
        c.args(["--browser", url]);
        Ok(c)
    }
    #[cfg(not(windows))]
    {
        // A development aid (see `run`): this program replays a recorded feed itself.
        if std::env::var_os("CAMALLO_FAKE_FEED").is_some() {
            let mut c = Command::new(exe);
            c.args(["--browser", url]);
            return Ok(c);
        }
        let helper = exe.with_file_name("camallo-browser");
        if !helper.is_file() {
            return Err(format!("{} is missing", helper.display()));
        }
        let mut c = Command::new(helper);
        c.arg(url).arg(profile);
        Ok(c)
    }
}

/// What to tell the user when the browser program ended soon after it started, from its exit code and what it
/// printed on stderr. None when it ended well, or when it was stopped on purpose (a signal gives no exit code).
pub fn failure(code: Option<i32>, stderr: &str) -> Option<String> {
    match code? {
        0 => None,
        _ if stderr.contains("libwebkit2gtk") || stderr.contains("libgtk-3") => {
            Some(WEBKIT_MISSING.into())
        }
        _ => {
            let last = stderr
                .lines()
                .rev()
                .map(str::trim)
                .find(|l| !l.is_empty())
                .unwrap_or("no message");
            let last: String = last.chars().take(120).collect();
            Some(format!("The built-in browser could not start: {last}"))
        }
    }
}

/// What the network layer saw, as the line the detector expects.
#[cfg(windows)]
fn describe(
    args: Option<webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2WebResourceResponseReceivedEventArgs>,
) -> Option<String> {
    use crate::net::{Resp, is_media, media_json};
    use webview2_com::Microsoft::Web::WebView2::Win32::{
        ICoreWebView2HttpRequestHeaders, ICoreWebView2HttpResponseHeaders,
    };
    use webview2_com::take_pwstr;
    use windows::core::{PCWSTR, PWSTR, w};

    fn from_response(h: &ICoreWebView2HttpResponseHeaders, name: PCWSTR) -> String {
        let mut v = PWSTR::null();
        if unsafe { h.GetHeader(name, &mut v) }.is_err() || v.is_null() {
            String::new()
        } else {
            take_pwstr(v)
        }
    }
    fn from_request(h: &ICoreWebView2HttpRequestHeaders, name: PCWSTR) -> String {
        let mut v = PWSTR::null();
        if unsafe { h.GetHeader(name, &mut v) }.is_err() || v.is_null() {
            String::new()
        } else {
            take_pwstr(v)
        }
    }

    let args = args?;
    let req = unsafe { args.Request() }.ok()?;
    let resp = unsafe { args.Response() }.ok()?;
    let mut uri = PWSTR::null();
    unsafe { req.Uri(&mut uri) }.ok()?;
    if uri.is_null() {
        return None;
    }
    let url = take_pwstr(uri);
    let mut status = 0i32;
    unsafe { resp.StatusCode(&mut status) }.ok()?;
    let h = unsafe { resp.Headers() }.ok()?;
    let content_type = from_response(&h, w!("Content-Type"));
    let content_range = from_response(&h, w!("Content-Range"));
    let content_length = from_response(&h, w!("Content-Length"))
        .trim()
        .parse::<u64>()
        .ok();
    let r = Resp {
        url: &url,
        status,
        content_type: &content_type,
        content_length,
        content_range: &content_range,
    };
    if !is_media(&r) {
        return None;
    }
    let rh = unsafe { req.Headers() }.ok();
    let referer = rh
        .as_ref()
        .map(|h| from_request(h, w!("Referer")))
        .unwrap_or_default();
    let ua = rh
        .as_ref()
        .map(|h| from_request(h, w!("User-Agent")))
        .unwrap_or_default();
    Some(media_json(&r, &referer, &ua))
}

/// Reports every media response to the event loop. If WebView2 is too old to offer the event, only the script works.
#[cfg(windows)]
fn listen_to_network(view: &wry::WebView, proxy: tao::event_loop::EventLoopProxy<String>) {
    use webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2_2;
    use webview2_com::WebResourceResponseReceivedEventHandler;
    use windows::core::Interface;
    use wry::WebViewExtWindows;

    let Ok(wv) = view.webview().cast::<ICoreWebView2_2>() else {
        return;
    };
    let mut token = 0i64;
    let handler = WebResourceResponseReceivedEventHandler::create(Box::new(move |_, args| {
        if let Some(line) = describe(args) {
            let _ = proxy.send_event(line);
        }
        Ok(())
    }));
    let _ = unsafe { wv.add_WebResourceResponseReceived(&handler, &mut token) };
}

#[cfg(windows)]
pub fn run(url: String) {
    use crate::detector::{Detector, Effect};
    use tao::event::{Event, WindowEvent};
    use tao::event_loop::{ControlFlow, EventLoopBuilder};
    use tao::window::WindowBuilder;
    use wry::{WebContext, WebViewBuilder};

    // Messages from the page (any frame) and from the network reach the event loop, which owns the webview and the detector.
    let ev = EventLoopBuilder::<String>::with_user_event().build();
    let proxy = ev.create_proxy();
    let mut window = WindowBuilder::new().with_title("Camallo Browser");
    if let Some((rgba, w, h)) = crate::icon::rgba() {
        window = window.with_window_icon(tao::window::Icon::from_rgba(rgba, w, h).ok());
    }
    let win = window.build(&ev).expect("window");
    let page_proxy = proxy.clone();
    // The browser's own profile (cookies, logins) lives with the rest of Camallo's data. By default WebView2 puts it
    // next to the program, which fails in a folder that cannot be written and travels with a copy of the program.
    let mut web_context = WebContext::new(Some(crate::deps::data_dir().join("webview")));
    let view = WebViewBuilder::new_with_web_context(&mut web_context)
        .with_url(&url)
        .with_initialization_script(SCRIPT)
        .with_ipc_handler(move |req| {
            let _ = page_proxy.send_event(req.body().clone());
        })
        .build(&win)
        .expect("WebView2 not available");
    listen_to_network(&view, proxy);
    let mut det = Detector::default();
    ev.run(move |event, _, flow| {
        *flow = ControlFlow::Wait;
        match event {
            Event::UserEvent(msg) => {
                for e in det.on_message(&msg) {
                    match e {
                        Effect::Print(line) => println!("{line}"),
                        Effect::Eval(js) => {
                            let _ = view.evaluate_script(js);
                        }
                    }
                }
            }
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => *flow = ControlFlow::Exit,
            _ => {}
        }
    });
}

#[cfg(not(windows))]
pub fn run(_url: String) {
    let _ = SCRIPT;
    // Development aid, the only thing this build can do: replay messages a page would send (one JSON per
    // line of the file named by CAMALLO_FAKE_FEED) through the detector, to try the main window without Windows.
    if let Ok(path) = std::env::var("CAMALLO_FAKE_FEED") {
        use crate::detector::{Detector, Effect};
        let mut det = Detector::default();
        for line in std::fs::read_to_string(path).unwrap_or_default().lines() {
            for e in det.on_message(line) {
                if let Effect::Print(l) = e {
                    println!("{l}");
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(1500));
        }
        std::thread::sleep(std::time::Duration::from_secs(600));
        return;
    }
    eprintln!("On Linux the built-in browser is the camallo-browser program, started by Camallo.");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_browser_that_ended_well_or_was_stopped_says_nothing() {
        assert_eq!(failure(Some(0), "any noise"), None);
        assert_eq!(failure(None, "killed by Camallo"), None);
    }

    #[test]
    fn a_missing_library_names_the_packages() {
        let said = "camallo-browser: error while loading shared libraries: libwebkit2gtk-4.1.so.0: cannot open shared object file: No such file or directory\n";
        let m = failure(Some(127), said).unwrap();
        for pkg in ["webkit2gtk4.1", "libwebkit2gtk-4.1-0", "webkit2gtk-4.1"] {
            assert!(m.contains(pkg), "{m}");
        }
    }

    #[test]
    fn any_other_failure_shows_the_last_line_it_printed() {
        let m = failure(Some(101), "thread 'main' panicked\nno display\n\n").unwrap();
        assert_eq!(m, "The built-in browser could not start: no display");
        let m = failure(Some(1), "").unwrap();
        assert!(m.ends_with("no message"));
        let long = "x".repeat(500);
        assert!(failure(Some(1), &long).unwrap().chars().count() < 200);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn without_the_helper_next_to_it_the_browser_says_what_is_missing() {
        // The test program lives where there is no camallo-browser.
        let e = command("https://example.com", Path::new("/tmp/p")).unwrap_err();
        assert!(
            e.contains("camallo-browser") && e.contains("missing"),
            "{e}"
        );
    }
}
