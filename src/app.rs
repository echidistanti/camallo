use crate::deps;
use crate::detector::{kind_of, Kind};
use crate::job::{self, Cand, Ctl, Job, Opts, State};
use crate::probe::{self, human_duration, human_size, Info};
use crate::settings::{self, Container, HistoryItem, Quality, Settings, MAX_PARALLEL};
use crate::ui;
use eframe::egui;
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// A media address the built-in browser has seen on the current page.
#[derive(Clone)]
struct Found {
    kind: Kind,
    url: String,
    referer: String,
    ua: String,
    title: String,
    size: Option<u64>,
}

#[derive(Clone)]
enum ProbeState {
    Loading,
    Ready(Info),
    Failed(String),
}

/// The "Download as..." window.
struct ProbeUi {
    cand: Cand,
    state: ProbeState,
}

struct Shared {
    jobs: Vec<Job>,
    found: Vec<Found>,
    status: String,
    set: Settings,
    history: Vec<HistoryItem>,
    /// Indexes into `jobs`, waiting for a free worker.
    queue: VecDeque<usize>,
    running: usize,
    page_title: String,
    /// A video was seen or started on the current page: the list of candidates is worth showing.
    armed: bool,
    /// yt-dlp and ffmpeg folder once ready, or why they are not.
    deps: Option<Result<(PathBuf, PathBuf), String>>,
    probe: Option<ProbeUi>,
}

impl Shared {
    fn new(set: Settings, history: Vec<HistoryItem>) -> Shared {
        Shared {
            jobs: vec![],
            found: vec![],
            status: "Starting...".into(),
            set,
            history,
            queue: VecDeque::new(),
            running: 0,
            page_title: String::new(),
            armed: false,
            deps: None,
            probe: None,
        }
    }

    /// First sign of a video on this page: flash the window so the new button gets noticed.
    fn arm(&mut self, ctx: &egui::Context) {
        if !self.armed {
            self.armed = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::RequestUserAttention(egui::UserAttentionType::Informational));
        }
    }
}

type Sy = Arc<(Mutex<Shared>, Condvar)>;

fn lock(m: &Mutex<Shared>) -> MutexGuard<'_, Shared> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn queue(sy: &Sy, c: Cand, opts: Option<Opts>, alts: Vec<Cand>) {
    let (m, cv) = &**sy;
    let mut s = lock(m);
    let opts = opts.unwrap_or_else(|| Opts::from_settings(&s.set));
    let dir = s.set.out_dir();
    s.jobs.push(Job::new(c, opts, dir, alts));
    let i = s.jobs.len() - 1;
    s.queue.push_back(i);
    cv.notify_all();
}

/// Gets yt-dlp and ffmpeg ready, then lets the workers start.
fn setup(sy: Sy, ctx: egui::Context) {
    let (m, cv) = &*sy;
    let say = |t: &str| {
        lock(m).status = t.into();
        ctx.request_repaint();
    };
    let r = deps::ensure(&mut |s| say(s));
    let mut g = lock(m);
    match r {
        Ok(d) => {
            g.status = "Ready".into();
            g.deps = Some(Ok((d.yt, d.bin)));
        }
        Err(e) => {
            g.status = format!("Setup failed: {e}");
            g.deps = Some(Err(e));
        }
    }
    cv.notify_all();
    ctx.request_repaint();
}

/// One of up to MAX_PARALLEL threads; the settings decide how many may run at once.
fn worker(sy: Sy, ctx: egui::Context) {
    let (m, cv) = &*sy;
    loop {
        let (i, job, deps) = {
            let g = lock(m);
            let mut g = cv
                .wait_while(g, |s| s.deps.is_none() || s.queue.is_empty() || s.running >= s.set.parallel)
                .unwrap_or_else(|e| e.into_inner());
            let Some(i) = g.queue.pop_front() else { continue };
            if g.jobs[i].state != State::Queued {
                continue; // cancelled while waiting
            }
            g.running += 1;
            g.jobs[i].state = State::Running(0.0);
            (i, g.jobs[i].clone(), g.deps.clone().unwrap())
        };
        ctx.request_repaint();
        let r = match deps {
            Err(e) => Err(format!("setup failed: {e}")),
            Ok((yt, bin)) => {
                let _ = std::fs::create_dir_all(&job.dir);
                job::run_chain(&yt, &job, &bin, |p| {
                    lock(m).jobs[i].state = State::Running(p);
                    ctx.request_repaint();
                })
            }
        };
        let mut g = lock(m);
        g.running -= 1;
        match r {
            Ok(path) => {
                let stem = path.as_ref().and_then(|p| p.file_stem()).map(|s| s.to_string_lossy().into_owned());
                let title = stem.clone().or_else(|| job.title.clone()).unwrap_or_else(|| job.url.clone());
                if g.jobs[i].title.is_none() {
                    g.jobs[i].title = stem;
                }
                g.jobs[i].out = path.clone();
                g.jobs[i].state = State::Done;
                g.history.push(HistoryItem {
                    title,
                    url: job.url.clone(),
                    path: path.map(|p| p.to_string_lossy().into_owned()).unwrap_or_default(),
                    when: now(),
                });
                settings::save_history(&g.history);
            }
            Err(e) if job.ctl.is_cancelled() && e == "Cancelled" => g.jobs[i].state = State::Cancelled,
            Err(e) => g.jobs[i].state = State::Failed(e),
        }
        cv.notify_all();
        drop(g);
        ctx.request_repaint();
    }
}

/// Size of a plain file from its headers, when the server answers.
fn head_size(url: &str, referer: &str, ua: &str) -> Option<u64> {
    let cfg = ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(8))).build();
    let agent: ureq::Agent = cfg.into();
    let mut r = agent.head(url);
    if !referer.is_empty() {
        r = r.header("Referer", referer);
    }
    if !ua.is_empty() {
        r = r.header("User-Agent", ua);
    }
    r.call().ok()?.headers().get("content-length")?.to_str().ok()?.parse().ok()
}

fn start_probe(sy: &Sy, ctx: &egui::Context, cand: Cand) {
    let yt = {
        let mut s = lock(&sy.0);
        let yt = match &s.deps {
            Some(Ok((yt, _))) => yt.clone(),
            _ => {
                s.status = "Still getting ready, try again in a moment".into();
                return;
            }
        };
        s.probe = Some(ProbeUi { cand: cand.clone(), state: ProbeState::Loading });
        yt
    };
    let (sy2, ctx2) = (sy.clone(), ctx.clone());
    std::thread::spawn(move || {
        let r = probe::probe(&yt, &cand);
        let mut s = lock(&sy2.0);
        // The window may have been closed or reopened for another address meanwhile.
        if let Some(p) = s.probe.as_mut().filter(|p| p.cand == cand) {
            p.state = match r {
                Ok(i) => ProbeState::Ready(i),
                Err(e) => ProbeState::Failed(e),
            };
        }
        ctx2.request_repaint();
    });
}

fn rank(k: Kind) -> u8 {
    match k {
        Kind::Embed => 0,
        Kind::Page => 1,
        Kind::Hls | Kind::Dash | Kind::Smooth => 2,
        Kind::File => 3,
    }
}

/// A line the browser process printed.
fn on_browser_line(sy: &Sy, ctx: &egui::Context, v: &serde_json::Value) {
    let text = |k: &str| v[k].as_str().unwrap_or_default().to_string();
    let opt = |c: &serde_json::Value, k: &str| c[k].as_str().filter(|x| !x.is_empty()).map(String::from);
    // A page candidate carries its own address as referer.
    let kind_for = |url: &str, referer: &str| kind_of(url, url == referer);
    match v["kind"].as_str() {
        Some("download") => {
            // The button in the browser: queue the best candidate, keep the others as fallbacks.
            let title = text("title");
            let mut cs: Vec<Cand> = v["candidates"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|c| {
                    let url = job::valid_url(&opt(c, "url")?)?;
                    let referer = opt(c, "referer");
                    let k = kind_for(&url, referer.as_deref().unwrap_or(""));
                    let title = (k.named_by_page() && !title.is_empty()).then(|| title.clone());
                    Some(Cand { url, referer, ua: opt(c, "ua"), title })
                })
                .collect();
            if !cs.is_empty() {
                let first = cs.remove(0);
                queue(sy, first, None, cs);
                ctx.request_repaint();
            }
        }
        Some("title") => {
            let new = text("title");
            let mut s = lock(&sy.0);
            let old = std::mem::replace(&mut s.page_title, new.clone());
            for f in s.found.iter_mut().filter(|f| f.title.is_empty() || f.title == old) {
                f.title = new.clone();
            }
            ctx.request_repaint();
        }
        Some("play") => {
            lock(&sy.0).arm(ctx);
            ctx.request_repaint();
        }
        Some(k @ ("page" | "media")) => {
            let page = k == "page";
            let (url, referer) = (text("url"), text("referer"));
            if job::valid_url(&url).is_none() {
                return;
            }
            let mut s = lock(&sy.0);
            if page {
                // A new page starts a new list, like the browser's own detector.
                s.found.clear();
                s.armed = false;
                s.page_title = text("title");
            } else if s.found.iter().any(|x| x.url == url) {
                return;
            } else {
                s.arm(ctx);
            }
            let kind = if page { Kind::Page } else { kind_for(&url, &referer) };
            let title = if page { text("title") } else { s.page_title.clone() };
            let size = v["size"].as_u64();
            s.found.push(Found { kind, url: url.clone(), referer: referer.clone(), ua: text("ua"), title, size });
            s.found.sort_by_key(|f| rank(f.kind)); // stable: arrival order breaks ties
            drop(s);
            ctx.request_repaint();
            if kind == Kind::File && size.is_none() {
                let (sy, ctx, ua) = (sy.clone(), ctx.clone(), text("ua"));
                std::thread::spawn(move || {
                    if let Some(n) = head_size(&url, &referer, &ua) {
                        if let Some(f) = lock(&sy.0).found.iter_mut().find(|f| f.url == url) {
                            f.size = Some(n);
                        }
                        ctx.request_repaint();
                    }
                });
            }
        }
        _ => {}
    }
}

/// Fills the window with made-up downloads in every state, to look at the layout. Debug builds only.
#[cfg(debug_assertions)]
fn demo(sy: &Sy, mode: &str) {
    let mut s = lock(&sy.0);
    if mode == "empty" {
        return;
    }
    if mode == "picker" {
        let cand = Cand { url: "https://www.youtube.com/watch?v=abc".into(), referer: None, ua: None, title: None };
        let q = |height, size| probe::Quality { height, size };
        let info = Info {
            title: "Lezione 3 - Introduzione alla sociologia dei media digitali".into(),
            duration: Some(3725),
            qualities: vec![q(2160, Some(910_000_000)), q(1080, Some(240_000_000)), q(720, Some(120_000_000)), q(480, None)],
            has_audio: true,
        };
        s.probe = Some(ProbeUi { cand, state: ProbeState::Ready(info) });
        return;
    }
    let dir = s.set.out_dir();
    let video = |h| Opts { audio: false, container: Container::Mp4, height: h };
    let audio = Opts { audio: true, container: Container::Mp4, height: None };
    let mk = |url: &str, title: &str, o: Opts, state: State| {
        let c = Cand { url: url.into(), referer: None, ua: None, title: Some(title.into()) };
        let mut j = Job::new(c, o, dir.clone(), vec![]);
        j.state = state;
        if j.state == State::Done {
            j.out = Some(PathBuf::from("/tmp/demo.mp4"));
        }
        j
    };
    s.jobs = vec![
        mk("https://www.youtube.com/watch?v=abc", "Lesson 3 - Introduction to photography: light, composition and exposure, explained from scratch", video(Some(1080)), State::Running(43.0)),
        mk("https://soundcloud.com/a/b", "A song with a rather long title", audio, State::Running(78.0)),
        mk("https://vimeo.com/123", "Intervista", video(None), State::Queued),
        mk("https://cdn.site.test/video/master.m3u8", "Conference 2026", video(Some(720)), State::Done),
        mk("https://example.com/private", "Private video", video(None), State::Failed("ERROR: [generic] Unable to download webpage: HTTP Error 403: Forbidden (caused by <HTTPError 403>)".into())),
        mk("https://example.org/clip", "Cancelled clip", video(None), State::Cancelled),
    ];
    s.armed = true;
    s.page_title = "Lesson 3 - Introduction to photography | Courses Platform".into();
    let f = |kind, url: &str, size| Found { kind, url: url.into(), referer: String::new(), ua: String::new(), title: String::new(), size };
    s.found = vec![
        f(Kind::Page, "https://courses.site.test/lesson/3", None),
        f(Kind::Hls, "https://cdn.site.test/video/master.m3u8", None),
        f(Kind::File, "https://cdn.site.test/video/720.mp4", Some(48_000_000)),
    ];
    s.history = (0..4)
        .map(|i| HistoryItem { title: format!("Downloaded video {i}"), url: "https://example.com".into(), path: "/tmp/a.mp4".into(), when: 0 })
        .collect();
}

enum Action {
    Typed { choose: bool },
    Download(Cand),
    /// The best candidate first, the others as fallbacks.
    DownloadAll(Vec<Cand>),
    DownloadAs(Cand),
    Pick(Cand, Opts),
    ClosePicker,
    Retry(usize),
    Cancel(usize),
    Hide(usize),
    ClearFinished,
    Browser(String),
    Open(PathBuf),
    Reveal(PathBuf),
    OpenFolder,
    ChangeFolder,
    ClearHistory,
    Settings(Settings),
}

pub struct App {
    sy: Sy,
    ctx: egui::Context,
    input: String,
    browser: Option<Child>,
    show_settings: bool,
}

impl App {
    pub fn new(ctx: egui::Context) -> Self {
        let set = Settings::load();
        let sy: Sy = Arc::new((Mutex::new(Shared::new(set, settings::load_history())), Condvar::new()));
        #[cfg(debug_assertions)]
        if let Ok(mode) = std::env::var("CAMALLO_DEMO") {
            demo(&sy, &mode);
        }
        let (s1, c1) = (sy.clone(), ctx.clone());
        std::thread::spawn(move || setup(s1, c1));
        for _ in 0..MAX_PARALLEL {
            let (s2, c2) = (sy.clone(), ctx.clone());
            std::thread::spawn(move || worker(s2, c2));
        }
        Self { sy, ctx, input: String::new(), browser: None, show_settings: false }
    }

    fn open_browser(&mut self, url: String) {
        if let Some(mut c) = self.browser.take() {
            let _ = c.kill();
        }
        let Ok(exe) = std::env::current_exe() else { return };
        let mut cmd = Command::new(exe);
        let child = job::quiet(&mut cmd).args(["--browser", &url]).stdout(Stdio::piped()).spawn();
        let Ok(mut child) = child else { return };
        let out = child.stdout.take().unwrap();
        let (sy, ctx) = (self.sy.clone(), self.ctx.clone());
        std::thread::spawn(move || {
            use std::io::{BufRead, BufReader};
            for line in BufReader::new(out).lines().map_while(Result::ok) {
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) {
                    on_browser_line(&sy, &ctx, &v);
                }
            }
        });
        self.browser = Some(child);
    }

    fn handle(&mut self, a: Action) {
        let (m, cv) = &*self.sy;
        match a {
            Action::Typed { choose } => {
                if let Some(url) = job::valid_url(&self.input) {
                    let c = Cand { url, referer: None, ua: None, title: None };
                    if choose {
                        start_probe(&self.sy, &self.ctx, c);
                    } else {
                        queue(&self.sy, c, None, vec![]);
                        self.input.clear();
                    }
                }
            }
            Action::Download(c) => queue(&self.sy, c, None, vec![]),
            Action::DownloadAll(mut cs) => {
                if !cs.is_empty() {
                    let first = cs.remove(0);
                    queue(&self.sy, first, None, cs);
                }
            }
            Action::DownloadAs(c) => start_probe(&self.sy, &self.ctx, c),
            Action::Pick(c, o) => {
                queue(&self.sy, c, Some(o), vec![]);
                lock(m).probe = None;
            }
            Action::ClosePicker => lock(m).probe = None,
            Action::Retry(i) => {
                let mut s = lock(m);
                if matches!(s.jobs[i].state, State::Failed(_) | State::Cancelled) {
                    s.jobs[i].state = State::Queued;
                    s.jobs[i].ctl = Arc::new(Ctl::default());
                    s.queue.push_back(i);
                    cv.notify_all();
                }
            }
            Action::Cancel(i) => {
                let ctl = {
                    let mut s = lock(m);
                    match s.jobs[i].state {
                        State::Queued => {
                            s.jobs[i].state = State::Cancelled;
                            None
                        }
                        State::Running(_) => Some(s.jobs[i].ctl.clone()),
                        _ => None,
                    }
                };
                if let Some(c) = ctl {
                    c.cancel();
                }
            }
            Action::Hide(i) => lock(m).jobs[i].hidden = true,
            Action::ClearFinished => {
                for j in lock(m).jobs.iter_mut().filter(|j| matches!(j.state, State::Done | State::Cancelled)) {
                    j.hidden = true;
                }
            }
            Action::Browser(u) => self.open_browser(u),
            Action::Open(p) => open_path(&p),
            Action::Reveal(p) => reveal(&p),
            Action::OpenFolder => {
                let dir = lock(m).set.out_dir();
                let _ = std::fs::create_dir_all(&dir);
                open_path(&dir);
            }
            Action::ChangeFolder => {
                let cur = lock(m).set.dir.clone();
                if let Some(p) = rfd::FileDialog::new().set_directory(&cur).pick_folder() {
                    let mut s = lock(m);
                    s.set.dir = p;
                    s.set.save();
                }
            }
            Action::ClearHistory => {
                let mut s = lock(m);
                s.history.clear();
                settings::save_history(&s.history);
            }
            Action::Settings(n) => {
                n.save();
                lock(m).set = n;
                cv.notify_all(); // a higher limit may let queued jobs start
            }
        }
    }
}

fn open_path(p: &Path) {
    #[cfg(windows)]
    let _ = Command::new("explorer").arg(p).spawn();
    #[cfg(not(windows))]
    let _ = Command::new("xdg-open").arg(p).spawn();
}

/// Opens the folder with the file selected.
fn reveal(p: &Path) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let _ = Command::new("explorer").raw_arg(format!("/select,\"{}\"", p.display())).spawn();
    }
    #[cfg(not(windows))]
    open_path(p.parent().unwrap_or(p));
}

impl Found {
    fn cand(&self) -> Cand {
        Cand {
            url: self.url.clone(),
            referer: nz(&self.referer),
            ua: nz(&self.ua),
            title: (self.kind.named_by_page() && !self.title.is_empty()).then(|| self.title.clone()),
        }
    }
}

/// Candidates in the order to try them, like the button in the browser: the list is already ranked.
fn best_cands(found: &[Found]) -> Vec<Cand> {
    found.iter().take(6).map(Found::cand).collect()
}

fn nz(s: &str) -> Option<String> {
    (!s.is_empty()).then(|| s.to_string())
}

fn red(ui: &mut egui::Ui, t: &str) {
    ui.label(egui::RichText::new(t).color(ui::DANGER));
}

fn muted(t: impl Into<String>) -> egui::RichText {
    egui::RichText::new(t.into()).color(ui::MUTED).size(13.0)
}

/// What the file will be, for the capsule under the title: "MP4 · 1080p", "MP3".
fn format_label(o: &Opts) -> String {
    if o.audio {
        return "MP3".into();
    }
    let q = o.height.map(|h| format!("{h}p")).unwrap_or_else(|| "Best".into());
    format!("{} · {}", o.container.short(), q)
}

/// Width taken by the buttons on the right of a download, so the title knows where to stop.
fn buttons_width(s: &State) -> f32 {
    match s {
        State::Queued | State::Running(_) => 90.0,
        State::Done => 175.0,
        State::Cancelled => 125.0,
        State::Failed(_) => 250.0,
    }
}

fn section_title(ui: &mut egui::Ui, text: &str) {
    ui.label(egui::RichText::new(text).size(16.0).strong());
}

/// A line of text that stops at the edge instead of pushing the buttons away.
fn clipped(ui: &mut egui::Ui, text: egui::RichText) -> egui::Response {
    ui.add(egui::Label::new(text).truncate())
}

/// Left part of a row gets `left` wide, the rest is for the buttons, which are laid out from the right.
fn split_row(
    ui: &mut egui::Ui,
    left: f32,
    height: f32,
    text: impl FnOnce(&mut egui::Ui),
    buttons: impl FnOnce(&mut egui::Ui),
) {
    ui.allocate_ui_with_layout(egui::vec2(left, height), egui::Layout::top_down(egui::Align::Min), |ui| {
        ui.set_width(left);
        text(ui);
    });
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), buttons);
}

fn detected_card(ui: &mut egui::Ui, found: &[Found], page_title: &str, actions: &mut Vec<Action>) {
    let only_page = found.iter().all(|f| f.kind == Kind::Page);
    let tint = egui::Color32::from_rgb(0x1c, 0x2a, 0x22);
    ui::card(ui, tint, egui::Stroke::new(1.5, ui::ACCENT), |ui| {
        ui.horizontal(|ui| {
            ui::badge(ui, ui::Badge::Play);
            let left = (ui.available_width() - 300.0).max(140.0);
            split_row(
                ui,
                left,
                ui::BADGE,
                |ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    ui.label(egui::RichText::new("Video detected").size(17.0).strong().color(ui::ACCENT));
                    if !page_title.is_empty() {
                        clipped(ui, muted(page_title));
                    }
                    if only_page {
                        ui.label(muted("No stream address was seen; the page itself can be tried."));
                    }
                },
                |ui| {
                    if ui::pill(ui, "Download", ui::Kind::Primary, 16.0).clicked() {
                        actions.push(Action::DownloadAll(best_cands(found)));
                    }
                    if ui::button(ui, "Choose quality", ui::Kind::Soft).clicked() {
                        actions.push(Action::DownloadAs(found[0].cand()));
                    }
                },
            );
        });
        if found.len() > 1 {
            ui.collapsing(format!("All {} addresses found", found.len()), |ui| {
                for f in found {
                    ui.horizontal(|ui| {
                        let left = (ui.available_width() - 200.0).max(120.0);
                        split_row(
                            ui,
                            left,
                            24.0,
                            |ui| {
                                ui.horizontal(|ui| {
                                    ui::chip(ui, f.kind.badge());
                                    if let Some(n) = f.size {
                                        ui::chip(ui, &human_size(n));
                                    }
                                    let name = if f.title.is_empty() { f.url.clone() } else { f.title.clone() };
                                    clipped(ui, muted(name)).on_hover_text(&f.url);
                                });
                            },
                            |ui| {
                                if ui::pill(ui, "Quality", ui::Kind::Ghost, 13.0).clicked() {
                                    actions.push(Action::DownloadAs(f.cand()));
                                }
                                if ui::pill(ui, "Download", ui::Kind::Soft, 13.0).clicked() {
                                    actions.push(Action::Download(f.cand()));
                                }
                            },
                        );
                    });
                }
            });
        }
    });
}

fn job_card(ui: &mut egui::Ui, i: usize, j: &Job, actions: &mut Vec<Action>) {
    let (badge, outline) = match &j.state {
        State::Queued => (ui::Badge::Queued, egui::Stroke::new(1.0, ui::LINE)),
        State::Running(p) => (ui::Badge::Running(*p), egui::Stroke::new(1.0, ui::LINE)),
        State::Done => (ui::Badge::Done, egui::Stroke::new(1.0, ui::LINE)),
        State::Cancelled => (ui::Badge::Cancelled, egui::Stroke::new(1.0, ui::LINE)),
        State::Failed(_) => (ui::Badge::Failed, egui::Stroke::new(1.0, ui::DANGER_DIM)),
    };
    ui::card(ui, ui::CARD, outline, |ui| {
        ui.horizontal(|ui| {
            ui::badge(ui, badge);
            let left = (ui.available_width() - buttons_width(&j.state)).max(140.0);
            split_row(
                ui,
                left,
                ui::BADGE,
                |ui| {
                    ui.spacing_mut().item_spacing.y = 4.0;
                    clipped(ui, egui::RichText::new(j.label()).size(15.0).strong()).on_hover_text(&j.url);
                    if let State::Running(p) = &j.state {
                        ui::bar(ui, *p);
                    }
                    ui.horizontal(|ui| {
                        match &j.state {
                            State::Queued => {
                                ui.label(muted("Waiting"));
                            }
                            State::Running(_) => {
                                ui.label(muted("Downloading"));
                            }
                            State::Done => {
                                ui.label(egui::RichText::new("Done").size(13.0).color(ui::ACCENT));
                            }
                            State::Cancelled => {
                                ui.label(muted("Cancelled"));
                            }
                            State::Failed(why) => {
                                let first = why.lines().next().unwrap_or("Failed");
                                clipped(ui, egui::RichText::new(first).size(13.0).color(ui::DANGER)).on_hover_text(why);
                            }
                        }
                        if !matches!(j.state, State::Failed(_)) {
                            ui::chip(ui, &format_label(&j.opts));
                            ui::chip(ui, &ui::host_of(&j.url));
                        }
                    });
                },
                |ui| match &j.state {
                    State::Queued | State::Running(_) => {
                        if ui::button(ui, "Cancel", ui::Kind::Ghost).clicked() {
                            actions.push(Action::Cancel(i));
                        }
                    }
                    State::Done => {
                        if ui::close(ui).on_hover_text("Remove from the list").clicked() {
                            actions.push(Action::Hide(i));
                        }
                        if let Some(p) = &j.out {
                            if ui::button(ui, "Show", ui::Kind::Soft).on_hover_text("Show in the folder").clicked() {
                                actions.push(Action::Reveal(p.clone()));
                            }
                            if ui::button(ui, "Open", ui::Kind::Primary).clicked() {
                                actions.push(Action::Open(p.clone()));
                            }
                        }
                    }
                    State::Cancelled => {
                        if ui::close(ui).on_hover_text("Remove from the list").clicked() {
                            actions.push(Action::Hide(i));
                        }
                        if ui::button(ui, "Retry", ui::Kind::Soft).clicked() {
                            actions.push(Action::Retry(i));
                        }
                    }
                    State::Failed(_) => {
                        if ui::close(ui).on_hover_text("Remove from the list").clicked() {
                            actions.push(Action::Hide(i));
                        }
                        if ui::button(ui, "Browser", ui::Kind::Ghost).on_hover_text("Open the page in the built-in browser").clicked() {
                            actions.push(Action::Browser(j.url.clone()));
                        }
                        if ui::button(ui, "Retry", ui::Kind::Soft).clicked() {
                            actions.push(Action::Retry(i));
                        }
                    }
                },
            );
        });
    });
}

fn empty_queue(ui: &mut egui::Ui) {
    ui.add_space((ui.available_height() * 0.12).max(10.0));
    ui.vertical_centered(|ui| {
        ui::empty_icon(ui);
        ui.add_space(6.0);
        ui.label(egui::RichText::new("Your queue is empty").size(19.0).strong());
        ui.label(muted("Paste a link above, or open the browser and play a video."));
    });
}

fn history_list(ui: &mut egui::Ui, history: &[HistoryItem], actions: &mut Vec<Action>) {
    ui.add_space(6.0);
    ui.collapsing(egui::RichText::new(format!("History ({})", history.len())).size(15.0).strong(), |ui| {
        if ui::pill(ui, "Clear history", ui::Kind::Ghost, 13.0).clicked() {
            actions.push(Action::ClearHistory);
        }
        for h in history.iter().rev().take(30) {
            ui.horizontal(|ui| {
                let left = (ui.available_width() - 130.0).max(120.0);
                split_row(
                    ui,
                    left,
                    24.0,
                    |ui| {
                        clipped(ui, muted(&h.title)).on_hover_text(&h.url);
                    },
                    |ui| {
                        if !h.path.is_empty() {
                            if ui::pill(ui, "Show", ui::Kind::Ghost, 13.0).clicked() {
                                actions.push(Action::Reveal(PathBuf::from(&h.path)));
                            }
                            if ui::pill(ui, "Open", ui::Kind::Soft, 13.0).clicked() {
                                actions.push(Action::Open(PathBuf::from(&h.path)));
                            }
                        }
                    },
                );
            });
        }
    });
}

/// The "Choose quality" window. Returns false when the user closed it.
fn picker_window(
    ctx: &egui::Context,
    cand: &Cand,
    state: &ProbeState,
    container: Container,
    actions: &mut Vec<Action>,
) -> bool {
    let mut open = true;
    egui::Window::new("Choose quality")
        .title_bar(false)
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .frame(
            egui::Frame::new()
                .fill(ui::PANEL)
                .stroke(egui::Stroke::new(1.0, ui::LINE))
                .corner_radius(egui::CornerRadius::same(14))
                .inner_margin(egui::Margin::same(18)),
        )
        .show(ctx, |ui| {
            ui.set_width(400.0);
            ui.spacing_mut().item_spacing.y = 10.0;
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("Choose quality").size(17.0).strong());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui::close(ui).on_hover_text("Close").clicked() {
                        open = false;
                    }
                });
            });
            if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                open = false;
            }
            ui.label(muted(ui::host_of(&cand.url)));
            match state {
                ProbeState::Loading => {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label("Looking at the video...");
                    });
                }
                ProbeState::Failed(e) => red(ui, e),
                ProbeState::Ready(info) => {
                    ui.add(egui::Label::new(egui::RichText::new(&info.title).size(16.0).strong()).wrap());
                    if let Some(d) = info.duration {
                        ui::chip(ui, &human_duration(d));
                    }
                    let video = Opts { audio: false, container, height: None };
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing = egui::vec2(8.0, 8.0);
                        if ui::button(ui, "Best available", ui::Kind::Primary).clicked() {
                            actions.push(Action::Pick(cand.clone(), video));
                        }
                        for q in &info.qualities {
                            let size = q.size.map(|s| format!("  ~{}", human_size(s))).unwrap_or_default();
                            if ui::button(ui, &format!("{}p{}", q.height, size), ui::Kind::Soft).clicked() {
                                actions.push(Action::Pick(cand.clone(), Opts { height: Some(q.height), ..video }));
                            }
                        }
                        if info.has_audio && ui::button(ui, "Audio only (MP3)", ui::Kind::Soft).clicked() {
                            actions.push(Action::Pick(cand.clone(), Opts { audio: true, ..video }));
                        }
                    });
                }
            }
        });
    open
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let mut actions: Vec<Action> = vec![];
        let (found, jobs, history, status, orig, picker, armed, page_title) = {
            let s = lock(&self.sy.0);
            let picker = s.probe.as_ref().map(|p| (p.cand.clone(), p.state.clone()));
            (
                s.found.clone(),
                s.jobs.clone(),
                s.history.clone(),
                s.status.clone(),
                s.set.clone(),
                picker,
                s.armed,
                s.page_title.clone(),
            )
        };
        let mut set = orig.clone();
        let line = egui::Stroke::new(1.0, ui::LINE);

        egui::Panel::top("top")
            .frame(egui::Frame::new().fill(ui::PANEL).inner_margin(egui::Margin::symmetric(16, 12)))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let kind = if self.show_settings { ui::Kind::Selected } else { ui::Kind::Soft };
                        if ui::button(ui, "Settings", kind).clicked() {
                            self.show_settings = !self.show_settings;
                        }
                        let tip = "Open a page and let Camallo find the video";
                        if ui::button(ui, "Browser", ui::Kind::Soft).on_hover_text(tip).clicked() {
                            let u = job::valid_url(&self.input).unwrap_or_else(|| "https://duckduckgo.com".into());
                            actions.push(Action::Browser(u));
                        }
                        if ui::button(ui, "Choose quality", ui::Kind::Soft).clicked() {
                            actions.push(Action::Typed { choose: true });
                        }
                        let go = ui::pill(ui, "Download", ui::Kind::Primary, 15.0).clicked();
                        let r = ui.add_sized(
                            [ui.available_width(), 38.0],
                            egui::TextEdit::singleline(&mut self.input)
                                .hint_text("Paste a video link")
                                .margin(egui::Margin::symmetric(14, 9))
                                .align(egui::Align2::LEFT_CENTER),
                        );
                        let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                        if go || enter {
                            actions.push(Action::Typed { choose: false });
                        }
                    });
                });
                if self.show_settings {
                    ui.add_space(10.0);
                    ui::card(ui, ui::CARD, line, |ui| {
                        egui::Grid::new("settings").num_columns(2).spacing([18.0, 10.0]).show(ui, |ui| {
                            ui.label(muted("Save to"));
                            ui.horizontal(|ui| {
                                if ui::button(ui, "Change", ui::Kind::Soft).clicked() {
                                    actions.push(Action::ChangeFolder);
                                }
                                if ui::button(ui, "Open folder", ui::Kind::Soft).clicked() {
                                    actions.push(Action::OpenFolder);
                                }
                                clipped(ui, muted(set.dir.display().to_string()));
                            });
                            ui.end_row();
                            ui.label(muted("Subfolder"));
                            ui.add(
                                egui::TextEdit::singleline(&mut set.subdir)
                                    .hint_text("optional")
                                    .desired_width(180.0)
                                    .margin(egui::Margin::symmetric(10, 6)),
                            );
                            ui.end_row();
                            ui.label(muted("Downloads at once"));
                            egui::ComboBox::from_id_salt("parallel").selected_text(set.parallel.to_string()).width(64.0).show_ui(ui, |ui| {
                                for n in 1..=MAX_PARALLEL {
                                    ui.selectable_value(&mut set.parallel, n, n.to_string());
                                }
                            });
                            ui.end_row();
                        });
                    });
                }
            });

        egui::Panel::bottom("bottom")
            .frame(egui::Frame::new().fill(ui::PANEL).inner_margin(egui::Margin::symmetric(16, 10)))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(muted("New downloads:"));
                    let video = if set.audio_only { ui::Kind::Soft } else { ui::Kind::Selected };
                    let audio = if set.audio_only { ui::Kind::Selected } else { ui::Kind::Soft };
                    if ui::button(ui, "Video", video).clicked() {
                        set.audio_only = false;
                    }
                    if ui::button(ui, "Audio (MP3)", audio).clicked() {
                        set.audio_only = true;
                    }
                    ui.add_enabled_ui(!set.audio_only, |ui| {
                        egui::ComboBox::from_id_salt("container").selected_text(set.container.short()).width(70.0).show_ui(ui, |ui| {
                            for c in Container::ALL {
                                ui.selectable_value(&mut set.container, c, c.label());
                            }
                        });
                        egui::ComboBox::from_id_salt("quality").selected_text(set.quality.label()).width(84.0).show_ui(ui, |ui| {
                            for q in Quality::ALL {
                                ui.selectable_value(&mut set.quality, q, q.label());
                            }
                        });
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui::button(ui, "Folder", ui::Kind::Ghost).on_hover_text("Open the download folder").clicked() {
                            actions.push(Action::OpenFolder);
                        }
                        let running = jobs.iter().filter(|j| matches!(j.state, State::Running(_))).count();
                        let waiting = jobs.iter().filter(|j| j.state == State::Queued).count();
                        let text = if status == "Ready" && running + waiting > 0 {
                            format!("{running} downloading, {waiting} waiting")
                        } else {
                            status.clone()
                        };
                        let color = if status.starts_with("Setup failed") { ui::DANGER } else { ui::MUTED };
                        clipped(ui, egui::RichText::new(text).size(13.0).color(color));
                    });
                });
            });

        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(ui::BG).inner_margin(egui::Margin::same(16)))
            .show(ui, |ui| {
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 10.0;
                    if armed && !found.is_empty() {
                        detected_card(ui, &found, &page_title, &mut actions);
                        ui.add_space(4.0);
                    }

                    let shown: Vec<(usize, &Job)> = jobs.iter().enumerate().rev().filter(|(_, j)| !j.hidden).collect();
                    ui.horizontal(|ui| {
                        section_title(ui, "Downloads");
                        if !shown.is_empty() {
                            ui::chip(ui, &shown.len().to_string());
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if !shown.is_empty() && ui::pill(ui, "Clear finished", ui::Kind::Ghost, 13.0).clicked() {
                                actions.push(Action::ClearFinished);
                            }
                        });
                    });
                    if shown.is_empty() {
                        empty_queue(ui);
                    }
                    for (i, j) in shown {
                        job_card(ui, i, j, &mut actions);
                    }
                    if !history.is_empty() {
                        history_list(ui, &history, &mut actions);
                    }
                });
            });

        if let Some((cand, state)) = picker
            && !picker_window(ui.ctx(), &cand, &state, set.container, &mut actions)
        {
            actions.push(Action::ClosePicker);
        }

        if set != orig {
            actions.push(Action::Settings(set));
        }
        for a in actions {
            self.handle(a);
        }
    }
}

impl Drop for App {
    fn drop(&mut self) {
        if let Some(mut c) = self.browser.take() {
            let _ = c.kill();
        }
        // Do not leave yt-dlp and ffmpeg running behind a closed window.
        let running: Vec<Arc<Ctl>> =
            lock(&self.sy.0).jobs.iter().filter(|j| matches!(j.state, State::Running(_))).map(|j| j.ctl.clone()).collect();
        for c in running {
            c.cancel();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn feed(sy: &Sy, v: serde_json::Value) {
        on_browser_line(sy, &egui::Context::default(), &v);
    }
    fn page(url: &str, title: &str) -> serde_json::Value {
        json!({"kind": "page", "url": url, "referer": url, "ua": "UA", "title": title})
    }
    fn media(url: &str, referer: &str) -> serde_json::Value {
        json!({"kind": "media", "url": url, "referer": referer, "ua": "UA", "title": ""})
    }

    #[test]
    fn browser_lines_build_the_found_list_and_the_download() {
        let sy: Sy = Arc::new((Mutex::new(Shared::new(Settings::default(), vec![])), Condvar::new()));
        let p = "https://site.test/p";
        feed(&sy, page(p, ""));
        // A late title reaches the page entry and the media seen afterwards.
        feed(&sy, json!({"kind": "title", "title": "Lezione 3"}));
        feed(&sy, media("https://127.0.0.1:1/master.m3u8", p));
        feed(&sy, media("https://127.0.0.1:1/clip.mp4", p));
        feed(&sy, media("https://127.0.0.1:1/clip.mp4", p)); // duplicate
        feed(&sy, media("https://www.dailymotion.com/video/x8abc12", p));
        feed(&sy, media("not a url", p)); // rejected
        {
            let s = lock(&sy.0);
            let k: Vec<Kind> = s.found.iter().map(|f| f.kind).collect();
            assert_eq!(k, [Kind::Embed, Kind::Page, Kind::Hls, Kind::File]);
            assert!(s.found.iter().all(|f| f.title == "Lezione 3"));
        }
        // The button in the browser: page first, then the stream, which is named by the page title.
        feed(
            &sy,
            json!({"kind": "download", "title": "Lezione 3", "candidates": [
                {"url": p, "referer": p, "ua": "UA"},
                {"url": "https://127.0.0.1:1/master.m3u8", "referer": p, "ua": "UA"}
            ]}),
        );
        let s = lock(&sy.0);
        assert_eq!(s.jobs.len(), 1);
        assert_eq!((s.jobs[0].url.as_str(), s.jobs[0].title.clone()), (p, None));
        assert_eq!(s.jobs[0].alts.len(), 1);
        assert_eq!(s.jobs[0].alts[0].title.as_deref(), Some("Lezione 3"));
        assert_eq!(s.queue.iter().copied().collect::<Vec<_>>(), [0]);
    }

    #[test]
    fn a_stream_or_a_play_arms_the_list_and_a_new_page_disarms_it() {
        let sy: Sy = Arc::new((Mutex::new(Shared::new(Settings::default(), vec![])), Condvar::new()));
        feed(&sy, page("https://site.test/1", "One"));
        assert!(!lock(&sy.0).armed, "a page alone is not a video");
        feed(&sy, json!({"kind": "play"}));
        assert!(lock(&sy.0).armed);
        feed(&sy, page("https://site.test/2", "Two"));
        assert!(!lock(&sy.0).armed);
        // A file whose size the network already told: no extra request, the size is kept.
        feed(&sy, json!({"kind": "media", "url": "https://127.0.0.1:1/v.mp4", "referer": "https://site.test/2", "ua": "UA", "size": 5_000_000}));
        let s = lock(&sy.0);
        assert!(s.armed);
        let f = s.found.iter().find(|f| f.kind == Kind::File).unwrap();
        assert_eq!(f.size, Some(5_000_000));
    }

    #[test]
    fn best_candidates_follow_the_ranking_and_stop_at_six() {
        let f = |kind, url: &str| Found { kind, url: url.into(), referer: String::new(), ua: String::new(), title: "T".into(), size: None };
        let mut v = vec![f(Kind::Embed, "https://e/1"), f(Kind::Page, "https://p/1"), f(Kind::Hls, "https://h/1.m3u8")];
        for i in 0..6 {
            v.push(f(Kind::File, &format!("https://f/{i}.mp4")));
        }
        let c = best_cands(&v);
        assert_eq!(c.len(), 6);
        assert_eq!(c[0].url, "https://e/1");
        // Only plain streams and files take the page title as file name.
        assert_eq!((c[0].title.clone(), c[1].title.clone()), (None, None));
        assert_eq!(c[2].title.as_deref(), Some("T"));
    }

    #[test]
    fn a_new_page_clears_the_list() {
        let sy: Sy = Arc::new((Mutex::new(Shared::new(Settings::default(), vec![])), Condvar::new()));
        feed(&sy, page("https://site.test/1", "One"));
        feed(&sy, media("https://127.0.0.1:1/a.m3u8", "https://site.test/1"));
        feed(&sy, page("https://site.test/2", "Two"));
        let s = lock(&sy.0);
        assert_eq!(s.found.len(), 1);
        assert_eq!((s.found[0].kind, s.found[0].title.as_str()), (Kind::Page, "Two"));
    }

    #[test]
    fn cancelled_queued_job_is_skipped_and_retry_requeues() {
        let sy: Sy = Arc::new((Mutex::new(Shared::new(Settings::default(), vec![])), Condvar::new()));
        let c = Cand { url: "https://example.com/v".into(), referer: None, ua: None, title: None };
        queue(&sy, c, None, vec![]);
        let mut s = lock(&sy.0);
        assert_eq!(s.jobs[0].state, State::Queued);
        s.jobs[0].state = State::Cancelled;
        // A worker popping the stale entry must not start it.
        let i = s.queue.pop_front().unwrap();
        assert!(s.jobs[i].state != State::Queued);
    }
}
