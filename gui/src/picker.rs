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

/// 主目录。`$HOME` 优先，取不到就退回当前目录。
fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_dir())
        .unwrap_or_else(|| PathBuf::from("."))
}

pub struct Picker {
    target: PickTarget,
    /// 只列目录（选目录），还是也列音频文件。
    dir_mode: bool,
    cwd: PathBuf,
    /// 文件模式下已勾选的条目。
    chosen: HashSet<PathBuf>,
    /// 手输路径（可以直接粘一个路径进来）。
    typed: String,
    err: Option<String>,
}

impl Picker {
    pub fn new(target: PickTarget, start: PathBuf) -> Self {
        let dir_mode = target != PickTarget::Files;
        // 起始位置尽量落在一个真实存在的目录上。
        let cwd = if start.is_dir() {
            start
        } else if let Some(p) = start.parent().filter(|p| p.is_dir()) {
            p.to_path_buf()
        } else {
            home_dir()
        };
        Self {
            target,
            dir_mode,
            cwd,
            chosen: HashSet::new(),
            typed: String::new(),
            err: None,
        }
    }

    pub fn target(&self) -> PickTarget {
        self.target
    }

    fn goto(&mut self, p: PathBuf) {
        self.cwd = p;
        self.chosen.clear();
        self.err = None;
    }

    /// 读当前目录：目录在前、文件在后，各自按路径排序。
    ///
    /// 隐藏条目直接跳过——`~` 下全是 `.cache` / `.local` 这类目录，列出来只会
    /// 把有用的内容挤出屏幕。
    fn entries(&self) -> Result<(Vec<PathBuf>, Vec<PathBuf>), String> {
        let rd = std::fs::read_dir(&self.cwd)
            .map_err(|e| format!("打不开 {}：{e}", self.cwd.display()))?;
        let mut dirs = Vec::new();
        let mut files = Vec::new();
        for e in rd.flatten() {
            if e.file_name().to_string_lossy().starts_with('.') {
                continue;
            }
            let p = e.path();
            if p.is_dir() {
                dirs.push(p);
            } else if !self.dir_mode && upmix_core::is_input_path(&p) {
                files.push(p);
            }
        }
        dirs.sort();
        files.sort();
        Ok((dirs, files))
    }

    /// 画选择器。返回 `Done` 表示用户确认，`Cancel` 表示放弃。
    pub fn show(&mut self, ctx: &egui::Context, t: &Theme) -> PickAction {
        // 目录只在切换时读一次，不在每帧读。
        let (dirs, files) = match self.entries() {
            Ok(v) => v,
            Err(e) => {
                self.err = Some(e);
                (Vec::new(), Vec::new())
            }
        };

        let mut action = PickAction::Pending;
        let mut nav: Option<PathBuf> = None;
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
                ui.horizontal(|ui| {
                    if ui.button("↑ 上一级").clicked() {
                        if let Some(p) = self.cwd.parent().map(Path::to_path_buf) {
                            nav = Some(p);
                        }
                    }
                    if ui.button("主目录").clicked() {
                        nav = Some(home_dir());
                    }
                    ui.label(RichText::new(self.cwd.display().to_string()).color(t.ink2));
                });

                // ---- 手输路径 ----
                ui.horizontal(|ui| {
                    ui.label(RichText::new("路径").color(t.ink2));
                    let te = ui.add(
                        egui::TextEdit::singleline(&mut self.typed)
                            .hint_text("也可以直接粘一个路径，回车跳过去")
                            .desired_width(380.0),
                    );
                    let enter = te.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    if enter || ui.button("跳转").clicked() {
                        let p = crate::app::expand(&self.typed);
                        if p.is_dir() {
                            nav = Some(p);
                        } else {
                            self.err = Some(format!("不是目录：{}", p.display()));
                        }
                    }
                });

                if let Some(e) = &self.err {
                    ui.label(RichText::new(e).color(t.danger));
                }
                ui.separator();

                // ---- 条目列表 ----
                egui::ScrollArea::vertical()
                    .max_height(320.0)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        if dirs.is_empty() && files.is_empty() {
                            ui.label(RichText::new("（这里没有可选项）").color(t.ink3));
                        }
                        for d in &dirs {
                            let name = d.file_name().unwrap_or_default().to_string_lossy();
                            if ui.selectable_label(false, format!("📁 {name}")).clicked() {
                                nav = Some(d.clone());
                            }
                        }
                        for f in &files {
                            let name = f.file_name().unwrap_or_default().to_string_lossy();
                            let on = self.chosen.contains(f);
                            let label = format!("{} {name}", if on { "☑" } else { "☐" });
                            if ui.selectable_label(on, label).clicked() {
                                if on {
                                    self.chosen.remove(f);
                                } else {
                                    self.chosen.insert(f.clone());
                                }
                            }
                        }
                    });

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

        if let Some(p) = nav {
            self.goto(p);
        }
        action
    }
}
