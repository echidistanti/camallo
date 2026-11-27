# Camallo

[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Latest release](https://img.shields.io/github/v/release/echidistanti/camallo)](https://github.com/echidistanti/camallo/releases/latest)
![Platform: Windows x64 and Linux x86_64](https://img.shields.io/badge/platform-Windows%20x64%20%C2%B7%20Linux%20x86__64%20(testing)-lightgrey)

A small app for saving videos, on Windows and, in testing, on Linux. Paste the link of a video or a playlist and pick the quality of each video, or open the built-in browser and let Camallo notice the video that a page is playing. The downloading itself is done by [yt-dlp](https://github.com/yt-dlp/yt-dlp) and [ffmpeg](https://ffmpeg.org/); Camallo is the interface, the queue and the part that finds videos yt-dlp does not know how to find on its own.

It is written in Rust, ships as a single executable on Windows and as an AppImage on Linux, and keeps its own copies of yt-dlp and ffmpeg up to date.

> Camallo is an independent project, written from scratch. It is not affiliated with, endorsed by or derived from the code of Downie, Video DownloadHelper, yt-dlp or ffmpeg.

## Contents

- [What it does](#what-it-does)
- [Install](#install)
- [Linux](#linux-testing)
- [Using Camallo](#using-camallo)
- [Settings](#settings)
- [Where things are stored](#where-things-are-stored)
- [How it works](#how-it-works)
- [Building from source](#building-from-source)
- [Releases and the installer](#releases-and-the-installer)
- [Repository layout](#repository-layout)
- [Responsible use](#responsible-use)
- [Third-party software](#third-party-software)
- [License](#license)

## What it does

- **Link in, cards out.** Paste a video or playlist address and press Add. yt-dlp reads it, and every video gets its own card with thumbnail, title, duration, estimated size and its own choice of type and quality. Nothing downloads until you press Download (unless you turn on [Start at once](#settings)).
- **Playlists stay together.** A playlist becomes a group of cards, one per video, up to 200. Lists inside lists, such as the tabs of a channel, are opened as well.
- **A browser that finds the video for you.** When yt-dlp cannot read a page, the card offers the built-in browser. Open the page, play the video, and a purple **Video detected** panel appears with a Download button. Detection works on what the network carries, so it does not depend on the page's markup.
- **Formats.** MP4, MKV or WebM for video, MP3 for audio only. Quality cap of Highest, 1080p, 720p or 480p, chosen per video or for all of them at once.
- **A real queue.** One to four downloads in parallel, progress ring per card, Cancel, Retry, Open and Show in folder, plus a history of finished downloads.
- **Safe file names.** Files never overwrite each other: a clash becomes `name (1)`, `name (2)` and so on. Titles are cleaned so that Windows and Linux accept them as file names.
- **Self-maintaining.** On first run Camallo downloads yt-dlp and ffmpeg and checks their SHA-256 checksums. Later runs look for a newer yt-dlp before the first video is read (waiting at most eight seconds on a slow connection), and the window says which version it is and whether it was updated.
- **A custom window.** Dark theme with a single purple accent and a black title bar drawn by the app. Layout inspired by [youtube-dl-gui](https://github.com/jely2002/youtube-dl-gui).
- **Windows and Linux.** The same program on both. The Linux version is new and marked as testing, see [Linux](#linux-testing).

## Install

### Requirements (Windows)

- Windows, 64-bit.
- The Microsoft Edge **WebView2 Runtime**, which powers the built-in browser. Windows 11 includes it. On Windows 10, install it from [Microsoft](https://developer.microsoft.com/microsoft-edge/webview2/) if it is missing.
- An internet connection on first run, to download yt-dlp and ffmpeg (about 30 MB).

### Option 1: installer

Download `camallo-<version>-windows-x64-setup.exe` from the [latest release](https://github.com/echidistanti/camallo/releases/latest) and run it.

- The wizard is available in English and Italian.
- It installs for the current user only, under `%LOCALAPPDATA%\Programs\Camallo`, and does not ask for administrator rights.
- It adds a Start menu entry, an optional desktop shortcut, and an entry in *Installed apps* so that Windows can uninstall it.
- The uninstaller asks whether to remove Camallo's settings, history, yt-dlp, ffmpeg and the built-in browser's profile as well. It deletes only those items, never the whole data folder, so videos are never touched even if the download folder was set inside it. A silent uninstall keeps the data.

### Option 2: portable archive

Download `camallo-<version>-windows-x64.7z` from the same page, extract it (Windows 11 opens `.7z` files natively; otherwise use [7-Zip](https://www.7-zip.org/)) and run `camallo.exe`. Nothing else is needed: the WebView2 loader is carried inside the executable.

## Linux (testing)

Download `camallo-<version>-linux-x86_64.AppImage` from the [latest release](https://github.com/echidistanti/camallo/releases/latest), make it executable (`chmod +x`) and run it.

- **64-bit x86 only**, on a distribution with a glibc at least as new as Ubuntu 22.04's (2.35).
- **WebKitGTK** powers the built-in browser, taken from the system: the package that provides `libwebkit2gtk-4.1` is `webkit2gtk4.1` on Fedora, `libwebkit2gtk-4.1-0` on Ubuntu and Debian and `webkit2gtk-4.1` on Arch. Without it Camallo starts and works, and the browser button says what to install.
- An internet connection on first run: yt-dlp (about 40 MB) and ffmpeg (about 60 MB) are downloaded automatically, as on Windows. The ffmpeg of the system is not used, because some distributions build theirs without the MP3 encoder.

The Linux version is new. What has been checked: the automated tests, the main window starting under a virtual display, and the browser opening a page with a video from a local server and reporting it with its size (GitHub runs that on every push, from inside the AppImage). What has not: a real desktop, a real GPU and the sites that need a video codec WebKitGTK may lack. Reports from real machines are the most useful thing you can send.

- **The browser window is blank.** Some graphics drivers, AMD ones under Wayland among them, do not get along with WebKitGTK's buffer renderer. Start Camallo with `WEBKIT_DISABLE_DMABUF_RENDERER=1` in the environment: the browser inherits it.
- **A page's video does not play in the built-in browser.** WebKitGTK plays what the system's GStreamer can decode, and some distributions ship without H.264. Detection still sees what the page asks for, so the video may be found all the same.
- **Show in folder** asks the file manager over D-Bus to select the file, and opens the folder when none answers.

## Using Camallo

1. **Add a link.** Paste an address in the bar at the top and press **Add**. Only single `http` or `https` addresses are accepted.
2. **Choose.** Each card shows what the video offers. Pick type (video or audio) and quality for that video, or use the bar at the bottom to set them for every video in the list.
3. **Download.** Press **Download** on a card, or the one at the bottom for all of them. Files go to the folder shown in the settings.
4. **Open the result.** A finished card offers **Open** (plays the file) and **Show** (reveals it in Explorer, or in the file manager on Linux).

### When yt-dlp cannot read the address

Many sites hide their video behind a player. In that case the card says so and offers a **Browser** button, and the globe icon in the top bar opens the built-in browser at any time.

1. Open the page in the built-in browser and start the video.
2. When Camallo sees the stream, the **Video detected** panel appears over the page. Press **Download** and the address is sent to the main window, which shows a card for it. The card waits for you like any other: choose and press **Download** there. Nothing a web page does can start a download by itself, not even with *Start at once* on.
3. If several candidates appear, Camallo tries them in order of reliability: a known embedded player first, then the page itself (yt-dlp may know the site), then streaming manifests, then plain files.

What the detector recognises: HLS (`.m3u8`), MPEG-DASH (`.mpd`), Microsoft Smooth Streaming, plain video files (`mp4`, `webm`, `m4v`, `mov`, `mkv`, `flv`, `ogv`), and embedded YouTube, Vimeo, Dailymotion and Streamable players. It ignores stream segments, which are never worth offering one by one, and anything smaller than 64 KiB, which is a tracking clip or an icon.

### Limits worth knowing

- Camallo does not break DRM. Protected streams are not downloaded.
- Playlists are cut at 200 videos.
- Camallo ignores your own yt-dlp configuration file (`--ignore-config`), so that a download behaves the same on every computer. The history keeps each address without the part after `?` or `#`, where signed links carry their keys.
- What can be downloaded is whatever yt-dlp and the page's network traffic allow. If a site changes, update yt-dlp (Camallo does this at startup) or open an issue.

## Settings

Open them with the gear icon. They are saved between runs.

| Setting | Values | Default |
| --- | --- | --- |
| Container | MP4, MKV, WebM | MP4 |
| Quality | Highest, 1080p, 720p, 480p | Highest |
| Audio only | on / off (saves MP3, best quality) | off |
| Parallel downloads | 1 to 4 | 2 |
| Download folder | any folder | `Videos\Camallo` (`~/Videos/Camallo` on Linux) |
| Sub-folder | optional single folder name inside the download folder, for example one per site or project | empty |
| Start at once | download each video as soon as yt-dlp has read it, without waiting for Download | off |

The settings panel also shows, as plain text, the version of yt-dlp in use and how the last look for an update ended; the same line sits at the bottom of the window when nothing is downloading.

MP4 prefers H.264 video with AAC audio, so the files play almost everywhere. MKV keeps whatever the site provides. WebM is re-encoded by ffmpeg when needed.

## Where things are stored

| What | Windows | Linux |
| --- | --- | --- |
| Program | `%LOCALAPPDATA%\Programs\Camallo\camallo.exe` (installer) | wherever the AppImage is kept |
| Settings | `%LOCALAPPDATA%\Camallo\settings.json` | `~/.local/share/camallo/settings.json` |
| History | `%LOCALAPPDATA%\Camallo\history.json` | `~/.local/share/camallo/history.json` |
| yt-dlp and ffmpeg | `%LOCALAPPDATA%\Camallo\bin\` | `~/.local/share/camallo/bin/` |
| WebView2 loader | `%LOCALAPPDATA%\Camallo\runtime\` | not used |
| Built-in browser profile (cookies, logins) | `%LOCALAPPDATA%\Camallo\webview\` (versions up to 0.6.0 kept it in `camallo.exe.WebView2`, next to the executable) | `~/.local/share/camallo/webview/` |
| Downloaded videos | the folder chosen in the settings | the folder chosen in the settings |

Camallo has no telemetry and no account. It talks to the network only to fetch yt-dlp from its GitHub releases, ffmpeg from [gyan.dev](https://www.gyan.dev/ffmpeg/builds/) (Windows) or [FFmpeg-Builds](https://github.com/FFmpeg-Builds/FFmpeg-Builds) (Linux), the pages and media you ask for, their thumbnails, and a size check on plain video files it detects.

## How it works

```
                     +---------------------------+
   link  ----------> |  main window (egui)       |
                     |  cards, queue, settings   |
                     +--+-----------+----------+-+
                        |           |          ^
          yt-dlp -J     |           | spawns   | JSON lines on stdout
          (info)        v           v          |
                  +-----------+  +------------------------+
                  |  yt-dlp   |  |  browser process       |
                  |  + ffmpeg |  |  (tao + wry; WebView2  |
                  +-----------+  |  or WebKitGTK)         |
                                 |  network + page script |
                                 |  -> Detector           |
                                 +------------------------+
```

- **Main window** (`src/app.rs`, `src/view.rs`, `src/ui.rs`): an [egui](https://github.com/emilk/egui) application. Widgets and icons are drawn by hand, so the look does not depend on which glyphs a font has. State is shared with worker threads that read addresses, fetch thumbnails and run downloads.
- **Reading an address** (`src/fetch.rs`, `src/probe.rs`): `yt-dlp -J` for a single video, `--flat-playlist` for a list, then one call per entry for its formats. The result is turned into the height and size options shown on a card.
- **Downloading** (`src/job.rs`): builds the yt-dlp command line. The format selector takes the best picture not taller than the chosen height with the best sound, and falls back to the best available if nothing fits. A job can carry alternative addresses and tries them in order. Stopping a download ends the whole process tree, because yt-dlp starts the real interpreter and ffmpeg: `taskkill /T` on Windows, a process group of its own on Linux.
- **Built-in browser** (`src/browser.rs`, `src/webkit.rs`, `src/bin/camallo-browser.rs`): a process of its own that prints what it finds as JSON lines on stdout, which the main window reads. This keeps a crashing page away from the queue. On Windows it is the same executable, started with `--browser <url>`, running a WebView2 window. On Linux it is `camallo-browser`, next to the main program, running a WebKitGTK window: the main program does not link WebKitGTK, so it starts on a system that lacks it and says what to install when the browser is asked for.
- **Detection** (`src/detect.js`, `src/net.rs`, `src/detector.rs`): two sources feed the detector. A script injected in every frame watches `fetch`, `XMLHttpRequest`, the page's resource timings, `<video>` and `<source>` elements, `og:video` metadata and iframes of known players. In parallel, a handler on the WebView2 `WebResourceResponseReceived` event (on Linux, WebKitGTK's `resource-load-started` and the response of each resource) sees every network response with its headers, so streams whose address has no telltale extension are caught by content type. The detector deduplicates and ranks the candidates, names them, and sends up to six, best first, to the main window.
- **Dependencies** (`src/deps.rs`): downloads yt-dlp and ffmpeg (`yt-dlp.exe` and the gyan.dev build on Windows; `yt-dlp_linux` and the GPL build of the newest release branch of FFmpeg-Builds on Linux), verifies SHA-256 checksums, and lets yt-dlp update itself on later starts.
- **WebView2 loader** (`vendor/webview2-com-sys`): on the `windows-gnu` target the upstream crate imports `WebView2Loader.dll`, and without that DLL next to the executable Windows refuses to start the program. The vendored copy looks up the five loader functions at first use and carries the DLL inside the executable, writing it once to a per-user folder. See [`vendor/webview2-com-sys/NOTE.md`](vendor/webview2-com-sys/NOTE.md).

## Building from source

You need a recent stable Rust (the project uses edition 2024, so Rust 1.85 or newer). On Linux the browser program also needs the development files of GTK 3 and WebKitGTK and `pkg-config`: `libwebkit2gtk-4.1-dev libgtk-3-dev` on Debian and Ubuntu, `webkit2gtk4.1-devel gtk3-devel` on Fedora.

```sh
cargo test                                   # unit tests
node tests/detect_test.js                    # tests for the injected page script
cargo build --release --target x86_64-pc-windows-gnu
```

Up to 0.6.2 the published executables were cross-compiled from Linux with the `x86_64-pc-windows-gnu` target and `mingw-w64`. From 0.6.3 GitHub builds them on Windows with the MSVC toolchain (see [Releases and the installer](#releases-and-the-installer)); the cross-compile above still works.

The icon is embedded by `build.rs`, which needs `windres` (the `mingw-w64` package) when cross-compiling. Without it the build still succeeds, with a warning, and the executable has no icon.

Release builds are optimised for size (`opt-level = "s"`, LTO, stripped symbols, `panic = "abort"`), which gives a single executable of roughly 10 MB.

### Tests that use the real tools

Some tests run the real yt-dlp and ffmpeg against a local web server. They are ignored by default:

```sh
export CAMALLO_YTDLP=/path/to/yt-dlp
export CAMALLO_FFDIR=/path/to/ffmpeg/bin
export CAMALLO_BASE=http://127.0.0.1:8765   # serves clip.mp4, hls/master.m3u8 and playlist.html with two <video> elements
cargo test -- --ignored real
```

### Linux

```sh
cargo build --release                        # target/release/camallo and target/release/camallo-browser
packaging/linux/build-appimage.sh 0.7.0      # dist/camallo-0.7.0-linux-x86_64.AppImage
xvfb-run -a packaging/linux/check-browser.sh target/release/camallo-browser
```

The last command is the check GitHub runs: the browser program opens a page with a video, served from this machine, and has to report the page, its title and the video with its size. It needs `python3` and `ffmpeg` (or a video file named by `CAMALLO_CHECK_VIDEO`), and a display, which `xvfb-run` provides.

For interface work, the browser can be replaced by recorded page messages, which are replayed through the detector:

```sh
CAMALLO_FAKE_FEED=messages.jsonl cargo run
```

Each line of the file is one JSON message such as the injected script would send. Press the globe icon: the browser process that starts feeds those messages to the main window, one every second and a half.

## Releases and the installer

Publishing a release on GitHub is all it takes. The workflow in [`.github/workflows/release.yml`](.github/workflows/release.yml) builds `camallo.exe` from the tag, attaches the portable `camallo-<version>-windows-x64.7z`, builds `camallo-<version>-linux-x86_64.AppImage` on Ubuntu 22.04 and attaches that too, and then calls [`.github/workflows/installer.yml`](.github/workflows/installer.yml), which builds the Windows installer with [Inno Setup](https://jrsoftware.org/isinfo.php) from [`installer/camallo.iss`](installer/camallo.iss) and uploads `camallo-<version>-windows-x64-setup.exe` to the same release. Every push to `main` also runs the tests on both systems and builds the exe and the AppImage ([`ci.yml`](.github/workflows/ci.yml)), kept for a week as artifacts.

To build the installer by hand, put `camallo.exe` in a folder, then run:

```
ISCC /DMyAppVersion=0.6.0 /DSourceDir=C:\path\to\folder installer\camallo.iss
```

The result lands in `installer\output\`. To rebuild the files of an existing release, run the *Release* workflow (everything) or the *Installer* workflow (only the setup) from the Actions tab and give it the tag.

## Repository layout

```
src/
  main.rs        entry point; --browser switches to the browser process (Windows)
  bin/camallo-browser.rs  the browser program on Linux, which shares the detector with the main program
  app.rs         application state, worker threads, actions
  view.rs        screens: list, cards, settings, bottom bar
  ui.rs          hand-drawn widgets, icons, theme, fonts
  fetch.rs       reading an address: one video or a list
  probe.rs       reading what a video offers (formats, sizes)
  job.rs         download jobs and the yt-dlp command line
  deps.rs        first-run download and verification of yt-dlp and ffmpeg
  settings.rs    settings, history, paths
  thumbs.rs      thumbnails for the list
  browser.rs     built-in browser: the WebView2 process on Windows, how the browser is started and what is said when it fails
  webkit.rs      the browser window on Linux (WebKitGTK), run by camallo-browser
  cand.rs        one way to fetch a video, shared by the main program and the browser
  filemanager.rs "Show in folder" on Linux, over D-Bus
  net.rs         which network responses are media
  detector.rs    ranking and naming of detected candidates
  detect.js      script injected into every frame of the browser
tests/           tests for detect.js
vendor/          webview2-com-sys with the loader carried inside the program
installer/       Inno Setup script
packaging/linux/ the AppImage script, the desktop entry and the browser check
assets/          the icon: icon.svg is the source, icon.ico goes into the executable, the PNGs into the windows
build.rs         embeds the icon in the Windows executable
.github/         workflows: tests and build, release, installer
```

## Responsible use

Camallo is a tool for saving videos you are entitled to save: your own, ones under a licence that allows copies, public-domain material, or copies for personal use where the law of your country allows it. Respect the terms of the sites you use and the rights of the people who made the videos. The authors take no responsibility for how it is used.

## Third-party software

| Software | Role | License |
| --- | --- | --- |
| [yt-dlp](https://github.com/yt-dlp/yt-dlp) | reads addresses and downloads | Unlicense, downloaded at first run |
| [FFmpeg](https://ffmpeg.org/), builds by [gyan.dev](https://www.gyan.dev/ffmpeg/builds/) (Windows) and [FFmpeg-Builds](https://github.com/FFmpeg-Builds/FFmpeg-Builds) (Linux) | merges, remuxes and converts | GPL or LGPL depending on the build; downloaded at first run, not redistributed here |
| [egui / eframe](https://github.com/emilk/egui) | interface | MIT or Apache-2.0 |
| [wry](https://github.com/tauri-apps/wry) and [tao](https://github.com/tauri-apps/tao) | built-in browser window | wry: Apache-2.0 or MIT; tao: Apache-2.0 |
| [WebKitGTK](https://webkitgtk.org/) | the browser engine on Linux, taken from the system | LGPL and BSD; not shipped |
| [winresource](https://github.com/BenjaminRi/winresource) | build-time only: embeds the icon in the executable | MIT |
| [webview2-com-sys](https://github.com/wravery/webview2-rs) | WebView2 bindings, vendored with changes | MIT, see [`vendor/webview2-com-sys/LICENSE`](vendor/webview2-com-sys/LICENSE) |
| `WebView2Loader.dll` | WebView2 loader, from the webview2-com-sys package, signed by Microsoft and carried inside the executable | Microsoft WebView2 SDK terms |

The full list of Rust dependencies is in [`Cargo.lock`](Cargo.lock).

## License

Camallo is released under the [MIT License](LICENSE). Copyright (c) 2026 echidistanti.
