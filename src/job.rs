use crate::settings::{Container, Settings};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

#[derive(Clone, PartialEq, Debug)]
pub enum State {
    Queued,
    Running(f32),
    Done,
    Failed(String),
    Cancelled,
}

/// One way to fetch a video: an address, the headers the page used and, for plain streams, a file name.
#[derive(Clone, Debug, PartialEq)]
pub struct Cand {
    pub url: String,
    pub referer: Option<String>,
    pub ua: Option<String>,
    pub title: Option<String>,
}

/// What to produce from the address.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Opts {
    pub audio: bool,
    pub container: Container,
    /// Tallest picture accepted, None for the best available.
    pub height: Option<u32>,
}

impl Opts {
    pub fn from_settings(s: &Settings) -> Opts {
        Opts { audio: s.audio_only, container: s.container, height: s.quality.height() }
    }
}

/// Shared with the UI so a running download can be stopped from outside its thread.
#[derive(Default)]
pub struct Ctl {
    cancelled: AtomicBool,
    pid: AtomicU32,
}

impl Ctl {
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
        let pid = self.pid.load(Ordering::SeqCst);
        if pid != 0 {
            kill_tree(pid);
        }
    }
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }
}

#[derive(Clone)]
pub struct Job {
    pub url: String,
    pub referer: Option<String>,
    pub ua: Option<String>,
    pub title: Option<String>,
    pub opts: Opts,
    /// Where the file goes; fixed when the job is queued.
    pub dir: PathBuf,
    pub state: State,
    /// Other ways to try, in order, when the first one fails.
    pub alts: Vec<Cand>,
    /// Final file, known once the download is done.
    pub out: Option<PathBuf>,
    pub ctl: Arc<Ctl>,
    /// Removed from the list by the user; the slot stays so indexes held by workers remain valid.
    pub hidden: bool,
}

impl Job {
    pub fn new(c: Cand, opts: Opts, dir: PathBuf, alts: Vec<Cand>) -> Job {
        Job {
            url: c.url,
            referer: c.referer,
            ua: c.ua,
            title: c.title,
            opts,
            dir,
            state: State::Queued,
            alts,
            out: None,
            ctl: Arc::new(Ctl::default()),
            hidden: false,
        }
    }
    fn cand(&self) -> Cand {
        Cand { url: self.url.clone(), referer: self.referer.clone(), ua: self.ua.clone(), title: self.title.clone() }
    }
    /// Name shown in lists: the page title when known, else the address.
    pub fn label(&self) -> String {
        self.title.clone().unwrap_or_else(|| self.url.clone())
    }
}

/// Stops a process and everything it started (yt-dlp.exe spawns the real interpreter and ffmpeg).
fn kill_tree(pid: u32) {
    #[cfg(windows)]
    {
        let mut c = Command::new("taskkill");
        let _ = quiet(&mut c).args(["/PID", &pid.to_string(), "/T", "/F"]).output();
    }
    #[cfg(not(windows))]
    {
        let _ = Command::new("kill").args(["-KILL", &pid.to_string()]).output();
    }
}

/// Hide the console window that Windows opens for child processes.
pub fn quiet(c: &mut Command) -> &mut Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0800_0000);
    }
    c
}

/// Accept only a single http(s) URL; anything else is not passed to yt-dlp.
pub fn valid_url(s: &str) -> Option<String> {
    let s = s.trim();
    let ok = (s.starts_with("http://") || s.starts_with("https://"))
        && !s.contains(char::is_whitespace)
        && s.len() > 10;
    ok.then(|| s.to_string())
}

/// A page title made safe to use as a file name, or None when nothing usable is left.
pub fn sanitize_title(t: &str) -> Option<String> {
    let cleaned: String = t.chars().map(|c| if "<>:\"/\\|?*".contains(c) || c.is_control() { ' ' } else { c }).collect();
    let joined = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    let s: String = joined.chars().take(120).collect();
    let s = s.trim_matches(|c: char| c == '.' || c.is_whitespace()).to_string();
    let stem = s.split('.').next().unwrap_or("").to_uppercase();
    let reserved = ["CON", "PRN", "AUX", "NUL"].contains(&stem.as_str())
        || ((stem.starts_with("COM") || stem.starts_with("LPT")) && stem.len() == 4 && stem[3..].chars().all(|c| c.is_ascii_digit()));
    (!s.is_empty() && !reserved).then_some(s)
}

/// First name that does not clash with an existing file: "x", then "x (1)", "x (2)"...
pub fn unique_stem(dir: &Path, stem: &str, ext: &str) -> String {
    let mut n = 0;
    loop {
        let s = if n == 0 { stem.to_string() } else { format!("{stem} ({n})") };
        if !dir.join(format!("{s}.{ext}")).exists() {
            return s;
        }
        n += 1;
    }
}

/// yt-dlp format selector: the best picture not taller than `height`, with the best sound; if nothing fits, the best there is.
pub fn video_selector(height: Option<u32>) -> String {
    match height {
        Some(h) => format!("bv*[height<={h}]+ba/b[height<={h}]/bv*+ba/b"),
        None => "bv*+ba/b".into(),
    }
}

pub fn args(job: &Job, ffmpeg_dir: &Path) -> Vec<String> {
    let mut a: Vec<String> = [
        "--no-playlist",
        "--newline",
        "--color",
        "never",
        "--progress",
        "--progress-template",
        "download:PCT %(progress._percent_str)s",
        "--print",
        "after_move:FILE %(filepath)s",
        "--ffmpeg-location",
    ]
    .map(String::from)
    .into();
    a.push(ffmpeg_dir.to_string_lossy().into());

    let ext = if job.opts.audio { "mp3" } else { job.opts.container.ext() };
    let name = match job.title.as_deref().and_then(sanitize_title) {
        // The title is literal text, so a percent sign must not be read as a template field.
        Some(t) => unique_stem(&job.dir, &t, ext).replace('%', "%%"),
        None => "%(title).150B".into(),
    };
    a.push("-o".into());
    a.push(job.dir.join(format!("{name}.%(ext)s")).to_string_lossy().into());

    if job.opts.audio {
        a.extend(["-x", "--audio-format", "mp3", "--audio-quality", "0"].map(String::from));
    } else {
        a.push("-f".into());
        a.push(video_selector(job.opts.height));
        match job.opts.container {
            Container::Mp4 => {
                a.extend(["-S", "res,vcodec:h264,acodec:aac", "--merge-output-format", "mp4", "--remux-video", "mp4"].map(String::from))
            }
            Container::Mkv => a.extend(["--merge-output-format", "mkv", "--remux-video", "mkv"].map(String::from)),
            Container::Webm => a.extend(["--merge-output-format", "mkv", "--recode-video", "webm"].map(String::from)),
        }
    }
    if let Some(r) = &job.referer {
        a.extend(["--referer".into(), r.clone()]);
    }
    if let Some(u) = &job.ua {
        a.extend(["--add-header".into(), format!("User-Agent:{u}")]);
    }
    a.push(job.url.clone());
    a
}

/// Runs yt-dlp to completion, reporting percent progress. Ok carries the final file when yt-dlp printed it.
pub fn run(yt: &Path, job: &Job, ffmpeg_dir: &Path, mut progress: impl FnMut(f32)) -> Result<Option<PathBuf>, String> {
    if job.ctl.is_cancelled() {
        return Err("Cancelled".into());
    }
    let mut cmd = Command::new(yt);
    quiet(&mut cmd)
        .args(args(job, ffmpeg_dir))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().map_err(|e| format!("cannot start yt-dlp: {e}"))?;
    job.ctl.pid.store(child.id(), Ordering::SeqCst);
    if job.ctl.is_cancelled() {
        kill_tree(child.id()); // cancelled between the check and the spawn
    }

    let last_err = Arc::new(Mutex::new(String::new()));
    let drm = Arc::new(AtomicBool::new(false));
    let err_pipe = child.stderr.take().unwrap();
    let (le, dr) = (last_err.clone(), drm.clone());
    let t = std::thread::spawn(move || {
        for line in BufReader::new(err_pipe).lines().map_while(Result::ok) {
            if line.contains("DRM") {
                dr.store(true, Ordering::SeqCst);
            }
            if line.starts_with("ERROR:") {
                *le.lock().unwrap() = line;
            }
        }
    });
    let mut out: Option<PathBuf> = None;
    for line in BufReader::new(child.stdout.take().unwrap()).lines().map_while(Result::ok) {
        if let Some(p) = line.strip_prefix("PCT ").and_then(|p| p.trim().trim_end_matches('%').parse::<f32>().ok()) {
            progress(p);
        } else if let Some(f) = line.strip_prefix("FILE ") {
            out = Some(PathBuf::from(f.trim()));
        }
    }
    let status = child.wait().map_err(|e| e.to_string());
    job.ctl.pid.store(0, Ordering::SeqCst);
    let _ = t.join();
    if job.ctl.is_cancelled() {
        return Err("Cancelled".into());
    }
    let status = status?;
    if status.success() {
        return Ok(out);
    }
    if drm.load(Ordering::SeqCst) {
        return Err("This video seems to be protected by DRM, which Camallo cannot download".into());
    }
    let e = last_err.lock().unwrap().clone();
    let e = e.trim_start_matches("ERROR:").trim();
    Err(if e.is_empty() {
        format!("yt-dlp exited with {status}")
    } else {
        e.chars().take(160).collect()
    })
}

/// Tries the job's address, then each alternative, until one works.
pub fn run_chain(yt: &Path, job: &Job, ffmpeg_dir: &Path, mut progress: impl FnMut(f32)) -> Result<Option<PathBuf>, String> {
    let mut tries = vec![job.cand()];
    tries.extend(job.alts.iter().cloned());
    let n = tries.len();
    let mut last = String::new();
    for c in tries {
        let j = Job { url: c.url, referer: c.referer, ua: c.ua, title: c.title, alts: vec![], ..job.clone() };
        match run(yt, &j, ffmpeg_dir, &mut progress) {
            Ok(p) => return Ok(p),
            Err(e) if job.ctl.is_cancelled() => return Err(e),
            Err(e) => last = e,
        }
    }
    Err(if n > 1 { format!("tried {n} ways, last error: {last}") } else { last })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cand(u: &str) -> Cand {
        Cand { url: u.into(), referer: None, ua: None, title: None }
    }
    fn opts() -> Opts {
        Opts { audio: false, container: Container::Mp4, height: None }
    }
    fn job(o: Opts) -> Job {
        Job::new(cand("https://example.com/v"), o, PathBuf::from("out"), vec![])
    }
    fn has(v: &[String], pair: [&str; 2]) -> bool {
        v.windows(2).any(|w| w == pair)
    }

    #[test]
    fn url_validation() {
        assert!(valid_url("  https://example.com/video  ").is_some());
        assert!(valid_url("file:///etc/passwd").is_none());
        assert!(valid_url("https://a.com/x --exec calc").is_none());
        assert!(valid_url("--version").is_none());
        assert!(valid_url("").is_none());
    }

    #[test]
    fn titles_become_safe_file_names() {
        assert_eq!(sanitize_title("  Lezione 1: Sociologia / media?  ").as_deref(), Some("Lezione 1 Sociologia media"));
        assert_eq!(sanitize_title("a\nb\t c").as_deref(), Some("a b c"));
        assert_eq!(sanitize_title("..."), None);
        assert_eq!(sanitize_title("   "), None);
        assert_eq!(sanitize_title("NUL"), None);
        assert_eq!(sanitize_title("com1.mp4"), None);
        assert_eq!(sanitize_title("console").as_deref(), Some("console"));
        assert_eq!(sanitize_title(&"x".repeat(300)).unwrap().chars().count(), 120);
    }

    #[test]
    fn names_never_overwrite() {
        let dir = std::env::temp_dir().join(format!("camallo-uniq-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(unique_stem(&dir, "clip", "mp4"), "clip");
        std::fs::write(dir.join("clip.mp4"), b"x").unwrap();
        assert_eq!(unique_stem(&dir, "clip", "mp4"), "clip (1)");
        std::fs::write(dir.join("clip (1).mp4"), b"x").unwrap();
        assert_eq!(unique_stem(&dir, "clip", "mp4"), "clip (2)");
        assert_eq!(unique_stem(&dir, "clip", "mkv"), "clip");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn selector_caps_height_with_fallback() {
        assert_eq!(video_selector(None), "bv*+ba/b");
        let s = video_selector(Some(720));
        assert!(s.starts_with("bv*[height<=720]+ba/") && s.ends_with("/bv*+ba/b"));
    }

    #[test]
    fn args_by_container_and_audio() {
        let f = PathBuf::from("bin");
        let v = args(&job(opts()), &f);
        assert_eq!(v.last().unwrap(), "https://example.com/v");
        assert!(has(&v, ["--remux-video", "mp4"]) && !v.contains(&"-x".to_string()));
        assert!(has(&v, ["--print", "after_move:FILE %(filepath)s"]));

        let v = args(&job(Opts { container: Container::Mkv, ..opts() }), &f);
        assert!(has(&v, ["--remux-video", "mkv"]));
        let v = args(&job(Opts { container: Container::Webm, ..opts() }), &f);
        assert!(has(&v, ["--recode-video", "webm"]));
        let v = args(&job(Opts { audio: true, ..opts() }), &f);
        assert!(v.contains(&"-x".to_string()) && !v.contains(&"-f".to_string()));
        let v = args(&job(Opts { height: Some(480), ..opts() }), &f);
        assert!(has(&v, ["-f", &video_selector(Some(480))]));
    }

    #[test]
    fn args_name_from_page_title_and_escape_percent() {
        let mut j = job(opts());
        j.title = Some("Sconto 50% / offerta".into());
        let v = args(&j, &PathBuf::from("b"));
        let o = v.iter().position(|a| a == "-o").map(|i| v[i + 1].clone()).unwrap();
        assert!(o.ends_with("Sconto 50%% offerta.%(ext)s"), "{o}");
        // Without a usable title yt-dlp names the file.
        j.title = Some("???".into());
        let v = args(&j, &PathBuf::from("b"));
        let o = v.iter().position(|a| a == "-o").map(|i| v[i + 1].clone()).unwrap();
        assert!(o.ends_with("%(title).150B.%(ext)s"), "{o}");
    }

    #[test]
    fn args_browser_headers() {
        let mut j = job(opts());
        j.referer = Some("https://site/page".into());
        j.ua = Some("UA/1".into());
        let v = args(&j, &PathBuf::from("b"));
        assert!(has(&v, ["--referer", "https://site/page"]));
        assert!(v.contains(&"User-Agent:UA/1".to_string()));
    }

    #[cfg(unix)]
    fn fake_yt(dir: &Path, body: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        std::fs::create_dir_all(dir).unwrap();
        let fake = dir.join("fake-yt");
        std::fs::write(&fake, body).unwrap();
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        fake
    }

    #[cfg(unix)]
    #[test]
    fn chain_falls_back_until_one_works() {
        let dir = std::env::temp_dir().join(format!("camallo-chain-{}", std::process::id()));
        // Fails when the last argument (the address) contains "bad", like yt-dlp on an unsupported page.
        let fake = fake_yt(
            &dir,
            "#!/bin/sh\nfor a; do last=$a; done\ncase $last in *bad*) echo 'ERROR: nope' >&2; exit 1;; esac\necho 'PCT 50.0%'\necho 'FILE /videos/x.mp4'\n",
        );
        let mut j = job(opts());
        j.url = "https://bad.test/1".into();
        j.alts = vec![cand("https://bad.test/2"), cand("https://good.test/3")];
        let mut seen = vec![];
        let r = run_chain(&fake, &j, &dir, |p| seen.push(p));
        assert_eq!(r, Ok(Some(PathBuf::from("/videos/x.mp4"))));
        assert_eq!(seen, [50.0]);

        j.alts = vec![cand("https://bad.test/2")];
        let e = run_chain(&fake, &j, &dir, |_| {}).unwrap_err();
        assert!(e.starts_with("tried 2 ways") && e.contains("nope"), "{e}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn drm_gets_its_own_message() {
        let dir = std::env::temp_dir().join(format!("camallo-drm-{}", std::process::id()));
        let fake = fake_yt(&dir, "#!/bin/sh\necho 'ERROR: [x] This video is DRM protected' >&2\nexit 1\n");
        let e = run(&fake, &job(opts()), &dir, |_| {}).unwrap_err();
        assert!(e.contains("DRM") && e.contains("cannot download"), "{e}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn cancel_stops_a_running_download() {
        let dir = std::env::temp_dir().join(format!("camallo-cancel-{}", std::process::id()));
        let fake = fake_yt(&dir, "#!/bin/sh\necho 'PCT 10.0%'\nexec sleep 30\n");
        let j = job(opts());
        let ctl = j.ctl.clone();
        let started = std::time::Instant::now();
        let h = std::thread::spawn({
            let (fake, dir, j) = (fake.clone(), dir.clone(), j.clone());
            move || run_chain(&fake, &j, &dir, |_| {})
        });
        std::thread::sleep(std::time::Duration::from_millis(500));
        ctl.cancel();
        let r = h.join().unwrap();
        assert_eq!(r, Err("Cancelled".to_string()));
        assert!(started.elapsed().as_secs() < 10, "cancel should not wait for the process");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// Runs the real yt-dlp against a local web server. Set CAMALLO_YTDLP, CAMALLO_FFDIR and CAMALLO_BASE
/// (for example http://127.0.0.1:8765) and run with `cargo test -- --ignored real`.
#[cfg(test)]
mod real {
    use super::*;

    fn opts() -> Opts {
        Opts { audio: false, container: Container::Mp4, height: None }
    }

    fn env() -> Option<(PathBuf, PathBuf, String)> {
        Some((
            std::env::var("CAMALLO_YTDLP").ok()?.into(),
            std::env::var("CAMALLO_FFDIR").ok()?.into(),
            std::env::var("CAMALLO_BASE").ok()?,
        ))
    }

    fn fresh(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("camallo-real-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn job_for(url: String, title: Option<&str>, o: Opts, dir: &Path) -> Job {
        let c = Cand { url, referer: None, ua: Some("CamalloTest/1".into()), title: title.map(String::from) };
        Job::new(c, o, dir.to_path_buf(), vec![])
    }

    /// Picture size as ffmpeg reports it.
    fn size_of(ff: &Path, f: &Path) -> String {
        let o = Command::new(ff.join("ffmpeg")).arg("-i").arg(f).output().unwrap();
        let t = String::from_utf8_lossy(&o.stderr).into_owned();
        let l = t.lines().find(|l| l.contains("Video:")).unwrap_or("").to_string();
        l.split(|c: char| c == ' ' || c == ',').find(|w| w.len() > 3 && w.contains('x') && w.chars().all(|c| c.is_ascii_digit() || c == 'x')).unwrap_or("").to_string()
    }

    #[test]
    #[ignore]
    fn real_file_named_by_title_and_never_overwritten() {
        let Some((yt, ff, base)) = env() else { return };
        let dir = fresh("file");
        let j = job_for(format!("{base}/clip.mp4"), Some("Lezione 1: prova 50%"), opts(), &dir);
        let mut last = 0.0;
        let a = run_chain(&yt, &j, &ff, |p| last = p).unwrap().expect("path reported");
        assert!(a.ends_with("Lezione 1 prova 50%.mp4"), "{a:?}");
        assert!(a.metadata().unwrap().len() > 1000);
        assert!(last > 0.0);
        let b = run_chain(&yt, &j, &ff, |_| {}).unwrap().unwrap();
        assert!(b.ends_with("Lezione 1 prova 50% (1).mp4"), "{b:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    #[ignore]
    fn real_hls_height_cap_and_containers() {
        let Some((yt, ff, base)) = env() else { return };
        let dir = fresh("hls");
        let url = format!("{base}/hls/master.m3u8");
        let best = run_chain(&yt, &job_for(url.clone(), Some("best"), opts(), &dir), &ff, |_| {}).unwrap().unwrap();
        assert_eq!(size_of(&ff, &best), "640x360");
        let low = Opts { height: Some(200), ..opts() };
        let low = run_chain(&yt, &job_for(url.clone(), Some("low"), low, &dir), &ff, |_| {}).unwrap().unwrap();
        assert_eq!(size_of(&ff, &low), "320x180");
        let mkv = Opts { container: Container::Mkv, ..opts() };
        let mkv = run_chain(&yt, &job_for(url.clone(), Some("mkv"), mkv, &dir), &ff, |_| {}).unwrap().unwrap();
        assert!(mkv.extension().unwrap() == "mkv" && mkv.exists(), "{mkv:?}");
        let au = Opts { audio: true, ..opts() };
        let au = run_chain(&yt, &job_for(url, Some("sound"), au, &dir), &ff, |_| {}).unwrap().unwrap();
        assert!(au.extension().unwrap() == "mp3" && au.exists(), "{au:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    #[ignore]
    fn real_failure_has_a_short_reason() {
        let Some((yt, ff, base)) = env() else { return };
        let dir = fresh("fail");
        let e = run_chain(&yt, &job_for(format!("{base}/missing.mp4"), None, opts(), &dir), &ff, |_| {}).unwrap_err();
        assert!(!e.is_empty() && e.chars().count() <= 160, "{e}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    #[ignore]
    fn real_probe_lists_heights() {
        let Some((yt, _ff, base)) = env() else { return };
        let c = Cand { url: format!("{base}/hls/master.m3u8"), referer: None, ua: None, title: None };
        let i = crate::probe::probe(&yt, &c).unwrap();
        let h: Vec<u32> = i.qualities.iter().map(|q| q.height).collect();
        assert_eq!(h, [360, 180], "{i:?}");
        assert!(i.has_audio);
    }
}
