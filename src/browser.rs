// Built-in browser: a separate window (own process) that reports media URLs on stdout, one JSON per line.
//
// Media are found two ways. The injected script sees what the page asks for and what it plays. The network
// listener sees every response WebView2 receives, from any frame, and judges it by its headers, as the
// extension does with its browser's request events. Both feed the same detector.

const SCRIPT: &str = include_str!("detect.js");

/// What the network layer saw, as the line the detector expects.
#[cfg(windows)]
fn describe(
    args: Option<webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2WebResourceResponseReceivedEventArgs>,
) -> Option<String> {
    use crate::net::{is_media, total_size, Resp};
    use webview2_com::take_pwstr;
    use webview2_com::Microsoft::Web::WebView2::Win32::{ICoreWebView2HttpRequestHeaders, ICoreWebView2HttpResponseHeaders};
    use windows::core::{w, PCWSTR, PWSTR};

    fn from_response(h: &ICoreWebView2HttpResponseHeaders, name: PCWSTR) -> String {
        let mut v = PWSTR::null();
        if unsafe { h.GetHeader(name, &mut v) }.is_err() || v.is_null() { String::new() } else { take_pwstr(v) }
    }
    fn from_request(h: &ICoreWebView2HttpRequestHeaders, name: PCWSTR) -> String {
        let mut v = PWSTR::null();
        if unsafe { h.GetHeader(name, &mut v) }.is_err() || v.is_null() { String::new() } else { take_pwstr(v) }
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
    let content_length = from_response(&h, w!("Content-Length")).trim().parse::<u64>().ok();
    let r = Resp { url: &url, status, content_type: &content_type, content_length, content_range: &content_range };
    if !is_media(&r) {
        return None;
    }
    let rh = unsafe { req.Headers() }.ok();
    let referer = rh.as_ref().map(|h| from_request(h, w!("Referer"))).unwrap_or_default();
    let ua = rh.as_ref().map(|h| from_request(h, w!("User-Agent"))).unwrap_or_default();
    Some(serde_json::json!({"kind": "media", "url": url, "referer": referer, "ua": ua, "size": total_size(&r)}).to_string())
}

/// Reports every media response to the event loop. If WebView2 is too old to offer the event, only the script works.
#[cfg(windows)]
fn listen_to_network(view: &wry::WebView, proxy: tao::event_loop::EventLoopProxy<String>) {
    use webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2_2;
    use webview2_com::WebResourceResponseReceivedEventHandler;
    use windows::core::Interface;
    use wry::WebViewExtWindows;

    let Ok(wv) = view.webview().cast::<ICoreWebView2_2>() else { return };
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
    use wry::WebViewBuilder;

    // Messages from the page (any frame) and from the network reach the event loop, which owns the webview and the detector.
    let ev = EventLoopBuilder::<String>::with_user_event().build();
    let proxy = ev.create_proxy();
    let win = WindowBuilder::new().with_title("Camallo Browser").build(&ev).expect("window");
    let page_proxy = proxy.clone();
    let view = WebViewBuilder::new()
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
            Event::WindowEvent { event: WindowEvent::CloseRequested, .. } => *flow = ControlFlow::Exit,
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
    eprintln!("The built-in browser needs Windows (WebView2).");
}
