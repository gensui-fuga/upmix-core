//! 应用内的文件 / 目录选择器。
//!
//! 之前这里调的是 zenity（Linux）和 PowerShell 的 `System.Windows.Forms`
//! （Windows）——本质上是**叫系统文件管理器来弹对话框**。三个问题：
//!
//! 1. 没装 zenity、或者没有 xdg-desktop-portal 的机器上，点了按钮**什么都不
//!    发生**（子进程启动失败，返回值被吞掉），用户只会觉得按钮坏了。
//! 2. 路径要过一次 stdout 的 UTF-8 转换。非 UTF-8 的文件名会被
//!    `from_utf8_lossy` 换成 `?`，拿到一个根本不存在的路径。
//! 3. 凭空多一个外部依赖，和「发行包里只有 ffmpeg 一个外部程序」的定位不符。
//!
//! 现在直接在 egui 里画，路径全程用 `PathBuf`，不经过字符串转换。
//!
//! == 1.0.1 修掉的几处 ==
//!
//! * **「↑ 上一级」会卡死**。`parent()` 是纯词法操作，不碰文件系统：只要
//!   `cwd` 一旦变成相对路径（`Music`、`.`、`..`），`parent()` 一下就变成
//!   `Some("")`，而 `read_dir("")` 必然失败；再按一次，
//!   `Path::new("").parent()` 返回 `None`，按钮从此无效。所以 `cwd` 从进选择器
//!   那一刻起就强制绝对路径（见 [`absolutize`]），「上一级」一路都是合法目录。
//! * **Windows 从第一帧就进死局**。Windows 上 `HOME` 经常没设（只有
//!   `USERPROFILE`），旧的 `home_dir()` 兜底成 `.`，正好是上面那个死局。现在
//!   Windows 先取 `USERPROFILE`，兜底也改成进程当前目录（绝对路径）。
//! * **根目录是死胡同**。`Path::new("C:\\").parent()` 是 `None`，Windows 上
//!   又没有别的盘符可换——走到根就出不来了。现在根目录时按钮置灰并提示，
//!   Windows 给出盘符列表，Unix 给出「根目录」按钮。
//! * **目录列表其实是每帧 `read_dir` 一次**（旧注释宣称「只在切换时读一次」，
//!   说的和做的是两回事）。60fps × 几千个条目 = 每秒几十万次 stat，再叠加
//!   几千个 widget 每帧全画，界面卡得像「点了没反应」「选不了」。现在只在
//!   切换 / 点刷新时读一次，行渲染用 `show_rows` 虚拟化，只画看得见的那几十行。
//! * 路径输入框现在和当前位置保持同步，随时能直接改；加了「刷新」。
//! * 文件模式不再把上一轮自己生成的 `*_5.1.flac` 当输入列出来（`scan_dir`
//!   早就有这道过滤，选择器漏了）；加了「显示全部文件」「显示隐藏项」。

use egui::RichText;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::theme::Theme;

/// 选中之后写回界面的哪个字段。
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum PickTarget {
    /// 输入目录
    InputDir,
    /// 输出目录
    OutputDir,
    /// 一个或多个音频文件
    Files,
}

/// 选择结果。
pub enum PickOutcome {
    Files(Vec<PathBuf>),
    Dir(PathBuf),
}

/// 选择器这一帧的状态。
pub enum PickAction {
    /// 还在选
    Pending,
    /// 用户确认
    Done(PickOutcome),
    /// 用户取消
    Cancel,
}

/// 主目录。Windows 上 `HOME` 经常没设（只有 `USERPROFILE`），所以按平台取。
/// 兜底用进程当前目录（绝对路径），**绝不能用 `.`**——`.` 的 `parent()` 是
/// `None`，「↑ 上一级」从第一帧起就是死的。
fn home_dir() -> PathBuf {
    let var = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    std::env::var_os(var)
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
        .filter(|p| p.is_dir())
        .unwrap_or_else(|| {
            std::env::current_dir().unwrap_or_else(|_| {
                if cfg!(windows) {
                    PathBuf::from("C:\\")
                } else {
                    PathBuf::from("/")
                }
            })
        })
}

/// Windows 的盘符列表。`Path::new("C:\\").parent()` 是 `None`，根目录没有
/// 「上一级」可去；不给盘符，Windows 用户一旦走到根就永远出不来了。
/// Linux/macOS 上返回空，调用方据此不画这一块。
fn drives() -> Vec<PathBuf> {
    if !cfg!(windows) {
        return Vec::new();
    }
    (b'A'..=b'Z')
        .map(|c| PathBuf::from(format!("{}:\\", c as char)))
        .filter(|p| p.is_dir())
        .collect()
}

/// 变成绝对路径，但不解析软链接。
///
/// `parent()` 是纯词法操作，不碰文件系统：只要路径是绝对的，「上一级」就能
/// 一路走到根，每一级都是合法路径。相对路径不行——`Music` 的 parent 是 `""`，
/// 谁也打不开。也不该用 `fs::canonicalize`：Windows 上它会给出 `\\?\C:\...`
/// 这种原样路径，显示出来吓人。
fn absolutize(p: &Path) -> PathBuf {
    if p.is_absolute() {
        return p.to_path_buf();
    }
    match std::env::current_dir() {
        Ok(base) => base.join(p),
        Err(_) => p.to_path_buf(),
    }
}

/// 隐藏条目：Unix 看 `.` 前缀；Windows 再看 `FILE_ATTRIBUTE_HIDDEN`(0x2)，
/// 否则「显示隐藏项」在 Windows 上就是摆设。
fn is_hidden(e: &std::fs::DirEntry) -> bool {
    if e.file_name().to_string_lossy().starts_with('.') {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        e.metadata()
            .map(|m| m.attributes() & 0x2 != 0)
            .unwrap_or(false)
    }
    #[cfg(not(windows))]
    {
        false
    }
}

pub struct Picker {
    target: PickTarget,
    /// 只列目录（选目录），还是也列音频文件。
    dir_mode: bool,
    cwd: PathBuf,
    /// 文件模式下已勾选的条目。
    chosen: HashSet<PathBuf>,
    /// 手输路径（可以直接粘一个路径进来）。随导航同步成当前路径。
    typed: String,
    err: Option<String>,
    /// 目录列表缓存：`(dirs, files)`。`show()` 每帧都跑，读目录只能在切换 /
    /// 点刷新时做一次，否则 60fps 下就是每帧一次 `read_dir`。
    /// `listed` 记录这份缓存对应哪个目录；空元组 = 没有可显示的条目。
    listing: (Vec<PathBuf>, Vec<PathBuf>),
    listed: Option<PathBuf>,
    /// 列出 `.` 开头（Windows 另加隐藏属性）的条目。
    show_hidden: bool,
    /// 列出所有文件，而不是只列认识的音频扩展名——核心那边任何格式都能吃
    /// （其余交给自带 ffmpeg），选择器没理由替它把关。
    show_all: bool,
}

impl Picker {
    pub fn new(target: PickTarget, start: PathBuf) -> Self {
        let dir_mode = target != PickTarget::Files;
        // 起始位置尽量落在一个真实存在的目录上，并且必须是绝对路径。
        let start = absolutize(&start);
        let cwd = if start.is_dir() {
            start
        } else if let Some(p) = start.parent().filter(|p| p.is_dir()) {
            p.to_path_buf()
        } else {
            home_dir()
        };
        let mut me = Self {
            target,
            dir_mode,
            cwd,
            chosen: HashSet::new(),
            typed: String::new(),
            err: None,
            listing: (Vec::new(), Vec::new()),
            listed: None,
            show_hidden: false,
            show_all: false,
        };
        me.sync_typed();
        me
    }

    pub fn target(&self) -> PickTarget {
        self.target
    }

    /// 路径框跟手：导航之后框里就是当前路径，想手改随时能改。
    fn sync_typed(&mut self) {
        self.typed = self.cwd.display().to_string();
    }

    fn goto(&mut self, p: PathBuf) {
        let p = absolutize(&p);
        self.cwd = p;
        self.chosen.clear();
        self.err = None;
        self.invalidate();
        self.sync_typed();
    }

    /// 缓存作废，下一帧重读。
    fn invalidate(&mut self) {
        self.listing = (Vec::new(), Vec::new());
        self.listed = None;
    }

    /// 读当前目录：目录在前、文件在后，各自按路径排序。
    ///
    /// 只在切换目录 / 点「刷新」时被调一次（见 [`Picker::listing`] 的注释）。
    fn read_listing(&self) -> Result<(Vec<PathBuf>, Vec<PathBuf>), String> {
        let rd = std::fs::read_dir(&self.cwd)
            .map_err(|e| format!("打不开 {}：{e}", self.cwd.display()))?;
        let mut dirs = Vec::new();
        let mut files = Vec::new();
        for e in rd.flatten() {
            if !self.show_hidden && is_hidden(&e) {
                continue;
            }
            let p = e.path();
            if p.is_dir() {
                dirs.push(p);
            } else if !self.dir_mode {
                // 别把自己的产物当输入。CLI 的批量一直有这道过滤，scan_dir 也有，
                // 选择器漏了：转完一轮 *_5.1.flac 就躺在源目录里，全被列出来。
                let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("");
                if stem.ends_with("_5.1") {
                    continue;
                }
                if self.show_all || upmix_core::fileio::is_input_path(&p) {
                    files.push(p);
                }
            }
        }
        dirs.sort();
        files.sort();
        Ok((dirs, files))
    }

    /// 缓存过期时重读一次。失败的目录也要记账，否则每个帧都会重读一遍。
    fn refresh_listing(&mut self) {
        match self.read_listing() {
            Ok(v) => self.listing = v,
            Err(e) => {
                self.listing = (Vec::new(), Vec::new());
                self.err = Some(e);
            }
        }
        self.listed = Some(self.cwd.clone());
    }

    /// 画选择器。返回 `Done` 表示用户确认，`Cancel` 表示放弃。
    pub fn show(&mut self, ctx: &egui::Context, t: &Theme) -> PickAction {
        // 缓存失效（首帧 / 切了目录 / 点了刷新）才读一次。
        if self.listed.as_deref() != Some(self.cwd.as_path()) {
            self.refresh_listing();
        }
        // take 出来再画：不能 clone（几千个 PathBuf 每帧堆分配一次，跟要修的
        // 问题同一性质），也不能把 &mut self 传进闭包。闭包借走，结尾还回来，
        // 两个 Vec::new() 是不分配的占位。
        let mut listing = std::mem::replace(&mut self.listing, (Vec::new(), Vec::new()));
        // 元组的两个字段可以同时可变借用（不相交）。
        let (dirs, files): (&mut Vec<PathBuf>, &mut Vec<PathBuf>) =
            (&mut listing.0, &mut listing.1);

        let mut action = PickAction::Pending;
        let mut nav: Option<PathBuf> = None;
        // 闭包里不能直接动 self.listing / self.listed（它们已经被 take 出来了），
        // 只能立旗子，出闭包再统一处理。
        let mut refresh = false;
        let title = if self.dir_mode {
            "选择文件夹"
        } else {
            "选择音频文件"
        };

        egui::Window::new(title)
            .collapsible(false)
            .resizable(true)
            .default_size([640.0, 480.0])
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                // ---- 导航 ----
                // cwd 单独放在下面一行：路径一长就会把按钮挤到看不见的地方。
                let at_root = self.cwd.parent().is_none();
                ui.horizontal(|ui| {
                    let up = ui.add_enabled(!at_root, egui::Button::new("↑ 上一级"));
                    if up.clicked() {
                        if let Some(p) = self.cwd.parent().map(Path::to_path_buf) {
                            nav = Some(p);
                        }
                    }
                    if ui.button("主目录").clicked() {
                        nav = Some(home_dir());
                    }
                    if cfg!(windows) {
                        for d in drives() {
                            if ui.button(d.display().to_string()).clicked() {
                                nav = Some(d);
                            }
                        }
                    } else if ui.button("根目录").clicked() {
                        nav = Some(PathBuf::from("/"));
                    }
                    if ui.button("刷新").clicked() {
                        refresh = true;
                    }
                    if at_root {
                        ui.label(RichText::new("（已在根目录）").color(t.ink3));
                    }
                });
                ui.label(RichText::new(self.cwd.display().to_string()).color(t.ink2));

                // ---- 手输路径 ----
                ui.horizontal(|ui| {
                    ui.label(RichText::new("路径").color(t.ink2));
                    let te = ui.add(
                        egui::TextEdit::singleline(&mut self.typed)
                            .hint_text("直接粘一个路径，回车或点跳转")
                            .desired_width(380.0),
                    );
                    let enter = te.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    if enter || ui.button("跳转").clicked() {
                        let raw = self.typed.trim().to_owned();
                        if raw.is_empty() {
                            self.err = Some("路径是空的".into());
                        } else {
                            let p = absolutize(&crate::app::expand(&raw));
                            if p.is_dir() {
                                nav = Some(p);
                            } else {
                                self.err = Some(format!("不是目录：{}", p.display()));
                            }
                        }
                    }
                });

                // ---- 过滤开关 ----
                ui.horizontal(|ui| {
                    if ui.checkbox(&mut self.show_hidden, "显示隐藏项").changed() {
                        refresh = true;
                    }
                    if !self.dir_mode
                        && ui
                            .checkbox(&mut self.show_all, "显示全部文件")
                            .changed()
                    {
                        refresh = true;
                    }
                });

                if let Some(e) = &self.err {
                    ui.label(RichText::new(e).color(t.danger));
                }
                ui.separator();

                // ---- 条目列表 ----
                // show_rows 只画看得见的那几十行；几千个条目也不至于把帧时间吃光。
                let total = dirs.len() + files.len();
                if total == 0 {
                    ui.label(RichText::new("（这里没有可选项）").color(t.ink3));
                } else {
                    // selectable_label 的实际高度：交互尺寸和文字行高取大者。
                    // show_rows 要的是「不含行间距」的行高，间距它自己加。
                    let row_h = ui
                        .spacing()
                        .interact_size
                        .y
                        .max(ui.text_style_height(&egui::TextStyle::Body));
                    egui::ScrollArea::vertical()
                        .id_salt("upmix-picker-list")
                        .max_height(320.0)
                        .auto_shrink([false, false])
                        .show_rows(ui, row_h, total, |ui, range| {
                            for i in range {
                                if i < dirs.len() {
                                    let d = &dirs[i];
                                    let name =
                                        d.file_name().unwrap_or_default().to_string_lossy();
                                    if ui.selectable_label(false, format!("📁 {name}")).clicked()
                                    {
                                        nav = Some(d.clone());
                                    }
                                } else {
                                    let f = &files[i - dirs.len()];
                                    let name =
                                        f.file_name().unwrap_or_default().to_string_lossy();
                                    let on = self.chosen.contains(f);
                                    let label =
                                        format!("{} {name}", if on { "☑" } else { "☐" });
                                    if ui.selectable_label(on, label).clicked() {
                                        if on {
                                            self.chosen.remove(f);
                                        } else {
                                            self.chosen.insert(f.clone());
                                        }
                                    }
                                }
                            }
                        });
                }

                ui.separator();

                // ---- 确认 ----
                ui.horizontal(|ui| {
                    if self.dir_mode {
                        if ui.button("就用这个目录").clicked() {
                            action = PickAction::Done(PickOutcome::Dir(self.cwd.clone()));
                        }
                    } else {
                        let n = self.chosen.len();
                        if ui
                            .add_enabled(n > 0, egui::Button::new(format!("加入所选（{n}）")))
                            .clicked()
                        {
                            let mut v: Vec<PathBuf> = self.chosen.iter().cloned().collect();
                            v.sort();
                            action = PickAction::Done(PickOutcome::Files(v));
                        }
                    }
                    if ui.button("取消").clicked() {
                        action = PickAction::Cancel;
                    }
                });
            });

        // 还回去（或作废重读）。
        if refresh {
            self.invalidate();
        } else {
            self.listing = listing;
        }
        if let Some(p) = nav {
            self.goto(p);
        }
        action
    }
}
