#![cfg_attr(windows, windows_subsystem = "windows")]

mod app;
mod browser;
mod deps;
#[cfg_attr(not(windows), allow(dead_code))]
mod detector;
mod job;
#[cfg_attr(not(windows), allow(dead_code))]
mod net;
mod probe;
mod settings;

fn main() -> eframe::Result {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("--browser") {
        browser::run(args.get(2).cloned().unwrap_or_else(|| "https://duckduckgo.com".into()));
        return Ok(());
    }
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default().with_inner_size([820.0, 560.0]).with_min_inner_size([620.0, 360.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Camallo",
        options,
        Box::new(|cc| Ok(Box::new(app::App::new(cc.egui_ctx.clone())))),
    )
}
