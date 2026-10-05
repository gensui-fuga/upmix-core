//! 内置源分离：纯 Rust + ONNX Runtime，替代外挂的 Python `demucs`。
//!
//! 优先走**离线路径**：从可执行文件旁边的 `models/` 读 manifest.json + *.onnx，
//! 直接用 `stem_splitter_core::core::engine` 推理，全程不联网、不需要 Python。
//!
//! 只有本地没有模型时，才回退到 stem-splitter-core 的 `split_file`（那次会联网下模型）。

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use crate::io::pcm::AudioBuffer;
use stem_splitter_core::model::model_manager::ModelHandle;

/// 引擎的加载结果，只算一次。
static ENGINE_ONCE: OnceLock<Result<(), String>> = OnceLock::new();

/// 只跑一次的闸门。
///
/// 抽成独立函数是为了能测：`separate()` 里用的是同一个机制，测试用局部的
/// `OnceLock` + 计数器闭包，验的是这段真代码而不是复刻品。
fn run_once(
    slot: &OnceLock<Result<(), String>>,
    f: impl FnOnce() -> Result<(), String>,
) -> Result<(), String> {
    slot.get_or_init(f).clone()
}

/// 真正把推理引擎拉起来：定后端 → 建 ORT 会话。
///
/// **为什么必须只调一次**：`stem_splitter_core::core::engine::preload()` 里
/// 只有 `ORT_INIT` 那一步是 `OnceCell`；其余每次都跑——包括
/// `ep::create_best_session()`，它会重新加载模型、重新探测执行提供者，然后
/// 因为 `SESSION` 早已设置而被 `OnceCell::set(..).ok()` **静默丢弃**。
///
/// 单个文件只建一次会话，看不出问题；批量模式是每个文件建一次、丢一次。
/// 用户实测（Windows + 自动分离）正是"单个能转、批量跑到第二个文件整窗消失"
/// ——反复建/毁 ORT 会话把内存吃穿，分配失败直接 abort，窗口瞬间没了。
/// 顺带这里也是批量慢的元凶：本来每个文件都在白读一遍 200MB 模型。
fn load_engine(handle: &ModelHandle) -> Result<(), String> {
    // 后端选择必须在 preload 之前落地：ORT 的 EP 是在建会话时定的。
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
    stem_splitter_core::core::engine::preload(handle)
        .map_err(|e| format!("加载 ONNX 模型失败: {e}"))
}

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

    // 引擎只拉一次。以前这里是裸调 preload：批量模式下每个文件都会重建一遍
    // ORT 会话（见 load_engine 的说明），Windows 上第二个文件就闪退。
    run_once(&ENGINE_ONCE, || load_engine(&handle)).map_err(|e| anyhow::anyhow!("{e}"))?;

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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// 闸门必须只放行一次——批量闪退就是因为它以前没闸门，每个文件都重建
    /// 一遍 ORT 会话。这个测试盯的是 `separate()` 实际使用的那段机制。
    #[test]
    fn run_once_calls_the_loader_exactly_once() {
        let slot: OnceLock<Result<(), String>> = OnceLock::new();
        let calls = AtomicUsize::new(0);

        for _ in 0..5 {
            run_once(&slot, || {
                calls.fetch_add(1, Ordering::SeqCst);
                Ok(())
            })
            .unwrap();
        }
        assert_eq!(calls.load(Ordering::SeqCst), 1, "加载器被跑了不止一次");

        // 换一个闭包也不能再跑：证明真的在复用第一次的结果。
        run_once(&slot, || {
            calls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        })
        .unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 1, "第二次的闭包仍然被执行了");
    }

    /// 失败也要记住：不能每个文件都重来一遍加载模型这种重活。
    #[test]
    fn run_once_caches_failure_without_retrying() {
        let slot: OnceLock<Result<(), String>> = OnceLock::new();
        let calls = AtomicUsize::new(0);
        let boom = |c: &AtomicUsize| -> Result<(), String> {
            c.fetch_add(1, Ordering::SeqCst);
            Err("模型读不了".to_string())
        };

        let e1 = run_once(&slot, || boom(&calls)).unwrap_err();
        let e2 = run_once(&slot, || boom(&calls)).unwrap_err();
        assert_eq!(calls.load(Ordering::SeqCst), 1, "失败后又被重试了一次");
        assert_eq!(e1, "模型读不了");
        assert_eq!(e2, "模型读不了");
    }
}
