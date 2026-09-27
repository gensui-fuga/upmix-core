//! Shared application UI (desktop). Tabs: Mix / Tutorial / Settings.

use eframe::egui;
use egui::RichText;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::time::Instant;

use upmix_core::{UpmixConfig, Upmixer};

use crate::theme::{self, Theme};

#[derive(PartialEq, Clone, Copy)]
enum Tab {
    Mix,
    Tutorial,
    Settings,
}

#[derive(PartialEq, Clone, Copy)]
enum Mode {
    Fast,
    Auto,
}

enum Msg {
    Progress(usize, usize),
    Stage(String),
    Done(Result<PathBuf, String>),
}

pub struct App {
    files: Vec<PathBuf>,
    selected: Option<usize>,

    mode: Mode,
    tab: Tab,

    theme: Theme,
    theme_idx: usize,
    accent: egui::Color32,

    // fast params
    lfe_gain_db: f32,
    surround_gain_db: f32,
    surround_delay_ms: f32,
    vocal_boost_db: f32,
    win_size: usize,

    // auto params
    demucs_model: String,
    keep_stems: bool,

    // input
    indir: String,

    // output
    outdir: String,
    batch: bool,

    // wallpaper
    wallpaper: Option<egui::TextureHandle>,
    wallpaper_path: String,
    wallpaper_msg: Option<String>,
    /// 材质卡片的透明度 0.0–1.0（仅对有材质层的主题生效）。
    card_alpha: f32,
    /// 上次已写入配置的组合，避免每帧落盘。
    last_saved: Option<(String, [u8; 4], u8)>,

    status: String,
    status_ok: Option<bool>,
    progress: f32,
    indeterminate: bool,
    running: bool,
    rx: Option<Receiver<Msg>>,
    started: Option<Instant>,
}

impl App {
    pub fn new(initial: Option<PathBuf>) -> Self {
        let indir = default_music_dir();
        let mut files = scan_dir(&indir);
        let mut selected = None;
        if let Some(p) = &initial {
            if !files.contains(p) {
                files.insert(0, p.clone());
            }
            selected = files.iter().position(|f| f == p);
        }
        let theme = theme::washi();
        let card_alpha = theme.card.to_srgba_unmultiplied()[3] as f32 / 255.0;
        let mut app = Self {
            files,
            selected,
            mode: Mode::Fast,
            tab: Tab::Mix,
            accent: theme.accent,
            theme,
            theme_idx: 0,
            lfe_gain_db: -6.0,
            surround_gain_db: -3.0,
            surround_delay_ms: 12.0,
            vocal_boost_db: 2.0,
            win_size: 4096,
            demucs_model: "htdemucs".into(),
            keep_stems: false,
            indir,
            outdir: String::new(),
            batch: false,
            wallpaper: None,
            wallpaper_path: String::new(),
            wallpaper_msg: None,
            card_alpha,
            last_saved: None,
            status: "选择一首歌，然后开始。".into(),
            status_ok: None,
            progress: 0.0,
            indeterminate: false,
            running: false,
            rx: None,
            started: None,
        };
        // 读上次的主题 / 强调色 / 透明度。
        if let Some((id, accent, alpha)) = load_config() {
            app.set_theme_by_id(&id);
            if let Some(a) = accent {
                app.accent = egui::Color32::from_rgba_unmultiplied(a[0], a[1], a[2], a[3]);
            }
            if let Some(v) = alpha {
                app.card_alpha = v.clamp(0.0, 1.0);
            }
            app.last_saved = Some((id, app.accent.to_srgba_unmultiplied(), (app.card_alpha * 255.0) as u8));
        }
        app
    }

    fn fast_config(&self) -> UpmixConfig {
        UpmixConfig {
            win_size: self.win_size,
            lfe_gain_db: self.lfe_gain_db as f64,
            surround_gain_db: self.surround_gain_db as f64,
            surround_delay_ms: self.surround_delay_ms as f64,
            vocal_boost_db: self.vocal_boost_db as f64,
            ..Default::default()
        }
    }

    fn start(&mut self) {
        let inputs: Vec<PathBuf> = if self.batch {
            self.files.clone()
        } else {
            match self.selected {
                Some(i) => vec![self.files[i].clone()],
                None => {
                    self.status = "先选择一首歌。".into();
                    self.status_ok = Some(false);
                    return;
                }
            }
        };
        if inputs.is_empty() {
            self.status = "没有可处理的文件。".into();
            self.status_ok = Some(false);
            return;
        }
        let outdir = self.outdir.trim().to_string();
        let (tx, rx) = channel();
        self.rx = Some(rx);
        self.running = true;
        self.progress = 0.0;
        self.status_ok = None;
        self.started = Some(Instant::now());

        match self.mode {
            Mode::Fast => {
                let cfg = self.fast_config();
                self.indeterminate = false;
                self.status = format!("快速重混 {} 个文件…", inputs.len());
                std::thread::spawn(move || {
                    let res = run_batch_fast(&inputs, &outdir, cfg, tx.clone());
                    let _ = tx.send(Msg::Done(res));
                });
            }
            Mode::Auto => {
                let model = self.demucs_model.clone();
                let keep = self.keep_stems;
                self.indeterminate = true;
                self.status = format!("自动分离 {} 个文件（Demucs，较慢）…", inputs.len());
                std::thread::spawn(move || {
                    let res = run_batch_auto(&inputs, &outdir, model, keep, tx.clone());
                    let _ = tx.send(Msg::Done(res));
                });
            }
        }
    }

    fn poll(&mut self) {
        let Some(rx) = &self.rx else { return };
        let mut done = None;
        while let Ok(m) = rx.try_recv() {
            match m {
                Msg::Progress(a, b) => {
                    self.progress = if b > 0 { a as f32 / b as f32 } else { 0.0 };
                    self.indeterminate = false;
                }
                Msg::Stage(s) => self.status = s,
                Msg::Done(r) => done = Some(r),
            }
        }
        if let Some(r) = done {
            self.running = false;
            self.rx = None;
            self.progress = 1.0;
            let secs = self.started.take().map(|s| s.elapsed().as_secs_f32()).unwrap_or(0.0);
            match r {
                Ok(p) => {
                    self.status = format!("完成 · {secs:.1}s → {}", p.display());
                    self.status_ok = Some(true);
                }
                Err(e) => {
                    self.status = format!("失败：{e}");
                    self.status_ok = Some(false);
                }
            }
            self.files = scan_dir(&self.indir);
            self.selected = None;
        }
    }

    fn match_selected(&self) -> bool {
        if self.batch {
            return !self.files.is_empty();
        }
        self.files
            .iter()
            .enumerate()
            .any(|(i, f)| self.selected == Some(i) && f.exists())
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.poll();
        if self.running {
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(80));
        }
        // 透明度可调：保留卡片原色（霓虹是紫的），只改 alpha。
        if self.theme.layered() {
            let [r, g, b, _] = self.theme.card.to_srgba_unmultiplied();
            let a = (self.card_alpha.clamp(0.0, 1.0) * 255.0).round() as u8;
            self.theme.card = egui::Color32::from_rgba_unmultiplied(r, g, b, a);
        }
        theme::apply(ui.ctx(), &self.theme, self.accent);
        match self.wallpaper.clone() {
            Some(tex) => theme::paint_wallpaper(ui, &self.theme, &tex),
            None => theme::paint_backdrop(ui, &self.theme),
        }

        // 把图片拖进窗口 = 设为壁纸。
        let dropped: Vec<std::path::PathBuf> = ui.ctx().input(|i| {
            i.raw
                .dropped_files
                .iter()
                .map(|f| f.path().to_path_buf())
                .collect()
        });
        if let Some(p) = dropped.first() {
            self.set_wallpaper(ui.ctx(), p);
        }

        // 主题 / 强调色 / 透明度一变就存盘，下次启动直接恢复。
        let sig = (
            self.theme.id.to_string(),
            self.accent.to_srgba_unmultiplied(),
            (self.card_alpha * 255.0) as u8,
        );
        if self.last_saved.as_ref() != Some(&sig) {
            save_config(self.theme.id, self.accent, self.card_alpha);
            self.last_saved = Some(sig);
        }

        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(egui::Color32::TRANSPARENT))
            .show(ui, |ui| {
                // header
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Upmix").size(28.0).color(self.theme.ink));
                    ui.add_space(12.0);
                    for (t, label) in [
                        (Tab::Mix, "重混"),
                        (Tab::Tutorial, "教程"),
                        (Tab::Settings, "设置"),
                    ] {
                        if ui.selectable_label(self.tab == t, label).clicked() {
                            self.tab = t;
                        }
                    }
                });
                ui.add_space(4.0);
                theme::hairline(ui, &self.theme);
                ui.add_space(10.0);

                egui::ScrollArea::vertical().show(ui, |ui| match self.tab {
                    Tab::Mix => self.tab_mix(ui),
                    Tab::Tutorial => tab_tutorial(ui, &self.theme),
                    Tab::Settings => self.tab_settings(ui),
                });
                ui.add_space(10.0);
            });
    }
}

impl App {
    fn tab_mix(&mut self, ui: &mut egui::Ui) {
        // mode
        theme::card(&self.theme).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new("模式").size(16.0).color(self.theme.ink));
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.mode, Mode::Fast, "快速（STFT）");
                ui.selectable_value(&mut self.mode, Mode::Auto, "自动分离（Demucs）");
            });
            ui.add_space(4.0);
            let hint = match self.mode {
                Mode::Fast => "秒级完成。用相位/相关性估计，人声居中、环绕包围感，质量受限于立体声信息。",
                Mode::Auto => "真源分离：把 vocals/drums/bass/other 拆开再摆位，最接近原生 5.1；CPU 上约 5× 实时（4 分钟歌 ≈ 20 分钟）。",
            };
            ui.label(RichText::new(hint).size(12.5).color(self.theme.ink3));
        });

        ui.add_space(12.0);

        // source
        theme::card(&self.theme).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(RichText::new("音源").size(16.0).color(self.theme.ink));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button("⟳ 扫描").clicked() {
                        self.files = scan_dir(&self.indir);
                        self.selected = None;
                    }
                });
            });
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new("音乐文件夹").color(self.theme.ink2));
                let resp = ui.add(
                    egui::TextEdit::singleline(&mut self.indir)
                        .hint_text("要转换的目录，回车扫描")
                        .desired_width(ui.available_width()),
                );
                if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    self.files = scan_dir(&self.indir);
                    self.selected = None;
                }
            });
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                if ui.button("选择文件…").clicked() {
                    for p in pick_audio_files() {
                        if !self.files.iter().any(|f| f == &p) {
                            self.files.push(p);
                        }
                    }
                    self.files.sort();
                }
                if ui.button("选择文件夹…").clicked() {
                    if let Some(d) = pick_music_dir() {
                        self.indir = d.to_string_lossy().into_owned();
                        self.files = scan_dir(&self.indir);
                        self.selected = None;
                    }
                }
            });
            ui.add_space(2.0);
            theme::hairline(ui, &self.theme);
            ui.add_space(6.0);
            if self.files.is_empty() {
                ui.label(RichText::new("没找到 .flac/.wav。上面填个音乐文件夹再回车。").color(self.theme.ink3));
            }
            egui::ScrollArea::vertical()
                .id_salt("filelist")
                .max_height(150.0)
                .show(ui, |ui| {
                    for (i, p) in self.files.iter().enumerate() {
                        let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("?");
                        let sel = self.selected == Some(i);
                        if ui
                            .selectable_label(sel, RichText::new(name).color(if sel { self.accent } else { self.theme.ink }))
                            .clicked()
                        {
                            self.selected = Some(i);
                        }
                    }
                });
            ui.add_space(6.0);
            ui.checkbox(&mut self.batch, "批量处理列表全部");
            ui.horizontal(|ui| {
                ui.label(RichText::new("输出目录").color(self.theme.ink2));
                ui.add(
                    egui::TextEdit::singleline(&mut self.outdir)
                        .hint_text("留空 = 源文件旁")
                        .desired_width(ui.available_width()),
                );
            });
        });

        ui.add_space(12.0);

        // params
        theme::card(&self.theme).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new("参数").size(16.0).color(self.theme.ink));
            ui.add_space(2.0);
            theme::hairline(ui, &self.theme);
            ui.add_space(6.0);
            match self.mode {
                Mode::Fast => {
                    ui.add(egui::Slider::new(&mut self.lfe_gain_db, -18.0..=6.0).text("LFE 低频增益").suffix(" dB"));
                    ui.add(egui::Slider::new(&mut self.surround_gain_db, -12.0..=0.0).text("环绕增益").suffix(" dB"));
                    ui.add(egui::Slider::new(&mut self.surround_delay_ms, 0.0..=30.0).text("环绕延迟").suffix(" ms"));
                    ui.add(egui::Slider::new(&mut self.vocal_boost_db, 0.0..=6.0).text("人声聚焦").suffix(" dB"));
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("STFT 窗").color(self.theme.ink2));
                        for w in [1024usize, 2048, 4096, 8192] {
                            ui.selectable_value(&mut self.win_size, w, w.to_string());
                        }
                    });
                }
                Mode::Auto => {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Demucs 模型").color(self.theme.ink2));
                        for m in ["htdemucs", "htdemucs_ft", "mdx_extra", "mdx_extra_q"] {
                            ui.selectable_value(&mut self.demucs_model, m.to_string(), m);
                        }
                    });
                    ui.label(RichText::new("htdemucs 均衡；_ft 最好但更慢；mdx_*_q 量化、更快。").size(12.0).color(self.theme.ink3));
                    ui.checkbox(&mut self.keep_stems, "保留分离出的 stems");
                    ui.label(RichText::new("路由：人声→中置，鼓/贝斯低频→LFE，other 侧向→环绕。").size(12.0).color(self.theme.ink3));
                }
            }
        });

        ui.add_space(12.0);

        // run
        theme::card(&self.theme).show(ui, |ui| {
            ui.set_width(ui.available_width());
            let enabled = !self.running && self.match_selected();
            ui.add_enabled_ui(enabled, |ui| {
                if ui
                    .add(
                        egui::Button::new(RichText::new("▶  开始重混  →  5.1").size(16.0).color(self.theme.card))
                            .fill(self.accent)
                            .corner_radius(10)
                            .min_size(egui::vec2(ui.available_width(), 44.0)),
                    )
                    .clicked()
                {
                    self.start();
                }
            });
            if self.running {
                ui.add_space(8.0);
                if self.indeterminate {
                    ui.add(egui::ProgressBar::new(1.0).animate(true).text("分离中…"));
                } else {
                    ui.add(egui::ProgressBar::new(self.progress).show_percentage());
                }
            }
            ui.add_space(6.0);
            let color = match self.status_ok {
                Some(true) => self.theme.ok,
                Some(false) => self.theme.danger,
                None => self.theme.ink2,
            };
            ui.label(RichText::new(&self.status).size(13.0).color(color));
        });
    }

    /// 按 id 切换主题（washi/millennium/neon/dream/ink/plain）。
    pub fn set_theme_by_id(&mut self, id: &str) {
        let presets = theme::presets();
        for (i, t) in presets.iter().enumerate() {
            if t.id == id {
                self.theme_idx = i;
                self.theme = t.clone();
                self.accent = t.accent;
                self.card_alpha = t.card.to_srgba_unmultiplied()[3] as f32 / 255.0;
                return;
            }
        }
    }

    pub fn set_wallpaper(&mut self, ctx: &egui::Context, path: &Path) {
        match theme::load_wallpaper(ctx, path) {
            Ok(tex) => {
                let name = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("?")
                    .to_string();
                self.wallpaper = Some(tex);
                self.wallpaper_path = path.to_string_lossy().into_owned();
                self.wallpaper_msg = Some(format!("壁纸已加载：{name}"));
            }
            Err(e) => {
                self.wallpaper_msg = Some(format!("加载失败：{e}"));
            }
        }
    }

    fn tab_settings(&mut self, ui: &mut egui::Ui) {
        theme::card(&self.theme).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new("主题").size(16.0).color(self.theme.ink));
            ui.add_space(2.0);
            theme::hairline(ui, &self.theme);
            ui.add_space(6.0);
            let presets = theme::presets();
            ui.horizontal_wrapped(|ui| {
                for (i, t) in presets.iter().enumerate() {
                    let sel = self.theme_idx == i;
                    if ui.selectable_label(sel, t.name).clicked() {
                        self.theme_idx = i;
                        self.theme = t.clone();
                        self.accent = t.accent;
                        self.card_alpha = t.card.to_srgba_unmultiplied()[3] as f32 / 255.0;
                    }
                }
            });
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new("强调色").color(self.theme.ink2));
                ui.color_edit_button_srgba(&mut self.accent);
                if ui.small_button("恢复默认").clicked() {
                    self.accent = self.theme.accent;
                }
            });
            if self.theme.layered() {
                ui.add_space(6.0);
                ui.add(
                    egui::Slider::new(&mut self.card_alpha, 0.0..=1.0)
                        .custom_formatter(|v, _| format!("{:.0}%", v * 100.0))
                        .text("卡片透明度"),
                );
                if ui.small_button("重置透明度").clicked() {
                    self.card_alpha = self.theme.card.to_srgba_unmultiplied()[3] as f32 / 255.0;
                }
            }
            ui.add_space(4.0);
            ui.label(
                RichText::new("千禧：银色铬金属 + 虹彩扫光。霓虹：黑紫底 + 发光描边。梦核：粉彩柔光、朦胧。三者都可用下面的壁纸增强。")
                    .size(12.0)
                    .color(self.theme.ink3),
            );
        });

        ui.add_space(12.0);

        // wallpaper
        let mut do_load = false;
        let mut do_clear = false;
        theme::card(&self.theme).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new("背景壁纸").size(16.0).color(self.theme.ink));
            ui.add_space(2.0);
            theme::hairline(ui, &self.theme);
            ui.add_space(6.0);
            ui.label(
                RichText::new("选一张图当背景，液态玻璃会透出它的颜色（这就是 iOS 的“颜色由周围内容决定”）。也可以直接把图片拖进窗口。")
                    .size(12.5)
                    .color(self.theme.ink3),
            );
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new("图片路径").color(self.theme.ink2));
                let resp = ui.add(
                    egui::TextEdit::singleline(&mut self.wallpaper_path)
                        .hint_text("/home/ye/Pictures/wall.jpg")
                        .desired_width(ui.available_width()),
                );
                if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    do_load = true;
                }
            });
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui.button("加载壁纸").clicked() {
                    do_load = true;
                }
                if ui.button("移除壁纸").clicked() {
                    do_clear = true;
                }
            });
            if let Some(m) = &self.wallpaper_msg {
                ui.add_space(4.0);
                let c = if self.wallpaper.is_some() { self.theme.ok } else { self.theme.ink3 };
                ui.label(RichText::new(m).size(12.5).color(c));
            }
        });
        if do_clear {
            self.wallpaper = None;
            self.wallpaper_msg = Some("已移除壁纸".into());
        }
        if do_load {
            let p = std::path::PathBuf::from(self.wallpaper_path.trim());
            self.set_wallpaper(ui.ctx(), &p);
        }
    }
}

fn tab_tutorial(ui: &mut egui::Ui, t: &Theme) {
    theme::card(t).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.label(RichText::new("教程 · 怎么用它把歌变成 5.1").size(18.0).color(t.ink));
        ui.add_space(6.0);
        theme::hairline(ui, t);
        ui.add_space(8.0);

        let h = |ui: &mut egui::Ui, s: &str| {
            ui.add_space(6.0);
            ui.label(RichText::new(s).size(15.0).color(t.accent));
            ui.add_space(2.0);
        };
        let p = |ui: &mut egui::Ui, s: &str| {
            ui.label(RichText::new(s).size(13.5).color(t.ink));
        };

        h(ui, "① 选音源");
        p(ui, "把 .flac / .wav 放进音乐目录，或点「重新扫描」，在列表里选中一首。");

        h(ui, "② 选模式");
        p(ui, "· 快速（STFT）：几秒完成。用左右声道相位差估计空间，人声自动居中、环绕有包围感。适合日常、批量。");
        p(ui, "· 自动分离（Demucs）：先用神经网络把歌拆成人声/鼓/贝斯/其他，再按原生 5.1 规则摆位。质量最好，但 CPU 上约 5× 实时（4 分钟歌约 20 分钟）。");

        h(ui, "③ 调参数");
        p(ui, "LFE 低频增益：控制低频冲击力（默认 -6 dB，回放会补 +10 dB）。");
        p(ui, "环绕增益 / 延迟：控制后方包围感的强弱与延迟（Haas 效应）。");
        p(ui, "人声聚焦：把中频人声往中置推。");
        p(ui, "STFT 窗：越大频率分辨率越高，越小瞬态越好。4096 是平衡点。");

        h(ui, "④ 开始");
        p(ui, "点「开始重混 → 5.1」，输出写在源文件旁：<名字>_5.1.flac（6 声道 FL FR FC LFE BL BR）。");

        h(ui, "通道是怎么摆的");
        p(ui, "FL/FR 前置＝音乐主体与立体声宽度；C 中置＝人声/主音；LFE＝20–120 Hz 低频；BL/BR 环绕＝环境、混响、扩散场。");
        p(ui, "下混兼容：把它折回立体声，电平和原曲基本一致，不会忽大忽小。");

        h(ui, "命令行也一样");
        p(ui, "快速：upmix-core 歌.flac -o 出.flac");
        p(ui, "自动：upmix-core 歌.flac -o 出.flac --mode auto --keep-stems");

        ui.add_space(6.0);
    });
}

// ---- workers ----

/// 选多个音频文件——调系统自己的文件对话框。
fn pick_audio_files() -> Vec<PathBuf> {
    #[cfg(target_os = "linux")]
    {
        return zenity_dialog(false);
    }
    #[cfg(target_os = "windows")]
    {
        return powershell_dialog(false);
    }
    #[allow(unreachable_code)]
    Vec::new()
}

/// 选一个文件夹。
fn pick_music_dir() -> Option<PathBuf> {
    #[cfg(target_os = "linux")]
    {
        return zenity_dialog(true).into_iter().next();
    }
    #[cfg(target_os = "windows")]
    {
        return powershell_dialog(true).into_iter().next();
    }
    #[allow(unreachable_code)]
    None
}

/// Linux：zenity（GNOME 的文件选择器，走 GTK 原生对话框）。
#[cfg(target_os = "linux")]
fn zenity_dialog(dir: bool) -> Vec<PathBuf> {
    let mut c = std::process::Command::new("zenity");
    c.arg("--file-selection");
    if dir {
        c.arg("--directory").arg("--title=选择音乐文件夹");
    } else {
        c.arg("--multiple")
            .arg("--separator=\n")
            .arg("--file-filter=音频 | *.flac *.wav")
            .arg("--title=选择音乐文件（可多选）");
    }
    let Ok(out) = c.output() else { return Vec::new() };
    if !out.status.success() {
        return Vec::new();
    }
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .map(PathBuf::from)
        .collect()
}

/// Windows：PowerShell 调 WinForms 的 OpenFileDialog / FolderBrowserDialog，
/// 就是资源管理器里那个原生对话框。-STA 是 WinForms 必须的。
#[cfg(target_os = "windows")]
fn powershell_dialog(dir: bool) -> Vec<PathBuf> {
    let script = if dir {
        r#"Add-Type -AssemblyName System.Windows.Forms; $d = New-Object System.Windows.Forms.FolderBrowserDialog; $d.Description = '选择音乐文件夹'; if ($d.ShowDialog() -eq 'OK') { [Console]::Out.Write($d.SelectedPath) }"#
    } else {
        r#"Add-Type -AssemblyName System.Windows.Forms; $f = New-Object System.Windows.Forms.OpenFileDialog; $f.Multiselect = $true; $f.Title = '选择音乐文件（可多选）'; $f.Filter = '音频 (*.flac;*.wav)|*.flac;*.wav|所有文件 (*.*)|*.*'; if ($f.ShowDialog() -eq 'OK') { [Console]::Out.Write(($f.FileNames -join [Environment]::NewLine)) }"#
    };
    let Ok(out) = std::process::Command::new("powershell")
        .args(["-NoProfile", "-STA", "-Command", script])
        .output()
    else {
        return Vec::new();
    };
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .map(PathBuf::from)
        .collect()
}

/// 展开 `~/`（Linux/macOS）或 `~\` 风格路径；拼接一律交给 PathBuf，Windows 会自己用 `\`。
fn expand(path: &str) -> PathBuf {
    let p = path.trim();
    if p == "~" {
        if let Some(h) = std::env::var_os("HOME") {
            return PathBuf::from(h);
        }
    }
    if let Some(rest) = p.strip_prefix("~/") {
        if let Some(h) = std::env::var_os("HOME") {
            return PathBuf::from(h).join(rest);
        }
    }
    if let Some(rest) = p.strip_prefix("~\\") {
        if let Some(h) = std::env::var_os("USERPROFILE") {
            return PathBuf::from(h).join(rest);
        }
    }
    PathBuf::from(p)
}

// ---- 配置持久化：记住主题 / 强调色 / 透明度 ----

fn config_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("APPDATA").map(PathBuf::from))
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("upmix-gui").join("config.txt"))
}

fn load_config() -> Option<(String, Option<[u8; 4]>, Option<f32>)> {
    let txt = std::fs::read_to_string(config_path()?).ok()?;
    let mut theme = String::new();
    let mut accent: Option<[u8; 4]> = None;
    let mut alpha: Option<f32> = None;
    for line in txt.lines() {
        let Some((k, v)) = line.split_once('=') else { continue };
        let v = v.trim();
        match k.trim() {
            "theme" => theme = v.to_string(),
            "accent" => {
                let n: Vec<u8> = v.split(',').filter_map(|x| x.trim().parse().ok()).collect();
                if n.len() == 4 {
                    let mut a = [0u8; 4];
                    a.copy_from_slice(&n);
                    accent = Some(a);
                }
            }
            "card_alpha" => alpha = v.parse().ok(),
            _ => {}
        }
    }
    if theme.is_empty() {
        None
    } else {
        Some((theme, accent, alpha))
    }
}

fn save_config(theme_id: &str, accent: egui::Color32, alpha: f32) {
    let Some(p) = config_path() else { return };
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir).ok();
    }
    let [r, g, b, a] = accent.to_srgba_unmultiplied();
    let body = format!("theme={theme_id}\naccent={r},{g},{b},{a}\ncard_alpha={alpha}\n");
    std::fs::write(p, body).ok();
}

fn out_path_for(input: &Path, outdir: &str) -> PathBuf {
    let stem = input.file_stem().and_then(|s| s.to_str()).unwrap_or("track");
    let name = format!("{stem}_5.1.flac");
    if outdir.is_empty() {
        input.with_file_name(name)
    } else {
        PathBuf::from(outdir).join(name)
    }
}

fn prepare(out_path: &Path) {
    if let Some(p) = out_path.parent() {
        std::fs::create_dir_all(p).ok();
    }
}

fn run_batch_fast(
    inputs: &[PathBuf],
    outdir: &str,
    cfg: UpmixConfig,
    tx: Sender<Msg>,
) -> Result<PathBuf, String> {
    let total = inputs.len();
    let mut last = PathBuf::new();
    for (i, input) in inputs.iter().enumerate() {
        let _ = tx.send(Msg::Stage(format!("快速重混 [{}/{}] {}", i + 1, total, file_name(input))));
        let buf = upmix_core::fileio::read_any(input).map_err(|e| e.to_string())?;
        if buf.num_channels() != 2 {
            return Err(format!("{} 是 {} 声道，需要立体声", file_name(input), buf.num_channels()));
        }
        let out = Upmixer::new(cfg.clone())
            .process_with_progress(&buf, |a, b| {
                let _ = tx.send(Msg::Progress(a, b));
            })
            .map_err(|e| e.to_string())?;
        let out_path = out_path_for(input, outdir);
        prepare(&out_path);
        upmix_core::fileio::write_any(&out_path, &out, Some(input)).map_err(|e| e.to_string())?;
        last = out_path;
    }
    Ok(last)
}

fn run_batch_auto(
    inputs: &[PathBuf],
    outdir: &str,
    model: String,
    keep: bool,
    tx: Sender<Msg>,
) -> Result<PathBuf, String> {
    let total = inputs.len();
    let mut last = PathBuf::new();
    for (i, input) in inputs.iter().enumerate() {
        let _ = tx.send(Msg::Stage(format!("自动分离 [{}/{}] {}", i + 1, total, file_name(input))));
        last = run_one_auto(input, outdir, &model, keep, &tx)?;
    }
    Ok(last)
}

fn run_one_auto(
    input: &Path,
    outdir: &str,
    model: &str,
    keep: bool,
    tx: &Sender<Msg>,
) -> Result<PathBuf, String> {
    let tmp = std::env::temp_dir().join("upmix-stem-cache");
    std::fs::create_dir_all(&tmp).ok();
    let track = input.file_stem().and_then(|s| s.to_str()).unwrap_or("track");

    let stems_dir = match find_stems(&tmp, track) {
        Some(d) => d,
        None => {
            let mut cmd = std::process::Command::new("demucs");
            cmd.arg("-n")
                .arg(model)
                .arg("-o")
                .arg(&tmp)
                .arg(input);
            // 国内直连 huggingface.co 下不动模型，默认走镜像。
            if std::env::var_os("HF_ENDPOINT").is_none() {
                cmd.env("HF_ENDPOINT", "https://hf-mirror.com");
            }
            let status = cmd
                .status()
                .map_err(|e| format!("找不到 demucs（{e}）。请先 pip install demucs"))?;
            if !status.success() {
                return Err(
                    "demucs 失败。如果一直在重试 huggingface.co，说明模型下不动——\
                     已在代码里默认走 hf-mirror.com 镜像。"
                        .to_string(),
                );
            }
            find_stems(&tmp, track).ok_or_else(|| "没找到分离结果".to_string())?
        }
    };

    let _ = tx.send(Msg::Stage("读取 stems 并路由 …".into()));
    let vocals = upmix_core::fileio::read_any(&stems_dir.join("vocals.wav")).map_err(|e| e.to_string())?;
    let drums = upmix_core::fileio::read_any(&stems_dir.join("drums.wav")).map_err(|e| e.to_string())?;
    let bass = upmix_core::fileio::read_any(&stems_dir.join("bass.wav")).map_err(|e| e.to_string())?;
    let other = upmix_core::fileio::read_any(&stems_dir.join("other.wav")).map_err(|e| e.to_string())?;
    let sr = vocals.sample_rate;
    let bits = vocals.bits_per_sample;
    let stems = upmix_core::auto::Stems {
        sample_rate: sr,
        bits_per_sample: bits,
        vocals,
        drums,
        bass,
        other,
    };
    let mut out = upmix_core::auto::route(&stems, &upmix_core::auto::StemRouting::default())
        .map_err(|e| e.to_string())?;
    let peak = out.peak();
    if peak > 1.0 {
        out.apply_gain(1.0 / peak);
    }
    let out_path = out_path_for(input, outdir);
    prepare(&out_path);
    upmix_core::fileio::write_any(&out_path, &out, Some(input)).map_err(|e| e.to_string())?;
    if !keep {
        let _ = std::fs::remove_dir_all(&stems_dir);
    }
    Ok(out_path)
}

fn file_name(p: &Path) -> String {
    p.file_name().and_then(|n| n.to_str()).unwrap_or("?").to_string()
}

fn find_stems(root: &Path, track: &str) -> Option<PathBuf> {
    fn walk(dir: &Path, track: &str) -> Option<PathBuf> {
        for e in std::fs::read_dir(dir).ok()?.flatten() {
            let p = e.path();
            if p.is_dir() {
                if p.file_name().and_then(|n| n.to_str()) == Some(track) && p.join("vocals.wav").exists() {
                    return Some(p);
                }
                if let Some(f) = walk(&p, track) {
                    return Some(f);
                }
            }
        }
        None
    }
    walk(root, track)
}

fn default_music_dir() -> String {
    if let Some(home) = std::env::var_os("HOME") {
        let m = PathBuf::from(&home).join("Music");
        if m.is_dir() {
            return m.to_string_lossy().into_owned();
        }
        return PathBuf::from(home).to_string_lossy().into_owned();
    }
    ".".into()
}

/// 扫描目录里的 .flac/.wav，向下最多两层。
fn scan_dir(dir: &str) -> Vec<PathBuf> {
    let dir = dir.trim();
    if dir.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    collect(&expand(dir), 2, &mut out);
    out.sort();
    out.dedup();
    out
}

fn collect(dir: &Path, depth: u32, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            if depth > 1 {
                collect(&p, depth - 1, out);
            }
        } else if let Some(ext) = p.extension().and_then(|x| x.to_str()) {
            if matches!(ext.to_ascii_lowercase().as_str(), "flac" | "wav") {
                out.push(p);
            }
        }
    }
}
