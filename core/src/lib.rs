//! upmix-core: high quality stereo -> 5.1 channel upmixer.
pub mod dsp {
    pub mod biquad;
    pub mod delay;
    pub mod resample;
    pub mod stft;
}
pub mod io {
    pub mod flac;
    pub mod pcm;
    pub mod wav;
}
pub mod auto;
pub mod backend;
pub mod builtin_sep;
pub mod cli;
pub mod metadata;
pub mod upmix;

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
        let name = if cfg!(windows) {
            "ffmpeg.exe"
        } else {
            "ffmpeg"
        };
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
        // 临时名必须唯一：以前只带 PID，同一个进程里连着解两个文件（GUI 批量、
        // CLI 批处理）会互相覆盖，读到的可能是上一个文件的音频。
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let tmp =
            std::env::temp_dir().join(format!("upmix-dec-{}-{nanos}.wav", std::process::id()));
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

    /// 能被处理的输入扩展名。以前这个列表在 CLI/TUI/GUI 里各写一份，结果
    /// 批量模式只认 flac/wav，mp3/m4a/ogg 连门都进不去。这里留唯一真源。
    ///
    /// 注意：任何格式其实都能吃（其余交给自带的 ffmpeg），列出来的只是
    /// “选文件时该显示什么”。
    pub const INPUT_EXTS: &[&str] = &[
        "flac", "wav", "wave", "mp3", "m4a", "mp4", "aac", "ogg", "oga", "opus", "wma", "aiff",
        "aif", "alac", "ape", "wv", "mpc", "caf", "mka", "w64", "amr", "ac3", "dts", "m4b",
    ];

    /// 这个扩展名是不是（我们主动推荐选择器显示的）音频输入。
    pub fn is_input_ext(ext: &str) -> bool {
        let e = ext.to_ascii_lowercase();
        INPUT_EXTS.contains(&e.as_str())
    }

    /// 这个路径是不是能当作输入。
    pub fn is_input_path(path: &Path) -> bool {
        path.extension()
            .and_then(|e| e.to_str())
            .map(is_input_ext)
            .unwrap_or(false)
    }

    pub fn read_any(path: &Path) -> Result<AudioBuffer> {
        match ext(path).as_str() {
            "flac" => {
                // claxon 对流严格：帧边界上读不到同步码（下载截断、流里夹了
                // 垃圾字节、文件头之后粘了别的数据）就直接报
                // "III-formed FLAC stream: frame sync code missing"。而
                // ffmpeg 的 flac 解码器会尽力容错、跳过坏帧继续解。真实世界
                // 的「有点损坏但能听」的 FLAC 很多，所以 claxon 失败时降级到
                // ffmpeg 再试一次，两个都死才报错。
                //
                // 只对 claxon::Error 降级：IO 类错误（文件根本读不了）降级也
                // 必然失败，报原始错误更有用。flac::read 用 .context() 保留了
                // source 链（见那边注释），这里的 downcast 才成立。
                flac::read(path).or_else(|e| {
                    let is_claxon = e
                        .chain()
                        .any(|c| c.downcast_ref::<claxon::Error>().is_some());
                    if is_claxon {
                        decode_with_ffmpeg(path)
                    } else {
                        Err(e)
                    }
                })
            }
            "wav" | "wave" => wav::read(path),
            // 其他格式（mp3/m4a/ogg/aac…）交给自带 ffmpeg，反正它就在旁边。
            _ => decode_with_ffmpeg(path),
        }
    }

    pub fn write_any(path: &Path, buf: &AudioBuffer, source: Option<&Path>) -> Result<()> {
        match ext(path).as_str() {
            "flac" => flac::write(path, buf, source)?,
            "wav" | "wave" => wav::write(path, buf, source)?,
            other => bail!("unsupported output format '.{other}' (use .flac or .wav)"),
        }
        // 输入旁边的同名 .lrc 跟着输出改名（播放器按音频名找歌词）。
        if let Some(src) = source {
            crate::metadata::copy_sidecar_lyrics(path, src);
        }
        Ok(())
    }
}

pub use upmix::{UpmixConfig, Upmixer};
