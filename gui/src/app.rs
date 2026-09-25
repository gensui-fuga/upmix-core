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

    // output
    outdir: String,
    batch: bool,

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
        let mut files = scan();
        let mut selected = None;
        if let Some(p) = &initial {
            if !files.contains(p) {
                files.insert(0, p.clone());
            }
            selected = files.iter().position(|f| f == p);
        }
        let theme = theme::washi();
        Self {
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
            outdir: String::new(),
            batch: false,
            status: "选择一首歌，然后开始。".into(),
            status_ok: None,
            progress: 0.0,
            indeterminate: false,
            running: false,
            rx: None,
            started: None,
        }
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
            self.files = scan();
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
        theme::apply(ui.ctx(), &self.theme, self.accent);
        theme::paint_backdrop(ui, &self.theme);

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
                    if ui.small_button("⟳ 重新扫描").clicked() {
                        self.files = scan();
                    }
                });
            });
            ui.add_space(2.0);
            theme::hairline(ui, &self.theme);
            ui.add_space(6.0);
            if self.files.is_empty() {
                ui.label(RichText::new("没找到 .flac/.wav。把文件放进 Music/ 或 Download/。").color(self.theme.ink3));
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
            ui.add_space(4.0);
            ui.label(
                RichText::new("液态玻璃：深色通透背景 + 半透明磨砂卡片（egui 无法真实模糊背景，用半透明+高光+阴影模拟）。")
                    .size(12.0)
                    .color(self.theme.ink3),
            );
        });
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
            let status = std::process::Command::new("demucs")
                .arg("-n")
                .arg(model)
                .arg("-o")
                .arg(&tmp)
                .arg(input)
                .status()
                .map_err(|e| format!("找不到 demucs（{e}）。请先 pip install demucs"))?;
            if !status.success() {
                return Err(format!("demucs 退出码 {status}"));
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

fn scan() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = vec![
        PathBuf::from("/storage/emulated/0/Music"),
        PathBuf::from("/storage/emulated/0/Download"),
    ];
    if let Some(home) = std::env::var_os("HOME") {
        dirs.push(PathBuf::from(&home).join("Music"));
        dirs.push(PathBuf::from(home).join("Downloads"));
    }
    dirs.push(PathBuf::from("."));
    let mut out = Vec::new();
    for d in dirs {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            if let Some(ext) = p.extension().and_then(|x| x.to_str()) {
                if matches!(ext.to_ascii_lowercase().as_str(), "flac" | "wav") {
                    out.push(p);
                }
            }
        }
    }
    out.sort();
    out.dedup();
    out
}
