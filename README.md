# upmix-core

**把立体声音乐变成真正的 5.1 声道。**

[![release](https://img.shields.io/github/v/release/gensui-fuga/upmix-core)](https://github.com/gensui-fuga/upmix-core/releases)
[![license](https://img.shields.io/badge/license-MIT-blue)](LICENSE)

两条路线：**秒级的快速模式**，和**真正把乐器拆开的自动模式**。

```
立体声 ──▶ 5.1  (FL FR FC LFE BL BR)
```

---

## 下载

| 平台 | 文件 |
|---|---|
| Windows | [upmix-core-windows-x86_64.zip](https://github.com/gensui-fuga/upmix-core/releases/latest) |
| Linux | [upmix-core-linux-x86_64.tar.gz](https://github.com/gensui-fuga/upmix-core/releases/latest) |

解压后：`upmix-gui`（图形界面）、`upmix-core`（命令行）、`upmix-tui`（终端界面）。

---

## 为什么不是简单下混/上混？

矩阵上混只是**数学估计**——靠相位差猜"这个词在不在后面"。做不出真正的 5.1。

好效果的来源是**源分离**：先把歌拆成人声 / 鼓 / 贝斯 / 其他，再按原生混音的规则摆位——人声进中置、鼓点低频进 LFE、环境侧向进环绕。这也是 [coderynx/upmixer](https://github.com/coderynx/audio-upmixer)、WEP Surround 以及商业工具（Abbey Road De-Mix、Penteo）的共同思路。

upmix-core 两种都给你：

| | 快速（STFT） | 自动（Demucs） |
|---|---|---|
| 原理 | 相关性 / 相位空间分解 | 神经源分离 + stem 路由 |
| 速度 | 秒级（4 分钟歌 ≈ 17 s） | CPU 上 ≈ 5× 实时（4 分钟歌 ≈ 20 min） |
| 依赖 | 无 | Python + `demucs` |
| 质量 | 好 | **最接近原生 5.1** |

---

## 用法

### 图形界面

```bash
upmix-gui
```

**六套主题**：和纸（默认）、千禧（银铬金属反光）、霓虹（黑底蓝红光幕）、梦核（粉彩柔光）、墨夜、素白。

- **可调卡片透明度**（设置页滑块，只改 alpha 保留卡片原色）
- **背景壁纸**：填路径或直接把图片拖进窗口；图片会缩到长边 640 再高斯模糊，玻璃/光幕透出它的颜色
- 自选强调色、内建教程页、批量处理

启动时可指定：

```bash
UPMIX_THEME=millennium upmix-gui                      # 主题
UPMIX_WALLPAPER=~/Pictures/wall.jpg upmix-gui         # 壁纸
```

主题 id：`washi` / `millennium` / `neon` / `dream` / `ink` / `plain`。

### 命令行

```bash
# 快速：秒级
upmix-core 歌.flac -o 出.flac

# 自动：真源分离（先 pip install demucs）
upmix-core 歌.flac -o 出.flac --mode auto --keep-stems

# 输出到指定目录
upmix-core 歌.flac --outdir ~/Music/51/

# 批量处理整张专辑
upmix-core --batch ~/Music/album --outdir ~/Music/album_51 --mode fast --skip-existing
```

常用参数：`--mode fast|auto`、`--model htdemucs|mdx_extra_q`、`--jobs N`、`--format flac|wav`、`--lfe-gain-db`、`--surround-gain-db`。

---

## 通道是怎么摆的

| 通道 | 内容 | 电平 |
|---|---|---|
| **FL / FR** 前置 | 音乐主体与立体声宽度 | 0 dB |
| **C** 中置 | 人声 / 主音（前方锚点） | 0 dB（下混 −3 dB） |
| **LFE** 低频 | 20–120 Hz 低频 | 录制 −10 dB（回放补 +10 dB） |
| **BL / BR** 环绕 | 环境 / 混响 / 扩散（无低频） | −3 dB，带 Haas 延迟 |

结果保证 **下混兼容**（ITU-R BS.775）：折回立体声与原曲电平基本一致，不忽大忽小。

**保留原始元数据**：标签、歌词、内嵌封面全部带到输出。

---

## 从源码构建

```bash
cargo build --release
# 产物：target/release/{upmix-core, upmix-gui, upmix-tui}
```

交叉编译 Windows：`cargo build --release --target x86_64-pc-windows-gnu`（需 mingw-w64）。

### 工程结构

```
core/    引擎：STFT 上混、stem 路由、FLAC/WAV IO、元数据
cli/     命令行
gui/     egui 桌面界面
tui/     终端界面
vendor/  打了补丁的纯 Rust FLAC 编码器（修 flacenc #256 Rice 溢出）
```

---

## 已知限制

- 自动模式依赖外部 `demucs`，CPU 上较慢；有 GPU 会快很多。
- 纯 Rust FLAC 编码器（flacenc 0.5.1）对 24-bit 有上游 bug，已在 `vendor/` 打补丁，但仍优先调用系统的 `flac`。

## 许可

MIT
