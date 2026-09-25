# upmix-core

把立体声音乐变成 **5.1 声道**，有两条路线：

* **快速模式（STFT）** — 用左右声道幅度差 / 相位差在时频域做空间分解，秒级完成；
* **自动模式（源分离）** — 用 [Demucs](https://github.com/facebookresearch/demucs) 真正把人声 / 鼓 / 贝斯 / 其他拆开，再按原生 5.1 的规则摆位。

输出 **FL FR FC LFE BL BR**（`ffprobe` 报告 `channel_layout=5.1`），保持源采样率与位深，**保留原始元数据（标签 / 歌词 / 封面）**。

## 为什么

直接下混/上混是数学估计，做不出真正的 5.1。好的效果来自 **源分离**：人声进中置、鼓点低频进 LFE、环境侧向进环绕——这正是 [coderynx/upmixer](https://github.com/coderynx/audio-upmixer)、WEP Surround 以及商业工具（Abbey Road De-Mix、Penteo）的共同思路。

## 快速模式 vs 自动模式

| | 快速（STFT） | 自动（Demucs） |
|---|---|---|
| 原理 | 相关性 / 相位空间分解 | 神经源分离 + stem 路由 |
| 速度 | 秒级（4 分钟歌 ≈ 17 s） | CPU 上 ≈ 5× 实时（4 分钟歌 ≈ 20 min） |
| 依赖 | 无 | Python + `demucs` |
| 质量 | 好 | 最接近原生 5.1 |

## 用法

```bash
# 快速
upmix-core 歌.flac -o 出.flac

# 自动（源分离）
upmix-core 歌.flac -o 出.flac --mode auto --keep-stems

# 输出到指定目录
upmix-core 歌.flac --outdir ~/Music/51/

# 批量处理一个目录
upmix-core --batch ~/Music/album --outdir ~/Music/album_51 --mode fast --skip-existing
```

图形界面：`upmix-gui`（多主题、液态玻璃、自选强调色、教程页、批量）。

## 通道分配

| 通道 | 内容 |
|---|---|
| FL / FR | 音乐主体与立体声宽度 |
| C | 人声 / 主音（前方锚点） |
| LFE | 20–120 Hz 低频（回放会 +10 dB，故录制 -10 dB） |
| BL / BR | 环境 / 混响 / 扩散（无低频，约 -3 dB，带 Haas 延迟） |

下混兼容（ITU-R BS.775）：折回立体声与原曲电平基本一致。

## 构建

```bash
cargo build --release
```

产物：`target/release/{upmix-core, upmix-gui, upmix-tui}`。

交叉编译 Windows：`cargo build --release --target x86_64-pc-windows-gnu`（需 mingw-w64）。

## 工作区结构

```
core/   引擎（STFT 上混、stem 路由、FLAC/WAV IO、元数据）
cli/    命令行
gui/    egui 图形界面（桌面）
tui/    终端界面
vendor/flacenc  打了补丁的纯 Rust FLAC 编码器（修 #256 Rice 溢出）
```

## 已知限制

* 自动模式依赖外部 `demucs`，CPU 上较慢；GPU 会快很多。
* 纯 Rust FLAC 编码器（flacenc 0.5.1）对 24-bit 有上游 bug（多 GB 帧），已在 `vendor/` 打补丁，但仍优先调用系统的 `flac`。

## 许可

MIT
