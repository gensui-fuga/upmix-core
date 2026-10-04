//! 内置源分离：纯 Rust + ONNX Runtime，替代外挂的 Python `demucs`。
//!
//! 优先走**离线路径**：从可执行文件旁边的 `models/` 读 manifest.json + *.onnx，
//! 直接用 `stem_splitter_core::core::engine` 推理，全程不联网、不需要 Python。
//!
//! 只有本地没有模型时，才回退到 stem-splitter-core 的 `split_file`（那次会联网下模型）。

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

use crate::io::pcm::AudioBuffer;

/// 四个 stem 的 PCM（都是 44.1kHz 立体声）。
pub struct StemAudio {
    pub vocals: AudioBuffer,
    pub drums: AudioBuffer,
    pub bass: AudioBuffer,
    pub other: AudioBuffer,
}

/// 把模型（manifest.json + *.onnx）拉到 `out_dir`，打包时用。
pub fn prepare_model(out_dir: &Path) -> Result<()> {
    let handle = stem_splitter_core::model::model_manager::ensure_model("htdemucs_ort_v1", None)
        .map_err(|e| anyhow::anyhow!("下载模型失败: {e}"))?;
    std::fs::create_dir_all(out_dir)?;
    let name = handle
        .local_path
        .file_name()
        .context("模型文件没有文件名")?;
    std::fs::copy(&handle.local_path, out_dir.join(name))
        .with_context(|| format!("拷贝 {}", handle.local_path.display()))?;
    let json = serde_json::to_string_pretty(&handle.manifest)?;
    std::fs::write(out_dir.join("manifest.json"), json)?;
    Ok(())
}

/// 找模型目录：优先程序旁边，再当前目录。
fn model_dir() -> Option<PathBuf> {
    let mut cands: Vec<PathBuf> = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            cands.push(dir.join("models"));
        }
    }
    cands.push(PathBuf::from("models"));
    cands.into_iter().find(|p| p.is_dir())
}

/// 从本地目录装配一个 ModelHandle（manifest + onnx）。
fn local_handle(dir: &Path) -> Option<stem_splitter_core::model::model_manager::ModelHandle> {
    let manifest_txt = std::fs::read_to_string(dir.join("manifest.json")).ok()?;
    let manifest: stem_splitter_core::ModelManifest = serde_json::from_str(&manifest_txt).ok()?;
    // 模型文件可能是 .ort 也可能是 .onnx，一律取 manifest.json 之外的那个。
    let model_file = std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .find(|p| {
            p.is_file()
                && p.file_name()
                    .and_then(|n| n.to_str())
                    .map(|n| !n.eq_ignore_ascii_case("manifest.json"))
                    .unwrap_or(false)
        })?;
    Some(stem_splitter_core::model::model_manager::ModelHandle {
        manifest,
        local_path: model_file,
    })
}

/// 把 f64 planar 输入转成模型要的 (left, right) f32。
fn to_stereo_f32(buf: &AudioBuffer) -> (Vec<f32>, Vec<f32>) {
    let ch = buf.num_channels();
    let n = buf.data.first().map(|c| c.len()).unwrap_or(0);
    let mut l = Vec::with_capacity(n);
    let mut r = Vec::with_capacity(n);
    for i in 0..n {
        let a = buf.data[0][i];
        let b = if ch > 1 { buf.data[1][i] } else { a };
        l.push(a as f32);
        r.push(b as f32);
    }
    (l, r)
}

/// 跑内置分离，返回四个 stem 的 PCM。
/// 输入必须是 44.1kHz 立体声（调用方负责重采样）。
pub fn separate(buf: &AudioBuffer) -> Result<StemAudio> {
    let dir = model_dir().context(
        "找不到 models/ 目录。发行包里自带它；如果是自己编译的，先跑 `upmix-core --prepare-model` 生成。",
    )?;
    let handle = local_handle(&dir).context("models/ 里的 manifest.json 或 *.onnx 读不了")?;

    // 后端选择必须在 preload 之前落地：ORT 的 EP 是在建会话时定的，而且
    // preload 本身是一次性（OnceCell）的，跑过就换不了。
    match crate::backend::apply_pending() {
        Ok(b) => {
            if b != crate::backend::Backend::Auto {
                eprintln!("ℹ️  推理后端：{}", crate::backend::describe());
            }
        }
        Err(e) => {
            // 选择不可行时回退自动，绝不让推理挂掉。
            eprintln!("warning: {e}；本次改用自动选择");
        }
    }

    stem_splitter_core::core::engine::preload(&handle)
        .map_err(|e| anyhow::anyhow!("加载 ONNX 模型失败: {e}"))?;

    let mf = stem_splitter_core::core::engine::manifest();
    if mf.sample_rate != 44100 {
        anyhow::bail!("模型要求 44.1kHz，manifest 里写的是 {}", mf.sample_rate);
    }
    let win = mf.window;
    let hop = mf.hop;
    if !(win > 0 && hop > 0 && hop <= win) {
        anyhow::bail!("manifest 里的 window/hop 不合法: {win}/{hop}");
    }
    let names: Vec<String> = if mf.stems.is_empty() {
        vec![
            "vocals".into(),
            "drums".into(),
            "bass".into(),
            "other".into(),
        ]
    } else {
        mf.stems.clone()
    };
    let idx_of = |key: &str, fallback: usize| -> usize {
        names
            .iter()
            .position(|n| n.eq_ignore_ascii_case(key))
            .unwrap_or(fallback)
    };
    let (vi, di, bi, oi) = (
        idx_of("vocals", 0),
        idx_of("drums", 1),
        idx_of("bass", 2),
        idx_of("other", 3),
    );

    let (src_l, src_r) = to_stereo_f32(buf);
    let n = src_l.len();

    let mut out: [Vec<f32>; 8] = [
        Vec::with_capacity(n),
        Vec::with_capacity(n),
        Vec::with_capacity(n),
        Vec::with_capacity(n),
        Vec::with_capacity(n),
        Vec::with_capacity(n),
        Vec::with_capacity(n),
        Vec::with_capacity(n),
    ];

    let mut left = vec![0f32; win];
    let mut right = vec![0f32; win];
    let mut pos = 0usize;
    while pos < n {
        for i in 0..win {
            let f = pos + i;
            if f < n {
                left[i] = src_l[f];
                right[i] = src_r[f];
            } else {
                left[i] = 0.0;
                right[i] = 0.0;
            }
        }

        let window = stem_splitter_core::core::engine::run_window_demucs(&left, &right)
            .map_err(|e| anyhow::anyhow!("推理失败: {e}"))?;
        let t_out = window.shape()[2];
        let copy = hop.min(t_out).min(n - pos);
        // window 的 shape 是 [stem, 声道, 时间]：声道 0 是 L、1 是 R。
        // 两个声道都要取，不然下游会报 "stem 'xxx' must be stereo"。
        for (k, src_idx) in [vi, di, bi, oi].iter().enumerate() {
            for i in 0..copy {
                out[k * 2].push(window[(*src_idx, 0, i)]);
                out[k * 2 + 1].push(window[(*src_idx, 1, i)]);
            }
        }
        pos += hop;
    }

    // 每个 stem 拼成双声道缓冲：data[0] = L，data[1] = R。
    let mk = |l: Vec<f32>, r: Vec<f32>| AudioBuffer {
        sample_rate: 44100,
        bits_per_sample: 32,
        data: vec![
            l.iter().map(|&x| x as f64).collect(),
            r.iter().map(|&x| x as f64).collect(),
        ],
    };

    let mut it = out.into_iter();
    Ok(StemAudio {
        vocals: mk(it.next().unwrap(), it.next().unwrap()),
        drums: mk(it.next().unwrap(), it.next().unwrap()),
        bass: mk(it.next().unwrap(), it.next().unwrap()),
        other: mk(it.next().unwrap(), it.next().unwrap()),
    })
}
