//! Shared egui GUI for upmix-core — warm-paper / liquid-glass themes, desktop.

pub mod app;
pub mod picker;
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
            let mut app = App::new(initial);
            // UPMIX_THEME=washi|millennium|neon|dream|ink|plain
            if let Ok(id) = std::env::var("UPMIX_THEME") {
                if !id.trim().is_empty() {
                    app.set_theme_by_id(id.trim());
                }
            }
            // UPMIX_WALLPAPER=/path/to/image 可在启动时直接带壁纸。
            if let Ok(p) = std::env::var("UPMIX_WALLPAPER") {
                if !p.trim().is_empty() {
                    app.set_wallpaper(&cc.egui_ctx, std::path::Path::new(p.trim()));
                }
            }
            Ok(Box::new(app))
        }),
    )
}
