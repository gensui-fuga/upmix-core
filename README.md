# upmix-core

**把立体声音乐变成真正的 5.1 声道。**

[![release](https://img.shields.io/github/v/release/gensui-fuga/upmix-core)](https://github.com/gensui-fuga/upmix-core/releases)
[![build](https://github.com/gensui-fuga/upmix-core/actions/workflows/build.yml/badge.svg)](https://github.com/gensui-fuga/upmix-core/actions/workflows/build.yml)
[![license](https://img.shields.io/badge/license-MIT-blue)](LICENSE)

```
立体声 ──▶ 5.1  (FL FR FC LFE BL BR)
```

**解压即用。不需要 Python、不需要 pip、不需要装 ffmpeg、不需要联网。**

---

## 下载

去 [Releases](https://github.com/gensui-fuga/upmix-core/releases/latest) 下最新版：

| 平台 | 文件 | 里面有什么 |
|---|---|---|
| Windows | `upmix-core-windows-x86_64.zip` | `upmix-gui.exe` / `upmix-core.exe` / `upmix-tui.exe` / `ffmpeg.exe` / `models/` |
| Linux | `upmix-core-linux-x86_64.tar.gz` | `upmix-gui` / `upmix-core` / `upmix-tui` / `ffmpeg` / `models/` |

解压后**直接双击 `upmix-gui.exe`**（Windows）或 `./upmix-gui`（Linux）就能用。

> **重要**：`ffmpeg` 和 `models/` 必须和程序**放在同一个目录**。
> 程序会自动找身边的 ffmpeg 来解码 mp3/m4a/ogg，并从这里加载分离模型——不联网。

**系统要求**：Windows 8 及以上 / 主流 Linux 发行版；能跑 OpenGL 的显卡。内存建议 4GB+（自动模式跑模型比较吃内存）。

---

## 两种模式的区别

这是整个软件最需要搞清的一件事。**两种模式是两套完全不同的原理，不是一个快一个慢那么简单。**

| | **快速模式（STFT）** | **自动模式（源分离）** |
|---|---|---|
| **原理** | 在时频域分析左右声道的**幅度差和相位差**，据此把声音分配到前/中/环绕 | 用 **HTDemucs 神经网络**把歌真正拆成 **人声 / 鼓 / 贝斯 / 其他**四轨，再按混音规则摆位 |
| **它怎么想** | "这个频段在左右声道相位相反 → 大概是环境声 → 丢到后面去" | "这段是人声 → 放中置；这是底鼓 → 低频给 LFE" |
| **速度** | **秒级**。4 分钟的歌约 **17 秒** | **慢**。4 分钟的歌在普通 CPU 上 **几分钟** |
| **依赖** | 无 | 模型随包附带（已内置） |
| **效果** | 好。有明确的包围感和宽度，但本质仍是**猜测** | **最接近原生 5.1**。分离质量决定了它上限很高 |
| **适合** | 快速批量处理、随便听听 | 认真听、要好效果 |

**一句话**：快速模式靠**数学猜**，自动模式靠**AI 真拆**。

**为什么自动模式更好？** 矩阵上混做不出真正的 5.1——它没法知道"这段到底该在人声位置还是环绕位置"。只有把乐器真的拆开，才能按原生的方式来摆。这也是 [coderynx/upmixer](https://github.com/coderynx/audio-upmixer)、WEP Surround 以及商业工具（Abbey Road De-Mix、Penteo）的共同思路。

---

## 使用方法

### 图形界面（`upmix-gui`）

**这是最省事的方式。**

1. **选音乐** —— 两种方式：
   - 点「**选择文件…**」→ 弹出系统文件对话框，**按住 Ctrl / Shift 可以多选**
   - 点「**选择文件夹…**」→ 选中整个专辑文件夹
   - 或者在「音乐文件夹」输入框里直接填路径（支持 `~/Music` 这种写法），回车扫描
2. **选模式** —— 「快速 (STFT)」还是「自动分离」
3. **勾选「批量处理列表全部」** —— 想整批处理就勾上
4. **（可选）填输出目录** —— 留空就输出到源文件旁边
5. **点「开始」**

**其他功能**：

- **主题**：设置页有 6 套 —— 和纸（默认）/ 千禧（银铬金属）/ 霓虹（黑底蓝红光幕）/ 梦核（粉彩柔光）/ 墨夜 / 素白。**选择会被记住**，下次打开还是它。
- **卡片透明度**：设置页可调，只改透明度、保留卡片原色
- **背景壁纸**：填路径，或者**直接把图片拖进窗口**。图片会缩到长边 640 再高斯模糊，玻璃/光幕会透出它的颜色
- **教程页**：程序里自带

**命令行**：

```bash
# 快速模式：秒级
upmix-core 歌.flac -o 出.flac

# 自动模式：好效果
upmix-core 歌.flac -o 出.flac --mode auto

# 输出到指定目录
upmix-core 歌.flac --outdir ~/Music/51/

# 批量处理整张专辑
upmix-core --batch ~/Music/album --outdir ~/Music/album_51 --mode fast --skip-existing

# mp3 / m4a / ogg / opus 也吃（用自带的 ffmpeg 解码）
upmix-core 歌.mp3 -o 出.flac

# 指定推理后端（只影响 --mode auto 的神经网络部分）
upmix-core 歌.flac -o 出.flac --mode auto --backend cpu
```

**常用参数**：

| 参数 | 说明 |
|---|---|
| `--mode fast\|auto` | 处理模式，默认 `fast` |
| `-o, --output <FILE>` | 单文件输出路径 |
| `--outdir <DIR>` | 输出目录（默认源文件旁边） |
| `--batch <DIR>` | 批量处理目录里所有音频（flac/wav/mp3/m4a/ogg/opus…） |
| `--format flac\|wav` | 输出格式 |
| `--backend auto\|cpu\|cuda\|directml\|coreml` | 推理后端，默认 `auto`（自己探测，失败退回 CPU） |
| `--gpu-device <N>` | CUDA 用第几块显卡（从 0 开始） |
| `--skip-existing` | 批量时跳过已存在的输出 |
| `--lfe-gain-db` / `--surround-gain-db` | LFE / 环绕增益 |
| `--keep-stems` | 保留分离出的四轨（回看用） |
| `--prepare-model --outdir models` | 只把模型下载到 `models/`（打包用） |
| `--external-demucs` | 改用外挂的 Python demucs（默认**不用**，只有你自己装了才需要） |

**元数据**：源文件的标签、封面、歌词会**原样带进输出**，Flac / WAV / MP3 / M4A / OGG / OPUS 输入都一样；输入旁边的同名 `.lrc` 会跟着输出改名（`song.lrc` → `song_5.1.lrc`）。

**关于 `--backend`**：发行包用的是**纯 CPU** 的 ONNX Runtime，所以 `cuda` / `directml` / `coreml` 会明确告诉你"这个构建没编入"，而不是中途崩。想用显卡要自己带相应 feature 编译，并配对应的 GPU 版 ORT。


**主题也可以在启动时指定**：

```bash
UPMIX_THEME=millennium upmix-gui                  # 主题
UPMIX_WALLPAPER=~/Pictures/wall.jpg upmix-gui     # 壁纸
```

主题 id：`washi` / `millennium` / `neon` / `dream` / `ink` / `plain`。

---

## 通道是怎么摆的

| 通道 | 内容 | 电平 |
|---|---|---|
| **FL / FR** 前置 | 音乐主体与立体声宽度 | 0 dB |
| **C** 中置 | 人声 / 主音（前方锚点） | 0 dB（下混 −3 dB） |
| **LFE** 低频 | 20–150 Hz 低频 | −3 dB（可用 `--lfe-gain-db` 调） |
| **BL / BR** 环绕 | 环境 / 混响 / 扩散（无低频） | −3 dB，带 Haas 延迟 |

结果保证**下混兼容**（ITU-R BS.775）：折回立体声和原曲电平基本一致，不会忽大忽小。

**觉得低音不够/太轰？** LFE 是独立通道，只影响低音炮，动它不会改主箱的人声和乐器：

```bash
upmix-core song.flac --lfe-gain-db 0 --lfe-high-hz 200   # 低音更强
upmix-core song.flac --lfe-gain-db -12                   # 低音更收敛
```

默认 `−3 dB / 150 Hz`。注意立体声设备（耳机、2.0 音箱）下混时会**按 BS.775 丢弃 LFE**，这不是 bug——要听低音得有 5.1 输出或低音炮。

**保留原始元数据**：标签、歌词、内嵌封面全部带到输出。

---

## 从源码构建

```bash
cargo build --release
# 产物：target/release/{upmix-core, upmix-gui, upmix-tui}
```

或者直接 `git push`——**仓库自带 GitHub Actions，Linux + Windows 双平台云端构建**。打 tag（`v*`）会自动发 Release 并把二进制、静态 ffmpeg、分离模型一起打包。

### 工程结构

```
core/    引擎：STFT 上混、stem 路由、FLAC/WAV IO、元数据、内置源分离
cli/     命令行
gui/     egui 桌面界面（6 套主题 + 壁纸 + 原生文件对话框）
tui/     终端界面
vendor/  打了补丁的纯 Rust FLAC 编码器（修 flacenc #256）
```

**内置源分离**基于 [`stem-splitter-core`](https://github.com/gentij/stem-splitter-core)（纯 Rust + ONNX Runtime）；模型产物目录由 `--prepare-model` 生成。

---

## 常见问题

**Q：需要装 Python 或者 demucs 吗？**
不要。自动模式是进程内的纯 Rust 引擎，模型随包附带。

**Q：需要装 ffmpeg 吗？**
不要。随包附带，程序优先用身边那个。重采样也是纯 Rust 实现的。

**Q：要联网吗？**
不要。模型和 ffmpeg 都在压缩包里。解压后断网照样用。

**Q：Windows 双击打不开 / 闪退？**
看显卡驱动的 OpenGL 支持。近十年的卡驱动都自带；虚拟机或很老的驱动可能不行。

**Q：`.tar.gz` 在 Windows 上解不开？**
Windows 自带解压不支持 `.tar.gz`。用 7-Zip / Bandizip，或者直接下 `.zip` 那个包。

## 许可

MIT
