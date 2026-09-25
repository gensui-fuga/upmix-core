//! Shared egui GUI for upmix-core — warm-paper / liquid-glass themes, desktop.

pub mod app;
pub mod theme;

pub use app::App;

/// Desktop entry point.
pub fn run_desktop(initial: Option<std::path::PathBuf>) -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([460.0, 780.0])
            .with_min_inner_size([400.0, 600.0])
            .with_title("Upmix 5.1"),
        ..Default::default()
    };
    eframe::run_native(
        "Upmix 5.1",
        options,
        Box::new(move |cc| {
            theme::install_fonts(&cc.egui_ctx);
            Ok(Box::new(App::new(initial)))
        }),
    )
}
