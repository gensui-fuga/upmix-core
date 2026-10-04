//! Command-line interface for upmix-core.
//!
//! Modes: `fast` (STFT) or `auto` (Demucs stem separation). Runs on a single
//! file, or over a whole directory with `--batch`. Output goes next to the
//! source, or into `--outdir`. Source metadata (tags, lyrics, cover art) is
//! carried into the output.

use crate::io::pcm::AudioBuffer;
use crate::io::wav;
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
    /// 输入文件（flac/wav/mp3/m4a/ogg/opus…，其他格式交给自带 ffmpeg），必须
    /// 是立体声。用 --batch 时省略此项。
    pub input: Option<PathBuf>,

    /// 处理这个目录下的所有音频文件。
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

    /// 只把内置分离的模型下载到本地缓存，不处理音频。打包时用。
    #[arg(long)]
    pub prepare_model: bool,

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
    #[arg(long, default_value_t = -3.0)]
    pub lfe_gain_db: f64,
    #[arg(long, default_value_t = 150.0)]
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

    /// 推理后端：auto | cpu | cuda | directml | coreml。
    /// auto 交给程序自己探测（失败会退回 CPU，最稳）；其余为强制指定，
    /// 当前构建不支持或本机没有对应硬件时会直接报错并说明原因。
    #[arg(long, default_value = "auto")]
    pub backend: String,

    /// CUDA 用第几块显卡（从 0 开始，仅 --backend cuda 时有意义）。
    #[arg(long)]
    pub gpu_device: Option<u32>,

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
    // 这里曾经有一份和 fileio::write_any 平行的实现，结果两边各自演化：
    // 这个版本把 WAV 的 source 参数丢了，于是 `--format wav` 出来的文件永远
    // 没有标签。统一走 fileio，别再分叉。
    crate::fileio::write_any(path, buf, source)
}

pub fn run() -> Result<()> {
    let cli = Cli::parse();

    // 打包用：把内置分离的模型先下到本地缓存就退出（CI 用它把模型收进发行包）。
    if cli.prepare_model {
        let out = cli
            .outdir
            .clone()
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("models"));
        crate::builtin_sep::prepare_model(&out)?;
        eprintln!("[model] 已写入 {}", out.display());
        return Ok(());
    }

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
    // 后端选择：只接受已知 id，并且要求当前构建/本机真的可行——否则宁可现在
    // 报错，也不要等到推理中途吃一个 "Failed to activate forced execution
    // provider"。
    match crate::backend::Backend::from_id(&cli.backend) {
        Some(b) => crate::backend::set_backend(b, cli.gpu_device)
            .map_err(|e| anyhow::anyhow!("--backend {} 不可用：{e}", cli.backend))?,
        None => bail!(
            "unknown --backend '{}' (auto|cpu|cuda|directml|coreml)",
            cli.backend
        ),
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
    let stem = input
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("output");
    let name = format!("{stem}_5.1.{format}");
    match outdir {
        Some(d) => d.join(name),
        None => input.with_file_name(name),
    }
}

fn run_batch(cli: &Cli, normalize: NormalizeMode, dir: &Path, t0: Instant) -> Result<()> {
    let mut files: Vec<PathBuf> = Vec::new();
    for e in std::fs::read_dir(dir)
        .with_context(|| format!("reading {}", dir.display()))?
        .flatten()
    {
        let p = e.path();
        if !p.is_file() {
            continue;
        }
        let ext = ext_of(&p);
        // 以前只放行 flac/wav，把 mp3/m4a/ogg 全挡在批量之外——和“所有格式都要
        // 有元数据”矛盾。现在用统一列表。
        if !crate::fileio::is_input_ext(&ext) {
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
        eprintln!(
            "{dir} 里没有找到音频文件（支持 flac/wav/mp3/m4a/ogg/opus 等）",
            dir = dir.display()
        );
        return Ok(());
    }
    eprintln!("batch: {} file(s) in {}", files.len(), dir.display());

    let mut ok = 0usize;
    let mut failed = 0usize;
    for (i, input) in files.iter().enumerate() {
        let out = default_output(input, cli.outdir.as_deref(), &cli.format);
        if cli.skip_existing && out.exists() {
            eprintln!(
                "[{}/{}] skip (exists): {}",
                i + 1,
                files.len(),
                out.display()
            );
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
    eprintln!(
        "batch done: {ok} ok, {failed} failed, {:.1} s total",
        t0.elapsed().as_secs_f64()
    );
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
        bail!(
            "input must be stereo, found {} channels",
            buf.num_channels()
        );
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
    let tmp = cli.stem_cache.clone().unwrap_or_else(|| {
        std::env::temp_dir().join(format!("upmix-stems-{}", std::process::id()))
    });
    std::fs::create_dir_all(&tmp).ok();

    // 默认走内置引擎（纯 Rust + ONNX Runtime），不依赖 Python / pip。
    // 想用外挂的 demucs 就加 --external-demucs。
    let wanted = if cli.external_demucs {
        if !cli.quiet {
            eprintln!(
                "[auto] separating stems with external demucs '{}' …",
                cli.model
            );
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
            // 内置离线引擎：读进来、非 44.1k 就先重采样，直接跑模型（不联网、不经文件）。
            let src = crate::fileio::read_any(input)?;
            let src_bits = src.bits_per_sample;
            let feed = if src.sample_rate == 44100 {
                src
            } else {
                if !cli.quiet {
                    eprintln!(
                        "[auto] resampling {} Hz -> 44100 Hz for the model",
                        src.sample_rate
                    );
                }
                resample(&src, 44100)
            };
            if feed.num_channels() < 2 {
                bail!("input must be stereo for auto mode");
            }
            let s = crate::builtin_sep::separate(&feed)?;
            // 模型内部固定 44.1k/32bit。把“位深”标回源值，采样率后面再重采样回源；
            // 否则 24bit/192k 的原文件会被降成 32bit/44.1k。
            let tag = |mut b: AudioBuffer| {
                b.bits_per_sample = src_bits;
                b
            };
            (tag(s.vocals), tag(s.drums), tag(s.bass), tag(s.other))
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
        mixed = resample(&mixed, src_sr);
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
    let track = input
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("track");
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
        "wav" | "wave" => hound::WavReader::open(path)
            .ok()
            .map(|r| r.spec().sample_rate),
        "flac" => claxon::FlacReader::open(path)
            .ok()
            .map(|r| r.streaminfo().sample_rate),
        _ => None,
    }
}

/// 纯 Rust 重采样（实现搬到 `dsp::resample`，GUI 也要用）。
fn resample(buf: &AudioBuffer, target_sr: u32) -> AudioBuffer {
    crate::dsp::resample::resample(buf, target_sr)
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
