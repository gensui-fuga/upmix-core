//! Command-line interface for upmix-core.
//!
//! Modes: `fast` (STFT) or `auto` (Demucs stem separation). Runs on a single
//! file, or over a whole directory with `--batch`. Output goes next to the
//! source, or into `--outdir`. Source metadata (tags, lyrics, cover art) is
//! carried into the output.

use crate::io::pcm::AudioBuffer;
use crate::io::{flac, wav};
use crate::upmix::{NormalizeMode, UpmixConfig, Upmixer};
use anyhow::{bail, Context, Result};
use clap::Parser;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

#[derive(Parser, Debug)]
#[command(
    name = "upmix-core",
    version,
    about = "Stereo -> 5.1 upmixer (fast STFT or auto stem-separation)"
)]
pub struct Cli {
    /// Input file (.flac or .wav), must be stereo. Omit when using --batch.
    pub input: Option<PathBuf>,

    /// Process every .flac/.wav in this directory.
    #[arg(long)]
    pub batch: Option<PathBuf>,

    /// Output directory (default: next to each source file).
    #[arg(long)]
    pub outdir: Option<PathBuf>,

    /// Output file for single-input mode. Format follows the extension.
    #[arg(short, long)]
    pub output: Option<PathBuf>,

    /// Processing mode: `fast` (STFT) or `auto` (Demucs stem separation).
    #[arg(long, default_value = "fast")]
    pub mode: String,

    /// Path to the `demucs` executable (only with --external-demucs).
    #[arg(long, default_value = "demucs")]
    pub demucs: String,

    /// 用外挂的 Python demucs 而不是内置引擎（需要自己装 demucs）。
    #[arg(long)]
    pub external_demucs: bool,

    /// Demucs model name (auto mode).
    #[arg(long, default_value = "htdemucs")]
    pub model: String,

    /// Parallel jobs for demucs (auto mode). Default: all cores.
    #[arg(long)]
    pub jobs: Option<usize>,

    /// Cache directory for separated stems (auto mode); reused on re-runs.
    #[arg(long)]
    pub stem_cache: Option<PathBuf>,

    // ---- fast-mode tuning ----
    #[arg(long, default_value_t = 4096)]
    pub win_size: usize,
    #[arg(long, default_value_t = -6.0)]
    pub lfe_gain_db: f64,
    #[arg(long, default_value_t = 120.0)]
    pub lfe_high_hz: f64,
    #[arg(long, default_value_t = -3.0)]
    pub surround_gain_db: f64,
    #[arg(long, default_value_t = 12.0)]
    pub surround_delay_ms: f64,
    #[arg(long, default_value_t = 200.0)]
    pub surround_low_hz: f64,
    #[arg(long, default_value_t = 2.0)]
    pub vocal_boost_db: f64,
    /// Headroom mode: peak | none | limiter.
    #[arg(long, default_value = "peak")]
    pub normalize: String,
    #[arg(long)]
    pub no_decorrelate: bool,

    /// Keep intermediate separated stems (auto mode).
    #[arg(long)]
    pub keep_stems: bool,

    /// Output extension/container: flac | wav.
    #[arg(long, default_value = "flac")]
    pub format: String,

    /// Skip files whose output already exists (batch).
    #[arg(long)]
    pub skip_existing: bool,

    #[arg(short, long)]
    pub quiet: bool,
}

fn ext_of(path: &Path) -> String {
    path.extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
}

fn write_any(path: &Path, buf: &AudioBuffer, source: Option<&Path>) -> Result<()> {
    match ext_of(path).as_str() {
        "flac" => flac::write(path, buf, source),
        "wav" | "wave" => wav::write(path, buf),
        other => bail!("unsupported output format '.{other}'"),
    }
}

pub fn run() -> Result<()> {
    let cli = Cli::parse();

    let normalize = match cli.normalize.as_str() {
        "peak" => NormalizeMode::Peak,
        "none" => NormalizeMode::None,
        "limiter" => NormalizeMode::Limiter,
        other => bail!("unknown --normalize mode '{other}' (peak|none|limiter)"),
    };
    if !matches!(cli.mode.as_str(), "fast" | "auto") {
        bail!("unknown --mode '{}' (fast|auto)", cli.mode);
    }
    if !matches!(cli.format.as_str(), "flac" | "wav") {
        bail!("unknown --format '{}' (flac|wav)", cli.format);
    }

    let t0 = Instant::now();
    if let Some(dir) = &cli.batch {
        run_batch(&cli, normalize, dir, t0)
    } else if let Some(input) = cli.input.clone() {
        let out = cli
            .output
            .clone()
            .unwrap_or_else(|| default_output(&input, cli.outdir.as_deref(), &cli.format));
        process_one(&cli, normalize, &input, &out, true, t0)?;
        Ok(())
    } else {
        bail!("provide an input file, or --batch <dir>")
    }
}

fn default_output(input: &Path, outdir: Option<&Path>, format: &str) -> PathBuf {
    let stem = input.file_stem().and_then(|s| s.to_str()).unwrap_or("output");
    let name = format!("{stem}_5.1.{format}");
    match outdir {
        Some(d) => d.join(name),
        None => input.with_file_name(name),
    }
}

fn run_batch(cli: &Cli, normalize: NormalizeMode, dir: &Path, t0: Instant) -> Result<()> {
    let mut files: Vec<PathBuf> = Vec::new();
    for e in std::fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))?.flatten() {
        let p = e.path();
        if !p.is_file() {
            continue;
        }
        let ext = ext_of(&p);
        if !matches!(ext.as_str(), "flac" | "wav") {
            continue;
        }
        // skip our own outputs
        let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("");
        if stem.ends_with("_5.1") {
            continue;
        }
        files.push(p);
    }
    files.sort();
    if files.is_empty() {
        eprintln!("no .flac/.wav found in {}", dir.display());
        return Ok(());
    }
    eprintln!("batch: {} file(s) in {}", files.len(), dir.display());

    let mut ok = 0usize;
    let mut failed = 0usize;
    for (i, input) in files.iter().enumerate() {
        let out = default_output(input, cli.outdir.as_deref(), &cli.format);
        if cli.skip_existing && out.exists() {
            eprintln!("[{}/{}] skip (exists): {}", i + 1, files.len(), out.display());
            continue;
        }
        eprintln!("[{}/{}] {}", i + 1, files.len(), input.display());
        match process_one(cli, normalize, input, &out, false, Instant::now()) {
            Ok(()) => ok += 1,
            Err(e) => {
                failed += 1;
                eprintln!("  ! failed: {e}");
            }
        }
    }
    eprintln!("batch done: {ok} ok, {failed} failed, {:.1} s total", t0.elapsed().as_secs_f64());
    Ok(())
}

fn process_one(
    cli: &Cli,
    normalize: NormalizeMode,
    input: &Path,
    out_path: &Path,
    verbose: bool,
    t0: Instant,
) -> Result<()> {
    if let Some(parent) = out_path.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    match cli.mode.as_str() {
        "fast" => run_fast(cli, normalize, input, out_path, verbose, t0),
        "auto" => run_auto(cli, input, out_path, verbose, t0),
        _ => unreachable!(),
    }
}

// ---------------------------------------------------------------- fast mode

fn run_fast(
    cli: &Cli,
    normalize: NormalizeMode,
    input: &Path,
    out_path: &Path,
    verbose: bool,
    t0: Instant,
) -> Result<()> {
    let cfg = UpmixConfig {
        win_size: cli.win_size,
        lfe_gain_db: cli.lfe_gain_db,
        lfe_high_hz: cli.lfe_high_hz,
        surround_gain_db: cli.surround_gain_db,
        surround_delay_ms: cli.surround_delay_ms,
        surround_low_hz: cli.surround_low_hz,
        vocal_boost_db: cli.vocal_boost_db,
        normalize,
        decorrelate: !cli.no_decorrelate,
        ..Default::default()
    };

    let buf = crate::fileio::read_any(input)?;
    buf.validate()?;
    if buf.num_channels() != 2 {
        bail!("input must be stereo, found {} channels", buf.num_channels());
    }
    let output = Upmixer::new(cfg).process(&buf)?;
    write_any(out_path, &output, Some(input))?;
    if verbose || !cli.quiet {
        report(out_path, &output, t0);
    }
    Ok(())
}

// ---------------------------------------------------------------- auto mode

fn run_auto(cli: &Cli, input: &Path, out_path: &Path, verbose: bool, t0: Instant) -> Result<()> {
    let tmp = cli
        .stem_cache
        .clone()
        .unwrap_or_else(|| std::env::temp_dir().join(format!("upmix-stems-{}", std::process::id())));
    std::fs::create_dir_all(&tmp).ok();

    // 默认走内置引擎（纯 Rust + ONNX Runtime），不依赖 Python / pip。
    // 想用外挂的 demucs 就加 --external-demucs。
    let wanted = if cli.external_demucs {
        if !cli.quiet {
            eprintln!("[auto] separating stems with external demucs '{}' …", cli.model);
        }
        Some(demucs_separate_dir(cli, input, &tmp)?)
    } else {
        if !cli.quiet {
            eprintln!("[auto] separating with the built-in engine (htdemucs, no Python) …");
        }
        None
    };

    let (vocals, drums, bass, other) = match wanted {
        Some(dir) => (
            wav::read(&dir.join("vocals.wav"))?,
            wav::read(&dir.join("drums.wav"))?,
            wav::read(&dir.join("bass.wav"))?,
            wav::read(&dir.join("other.wav"))?,
        ),
        None => {
            // 内置模型固定按 44.1k 训练：源不是 44.1k 就先重采样，写个临时 WAV 喂进去。
            let feed = {
                let src = crate::fileio::read_any(input)?;
                if src.sample_rate == 44100 {
                    input.to_path_buf()
                } else {
                    if !cli.quiet {
                        eprintln!("[auto] resampling {} Hz -> 44100 Hz for the model", src.sample_rate);
                    }
                    let rs = resample(&src, 44100)?;
                    let p = tmp.join("builtin-in-44k.wav");
                    wav::write(&p, &rs)?;
                    p
                }
            };
            let s = crate::builtin_sep::separate(&feed, &tmp.join("builtin"))?;
            (
                wav::read(&s.vocals)?,
                wav::read(&s.drums)?,
                wav::read(&s.bass)?,
                wav::read(&s.other)?,
            )
        }
    };
    let stem_sr = vocals.sample_rate;
    let bits = vocals.bits_per_sample;
    let src_sr = probe_sample_rate(input).unwrap_or(stem_sr);

    let stems = crate::auto::Stems {
        sample_rate: stem_sr,
        bits_per_sample: bits,
        vocals,
        drums,
        bass,
        other,
    };
    let mut mixed = crate::auto::route(&stems, &crate::auto::StemRouting::default())?;

    if src_sr != stem_sr {
        if !cli.quiet {
            eprintln!("[auto] resampling {} Hz -> {} Hz", stem_sr, src_sr);
        }
        mixed = resample(&mixed, src_sr)?;
    }

    let peak = mixed.peak();
    if peak > 1.0 {
        mixed.apply_gain(1.0 / peak);
    }
    write_any(out_path, &mixed, Some(input))?;

    if cli.stem_cache.is_none() && !cli.keep_stems {
        let _ = std::fs::remove_dir_all(&tmp);
    }
    if verbose || !cli.quiet {
        report(out_path, &mixed, t0);
    }
    Ok(())
}

fn find_stems_dir(root: &Path, input: &Path) -> Result<PathBuf> {
    let track = input.file_stem().and_then(|s| s.to_str()).unwrap_or("track");
    fn walk(dir: &Path, track: &str) -> Option<PathBuf> {
        for e in std::fs::read_dir(dir).ok()?.flatten() {
            let p = e.path();
            if p.is_dir() {
                if p.file_name().and_then(|n| n.to_str()) == Some(track)
                    && p.join("vocals.wav").exists()
                {
                    return Some(p);
                }
                if let Some(f) = walk(&p, track) {
                    return Some(f);
                }
            }
        }
        None
    }
    walk(root, track).with_context(|| format!("no stems found under {}", root.display()))
}

/// 外挂 demucs 的分支（只有 --external-demucs 时才走）。
fn demucs_separate_dir(cli: &Cli, input: &Path, tmp: &Path) -> Result<PathBuf> {
    if let Some(dir) = find_stems_dir(tmp, input)
        .ok()
        .filter(|d| d.join("vocals.wav").exists())
    {
        if !cli.quiet {
            eprintln!("[auto] reusing cached stems in {}", dir.display());
        }
        return Ok(dir);
    }
    let mut cmd = Command::new(&cli.demucs);
    cmd.arg("-n").arg(&cli.model).arg("-o").arg(tmp);
    // 国内直连 huggingface.co 基本下不动模型（会一直在 Retry），默认走镜像。
    if std::env::var_os("HF_ENDPOINT").is_none() {
        cmd.env("HF_ENDPOINT", "https://hf-mirror.com");
    }
    if std::env::var_os("HF_HUB_DISABLE_TELEMETRY").is_none() {
        cmd.env("HF_HUB_DISABLE_TELEMETRY", "1");
    }
    if let Some(j) = cli.jobs {
        cmd.arg("-j").arg(j.to_string());
    }
    let status = cmd
        .arg(input)
        .status()
        .with_context(|| format!("could not run '{}' (pip install demucs)", cli.demucs))?;
    if !status.success() {
        bail!("demucs exited with {status}");
    }
    find_stems_dir(tmp, input)
}

/// 读文件的采样率，纯 Rust（FLAC 用 claxon、WAV 用 hound），不依赖 ffprobe。
fn probe_sample_rate(path: &Path) -> Option<u32> {
    match ext_of(path).as_str() {
        "wav" | "wave" => hound::WavReader::open(path).ok().map(|r| r.spec().sample_rate),
        "flac" => claxon::FlacReader::open(path)
            .ok()
            .map(|r| r.streaminfo().sample_rate),
        _ => None,
    }
}

/// 纯 Rust 重采样（Blackman 窗 sinc 插值），不依赖 ffmpeg。
fn resample(buf: &AudioBuffer, target_sr: u32) -> Result<AudioBuffer> {
    let src_sr = buf.sample_rate;
    if src_sr == target_sr || src_sr == 0 || buf.data.is_empty() {
        return Ok(buf.clone());
    }
    let ratio = target_sr as f64 / src_sr as f64;
    let frames_in = buf.data[0].len();
    if frames_in == 0 {
        return Ok(buf.clone());
    }
    let frames_out = ((frames_in as f64) * ratio).round() as usize;
    // 降采样时把 sinc 截止频率压到目标奈奎斯特，避免混叠。
    let cutoff = ratio.min(1.0);
    let taps: i64 = 16;

    let mut data = Vec::with_capacity(buf.num_channels());
    for ch in &buf.data {
        let mut out = Vec::with_capacity(frames_out);
        for i in 0..frames_out {
            let center = i as f64 / ratio;
            let first = center.floor() as i64 - taps / 2;
            let mut acc = 0.0f64;
            let mut norm = 0.0f64;
            for k in 0..taps {
                let idx = first + k;
                if idx < 0 || idx as usize >= frames_in {
                    continue;
                }
                let dist = center - idx as f64;
                let x = std::f64::consts::PI * dist * cutoff;
                let sinc = if x.abs() < 1e-9 { 1.0 } else { x.sin() / x };
                // Blackman 窗
                let wp = (k as f64 / (taps - 1) as f64) * 2.0 - 1.0;
                let w = 0.42
                    + 0.5 * (std::f64::consts::PI * wp).cos()
                    + 0.08 * (2.0 * std::f64::consts::PI * wp).cos();
                let h = sinc * w * cutoff;
                acc += ch[idx as usize] * h;
                norm += h;
            }
            out.push(if norm.abs() > 1e-12 { acc / norm } else { acc });
        }
        data.push(out);
    }
    Ok(AudioBuffer {
        sample_rate: target_sr,
        bits_per_sample: buf.bits_per_sample,
        data,
    })
}

fn report(out_path: &Path, out: &AudioBuffer, t0: Instant) {
    let peak_db = 20.0 * out.peak().max(1e-12).log10();
    eprintln!(
        "  -> {} ({} ch, {} Hz, {}-bit) peak {:.1} dBFS  [{:.1}s]",
        out_path.display(),
        out.num_channels(),
        out.sample_rate,
        out.bits_per_sample,
        peak_db,
        t0.elapsed().as_secs_f64()
    );
}
