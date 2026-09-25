//! upmix-core: high quality stereo -> 5.1 channel upmixer.
pub mod dsp {
    pub mod biquad;
    pub mod delay;
    pub mod stft;
}
pub mod io {
    pub mod pcm;
    pub mod wav;
    pub mod flac;
}
pub mod upmix;
pub mod auto;
pub mod metadata;
pub mod cli;

/// Format-agnostic file IO helpers shared by the CLI, TUI and Android builds.
pub mod fileio {
    use crate::io::pcm::AudioBuffer;
    use crate::io::{flac, wav};
    use anyhow::{bail, Result};
    use std::path::Path;

    fn ext(path: &Path) -> String {
        path.extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase()
    }

    pub fn read_any(path: &Path) -> Result<AudioBuffer> {
        match ext(path).as_str() {
            "flac" => flac::read(path),
            "wav" | "wave" => wav::read(path),
            other => bail!("unsupported input format '.{other}' (use .flac or .wav)"),
        }
    }

    pub fn write_any(path: &Path, buf: &AudioBuffer, source: Option<&Path>) -> Result<()> {
        match ext(path).as_str() {
            "flac" => flac::write(path, buf, source),
            "wav" | "wave" => wav::write(path, buf),
            other => bail!("unsupported output format '.{other}' (use .flac or .wav)"),
        }
    }
}

pub use upmix::{UpmixConfig, Upmixer};
