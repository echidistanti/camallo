use crate::deps;
use crate::detector::{kind_of, Kind};
use crate::fetch::{self, Entry, Fetched};
use crate::job::{self, Cand, Ctl, Job, Opts, State};
use crate::probe::{self, Info};
use crate::settings::{self, HistoryItem, Settings, MAX_PARALLEL};
use crate::thumbs;
use eframe::egui;
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Threads that ask yt-dlp about addresses and videos, so a long playlist fills in quickly.
const FETCHERS: usize = 3;
const THUMBERS: usize = 2;
/// A page can announce as many addresses as it likes; the list keeps the first ones.
const MAX_FOUND: usize = 50;
/// Cards from the browser's Download button that may wait for the user at once.
const MAX_BROWSER_CARDS: usize = 10;
/// Size requests in flight at once; beyond that a file's size just stays unknown.
const MAX_HEADS: usize = 4;
static HEADS: AtomicUsize = AtomicUsize::new(0);

/// One of the MAX_HEADS places for a size request, given back when dropped.
struct HeadSlot;

impl HeadSlot {
    fn take() -> Option<HeadSlot> {
        HEADS.fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| (n < MAX_HEADS).then_some(n + 1)).ok().map(|_| HeadSlot)
    }
}

impl Drop for HeadSlot {
    fn drop(&mut self) {
        HEADS.fetch_sub(1, Ordering::SeqCst);
    }
}

/// A media address the built-in browser has seen on the current page.
#[derive(Clone)]
pub struct Found {
    pub kind: Kind,
    pub url: String,
    pub referer: String,
    pub ua: String,
    pub title: String,
    pub size: Option<u64>,
}

/// What a fetch thread has to do.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Fetch {
    /// An address the user added: one video or a list.
    List(usize),
    /// One video of a list.
    Detail(usize),
}

impl Fetch {
    fn index(self) -> usize {
        match self {
            Fetch::List(i) | Fetch::Detail(i) => i,
        }
    }
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
    /// Addresses and videos waiting for yt-dlp to read them.
    fetchq: VecDeque<Fetch>,
    /// Pictures waiting to be fetched.
    thumbq: VecDeque<String>,
    /// Every picture asked for; None while it loads or when it could not be had.
    thumbs: HashMap<String, Option<egui::TextureHandle>>,
    batches: u64,
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
            fetchq: VecDeque::new(),
            thumbq: VecDeque::new(),
            thumbs: HashMap::new(),
            batches: 0,
        }
    }

    /// First sign of a video on this page: flash the window so the new button gets noticed.
    fn arm(&mut self, ctx: &egui::Context) {
        if !self.armed {
            self.armed = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::RequestUserAttention(egui::UserAttentionType::Informational));
        }
    }

    fn next_batch(&mut self) -> u64 {
        self.batches += 1;
        self.batches
    }

    /// Forgets the pictures of cards that were removed, so a long session does not keep them all.
    fn prune_thumbs(&mut self) {
        let wanted: std::collections::HashSet<&str> =
            self.jobs.iter().filter(|j| !j.hidden).filter_map(|j| j.thumb.as_deref()).collect();
        self.thumbs.retain(|k, _| wanted.contains(k.as_str()));
    }

    fn want_thumb(&mut self, url: &str) {
        if !self.thumbs.contains_key(url) {
            self.thumbs.insert(url.to_string(), None);
            self.thumbq.push_back(url.to_string());
        }
    }

    /// An address for yt-dlp to read. Returns the slot of its card.
    fn add_pending(&mut self, c: Cand) -> usize {
        let batch = self.next_batch();
        let (opts, dir) = (Opts::from_settings(&self.set), self.set.out_dir());
        self.jobs.push(Job::pending(c, opts, dir, batch));
        let i = self.jobs.len() - 1;
        self.fetchq.push_back(Fetch::List(i));
        i
    }
}

type Sy = Arc<(Mutex<Shared>, Condvar)>;

fn lock(m: &Mutex<Shared>) -> MutexGuard<'_, Shared> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Straight into the download queue, without reading the address first (the big button).
fn queue(sy: &Sy, c: Cand, opts: Option<Opts>, alts: Vec<Cand>) {
    let (m, cv) = &**sy;
    let mut s = lock(m);
    let opts = opts.unwrap_or_else(|| Opts::from_settings(&s.set));
    let dir = s.set.out_dir();
    let mut j = Job::new(c, opts, dir, alts);
    j.batch = s.next_batch();
    s.jobs.push(j);
    let i = s.jobs.len() - 1;
    s.queue.push_back(i);
    cv.notify_all();
}

/// The text of the address field: a card appears at once and yt-dlp reads the address in the background.
/// False when the text is not a link.
fn add_typed(sy: &Sy, input: &str) -> bool {
    let Some(url) = job::valid_url(input) else { return false };
    let (m, cv) = &**sy;
    lock(m).add_pending(Cand { url, referer: None, ua: None, title: None });
    cv.notify_all();
    true
}

/// The picture height that really exists and fits the wish: "up to 1080p" becomes the best height at or under it.
/// None means no limit, which is also what is left when every height is taller than the wish.
pub fn snap_height(cap: Option<u32>, info: &Info) -> Option<u32> {
    let cap = cap?;
    info.qualities.iter().map(|q| q.height).find(|h| *h <= cap)
}

/// Moves a card from "reading" to "choose and download", with the choices taken from the defaults.
fn set_ready(s: &mut Shared, i: usize, info: Info) {
    let (audio, cap) = (s.set.audio_only, s.set.quality.height());
    let j = &mut s.jobs[i];
    if !info.title.is_empty() {
        j.shown = Some(info.title.clone());
    }
    j.duration = info.duration.or(j.duration);
    if j.thumb.is_none() {
        j.thumb = info.thumbnail.clone();
    }
    j.opts.audio = audio;
    j.opts.height = snap_height(cap, &info);
    j.info = Some(info);
    j.state = State::Ready;
    let thumb = j.thumb.clone();
    if let Some(t) = thumb {
        s.want_thumb(&t);
    }
    if s.set.auto_start && !s.jobs[i].manual {
        start_job(s, i);
    }
}

/// Ready to queued; the folder and container are those of the moment the button is pressed.
fn start_job(s: &mut Shared, i: usize) {
    if s.jobs[i].state != State::Ready || s.jobs[i].hidden {
        return;
    }
    s.jobs[i].opts.container = s.set.container;
    s.jobs[i].dir = s.set.out_dir();
    s.jobs[i].state = State::Queued;
    s.queue.push_back(i);
}

/// The new default type and quality also go to every video still waiting for its Download.
fn apply_defaults(s: &mut Shared) {
    let (audio, cap) = (s.set.audio_only, s.set.quality.height());
    for j in s.jobs.iter_mut().filter(|j| j.state == State::Ready && !j.hidden) {
        j.opts.audio = audio;
        if let Some(info) = &j.info {
            j.opts.height = snap_height(cap, info);
        }
    }
}

/// A failed or cancelled download goes back to the queue; an address yt-dlp could not read is read again.
fn retry(s: &mut Shared, i: usize) {
    match s.jobs[i].state {
        State::Failed(_) | State::Cancelled => {
            s.jobs[i].state = State::Queued;
            s.jobs[i].ctl = Arc::new(Ctl::default());
            s.queue.push_back(i);
        }
        State::NoInfo(_) => {
            s.jobs[i].state = State::Fetching;
            let task = if s.jobs[i].is_entry { Fetch::Detail(i) } else { Fetch::List(i) };
            s.fetchq.push_back(task);
        }
        _ => {}
    }
}

/// What yt-dlp found out about a card that was being read.
fn apply_fetched(s: &mut Shared, i: usize, r: Result<Fetched, String>) {
    // The card may have been removed while yt-dlp was working.
    if s.jobs[i].state != State::Fetching || s.jobs[i].hidden {
        return;
    }
    match r {
        Err(e) => s.jobs[i].state = State::NoInfo(e),
        Ok(Fetched::Video(info)) => set_ready(s, i, info),
        Ok(Fetched::List { title, entries }) => {
            let (batch, dir, opts, referer, ua, manual) = {
                let j = &s.jobs[i];
                (j.batch, j.dir.clone(), j.opts, j.referer.clone(), j.ua.clone(), j.manual)
            };
            // The address gives way to the videos it holds.
            s.jobs[i].hidden = true;
            s.jobs[i].state = State::Cancelled;
            for Entry { url, title: name, duration, thumb, info } in entries {
                let c = Cand { url, referer: referer.clone(), ua: ua.clone(), title: None };
                let mut j = Job::pending(c, opts, dir.clone(), batch);
                j.group = Some(title.clone());
                j.is_entry = true;
                j.manual = manual;
                j.shown = (!name.is_empty()).then_some(name);
                j.duration = duration;
                j.thumb = thumb;
                s.jobs.push(j);
                let k = s.jobs.len() - 1;
                match info {
                    Some(info) => set_ready(s, k, info),
                    None => {
                        s.fetchq.push_back(Fetch::Detail(k));
                        if let Some(t) = s.jobs[k].thumb.clone() {
                            s.want_thumb(&t);
                        }
                    }
                }
            }
        }
    }
}

/// One of FETCHERS threads: reads addresses and the videos of lists.
fn fetcher(sy: Sy, ctx: egui::Context) {
    let (m, cv) = &*sy;
    loop {
        let (task, cand, deps) = {
            let g = lock(m);
            let mut g = cv
                .wait_while(g, |s| s.deps.is_none() || s.fetchq.is_empty())
                .unwrap_or_else(|e| e.into_inner());
            let Some(task) = g.fetchq.pop_front() else { continue };
            let i = task.index();
            if g.jobs[i].state != State::Fetching || g.jobs[i].hidden {
                continue; // removed while waiting
            }
            (task, g.jobs[i].cand(), g.deps.clone().unwrap())
        };
        let r = match deps {
            Err(e) => Err(format!("setup failed: {e}")),
            Ok((yt, _)) => match task {
                Fetch::List(_) => fetch::run(&yt, &cand),
                Fetch::Detail(_) => probe::probe(&yt, &cand).map(Fetched::Video),
            },
        };
        apply_fetched(&mut lock(m), task.index(), r);
        cv.notify_all(); // a video that starts by itself needs a worker
        ctx.request_repaint();
    }
}

/// One of THUMBERS threads: pictures for the cards.
fn thumber(sy: Sy, ctx: egui::Context) {
    let (m, cv) = &*sy;
    loop {
        let url = {
            let g = lock(m);
            let mut g = cv.wait_while(g, |s| s.thumbq.is_empty()).unwrap_or_else(|e| e.into_inner());
            let Some(u) = g.thumbq.pop_front() else { continue };
            u
        };
        if let Some(img) = thumbs::fetch(&url) {
            let tex = ctx.load_texture(url.clone(), img, egui::TextureOptions::LINEAR);
            // Only if some card still wants it: the cards may have been removed meanwhile.
            if let Some(slot) = lock(m).thumbs.get_mut(&url) {
                *slot = Some(tex);
            }
            ctx.request_repaint();
        }
    }
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
            g.deps = Some(Ok((d.yt.clone(), d.bin)));
            if d.may_be_old {
                std::thread::spawn(move || deps::update(&d.yt));
            }
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
                    url: settings::plain_url(&job.url),
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
            // The button in the browser. Any script on a page can send this same message, so it only adds a card
            // that waits for Download in this window: nothing starts by itself, whatever the settings say.
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
                let mut s = lock(&sy.0);
                let waiting = s.jobs.iter().filter(|j| j.manual && !j.is_entry && !j.hidden && matches!(j.state, State::Fetching | State::Ready)).count();
                if waiting < MAX_BROWSER_CARDS {
                    let i = s.add_pending(first);
                    s.jobs[i].alts = cs;
                    s.jobs[i].manual = true;
                    drop(s);
                    sy.1.notify_all();
                    ctx.request_repaint();
                }
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
            } else if s.found.iter().any(|x| x.url == url) || s.found.len() >= MAX_FOUND {
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
            let slot = if kind == Kind::File && size.is_none() { HeadSlot::take() } else { None };
            if let Some(slot) = slot {
                let (sy, ctx, ua) = (sy.clone(), ctx.clone(), text("ua"));
                std::thread::spawn(move || {
                    let _slot = slot;
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

/// Fills the window with made-up videos in every state, to look at the layout. Debug builds only.
#[cfg(debug_assertions)]
fn demo(sy: &Sy, ctx: &egui::Context, mode: &str) {
    let mut s = lock(&sy.0);
    if mode == "empty" {
        return;
    }
    // Pictures drawn on the spot: a colour gradient per video.
    let picture = |s: &mut Shared, n: u8| -> String {
        let (w, h) = (128usize, 72usize);
        let mut px = Vec::with_capacity(w * h * 4);
        for y in 0..h {
            for x in 0..w {
                let t = (x + y) as f32 / (w + h) as f32;
                let (r, g, b) = (60.0 + 140.0 * t, 40.0 + 60.0 * (1.0 - t), 120.0 + 110.0 * ((n as f32 * 0.37 + t) % 1.0));
                px.extend([r as u8, g as u8, b as u8, 255]);
            }
        }
        let key = format!("demo:{n}");
        let tex = ctx.load_texture(key.clone(), egui::ColorImage::from_rgba_unmultiplied([w, h], &px), egui::TextureOptions::LINEAR);
        s.thumbs.insert(key.clone(), Some(tex));
        key
    };
    let dir = s.set.out_dir();
    let q = |height, size| probe::Quality { height, size };
    let info = |title: &str, dur: u64| Info {
        title: title.into(),
        duration: Some(dur),
        qualities: vec![q(2160, Some(910_000_000)), q(1080, Some(240_000_000)), q(720, Some(120_000_000)), q(480, None)],
        has_audio: true,
        thumbnail: None,
    };
    let video = |h| Opts { audio: false, container: settings::Container::Mp4, height: h };
    let audio = Opts { audio: true, container: settings::Container::Mp4, height: None };
    let mut jobs: Vec<Job> = vec![];
    let mut add = |s: &mut Shared, batch: u64, group: Option<&str>, url: &str, title: &str, o: Opts, state: State, n: u8| {
        let c = Cand { url: url.into(), referer: None, ua: None, title: None };
        let mut j = Job::pending(c, o, dir.clone(), batch);
        j.shown = Some(title.into());
        j.group = group.map(String::from);
        j.is_entry = group.is_some();
        j.duration = Some(125 + n as u64 * 400);
        if n > 0 {
            j.thumb = Some(picture(s, n));
        }
        if matches!(state, State::Ready) {
            j.info = Some(info(title, j.duration.unwrap()));
        }
        if state == State::Done {
            j.out = Some(PathBuf::from("/tmp/demo.mp4"));
        }
        j.state = state;
        jobs.push(j);
    };
    if mode == "playlist" {
        let g = Some("Introduction to photography, 2026 lessons");
        let titles = ["Lesson 1 - Introduction and method", "Lesson 2 - Light and exposure", "Lesson 3 - Composition", "Lesson 4 - Editing and archives", "Lesson 5 - Printing and display"];
        for (k, t) in titles.iter().enumerate() {
            let state = if k < 3 { State::Ready } else { State::Fetching };
            add(&mut s, 1, g, &format!("https://v.test/{k}"), t, video(None), state, if k < 4 { k as u8 + 1 } else { 0 });
        }
        s.jobs = jobs;
        s.status = "Ready".into();
        return;
    }
    add(&mut s, 7, None, "https://www.youtube.com/watch?v=abc", "Lesson 3 - Introduction to photography: light, composition and exposure, explained from scratch", video(Some(1080)), State::Running(43.0), 1);
    add(&mut s, 6, None, "https://soundcloud.com/a/b", "A song with a rather long title", audio, State::Running(78.0), 2);
    add(&mut s, 5, None, "https://vimeo.com/123", "Interview with a researcher", video(Some(720)), State::Ready, 3);
    add(&mut s, 4, None, "https://cdn.site.test/video/master.m3u8", "Conference 2026", video(Some(720)), State::Done, 4);
    add(&mut s, 3, None, "https://example.com/private", "Private video", video(None), State::Failed("ERROR: [generic] Unable to download webpage: HTTP Error 403: Forbidden (caused by <HTTPError 403>)".into()), 0);
    add(&mut s, 2, None, "https://example.org/clip", "Cancelled clip", video(None), State::Cancelled, 0);
    add(&mut s, 1, None, "https://example.net/page-with-video", "https://example.net/page-with-video", video(None), State::NoInfo("Unsupported URL: https://example.net/page-with-video".into()), 0);
    add(&mut s, 8, None, "https://example.net/being-read", "https://example.net/being-read", video(None), State::Fetching, 0);
    s.jobs = jobs;
    s.status = "Ready".into();
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

pub enum Action {
    /// The text of the address field.
    AddTyped,
    /// A video the browser found, to be read by yt-dlp and given options like any other.
    AddCand(Cand),
    /// Straight to the queue (the big button).
    Download(Cand),
    /// The best candidate first, the others as fallbacks.
    DownloadAll(Vec<Cand>),
    /// Type (audio only or not) and picture height of one video.
    Choose(usize, bool, Option<u32>),
    Start(usize),
    StartAll,
    StartBatch(u64),
    HideBatch(u64),
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

/// What the window needs to draw one frame, copied so the lock is not held while drawing.
pub struct Snapshot {
    pub found: Vec<Found>,
    pub jobs: Vec<Job>,
    pub history: Vec<HistoryItem>,
    pub status: String,
    pub set: Settings,
    pub armed: bool,
    pub page_title: String,
    pub thumbs: HashMap<String, egui::TextureHandle>,
}

pub struct App {
    sy: Sy,
    ctx: egui::Context,
    pub input: String,
    /// The last Add was refused because the text is not a link.
    pub bad_input: bool,
    browser: Option<Child>,
    pub show_settings: bool,
}

impl App {
    pub fn new(ctx: egui::Context) -> Self {
        let set = Settings::load();
        let sy: Sy = Arc::new((Mutex::new(Shared::new(set, settings::load_history())), Condvar::new()));
        #[cfg(debug_assertions)]
        if let Ok(mode) = std::env::var("CAMALLO_DEMO") {
            demo(&sy, &ctx, &mode);
        }
        let (s1, c1) = (sy.clone(), ctx.clone());
        std::thread::spawn(move || setup(s1, c1));
        for _ in 0..MAX_PARALLEL {
            let (s2, c2) = (sy.clone(), ctx.clone());
            std::thread::spawn(move || worker(s2, c2));
        }
        for _ in 0..FETCHERS {
            let (s2, c2) = (sy.clone(), ctx.clone());
            std::thread::spawn(move || fetcher(s2, c2));
        }
        for _ in 0..THUMBERS {
            let (s2, c2) = (sy.clone(), ctx.clone());
            std::thread::spawn(move || thumber(s2, c2));
        }
        Self { sy, ctx, input: String::new(), bad_input: false, browser: None, show_settings: false }
    }

    pub fn snapshot(&self) -> Snapshot {
        let mut s = lock(&self.sy.0);
        s.prune_thumbs();
        Snapshot {
            found: s.found.clone(),
            jobs: s.jobs.clone(),
            history: s.history.clone(),
            status: s.status.clone(),
            set: s.set.clone(),
            armed: s.armed,
            page_title: s.page_title.clone(),
            thumbs: s.thumbs.iter().filter_map(|(k, v)| Some((k.clone(), v.clone()?))).collect(),
        }
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

    pub fn handle(&mut self, a: Action) {
        let (m, cv) = &*self.sy;
        match a {
            Action::AddTyped => {
                if add_typed(&self.sy, &self.input) {
                    self.input.clear();
                    self.bad_input = false;
                } else {
                    self.bad_input = !self.input.trim().is_empty();
                }
            }
            Action::AddCand(c) => {
                lock(m).add_pending(c);
                cv.notify_all();
            }
            Action::Download(c) => queue(&self.sy, c, None, vec![]),
            Action::DownloadAll(mut cs) => {
                if !cs.is_empty() {
                    let first = cs.remove(0);
                    queue(&self.sy, first, None, cs);
                }
            }
            Action::Choose(i, audio, height) => {
                let mut s = lock(m);
                if s.jobs[i].state == State::Ready {
                    s.jobs[i].opts.audio = audio;
                    s.jobs[i].opts.height = height;
                }
            }
            Action::Start(i) => {
                start_job(&mut lock(m), i);
                cv.notify_all();
            }
            Action::StartAll => {
                let mut s = lock(m);
                for i in 0..s.jobs.len() {
                    start_job(&mut s, i);
                }
                cv.notify_all();
            }
            Action::StartBatch(b) => {
                let mut s = lock(m);
                for i in 0..s.jobs.len() {
                    if s.jobs[i].batch == b {
                        start_job(&mut s, i);
                    }
                }
                cv.notify_all();
            }
            Action::HideBatch(b) => {
                for j in lock(m).jobs.iter_mut().filter(|j| j.batch == b && !matches!(j.state, State::Queued | State::Running(_))) {
                    j.hidden = true;
                }
            }
            Action::Retry(i) => {
                retry(&mut lock(m), i);
                cv.notify_all();
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
                let mut s = lock(m);
                let moved = s.set.audio_only != n.audio_only || s.set.quality != n.quality;
                s.set = n;
                if moved {
                    apply_defaults(&mut s);
                }
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
    pub fn cand(&self) -> Cand {
        Cand {
            url: self.url.clone(),
            referer: nz(&self.referer),
            ua: nz(&self.ua),
            title: (self.kind.named_by_page() && !self.title.is_empty()).then(|| self.title.clone()),
        }
    }
}

/// Candidates in the order to try them, like the button in the browser: the list is already ranked.
pub fn best_cands(found: &[Found]) -> Vec<Cand> {
    found.iter().take(6).map(Found::cand).collect()
}

fn nz(s: &str) -> Option<String> {
    (!s.is_empty()).then(|| s.to_string())
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
    use crate::settings::{Container, Quality};
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
        // It waits for the user: a card to read and choose, not a queued download.
        assert_eq!(s.jobs[0].state, State::Fetching);
        assert!(s.jobs[0].manual && s.queue.is_empty());
    }

    fn forged_download(n: usize) -> serde_json::Value {
        json!({"kind": "download", "title": "x", "candidates": [
            {"url": format!("https://evil.test/{n}.mp4"), "referer": "https://evil.test/", "ua": "UA"}
        ]})
    }

    #[test]
    fn a_download_message_never_starts_anything_even_with_start_at_once() {
        let set = Settings { auto_start: true, ..Settings::default() };
        let sy: Sy = Arc::new((Mutex::new(Shared::new(set, vec![])), Condvar::new()));
        feed(&sy, forged_download(0));
        let mut s = lock(&sy.0);
        assert!(s.queue.is_empty() && s.jobs[0].state == State::Fetching);
        // yt-dlp reads it: the card becomes Ready and stays there.
        let info = Info { title: "t".into(), duration: None, qualities: vec![], has_audio: true, thumbnail: None };
        apply_fetched(&mut s, 0, Ok(Fetched::Video(info.clone())));
        assert_eq!(s.jobs[0].state, State::Ready);
        assert!(s.queue.is_empty());
        // The user's own Download does start it.
        start_job(&mut s, 0);
        assert_eq!(s.queue.iter().copied().collect::<Vec<_>>(), [0]);
        // Typed addresses keep honouring "Start at once".
        drop(s);
        assert!(add_typed(&sy, "https://example.com/typed"));
        let mut s = lock(&sy.0);
        apply_fetched(&mut s, 1, Ok(Fetched::Video(info)));
        assert_eq!(s.jobs[1].state, State::Queued);
    }

    #[test]
    fn the_videos_of_a_list_from_the_browser_also_wait_for_the_user() {
        let set = Settings { auto_start: true, ..Settings::default() };
        let sy: Sy = Arc::new((Mutex::new(Shared::new(set, vec![])), Condvar::new()));
        feed(&sy, forged_download(0));
        let info = Info { title: "t".into(), duration: None, qualities: vec![], has_audio: true, thumbnail: None };
        let entry = Entry { url: "https://evil.test/1".into(), title: "one".into(), duration: None, thumb: None, info: Some(info) };
        let mut s = lock(&sy.0);
        apply_fetched(&mut s, 0, Ok(Fetched::List { title: "list".into(), entries: vec![entry] }));
        assert_eq!(s.jobs[1].state, State::Ready);
        assert!(s.queue.is_empty());
    }

    #[test]
    fn a_flood_of_messages_from_a_page_stays_small() {
        let sy: Sy = Arc::new((Mutex::new(Shared::new(Settings::default(), vec![])), Condvar::new()));
        feed(&sy, page("https://evil.test/", "x"));
        for n in 0..500 {
            // A size is given, so no request goes out from the test.
            feed(&sy, json!({"kind": "media", "url": format!("https://evil.test/{n}.mp4"), "referer": "https://evil.test/", "ua": "UA", "size": 9_000_000}));
            feed(&sy, forged_download(n));
        }
        let s = lock(&sy.0);
        assert_eq!(s.found.len(), MAX_FOUND);
        assert_eq!(s.jobs.len(), MAX_BROWSER_CARDS);
        assert!(s.queue.is_empty());
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
    fn pictures_of_removed_cards_are_forgotten() {
        let mut s = Shared::new(Settings::default(), vec![]);
        let c = |u: &str| Cand { url: u.into(), referer: None, ua: None, title: None };
        for n in 0..3 {
            let mut j = Job::pending(c(&format!("https://v.test/{n}")), Opts::from_settings(&s.set), PathBuf::from("o"), 1);
            j.thumb = Some(format!("https://img.test/{n}.jpg"));
            s.jobs.push(j);
            s.want_thumb(&format!("https://img.test/{n}.jpg"));
        }
        s.jobs[1].hidden = true;
        s.prune_thumbs();
        let mut keys: Vec<&str> = s.thumbs.keys().map(String::as_str).collect();
        keys.sort();
        assert_eq!(keys, ["https://img.test/0.jpg", "https://img.test/2.jpg"]);
        // A picture asked for again after its card came back is fetched again.
        s.jobs[1].hidden = false;
        s.want_thumb("https://img.test/1.jpg");
        assert_eq!(s.thumbs.len(), 3);
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

    fn shared() -> Shared {
        Shared::new(Settings::default(), vec![])
    }
    fn addr(u: &str) -> Cand {
        Cand { url: u.into(), referer: None, ua: None, title: None }
    }
    fn info(title: &str, heights: &[u32]) -> Info {
        Info {
            title: title.into(),
            duration: Some(60),
            qualities: heights.iter().map(|h| probe::Quality { height: *h, size: None }).collect(),
            has_audio: true,
            thumbnail: Some(format!("https://i/{title}.jpg")),
        }
    }

    #[test]
    fn a_read_video_waits_for_download_with_the_default_choice() {
        let mut s = shared();
        s.set.quality = Quality::P1080;
        let i = s.add_pending(addr("https://site.test/v"));
        assert_eq!(s.jobs[i].state, State::Fetching);
        assert_eq!(s.fetchq.iter().copied().collect::<Vec<_>>(), [Fetch::List(i)]);
        apply_fetched(&mut s, i, Ok(Fetched::Video(info("Solo", &[2160, 720, 480]))));
        let j = &s.jobs[i];
        assert_eq!(j.state, State::Ready);
        assert_eq!(j.label(), "Solo");
        assert_eq!(j.opts.height, Some(720), "1080p is not offered: the best below it is");
        assert!(s.queue.is_empty(), "nothing downloads until the user says so");
        assert_eq!(s.thumbq.iter().cloned().collect::<Vec<_>>(), ["https://i/Solo.jpg"]);
    }

    #[test]
    fn a_list_replaces_its_address_with_one_card_per_video_in_order() {
        let mut s = shared();
        let i = s.add_pending(addr("https://site.test/list"));
        let e = |n: &str, with_info: bool| Entry {
            url: format!("https://v.test/{n}"),
            title: n.into(),
            duration: Some(5),
            thumb: Some(format!("https://i/{n}.jpg")),
            info: with_info.then(|| info(n, &[720])),
        };
        apply_fetched(&mut s, i, Ok(Fetched::List { title: "Corso".into(), entries: vec![e("a", false), e("b", true), e("c", false)] }));
        assert!(s.jobs[i].hidden, "the address itself gives way");
        let cards: Vec<&Job> = s.jobs.iter().filter(|j| !j.hidden).collect();
        assert_eq!(cards.iter().map(|j| j.label()).collect::<Vec<_>>(), ["a", "b", "c"]);
        let batch = s.jobs[i].batch;
        assert!(cards.iter().all(|j| j.batch == batch && j.group.as_deref() == Some("Corso") && j.is_entry));
        assert_eq!(cards.iter().map(|j| j.state.clone()).collect::<Vec<_>>(), [State::Fetching, State::Ready, State::Fetching]);
        // Only the videos that came without formats need their own request.
        let details: Vec<Fetch> = s.fetchq.iter().copied().filter(|f| matches!(f, Fetch::Detail(_))).collect();
        assert_eq!(details, [Fetch::Detail(1), Fetch::Detail(3)]);
        assert_eq!(s.thumbq.len(), 3);
    }

    #[test]
    fn an_unreadable_address_keeps_its_card_with_the_reason_and_retry_asks_again() {
        let mut s = shared();
        let i = s.add_pending(addr("https://site.test/odd"));
        s.fetchq.clear();
        apply_fetched(&mut s, i, Err("Unsupported URL".into()));
        assert_eq!(s.jobs[i].state, State::NoInfo("Unsupported URL".into()));
        retry(&mut s, i);
        assert_eq!(s.jobs[i].state, State::Fetching);
        assert_eq!(s.fetchq.iter().copied().collect::<Vec<_>>(), [Fetch::List(i)]);
        // A video of a list is asked for on its own.
        s.jobs[i].is_entry = true;
        s.jobs[i].state = State::NoInfo("x".into());
        s.fetchq.clear();
        retry(&mut s, i);
        assert_eq!(s.fetchq.iter().copied().collect::<Vec<_>>(), [Fetch::Detail(i)]);
    }

    #[test]
    fn a_card_removed_while_it_was_being_read_stays_removed() {
        let mut s = shared();
        let i = s.add_pending(addr("https://site.test/v"));
        s.jobs[i].hidden = true;
        apply_fetched(&mut s, i, Ok(Fetched::Video(info("x", &[720]))));
        assert_eq!(s.jobs[i].state, State::Fetching);
    }

    #[test]
    fn download_starts_only_what_is_ready_with_the_folder_and_container_of_that_moment() {
        let mut s = shared();
        let a = s.add_pending(addr("https://a.test/aaa"));
        let b = s.add_pending(addr("https://b.test/bbb"));
        apply_fetched(&mut s, a, Ok(Fetched::Video(info("A", &[720]))));
        s.set.container = Container::Mkv;
        s.set.subdir = "Corsi".into();
        for i in 0..s.jobs.len() {
            start_job(&mut s, i);
        }
        assert_eq!((s.jobs[a].state.clone(), s.jobs[b].state.clone()), (State::Queued, State::Fetching));
        assert_eq!(s.jobs[a].opts.container, Container::Mkv);
        assert!(s.jobs[a].dir.ends_with("Corsi"));
        start_job(&mut s, a); // already queued: no second entry
        assert_eq!(s.queue.iter().copied().collect::<Vec<_>>(), [a]);
    }

    #[test]
    fn auto_start_queues_a_video_as_soon_as_it_is_read() {
        let mut s = shared();
        s.set.auto_start = true;
        let i = s.add_pending(addr("https://a.test/aaa"));
        apply_fetched(&mut s, i, Ok(Fetched::Video(info("A", &[720]))));
        assert_eq!(s.jobs[i].state, State::Queued);
        assert_eq!(s.queue.iter().copied().collect::<Vec<_>>(), [i]);
    }

    #[test]
    fn new_defaults_reach_ready_videos_only() {
        let mut s = shared();
        let a = s.add_pending(addr("https://a.test/aaa"));
        apply_fetched(&mut s, a, Ok(Fetched::Video(info("A", &[1080, 480]))));
        let b = s.add_pending(addr("https://b.test/bbb"));
        s.jobs[b].state = State::Running(10.0);
        s.set.audio_only = true;
        s.set.quality = Quality::P720;
        apply_defaults(&mut s);
        assert_eq!((s.jobs[a].opts.audio, s.jobs[a].opts.height), (true, Some(480)));
        assert_eq!((s.jobs[b].opts.audio, s.jobs[b].opts.height), (false, None));
    }

    #[test]
    fn heights_snap_to_what_exists() {
        let i = info("x", &[1080, 720, 480]);
        assert_eq!(snap_height(None, &i), None);
        assert_eq!(snap_height(Some(1080), &i), Some(1080));
        assert_eq!(snap_height(Some(900), &i), Some(720));
        assert_eq!(snap_height(Some(360), &i), None, "everything is taller: take the best");
        assert_eq!(snap_height(Some(720), &Info::default()), None);
    }

    #[test]
    fn only_links_are_added_from_the_field() {
        let sy: Sy = Arc::new((Mutex::new(shared()), Condvar::new()));
        assert!(!add_typed(&sy, "ciao a tutti"));
        assert!(!add_typed(&sy, "   "));
        assert!(add_typed(&sy, "  https://site.test/video  "));
        let s = lock(&sy.0);
        assert_eq!(s.jobs.len(), 1);
        assert_eq!((s.jobs[0].state.clone(), s.jobs[0].url.as_str()), (State::Fetching, "https://site.test/video"));
    }
}
