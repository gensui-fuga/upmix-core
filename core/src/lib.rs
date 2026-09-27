//! upmix-core: high quality stereo -> 5.1 channel upmixer.
pub mod dsp {
    pub mod biquad;
    pub mod delay;
    pub mod resample;
    pub mod stft;
}
pub mod io {
    pub mod pcm;
    pub mod wav;
    pub mod flac;
}
pub mod upmix;
pub mod auto;
pub mod builtin_sep;
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

    /// 找一个 ffmpeg：优先程序自己旁边的（发行包随机附带），再 PATH。
    pub fn ffmpeg_path() -> std::path::PathBuf {
        let name = if cfg!(windows) { "ffmpeg.exe" } else { "ffmpeg" };
        if let Ok(exe) = std::env::current_exe() {
            if let Some(dir) = exe.parent() {
                let p = dir.join(name);
                if p.exists() {
                    return p;
                }
            }
        }
        std::path::PathBuf::from(name)
    }

    /// 用自带的 ffmpeg 把 mp3 / m4a / ogg 之类解码成 WAV 再读。
    fn decode_with_ffmpeg(path: &Path) -> Result<AudioBuffer> {
        let ff = ffmpeg_path();
        let tmp = std::env::temp_dir().join(format!("upmix-dec-{}.wav", std::process::id()));
        let out = std::process::Command::new(&ff)
            .args(["-hide_banner", "-loglevel", "error", "-y", "-i"])
            .arg(path)
            .args(["-c:a", "pcm_s24le"])
            .arg(&tmp)
            .output();
        match out {
            Ok(o) if o.status.success() => {
                let buf = wav::read(&tmp);
                let _ = std::fs::remove_file(&tmp);
                buf
            }
            Ok(_) => bail!("ffmpeg 解不开 {}", path.display()),
            Err(e) => bail!(
                "不支持 .{} 格式，而且旁边没有可用的 ffmpeg（{e}）。\n\
                 发行包里就带了一份 ffmpeg，把它和程序放在同一目录即可。",
                ext(path)
            ),
        }
    }

    pub fn read_any(path: &Path) -> Result<AudioBuffer> {
        match ext(path).as_str() {
            "flac" => flac::read(path),
            "wav" | "wave" => wav::read(path),
            // 其他格式（mp3/m4a/ogg/aac…）交给自带 ffmpeg，反正它就在旁边。
            _ => decode_with_ffmpeg(path),
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
