fn main() {
    if let Err(e) = upmix_core::cli::run() {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}
