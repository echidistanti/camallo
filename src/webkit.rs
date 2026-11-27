//! The built-in browser on Linux: a window with WebKitGTK, run by its own program (`camallo-browser`) so that
//! the main program does not need WebKitGTK to start. It reports media addresses on stdout, one JSON per line,
//! exactly as the Windows browser does.
//!
//! Media are found two ways. The injected script sees what the page asks for and what it plays. The network
//! listener sees every response WebKitGTK receives, from any frame, and judges it by its headers.

use crate::detector::{Detector, Effect};
use crate::net::{Resp, is_media, media_json};
use std::path::PathBuf;
use tao::event::{Event, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy};
use tao::platform::unix::WindowExtUnix;
use tao::window::WindowBuilder;
use webkit2gtk::{SettingsExt, URIRequestExt, URIResponseExt, WebResourceExt, WebViewExt};
use wry::{WebContext, WebViewBuilder, WebViewBuilderExtUnix, WebViewExtUnix};

const SCRIPT: &str = include_str!("detect.js");

/// What the network layer saw, as the line the detector expects; None when it was not media.
fn describe(res: &webkit2gtk::WebResource, referer: &str, ua: &str) -> Option<String> {
    let url = res.uri()?.to_string();
    let resp = res.response()?;
    let headers = resp.http_headers();
    let header = |name: &str| {
        headers
            .as_ref()
            .and_then(|h| h.one(name))
            .map(|v| v.to_string())
            .unwrap_or_default()
    };
    let mut content_type = header("Content-Type");
    if content_type.is_empty() {
        content_type = resp.mime_type().map(|m| m.to_string()).unwrap_or_default();
    }
    let content_range = header("Content-Range");
    let content_length = header("Content-Length").trim().parse::<u64>().ok();
    let r = Resp {
        url: &url,
        status: resp.status_code() as i32,
        content_type: &content_type,
        content_length,
        content_range: &content_range,
    };
    is_media(&r).then(|| media_json(&r, referer, ua))
}

/// Reports every media response to the event loop.
fn listen_to_network(view: &wry::WebView, proxy: EventLoopProxy<String>) {
    let wv = view.webview();
    let ua = wv
        .settings()
        .and_then(|s| s.user_agent())
        .map(|u| u.to_string())
        .unwrap_or_default();
    wv.connect_resource_load_started(move |_, resource, request| {
        let referer = request
            .http_headers()
            .and_then(|h| h.one("Referer"))
            .map(|v| v.to_string())
            .unwrap_or_default();
        let (proxy, ua) = (proxy.clone(), ua.clone());
        // The response is not there yet when the load starts: it arrives with this notification.
        resource.connect_response_notify(move |res| {
            if let Some(line) = describe(res, &referer, &ua) {
                let _ = proxy.send_event(line);
            }
        });
    });
}

/// Opens the window on `url` and runs until it is closed. The browser's own profile (cookies, logins) lives in
/// `profile`, with the rest of Camallo's data.
pub fn run(url: String, profile: PathBuf) {
    // The name a desktop matches with camallo.desktop to show Camallo's icon (GTK takes it as the window's app id).
    gtk::glib::set_prgname(Some("camallo"));
    // Messages from the page (any frame) and from the network reach the event loop, which owns the webview and the detector.
    let ev = EventLoopBuilder::<String>::with_user_event().build();
    let proxy = ev.create_proxy();
    let mut window = WindowBuilder::new()
        .with_title("Camallo Browser")
        .with_inner_size(tao::dpi::LogicalSize::new(1100.0, 760.0));
    if let Some((rgba, w, h)) = crate::icon::rgba() {
        window = window.with_window_icon(tao::window::Icon::from_rgba(rgba, w, h).ok());
    }
    let win = window.build(&ev).expect("window");
    let page_proxy = proxy.clone();
    let mut web_context = WebContext::new(Some(profile));
    let builder = WebViewBuilder::new_with_web_context(&mut web_context)
        .with_url(&url)
        .with_initialization_script(SCRIPT)
        .with_ipc_handler(move |req| {
            let _ = page_proxy.send_event(req.body().clone());
        });
    let vbox = win.default_vbox().expect("the window has a GTK box");
    let view = builder.build_gtk(vbox).expect("WebKitGTK not available");
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
                            let _ = view.evaluate_script(&js);
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
