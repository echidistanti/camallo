//! Everything that is drawn: title bar, address bar, the list of videos, the bottom toolbar.
//! The window state lives in `app`; this file only reads a snapshot and says what the user did.

use crate::app::{Action, App, Found, Snapshot, best_cands};
use crate::detector::Kind;
use crate::job::{Job, Opts, State};
use crate::probe::{Info, human_duration, human_size};
use crate::settings::{Container, HistoryItem, MAX_PARALLEL, Quality};
use crate::ui;
use eframe::egui::{self, Color32, Margin, RichText, Stroke};
use std::path::PathBuf;

fn muted(t: impl Into<String>) -> RichText {
    RichText::new(t.into()).color(ui::MUTED).size(13.0)
}

/// A line of text that stops at the edge instead of pushing the buttons away.
fn clipped(ui: &mut egui::Ui, text: RichText) -> egui::Response {
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
    ui.allocate_ui_with_layout(
        egui::vec2(left, height),
        egui::Layout::top_down(egui::Align::Min),
        |ui| {
            ui.set_width(left);
            text(ui);
        },
    );
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), buttons);
}

/// What the file will be, in the selector of a video: "Best available", "1080p  ~240 MB", "Audio only (MP3)".
fn choice_label(o: &Opts, info: Option<&Info>) -> String {
    if o.audio {
        return "Audio only (MP3)".into();
    }
    let size = info
        .and_then(|i| i.size_for(o.height))
        .map(|s| format!("  ~{}", human_size(s)))
        .unwrap_or_default();
    match o.height {
        Some(h) => format!("{h}p{size}"),
        None => format!("Best available{size}"),
    }
}

/// The two ends of the card: picture on the left, a column of text, and what the buttons need on the right.
fn card_row(
    ui: &mut egui::Ui,
    thumb: impl FnOnce(&mut egui::Ui),
    right: f32,
    text: impl FnOnce(&mut egui::Ui),
    buttons: impl FnOnce(&mut egui::Ui),
) {
    ui.horizontal(|ui| {
        thumb(ui);
        let left = (ui.available_width() - right).max(150.0);
        split_row(ui, left, ui::THUMB.y, text, buttons);
    });
}

fn detected_card(ui: &mut egui::Ui, found: &[Found], page_title: &str, actions: &mut Vec<Action>) {
    let only_page = found.iter().all(|f| f.kind == Kind::Page);
    let tint = Color32::from_rgb(0x25, 0x1e, 0x3d);
    ui::card(ui, tint, Stroke::new(1.5, ui::ACCENT), |ui| {
        ui.horizontal(|ui| {
            ui::play_badge(ui);
            let left = (ui.available_width() - 300.0).max(140.0);
            split_row(
                ui,
                left,
                ui::BADGE,
                |ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    ui.label(
                        RichText::new("Video detected")
                            .size(17.0)
                            .strong()
                            .color(ui::ACCENT_TEXT),
                    );
                    if !page_title.is_empty() {
                        clipped(ui, muted(page_title));
                    }
                    if only_page {
                        ui.label(muted(
                            "No stream address was seen; the page itself can be tried.",
                        ));
                    }
                },
                |ui| {
                    if ui::pill(ui, "Download", ui::Kind::Primary, 16.0).clicked() {
                        actions.push(Action::DownloadAll(best_cands(found)));
                    }
                    if ui::button(ui, "Add to list", ui::Kind::Soft)
                        .on_hover_text("Read it with yt-dlp and choose quality first")
                        .clicked()
                    {
                        actions.push(Action::AddCand(found[0].cand()));
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
                                    let name = if f.title.is_empty() {
                                        f.url.clone()
                                    } else {
                                        f.title.clone()
                                    };
                                    clipped(ui, muted(name)).on_hover_text(&f.url);
                                });
                            },
                            |ui| {
                                if ui::pill(ui, "Add to list", ui::Kind::Ghost, 13.0).clicked() {
                                    actions.push(Action::AddCand(f.cand()));
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

/// Header over the videos of one playlist, with the actions that act on all of them.
fn group_header(
    ui: &mut egui::Ui,
    name: &str,
    batch: u64,
    jobs: &[(usize, &Job)],
    actions: &mut Vec<Action>,
) {
    let mine = || jobs.iter().filter(|(_, j)| j.batch == batch);
    let ready = mine().filter(|(_, j)| j.state == State::Ready).count();
    let reading = mine().filter(|(_, j)| j.state == State::Fetching).count();
    let total = mine().count();
    ui.horizontal(|ui| {
        ui.add_space(2.0);
        ui::draw_icon_inline(ui, ui::Icon::Folder, ui::ACCENT_TEXT);
        let right = if ready > 0 { 230.0 } else { 70.0 };
        let left = (ui.available_width() - right).max(120.0);
        split_row(
            ui,
            left,
            24.0,
            |ui| {
                ui.horizontal(|ui| {
                    clipped(ui, RichText::new(name).size(15.0).strong());
                });
            },
            |ui| {
                if ui::close(ui)
                    .on_hover_text("Remove these from the list")
                    .clicked()
                {
                    actions.push(Action::HideBatch(batch));
                }
                if ready > 0
                    && ui::pill(ui, &format!("Download {ready}"), ui::Kind::Soft, 13.0).clicked()
                {
                    actions.push(Action::StartBatch(batch));
                }
                let n = if reading > 0 {
                    format!("{total} videos, {reading} being read")
                } else {
                    format!("{total} videos")
                };
                ui.label(muted(n));
            },
        );
    });
}

fn video_card(ui: &mut egui::Ui, i: usize, j: &Job, snap: &Snapshot, actions: &mut Vec<Action>) {
    let outline = match &j.state {
        State::Failed(_) => Stroke::new(1.0, ui::DANGER_DIM),
        State::NoInfo(_) => Stroke::new(1.0, ui::ACCENT_DIM),
        _ => Stroke::new(1.0, ui::LINE),
    };
    let tex = j.thumb.as_ref().and_then(|u| snap.thumbs.get(u));
    let duration = j.duration.map(human_duration);
    let reading = j.state == State::Fetching;
    let right = match &j.state {
        State::Fetching => 40.0,
        State::NoInfo(_) => 270.0,
        State::Ready => 330.0,
        State::Queued | State::Running(_) => 100.0,
        State::Done => 185.0,
        State::Cancelled => 135.0,
        State::Failed(_) => 260.0,
    };
    ui::card(ui, ui::CARD, outline, |ui| {
        card_row(
            ui,
            |ui| ui::thumb(ui, tex, reading, duration.as_deref()),
            right,
            |ui| {
                ui.spacing_mut().item_spacing.y = 4.0;
                clipped(ui, RichText::new(j.label()).size(15.0).strong()).on_hover_text(&j.url);
                match &j.state {
                    State::Running(p) => ui::bar(ui, *p),
                    State::Fetching => {
                        ui.label(muted("Reading with yt-dlp..."));
                    }
                    State::NoInfo(why) => {
                        let first = why
                            .lines()
                            .next()
                            .unwrap_or("yt-dlp could not read this address");
                        clipped(ui, RichText::new(first).size(13.0).color(ui::DANGER))
                            .on_hover_text(why);
                    }
                    _ => {}
                }
                ui.horizontal(|ui| {
                    match &j.state {
                        State::Queued => {
                            ui.label(muted("Waiting"));
                        }
                        State::Running(p) => {
                            ui.label(muted(format!("Downloading {}%", p.round() as u32)));
                        }
                        State::Done => {
                            ui.label(RichText::new("Done").size(13.0).color(ui::ACCENT_TEXT));
                        }
                        State::Cancelled => {
                            ui.label(muted("Cancelled"));
                        }
                        State::Failed(why) => {
                            let first = why.lines().next().unwrap_or("Failed");
                            clipped(ui, RichText::new(first).size(13.0).color(ui::DANGER)).on_hover_text(why);
                        }
                        State::NoInfo(_) => {
                            ui.label(muted("Open the page in the built-in browser and play the video: a Download button will appear."));
                        }
                        State::Fetching | State::Ready => {}
                    }
                    if !matches!(j.state, State::Failed(_) | State::NoInfo(_)) {
                        ui::chip(ui, &ui::host_of(&j.url));
                    }
                    if matches!(j.state, State::Queued | State::Running(_) | State::Done) {
                        ui::chip(ui, &choice_label(&j.opts, None));
                    }
                });
            },
            |ui| match &j.state {
                State::Fetching => {
                    if ui::close(ui)
                        .on_hover_text("Remove from the list")
                        .clicked()
                    {
                        actions.push(Action::Hide(i));
                    }
                }
                State::NoInfo(_) => {
                    if ui::close(ui)
                        .on_hover_text("Remove from the list")
                        .clicked()
                    {
                        actions.push(Action::Hide(i));
                    }
                    if ui::button(ui, "Retry", ui::Kind::Ghost).clicked() {
                        actions.push(Action::Retry(i));
                    }
                    if ui::pill(ui, "Open in browser", ui::Kind::Selected, 14.0)
                        .on_hover_text("Open the page and let Camallo find the video")
                        .clicked()
                    {
                        actions.push(Action::Browser(j.url.clone()));
                    }
                }
                State::Ready => {
                    if ui::close(ui)
                        .on_hover_text("Remove from the list")
                        .clicked()
                    {
                        actions.push(Action::Hide(i));
                    }
                    if ui::pill(ui, "Download", ui::Kind::Primary, 14.0).clicked() {
                        actions.push(Action::Start(i));
                    }
                    format_select(ui, i, j, actions);
                }
                State::Queued | State::Running(_) => {
                    if ui::button(ui, "Cancel", ui::Kind::Ghost).clicked() {
                        actions.push(Action::Cancel(i));
                    }
                }
                State::Done => {
                    if ui::close(ui)
                        .on_hover_text("Remove from the list")
                        .clicked()
                    {
                        actions.push(Action::Hide(i));
                    }
                    if let Some(p) = &j.out {
                        if ui::button(ui, "Show", ui::Kind::Soft)
                            .on_hover_text("Show in the folder")
                            .clicked()
                        {
                            actions.push(Action::Reveal(p.clone()));
                        }
                        if ui::button(ui, "Open", ui::Kind::Primary).clicked() {
                            actions.push(Action::Open(p.clone()));
                        }
                    }
                }
                State::Cancelled => {
                    if ui::close(ui)
                        .on_hover_text("Remove from the list")
                        .clicked()
                    {
                        actions.push(Action::Hide(i));
                    }
                    if ui::button(ui, "Retry", ui::Kind::Soft).clicked() {
                        actions.push(Action::Retry(i));
                    }
                }
                State::Failed(_) => {
                    if ui::close(ui)
                        .on_hover_text("Remove from the list")
                        .clicked()
                    {
                        actions.push(Action::Hide(i));
                    }
                    if ui::button(ui, "Browser", ui::Kind::Ghost)
                        .on_hover_text("Open the page in the built-in browser")
                        .clicked()
                    {
                        actions.push(Action::Browser(j.url.clone()));
                    }
                    if ui::button(ui, "Retry", ui::Kind::Soft).clicked() {
                        actions.push(Action::Retry(i));
                    }
                }
            },
        );
    });
}

/// The choice of one video: best, each height that exists, or only the sound.
fn format_select(ui: &mut egui::Ui, i: usize, j: &Job, actions: &mut Vec<Action>) {
    let info = j.info.as_ref();
    egui::ComboBox::from_id_salt(("format", i))
        .selected_text(choice_label(&j.opts, info))
        .width(170.0)
        .show_ui(ui, |ui| {
            let mut pick = |ui: &mut egui::Ui, audio: bool, height: Option<u32>| {
                let o = Opts {
                    audio,
                    height,
                    ..j.opts
                };
                let on = j.opts.audio == audio && (audio || j.opts.height == height);
                if ui.selectable_label(on, choice_label(&o, info)).clicked() {
                    actions.push(Action::Choose(i, audio, height));
                }
            };
            pick(ui, false, None);
            if let Some(info) = info {
                for q in &info.qualities {
                    pick(ui, false, Some(q.height));
                }
                if info.has_audio {
                    pick(ui, true, None);
                }
            }
        });
}

fn empty_list(ui: &mut egui::Ui) {
    ui.add_space((ui.available_height() * 0.10).max(10.0));
    ui.vertical_centered(|ui| {
        ui::empty_icon(ui);
        ui.add_space(6.0);
        ui.label(
            RichText::new("Add a video or a playlist")
                .size(19.0)
                .strong(),
        );
        ui.label(muted(
            "Paste a link above and press Add. Camallo reads it with yt-dlp,",
        ));
        ui.label(muted(
            "you choose type and quality for each video, then press Download.",
        ));
        ui.add_space(4.0);
        ui.label(muted(
            "If a site is not supported, open it in the built-in browser and play the video.",
        ));
    });
}

fn history_list(ui: &mut egui::Ui, history: &[HistoryItem], actions: &mut Vec<Action>) {
    ui.add_space(6.0);
    ui.collapsing(
        RichText::new(format!("History ({})", history.len()))
            .size(15.0)
            .strong(),
        |ui| {
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
        },
    );
}

impl App {
    fn address_bar(&mut self, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui::icon_button(ui, ui::Icon::Gear, self.show_settings)
                    .on_hover_text("Settings")
                    .clicked()
                {
                    self.show_settings = !self.show_settings;
                }
                let tip = "Open a page and let Camallo find the video";
                if ui::icon_button(ui, ui::Icon::Globe, false)
                    .on_hover_text(tip)
                    .clicked()
                {
                    let u = crate::job::valid_url(&self.input)
                        .unwrap_or_else(|| "https://duckduckgo.com".into());
                    actions.push(Action::Browser(u));
                }
                let go = ui::pill(ui, "Add", ui::Kind::Primary, 15.0).clicked();
                let r = ui.add_sized(
                    [ui.available_width(), 38.0],
                    egui::TextEdit::singleline(&mut self.input)
                        .hint_text("Paste the link of a video or a playlist")
                        .margin(Margin::symmetric(14, 9))
                        .align(egui::Align2::LEFT_CENTER),
                );
                if r.changed() {
                    self.bad_input = false;
                }
                let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                if go || enter {
                    actions.push(Action::AddTyped);
                }
            });
        });
        if self.bad_input {
            ui.add_space(4.0);
            ui.label(
                RichText::new(
                    "That does not look like a link: it should start with http:// or https://",
                )
                .size(13.0)
                .color(ui::DANGER),
            );
        }
    }

    fn settings_card(
        &mut self,
        ui: &mut egui::Ui,
        set: &mut crate::settings::Settings,
        ytdlp: &str,
        actions: &mut Vec<Action>,
    ) {
        ui.add_space(10.0);
        ui::card(ui, ui::CARD, Stroke::new(1.0, ui::LINE), |ui| {
            egui::Grid::new("settings")
                .num_columns(2)
                .spacing([18.0, 10.0])
                .show(ui, |ui| {
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
                            .margin(Margin::symmetric(10, 6)),
                    );
                    ui.end_row();
                    ui.label(muted("Downloads at once"));
                    egui::ComboBox::from_id_salt("parallel")
                        .selected_text(set.parallel.to_string())
                        .width(64.0)
                        .show_ui(ui, |ui| {
                            for n in 1..=MAX_PARALLEL {
                                ui.selectable_value(&mut set.parallel, n, n.to_string());
                            }
                        });
                    ui.end_row();
                    ui.label(muted("Start at once"));
                    ui.checkbox(
                        &mut set.auto_start,
                        "Download a video as soon as it has been read",
                    );
                    ui.end_row();
                    ui.label(muted("yt-dlp"));
                    ui.label(muted(if ytdlp.is_empty() {
                        "starting..."
                    } else {
                        ytdlp
                    }));
                    ui.end_row();
                });
        });
    }

    fn toolbar(
        &mut self,
        ui: &mut egui::Ui,
        snap: &Snapshot,
        set: &mut crate::settings::Settings,
        actions: &mut Vec<Action>,
    ) {
        let ready = snap
            .jobs
            .iter()
            .filter(|j| !j.hidden && j.state == State::Ready)
            .count();
        let running = snap
            .jobs
            .iter()
            .filter(|j| matches!(j.state, State::Running(_)))
            .count();
        let waiting = snap
            .jobs
            .iter()
            .filter(|j| j.state == State::Queued)
            .count();
        let reading = snap
            .jobs
            .iter()
            .filter(|j| !j.hidden && j.state == State::Fetching)
            .count();
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            if ui::icon_button(ui, ui::Icon::Folder, false)
                .on_hover_text("Open the download folder")
                .clicked()
            {
                actions.push(Action::OpenFolder);
            }
            if ui::icon_button(ui, ui::Icon::Trash, false)
                .on_hover_text("Clear finished")
                .clicked()
            {
                actions.push(Action::ClearFinished);
            }
            ui.add_space(6.0);
            ui.label(muted("For all:"));
            let video = if set.audio_only {
                ui::Kind::Soft
            } else {
                ui::Kind::Selected
            };
            let audio = if set.audio_only {
                ui::Kind::Selected
            } else {
                ui::Kind::Soft
            };
            if ui::button(ui, "Video", video).clicked() {
                set.audio_only = false;
            }
            if ui::button(ui, "Audio (MP3)", audio).clicked() {
                set.audio_only = true;
            }
            ui.add_enabled_ui(!set.audio_only, |ui| {
                egui::ComboBox::from_id_salt("container")
                    .selected_text(set.container.short())
                    .width(66.0)
                    .show_ui(ui, |ui| {
                        for c in Container::ALL {
                            ui.selectable_value(&mut set.container, c, c.label());
                        }
                    });
                egui::ComboBox::from_id_salt("quality")
                    .selected_text(set.quality.label())
                    .width(84.0)
                    .show_ui(ui, |ui| {
                        for q in Quality::ALL {
                            ui.selectable_value(&mut set.quality, q, q.label());
                        }
                    });
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let label = if ready > 1 {
                    format!("Download all ({ready})")
                } else {
                    "Download".to_string()
                };
                ui.add_enabled_ui(ready > 0, |ui| {
                    if ui::pill(ui, &label, ui::Kind::Primary, 16.0).clicked() {
                        actions.push(Action::StartAll);
                    }
                });
                let text = if snap.status != "Ready" {
                    snap.status.clone()
                } else if running + waiting + reading > 0 {
                    let mut parts = vec![];
                    if reading > 0 {
                        parts.push(format!("{reading} being read"));
                    }
                    if running + waiting > 0 {
                        parts.push(format!("{running} downloading, {waiting} waiting"));
                    }
                    parts.join(" · ")
                } else {
                    snap.ytdlp.clone()
                };
                let color = if snap.status.starts_with("Setup failed") {
                    ui::DANGER
                } else {
                    ui::MUTED
                };
                clipped(ui, RichText::new(text).size(13.0).color(color));
            });
        });
    }

    fn list(&mut self, ui: &mut egui::Ui, snap: &Snapshot, actions: &mut Vec<Action>) {
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 10.0;
                if snap.armed && !snap.found.is_empty() {
                    detected_card(ui, &snap.found, &snap.page_title, actions);
                    ui.add_space(4.0);
                }
                // Newest address first; the videos of a playlist keep their order inside it.
                let mut order: Vec<(usize, &Job)> = snap
                    .jobs
                    .iter()
                    .enumerate()
                    .filter(|(_, j)| !j.hidden)
                    .collect();
                order.sort_by(|(ia, a), (ib, b)| b.batch.cmp(&a.batch).then(ia.cmp(ib)));
                if order.is_empty() {
                    empty_list(ui);
                }
                let mut last_batch = None;
                for &(i, j) in &order {
                    if j.is_entry && last_batch != Some(j.batch) {
                        group_header(
                            ui,
                            j.group.as_deref().unwrap_or("Playlist"),
                            j.batch,
                            &order,
                            actions,
                        );
                    }
                    last_batch = Some(j.batch);
                    video_card(ui, i, j, snap, actions);
                }
                if !snap.history.is_empty() {
                    history_list(ui, &snap.history, actions);
                }
            });
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let mut actions: Vec<Action> = vec![];
        let snap = self.snapshot();
        let mut set = snap.set.clone();

        egui::Panel::top("title")
            .exact_size(ui::TITLE_HEIGHT)
            .frame(egui::Frame::new().fill(ui::TITLE))
            .show_separator_line(false)
            .show(ui, ui::title_bar);

        egui::Panel::top("address")
            .frame(
                egui::Frame::new()
                    .fill(ui::PANEL)
                    .inner_margin(Margin::symmetric(16, 12)),
            )
            .show(ui, |ui| {
                self.address_bar(ui, &mut actions);
                if self.show_settings {
                    self.settings_card(ui, &mut set, &snap.ytdlp, &mut actions);
                }
            });

        egui::Panel::bottom("toolbar")
            .frame(
                egui::Frame::new()
                    .fill(ui::PANEL)
                    .inner_margin(Margin::symmetric(16, 10)),
            )
            .show(ui, |ui| self.toolbar(ui, &snap, &mut set, &mut actions));

        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(ui::BG)
                    .inner_margin(Margin::same(16)),
            )
            .show(ui, |ui| {
                self.list(ui, &snap, &mut actions);
            });

        ui::border(ui.ctx());
        ui::resize_edges(ui.ctx());

        if set != snap.set {
            actions.push(Action::Settings(set));
        }
        for a in actions {
            self.handle(a);
        }
    }
}
