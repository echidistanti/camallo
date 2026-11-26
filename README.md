# Camallo

[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Latest release](https://img.shields.io/github/v/release/echidistanti/camallo)](https://github.com/echidistanti/camallo/releases/latest)
![Platform: Windows x64](https://img.shields.io/badge/platform-Windows%20x64-lightgrey)

A small Windows app for saving videos. Paste the link of a video or a playlist and pick the quality of each video, or open the built-in browser and let Camallo notice the video that a page is playing. The downloading itself is done by [yt-dlp](https://github.com/yt-dlp/yt-dlp) and [ffmpeg](https://ffmpeg.org/); Camallo is the interface, the queue and the part that finds videos yt-dlp does not know how to find on its own.

It is written in Rust, ships as a single executable and keeps its own copies of yt-dlp and ffmpeg up to date.

> Camallo is an independent project, written from scratch. It is not affiliated with, endorsed by or derived from the code of Downie, Video DownloadHelper, yt-dlp or ffmpeg.

## Contents

- [What it does](#what-it-does)
- [Install](#install)
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
- **Safe file names.** Files never overwrite each other: a clash becomes `name (1)`, `name (2)` and so on. Titles are cleaned so that Windows accepts them as file names.
- **Self-maintaining.** On first run Camallo downloads yt-dlp and ffmpeg and checks their SHA-256 checksums. Later runs look for a newer yt-dlp before the first video is read (waiting at most eight seconds on a slow connection), and the window says which version it is and whether it was updated.
- **A custom window.** Dark theme with a single purple accent and a black title bar drawn by the app. Layout inspired by [youtube-dl-gui](https://github.com/jely2002/youtube-dl-gui).

## Install

### Requirements

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

## Using Camallo

1. **Add a link.** Paste an address in the bar at the top and press **Add**. Only single `http` or `https` addresses are accepted.
2. **Choose.** Each card shows what the video offers. Pick type (video or audio) and quality for that video, or use the bar at the bottom to set them for every video in the list.
3. **Download.** Press **Download** on a card, or the one at the bottom for all of them. Files go to the folder shown in the settings.
4. **Open the result.** A finished card offers **Open** (plays the file) and **Show** (reveals it in Explorer).

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
| Download folder | any folder | `Videos\Camallo` |
| Sub-folder | optional single folder name inside the download folder, for example one per site or project | empty |
| Start at once | download each video as soon as yt-dlp has read it, without waiting for Download | off |

The settings panel also shows, as plain text, the version of yt-dlp in use and how the last look for an update ended; the same line sits at the bottom of the window when nothing is downloading.

MP4 prefers H.264 video with AAC audio, so the files play almost everywhere. MKV keeps whatever the site provides. WebM is re-encoded by ffmpeg when needed.

## Where things are stored

| What | Where |
| --- | --- |
| Program (installer) | `%LOCALAPPDATA%\Programs\Camallo\camallo.exe` |
| Settings | `%LOCALAPPDATA%\Camallo\settings.json` |
| History | `%LOCALAPPDATA%\Camallo\history.json` |
| yt-dlp and ffmpeg | `%LOCALAPPDATA%\Camallo\bin\` |
| WebView2 loader | `%LOCALAPPDATA%\Camallo\runtime\` |
| Built-in browser profile (cookies, logins) | `%LOCALAPPDATA%\Camallo\webview\` (versions up to 0.6.0 kept it in `camallo.exe.WebView2`, next to the executable) |
| Downloaded videos | the folder chosen in the settings |

Camallo has no telemetry and no account. It talks to the network only to fetch yt-dlp from its GitHub releases, ffmpeg from [gyan.dev](https://www.gyan.dev/ffmpeg/builds/), the pages and media you ask for, their thumbnails, and a size check on plain video files it detects.

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
                  |  yt-dlp   |  |  camallo.exe --browser |
                  |  + ffmpeg |  |  (tao + wry + WebView2)|
                  +-----------+  |  network + page script |
                                 |  -> Detector           |
                                 +------------------------+
```

- **Main window** (`src/app.rs`, `src/view.rs`, `src/ui.rs`): an [egui](https://github.com/emilk/egui) application. Widgets and icons are drawn by hand, so the look does not depend on which glyphs a font has. State is shared with worker threads that read addresses, fetch thumbnails and run downloads.
- **Reading an address** (`src/fetch.rs`, `src/probe.rs`): `yt-dlp -J` for a single video, `--flat-playlist` for a list, then one call per entry for its formats. The result is turned into the height and size options shown on a card.
- **Downloading** (`src/job.rs`): builds the yt-dlp command line. The format selector takes the best picture not taller than the chosen height with the best sound, and falls back to the best available if nothing fits. A job can carry alternative addresses and tries them in order. Stopping a download ends the whole process tree, because `yt-dlp.exe` starts the real interpreter and ffmpeg.
- **Built-in browser** (`src/browser.rs`): the same executable, started with `--browser <url>`. It runs a WebView2 window and prints what it finds as JSON lines on stdout, which the main window reads. This keeps a crashing page away from the queue.
- **Detection** (`src/detect.js`, `src/net.rs`, `src/detector.rs`): two sources feed the detector. A script injected in every frame watches `fetch`, `XMLHttpRequest`, the page's resource timings, `<video>` and `<source>` elements, `og:video` metadata and iframes of known players. In parallel, a handler on the WebView2 `WebResourceResponseReceived` event sees every network response with its headers, so streams whose address has no telltale extension are caught by content type. The detector deduplicates and ranks the candidates, names them, and sends up to six, best first, to the main window.
- **Dependencies** (`src/deps.rs`): downloads yt-dlp and ffmpeg, verifies SHA-256 checksums, and lets yt-dlp update itself on later starts.
- **WebView2 loader** (`vendor/webview2-com-sys`): on the `windows-gnu` target the upstream crate imports `WebView2Loader.dll`, and without that DLL next to the executable Windows refuses to start the program. The vendored copy looks up the five loader functions at first use and carries the DLL inside the executable, writing it once to a per-user folder. See [`vendor/webview2-com-sys/NOTE.md`](vendor/webview2-com-sys/NOTE.md).

## Building from source

You need a recent stable Rust (the project uses edition 2024, so Rust 1.85 or newer).

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

### Working on Linux

The window starts on Linux, which is handy for interface work, but the built-in browser needs WebView2 and does not. To try the main window anyway, replay recorded page messages through the detector:

```sh
CAMALLO_FAKE_FEED=messages.jsonl cargo run
```

Each line of the file is one JSON message such as the injected script would send. Press the globe icon: the browser process that starts feeds those messages to the main window, one every second and a half.

## Releases and the installer

Publishing a release on GitHub is all it takes. The workflow in [`.github/workflows/release.yml`](.github/workflows/release.yml) builds `camallo.exe` from the tag, attaches the portable `camallo-<version>-windows-x64.7z`, and then calls [`.github/workflows/installer.yml`](.github/workflows/installer.yml), which builds the Windows installer with [Inno Setup](https://jrsoftware.org/isinfo.php) from [`installer/camallo.iss`](installer/camallo.iss) and uploads `camallo-<version>-windows-x64-setup.exe` to the same release. Every push to `main` also runs the tests and builds the exe ([`ci.yml`](.github/workflows/ci.yml)), kept for a week as an artifact.

To build the installer by hand, put `camallo.exe` in a folder, then run:

```
ISCC /DMyAppVersion=0.6.0 /DSourceDir=C:\path\to\folder installer\camallo.iss
```

The result lands in `installer\output\`. To rebuild the files of an existing release, run the *Release* workflow (everything) or the *Installer* workflow (only the setup) from the Actions tab and give it the tag.

## Repository layout

```
src/
  main.rs        entry point; --browser switches to the browser process
  app.rs         application state, worker threads, actions
  view.rs        screens: list, cards, settings, bottom bar
  ui.rs          hand-drawn widgets, icons, theme, fonts
  fetch.rs       reading an address: one video or a list
  probe.rs       reading what a video offers (formats, sizes)
  job.rs         download jobs and the yt-dlp command line
  deps.rs        first-run download and verification of yt-dlp and ffmpeg
  settings.rs    settings, history, paths
  thumbs.rs      thumbnails for the list
  browser.rs     built-in browser process (WebView2)
  net.rs         which network responses are media
  detector.rs    ranking and naming of detected candidates
  detect.js      script injected into every frame of the browser
tests/           tests for detect.js
vendor/          webview2-com-sys with the loader carried inside the program
installer/       Inno Setup script
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
| [FFmpeg](https://ffmpeg.org/), build by [gyan.dev](https://www.gyan.dev/ffmpeg/builds/) | merges, remuxes and converts | GPL or LGPL depending on the build (see gyan.dev); downloaded at first run, not redistributed here |
| [egui / eframe](https://github.com/emilk/egui) | interface | MIT or Apache-2.0 |
| [wry](https://github.com/tauri-apps/wry) and [tao](https://github.com/tauri-apps/tao) | built-in browser window | wry: Apache-2.0 or MIT; tao: Apache-2.0 |
| [winresource](https://github.com/BenjaminRi/winresource) | build-time only: embeds the icon in the executable | MIT |
| [webview2-com-sys](https://github.com/wravery/webview2-rs) | WebView2 bindings, vendored with changes | MIT, see [`vendor/webview2-com-sys/LICENSE`](vendor/webview2-com-sys/LICENSE) |
| `WebView2Loader.dll` | WebView2 loader, from the webview2-com-sys package, signed by Microsoft and carried inside the executable | Microsoft WebView2 SDK terms |

The full list of Rust dependencies is in [`Cargo.lock`](Cargo.lock).

## License

Camallo is released under the [MIT License](LICENSE). Copyright (c) 2026 echidistanti.
