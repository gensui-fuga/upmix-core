//! 推理后端（ONNX Runtime 的 Execution Provider）选择。
//!
//! # 为什么这里要这么小心
//!
//! `stem-splitter-core` 用 `STEMMER_EP_FORCE` 强制指定 EP，但**强制失败是硬
//! 报错**，不会回退 CPU：
//!
//! ```text
//! Failed to activate forced execution provider 'cuda': not available in this ONNX Runtime build
//! ```
//!
//! 而我们的发行包是拿自编的**纯 CPU** ONNX Runtime 构建的
//! （`stem-splitter-core = { default-features = false }`，见 workspace Cargo.toml），
//! 所以 `cuda` feature 根本没编进来。要是在这种包里无条件写
//! `STEMMER_EP_FORCE=cuda`，用户一点“GPU”就直接报错。
//!
//! 因此本模块的规矩是：
//! 1. **不强制**时（Auto）什么都不设，让 stem-splitter-core 走它自己的默认
//!    探测顺序（linux 下是 Cuda → OneDNN → Xnnpack），失败会静默跳过并回退
//!    CPU。这是最安全的默认。
//! 2. **强制**某个后端之前，先确认“编译期支持存在 + 运行时看起来可行”，
//!    不满足就拒绝并说明原因，绝不盲设环境变量。
//! 3. 只碰我们自己设过的环境变量，用户在 shell 里 export 的不动。
//!
//! # 关于“选择显卡”
//!
//! `stem-splitter-core` 的 `EpKind` 没有设备号字段（`SplitOptions` 里也没有），
//! 所以**不能在它内部选第几块卡**。能做的只有两条：
//!   * 选**后端**（CPU / CUDA / DirectML / CoreML）；
//!   * 用 `CUDA_VISIBLE_DEVICES` 限定 CUDA 能看见哪块卡——这是唯一有效的
//!     “选具体显卡”的途径，且只在 CUDA 下有意义。
//!
//! # 生效时机
//!
//! 环境变量必须在 ONNX Runtime 会话创建**之前**设好。`engine::preload()` 是
//! `OnceCell` 门控的一次性调用，进程内跑过第一次推理后就无法再换 EP，改后端
//! 需要重启程序。

use std::path::Path;
use std::sync::{Mutex, OnceLock};

/// 可选的后端。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Backend {
    /// 交给 stem-splitter-core 自己探测（默认，最安全）。
    Auto,
    /// 强制纯 CPU。
    Cpu,
    /// NVIDIA CUDA（需要编入 `gpu-cuda`，且发行包用的是 CUDA 版 ORT）。
    Cuda,
    /// Windows DirectML（需要编入 `gpu-directml`）。
    DirectMl,
    /// macOS CoreML（需要编入 `gpu-coreml`）。
    CoreMl,
}

impl Backend {
    pub const ALL: &'static [Backend] = &[
        Backend::Auto,
        Backend::Cpu,
        Backend::Cuda,
        Backend::DirectMl,
        Backend::CoreMl,
    ];

    /// 稳定的机器可读 id，用于命令行与配置文件。
    pub fn id(self) -> &'static str {
        match self {
            Backend::Auto => "auto",
            Backend::Cpu => "cpu",
            Backend::Cuda => "cuda",
            Backend::DirectMl => "directml",
            Backend::CoreMl => "coreml",
        }
    }

    /// 界面上显示的名字。
    pub fn label(self) -> &'static str {
        match self {
            Backend::Auto => "自动（推荐）",
            Backend::Cpu => "CPU（最稳，兼容性最好）",
            Backend::Cuda => "GPU · NVIDIA CUDA",
            Backend::DirectMl => "GPU · DirectML（Windows）",
            Backend::CoreMl => "GPU · CoreML（macOS）",
        }
    }

    pub fn from_id(s: &str) -> Option<Self> {
        let s = s.trim().to_ascii_lowercase();
        Backend::ALL.iter().copied().find(|b| b.id() == s)
    }

    /// 这个后端在当前构建里有没有编译期支持。
    pub fn compiled_in(self) -> bool {
        match self {
            Backend::Auto | Backend::Cpu => true,
            Backend::Cuda => cfg!(feature = "gpu-cuda"),
            Backend::DirectMl => cfg!(feature = "gpu-directml"),
            Backend::CoreMl => cfg!(feature = "gpu-coreml"),
        }
    }
}

/// 本机一块显卡。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GpuInfo {
    /// DRM 节点名，例如 `card1`。
    pub card: String,
    pub vendor_id: u16,
    pub device_id: u16,
}

impl GpuInfo {
    pub fn vendor_name(&self) -> &'static str {
        vendor_name(self.vendor_id)
    }
}

/// PCI 厂商号 → 名字。
pub fn vendor_name(vendor_id: u16) -> &'static str {
    match vendor_id {
        0x10de => "NVIDIA",
        0x1002 | 0x1022 => "AMD",
        0x8086 => "Intel",
        0x106b => "Apple",
        0x13b5 => "ARM",
        0x5143 => "Qualcomm",
        0x1af4 => "Virtio",
        _ => "未知",
    }
}

fn read_hex_u16(path: &Path) -> Option<u16> {
    let s = std::fs::read_to_string(path).ok()?;
    let s = s.trim();
    let s = s
        .strip_prefix("0x")
        .or_else(|| s.strip_prefix("0X"))
        .unwrap_or(s);
    u16::from_str_radix(s, 16).ok()
}

/// 枚举本机显卡。纯文件 I/O，不加载任何驱动、不跑推理，开销可忽略。
///
/// 只认 `/sys/class/drm/card<数字>`，这样 `card1-DP-1` 这类连接器条目会被跳过。
pub fn list_gpus() -> Vec<GpuInfo> {
    // 显式标注：非 Linux 时下面整块是 cfg 掉的，`Vec::new()` 推不出元素类型。
    let mut out: Vec<GpuInfo> = Vec::new();
    #[cfg(target_os = "linux")]
    {
        let Ok(rd) = std::fs::read_dir("/sys/class/drm") else {
            return out;
        };
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            let Some(num) = name.strip_prefix("card") else {
                continue;
            };
            // 排除 card0-DP-1、card0-eDP-1 这类
            if num.is_empty() || !num.bytes().all(|b| b.is_ascii_digit()) {
                continue;
            }
            let dev = e.path().join("device");
            if let Some(vendor_id) = read_hex_u16(&dev.join("vendor")) {
                out.push(GpuInfo {
                    card: name,
                    vendor_id,
                    device_id: read_hex_u16(&dev.join("device")).unwrap_or(0),
                });
            }
        }
    }
    out.sort_by(|a, b| a.card.cmp(&b.card));
    out
}

/// 有没有 NVIDIA 驱动。没有的话强行指定 CUDA 只会换来一句硬报错。
///
/// Windows 上没法廉价判断（要查注册表/服务），直接返回 true，交给 EP 自己
/// 探测——反正 Windows 包默认不开 CUDA。
pub fn nvidia_driver_present() -> bool {
    #[cfg(target_os = "linux")]
    {
        Path::new("/dev/nvidia0").exists() || Path::new("/proc/driver/nvidia/version").exists()
    }
    #[cfg(target_os = "windows")]
    {
        true
    }
    #[cfg(not(any(target_os = "linux", target_os = "windows")))]
    {
        false
    }
}

/// 当前机器的能力快照。做成一个结构体是为了让决策逻辑能被单元测试覆盖，
/// 不需要真的插一张显卡。
#[derive(Clone, Debug)]
pub struct Caps {
    pub cuda_compiled: bool,
    pub directml_compiled: bool,
    pub coreml_compiled: bool,
    pub nvidia_driver: bool,
    pub os: &'static str,
    pub gpus: Vec<GpuInfo>,
}

impl Caps {
    pub fn detect() -> Self {
        Caps {
            cuda_compiled: Backend::Cuda.compiled_in(),
            directml_compiled: Backend::DirectMl.compiled_in(),
            coreml_compiled: Backend::CoreMl.compiled_in(),
            nvidia_driver: nvidia_driver_present(),
            os: std::env::consts::OS,
            gpus: list_gpus(),
        }
    }
}

/// 要设/要清的环境变量。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EnvPlan {
    pub set: Vec<(&'static str, String)>,
    pub unset: Vec<&'static str>,
}

/// 纯决策函数：给定后端与能力快照，算出该设哪些环境变量。
///
/// 不满足条件时返回 `Err(理由)`，供界面显示。**绝不**返回一个明知会失败的方案。
pub fn plan(backend: Backend, caps: &Caps, device: Option<u32>) -> Result<EnvPlan, String> {
    let mut p = EnvPlan::default();

    match backend {
        Backend::Auto => {
            // 不设任何强制变量：交给 stem-splitter-core 按默认顺序探测，失败
            // 会退回 CPU。这里只清掉我们自己之前设过的强制值。
            p.unset.push("STEMMER_FORCE_CPU");
            p.unset.push("STEMMER_EP_FORCE");
        }
        Backend::Cpu => {
            p.unset.push("STEMMER_EP_FORCE");
            p.set.push(("STEMMER_FORCE_CPU", "1".to_string()));
        }
        Backend::Cuda => {
            if caps.os != "linux" && caps.os != "windows" {
                return Err("CUDA 只支持 Linux 和 Windows".to_string());
            }
            if !caps.cuda_compiled {
                return Err("这个构建没编入 CUDA（发行包用的是纯 CPU 版 ONNX Runtime）".to_string());
            }
            if !caps.nvidia_driver {
                return Err("没检测到 NVIDIA 驱动（/dev/nvidia0 不存在）".to_string());
            }
            p.unset.push("STEMMER_FORCE_CPU");
            p.set.push(("STEMMER_EP_FORCE", "cuda".to_string()));
        }
        Backend::DirectMl => {
            if caps.os != "windows" {
                return Err("DirectML 只在 Windows 上可用".to_string());
            }
            if !caps.directml_compiled {
                return Err("这个构建没编入 DirectML".to_string());
            }
            p.unset.push("STEMMER_FORCE_CPU");
            p.set.push(("STEMMER_EP_FORCE", "directml".to_string()));
        }
        Backend::CoreMl => {
            if caps.os != "macos" {
                return Err("CoreML 只在 macOS 上可用".to_string());
            }
            if !caps.coreml_compiled {
                return Err("这个构建没编入 CoreML".to_string());
            }
            p.unset.push("STEMMER_FORCE_CPU");
            p.set.push(("STEMMER_EP_FORCE", "coreml".to_string()));
        }
    }

    // 设备号只对 CUDA 有意义：EpKind 里没有设备字段，只能用
    // CUDA_VISIBLE_DEVICES 让 CUDA 只看见指定的那块。
    if let Some(d) = device {
        if matches!(backend, Backend::Cuda) || (backend == Backend::Auto && d > 0) {
            p.set.push(("CUDA_VISIBLE_DEVICES", d.to_string()));
        }
    }

    Ok(p)
}

/// 给界面用：列出每个后端是否可用，不可用就带上原因。
pub fn availability(caps: &Caps) -> Vec<(Backend, Option<String>)> {
    Backend::ALL
        .iter()
        .copied()
        .map(|b| (b, plan(b, caps, None).err()))
        .collect()
}

// =====================================================================
// 进程内的选择 + 应用
// =====================================================================

/// 我们设过的环境变量键。只清我们自己的，不动用户在 shell 里 export 的。
static OURS: Mutex<Vec<&'static str>> = Mutex::new(Vec::new());
static CHOSEN: OnceLock<Mutex<(Backend, Option<u32>)>> = OnceLock::new();

fn chosen() -> &'static Mutex<(Backend, Option<u32>)> {
    CHOSEN.get_or_init(|| Mutex::new((Backend::Auto, None)))
}

/// 记住用户选的后端。真正生效在下次 `apply_pending()`（即下次推理前）。
///
/// 返回 `Err` 表示这个选择在当前构建/机器上不可行——界面应当据此禁用选项，
/// 而不是存下一个注定报错的值。
pub fn set_backend(backend: Backend, device: Option<u32>) -> Result<(), String> {
    let caps = Caps::detect();
    plan(backend, &caps, device)?; // 先验证
    if let Ok(mut c) = chosen().lock() {
        *c = (backend, device);
    }
    Ok(())
}

/// 当前选的后端。
pub fn current() -> (Backend, Option<u32>) {
    chosen().lock().map(|c| *c).unwrap_or((Backend::Auto, None))
}

/// 把当前选择写进环境变量。在 `preload()` 之前调用；重复调用是安全的。
///
/// 返回实际生效的后端（可能与选择不同：比如选了 CUDA 但环境里没有 NVIDIA
/// 驱动时不会走到这里，因为 `set_backend` 已经拦下了）。
pub fn apply_pending() -> Result<Backend, String> {
    let (backend, device) = current();
    let caps = Caps::detect();
    let p = plan(backend, &caps, device)?;

    // 先清掉我们之前设过、这次不该留的。
    let mut ours = OURS.lock().map_err(|e| e.to_string())?;
    for k in &p.unset {
        if ours.contains(k) {
            std::env::remove_var(k);
            ours.retain(|x| x != k);
        }
    }
    for (k, v) in &p.set {
        std::env::set_var(k, v);
        if !ours.contains(k) {
            ours.push(k);
        }
    }
    Ok(backend)
}

/// 一句话描述当前后端，用于日志。
pub fn describe() -> String {
    let (b, d) = current();
    match d {
        Some(d) => format!("{}（设备 {d}）", b.id()),
        None => b.id().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn caps_cpu_only() -> Caps {
        Caps {
            cuda_compiled: false,
            directml_compiled: false,
            coreml_compiled: false,
            nvidia_driver: false,
            os: "linux",
            gpus: vec![],
        }
    }

    fn caps_cuda_ready() -> Caps {
        Caps {
            cuda_compiled: true,
            directml_compiled: false,
            coreml_compiled: false,
            nvidia_driver: true,
            os: "linux",
            gpus: vec![],
        }
    }

    #[test]
    fn id_roundtrip() {
        for b in Backend::ALL {
            assert_eq!(Backend::from_id(b.id()), Some(*b), "{:?}", b.id());
        }
        assert_eq!(Backend::from_id("CUDA"), Some(Backend::Cuda));
        assert_eq!(Backend::from_id("  cpu "), Some(Backend::Cpu));
        assert_eq!(Backend::from_id("nope"), None);
    }

    #[test]
    fn vendor_mapping() {
        assert_eq!(vendor_name(0x10de), "NVIDIA");
        assert_eq!(vendor_name(0x1002), "AMD");
        assert_eq!(vendor_name(0x8086), "Intel");
        assert_eq!(vendor_name(0x1234), "未知");
    }

    #[test]
    fn auto_sets_nothing_forced() {
        let p = plan(Backend::Auto, &caps_cpu_only(), None).unwrap();
        assert!(p.set.is_empty(), "自动不该强制任何东西: {:?}", p.set);
        assert!(p.unset.contains(&"STEMMER_EP_FORCE"));
    }

    #[test]
    fn cpu_forces_cpu_flag() {
        let p = plan(Backend::Cpu, &caps_cpu_only(), None).unwrap();
        assert!(p.set.contains(&("STEMMER_FORCE_CPU", "1".to_string())));
        assert!(!p.set.iter().any(|(k, _)| *k == "STEMMER_EP_FORCE"));
    }

    #[test]
    fn cuda_refused_on_cpu_only_build() {
        // 这是最关键的一条：纯 CPU 包里选 CUDA 必须被拒绝，而不是设上变量让
        // 用户在推理时吃一个硬报错。
        let err = plan(Backend::Cuda, &caps_cpu_only(), None).unwrap_err();
        assert!(err.contains("没编入 CUDA"), "{err}");
    }

    #[test]
    fn cuda_refused_without_driver() {
        let mut c = caps_cuda_ready();
        c.nvidia_driver = false;
        let err = plan(Backend::Cuda, &c, None).unwrap_err();
        assert!(err.contains("NVIDIA 驱动"), "{err}");
    }

    #[test]
    fn cuda_ok_when_compiled_and_driver_present() {
        let p = plan(Backend::Cuda, &caps_cuda_ready(), None).unwrap();
        assert!(p.set.contains(&("STEMMER_EP_FORCE", "cuda".to_string())));
        assert!(p.unset.contains(&"STEMMER_FORCE_CPU"));
    }

    #[test]
    fn device_goes_to_cuda_visible_devices() {
        let p = plan(Backend::Cuda, &caps_cuda_ready(), Some(1)).unwrap();
        assert!(p.set.contains(&("CUDA_VISIBLE_DEVICES", "1".to_string())));
    }

    #[test]
    fn device_ignored_for_cpu() {
        // 选 CPU 时设备号没意义，不该写出 CUDA_VISIBLE_DEVICES 误导人。
        let p = plan(Backend::Cpu, &caps_cpu_only(), Some(1)).unwrap();
        assert!(!p.set.iter().any(|(k, _)| *k == "CUDA_VISIBLE_DEVICES"));
    }

    #[test]
    fn directml_only_on_windows() {
        let err = plan(Backend::DirectMl, &caps_cpu_only(), None).unwrap_err();
        assert!(err.contains("Windows"), "{err}");
    }

    #[test]
    fn coreml_only_on_macos() {
        let err = plan(Backend::CoreMl, &caps_cpu_only(), None).unwrap_err();
        assert!(err.contains("macOS"), "{err}");
    }

    #[test]
    fn availability_marks_reasons() {
        let a = availability(&caps_cpu_only());
        let cuda = a.iter().find(|(b, _)| *b == Backend::Cuda).unwrap();
        assert!(cuda.1.is_some(), "CUDA 在纯 CPU 包里应是不可用");
        let auto = a.iter().find(|(b, _)| *b == Backend::Auto).unwrap();
        assert!(auto.1.is_none(), "自动永远可用");
        let cpu = a.iter().find(|(b, _)| *b == Backend::Cpu).unwrap();
        assert!(cpu.1.is_none(), "CPU 永远可用");
    }

    #[test]
    fn list_gpus_smoke() {
        // 不假设本机有没有显卡，只要求不 panic、卡名合法。
        for g in list_gpus() {
            assert!(g.card.starts_with("card"));
            assert!(g.card[4..].bytes().all(|b| b.is_ascii_digit()));
        }
    }
}
