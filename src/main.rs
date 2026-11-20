#![cfg_attr(windows, windows_subsystem = "windows")]

mod app;
mod browser;
mod deps;
#[cfg_attr(not(windows), allow(dead_code))]
mod detector;
mod fetch;
mod icon;
mod job;
#[cfg_attr(not(windows), allow(dead_code))]
mod net;
mod probe;
mod settings;
mod thumbs;
mod ui;
mod view;

fn main() -> eframe::Result {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("--browser") {
        browser::run(args.get(2).cloned().unwrap_or_else(|| "https://duckduckgo.com".into()));
        return Ok(());
    }
    let mut viewport = eframe::egui::ViewportBuilder::default()
        .with_decorations(false)
        .with_inner_size([900.0, 640.0])
        .with_min_inner_size([760.0, 420.0]);
    if let Some((rgba, width, height)) = icon::rgba() {
        viewport = viewport.with_icon(eframe::egui::IconData { rgba, width, height });
    }
    let options = eframe::NativeOptions { viewport, ..Default::default() };
    eframe::run_native(
        "Camallo",
        options,
        Box::new(|cc| {
            ui::fonts(&cc.egui_ctx);
            ui::apply(&cc.egui_ctx);
            Ok(Box::new(app::App::new(cc.egui_ctx.clone())))
        }),
    )
}
