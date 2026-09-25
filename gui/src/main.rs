//! Desktop binary for the shared GUI.

fn main() -> eframe::Result<()> {
    let input = std::env::args().nth(1).map(std::path::PathBuf::from);
    upmix_gui::run_desktop(input)
}
