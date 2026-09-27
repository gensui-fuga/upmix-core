//! 内置源分离：纯 Rust + ONNX Runtime，替代外挂的 Python `demucs`。
//!
//! 用 [stem-splitter-core] 跑 htdemucs，产出 vocals / drums / bass / other 四个
//! 44.1kHz 立体声 WAV，再由 `auto::route` 摆到 5.1。
//!
//! 不需要 Python、不需要 pip。模型由 stem-splitter-core 管理（带 SHA-256 校验），
//! 默认缓存在用户目录；若随发行包预置了模型，则不会联网。

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

/// 四个 stem 的 WAV 路径。
#[derive(Debug, Clone)]
pub struct Stems {
    pub vocals: PathBuf,
    pub drums: PathBuf,
    pub bass: PathBuf,
    pub other: PathBuf,
}

impl Stems {
    /// 按 vocals / drums / bass / other 顺序返回，方便依次读取。
    pub fn all(&self) -> [&Path; 4] {
        [
            &self.vocals,
            &self.drums,
            &self.bass,
            &self.other,
        ]
    }
}

/// 跑一次内置分离，把四个 stem 写进 `out_dir`，返回它们的路径。
pub fn separate(input: &Path, out_dir: &Path) -> Result<Stems> {
    std::fs::create_dir_all(out_dir)
        .with_context(|| format!("creating {}", out_dir.display()))?;

    let opts = stem_splitter_core::SplitOptions {
        output_dir: out_dir.to_string_lossy().into_owned(),
        model_name: "htdemucs".to_string(),
        manifest_url_override: None,
    };

    stem_splitter_core::split_file(&input.to_string_lossy(), opts)
        .map_err(|e| anyhow::anyhow!("built-in separation failed: {e}"))?;

    let stem = input
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("output");
    let p = |kind: &str| out_dir.join(format!("{stem}_{kind}.wav"));

    let stems = Stems {
        vocals: p("vocals"),
        drums: p("drums"),
        bass: p("bass"),
        other: p("other"),
    };

    for f in stems.all() {
        if !f.exists() {
            anyhow::bail!("内置分离没产出 {}", f.display());
        }
    }
    Ok(stems)
}

/// 首次运行时确保模型就位（会联网下载，除非已经缓存或随包预置）。
/// 提前调用可以让 GUI 在开始处理前就把模型准备好。
pub fn prepare_model() -> Result<()> {
    stem_splitter_core::prepare_model("htdemucs", None)
        .map_err(|e| anyhow::anyhow!("preparing model: {e}"))
}
