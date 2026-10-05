//! Theme system: several presets (warm paper, millennium chrome, neon, dreamcore,
//! ink night, plain), plus a user-selectable accent colour.

use egui::{Color32, CornerRadius, Context, Frame, Margin, Shadow, Stroke, Visuals};

/// Visual style family — decides the material effects layered on top of colours.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Style {
    /// 和纸 / 素白 / 墨夜：纯平面。
    Flat,
    /// 千禧风：银色铬金属 + 虹彩扫光。
    Metal,
    /// 霓虹：黑紫底 + 发光描边。
    Neon,
    /// 梦核：粉彩柔光、朦胧。
    Dream,
}

#[derive(Clone, Debug)]
pub struct Theme {
    pub id: &'static str,
    pub name: &'static str,
    pub dark: bool,
    pub style: Style,
    pub paper: Color32,
    /// Backdrop gradient end (dark/layered themes use a subtle gradient).
    pub paper2: Color32,
    pub card: Color32,
    pub stroke: Color32,
    pub ink: Color32,
    pub ink2: Color32,
    pub ink3: Color32,
    pub accent: Color32,
    pub ok: Color32,
    pub danger: Color32,
}

impl Theme {
    /// 是否有“材质”层（非平面）——决定阴影、圆角、控件填充。
    pub fn layered(&self) -> bool {
        self.style != Style::Flat
    }
}

pub fn washi() -> Theme {
    Theme {
        id: "washi",
        name: "和纸",
        dark: false,
        style: Style::Flat,
        paper: Color32::from_rgb(0xF8, 0xF4, 0xED),
        paper2: Color32::from_rgb(0xF3, 0xEC, 0xE0),
        card: Color32::from_rgb(0xFC, 0xFA, 0xF5),
        stroke: Color32::from_rgba_premultiplied(0x22, 0x1B, 0x19, 0x2E),
        ink: Color32::from_rgb(0x3B, 0x3D, 0x3F),
        ink2: Color32::from_rgb(0x6B, 0x6F, 0x73),
        ink3: Color32::from_rgb(0x8E, 0x91, 0x96),
        accent: Color32::from_rgb(0x53, 0x7D, 0x96),
        ok: Color32::from_rgb(0x7B, 0xAE, 0x7F),
        danger: Color32::from_rgb(0x8B, 0x3A, 0x3A),
    }
}

pub fn plain() -> Theme {
    Theme {
        id: "plain",
        name: "素白",
        dark: false,
        style: Style::Flat,
        paper: Color32::from_rgb(0xF4, 0xF5, 0xF7),
        paper2: Color32::from_rgb(0xEC, 0xEE, 0xF1),
        card: Color32::from_rgb(0xFF, 0xFF, 0xFF),
        stroke: Color32::from_rgba_premultiplied(0x20, 0x24, 0x2A, 0x24),
        ink: Color32::from_rgb(0x2C, 0x2F, 0x33),
        ink2: Color32::from_rgb(0x62, 0x68, 0x70),
        ink3: Color32::from_rgb(0x90, 0x96, 0x9E),
        accent: Color32::from_rgb(0x4A, 0x6F, 0x8A),
        ok: Color32::from_rgb(0x5F, 0x9E, 0x6B),
        danger: Color32::from_rgb(0x9E, 0x4A, 0x4A),
    }
}

pub fn ink_night() -> Theme {
    Theme {
        id: "ink",
        name: "墨夜",
        dark: true,
        style: Style::Flat,
        paper: Color32::from_rgb(0x1B, 0x1D, 0x22),
        paper2: Color32::from_rgb(0x14, 0x16, 0x1A),
        card: Color32::from_rgb(0x24, 0x27, 0x2E),
        stroke: Color32::from_rgba_premultiplied(0xC8, 0xD0, 0xDA, 0x22),
        ink: Color32::from_rgb(0xE6, 0xE8, 0xEC),
        ink2: Color32::from_rgb(0xA8, 0xAE, 0xB8),
        ink3: Color32::from_rgb(0x7A, 0x81, 0x8C),
        accent: Color32::from_rgb(0x7E, 0xA8, 0xC4),
        ok: Color32::from_rgb(0x8B, 0xC0, 0x94),
        danger: Color32::from_rgb(0xC8, 0x6B, 0x6B),
    }
}

/// 千禧：银色铬金属底 + 半透明白玻璃卡片 + 强镜面反光。
///
/// 旧版 accent / ok / danger **全是灰色**：accent 对卡片只有 2.44:1、ok 只有
/// 2.73:1，两条都低于 3:1。后果不是"不好看"，是**功能性的**——「成功」和
/// 「失败」在界面上根本分不出来，强调色也等于不存在。现在换成 Y2K 的水蓝
/// 强调 + 真正的绿/红语义色，全部 ≥4.5:1。
pub fn millennium() -> Theme {
    Theme {
        id: "millennium",
        name: "千禧",
        dark: false,
        style: Style::Metal,
        paper: Color32::from_rgb(0xC9, 0xCB, 0xD1),
        paper2: Color32::from_rgb(0xF0, 0xF1, 0xF4),
        card: Color32::from_rgba_unmultiplied(0xFF, 0xFF, 0xFF, 205),
        stroke: Color32::from_rgba_unmultiplied(0xFF, 0xFF, 0xFF, 242),
        ink: Color32::from_rgb(0x1F, 0x21, 0x28),
        ink2: Color32::from_rgb(0x4A, 0x4D, 0x57),
        ink3: Color32::from_rgb(0x68, 0x6C, 0x78),
        accent: Color32::from_rgb(0x0F, 0x6E, 0x96),
        ok: Color32::from_rgb(0x2A, 0x7A, 0x4C),
        danger: Color32::from_rgb(0xA8, 0x1E, 0x12),
    }
}

/// 霓虹：午夜蓝炭灰底 + 品红强调 + 青色细边，发光只留给标题和主按钮。
///
/// 旧版踩了霓虹设计里最典型的两条坑：
/// 1. 底色是**高饱和深紫**（`card` 甚至是 63% 不透明的饱和紫）。霓虹色在饱和
///    底色上会互相洇染，粉字压在紫底上就糊成一团。指南的原话是"底色要深色、
///    中性，让霓虹在没有竞争的情况下发光"。
/// 2. `neon_curtain` 给**每一张卡片**都加光幕 + 扫描线 + 发光描边 + 外发光，
///    整屏扫描线再加一层。指南明说"過多的霓虹燈會消弭光影之間戲劇性的相互
///    作用"——霓虹是强调点，不是主导元素。
/// 现在底色换成去饱和的午夜蓝炭灰（不用纯黑，缓和 halation），卡片给实底当
/// 文字遮罩，正文用离白压到 14.8:1，饱和色只出现在标题/强调/主按钮上。
pub fn neon() -> Theme {
    Theme {
        id: "neon",
        name: "霓虹",
        dark: true,
        style: Style::Neon,
        paper: Color32::from_rgb(0x0E, 0x11, 0x17),
        paper2: Color32::from_rgb(0x12, 0x16, 0x1E),
        card: Color32::from_rgb(0x19, 0x1D, 0x26),
        // 经典霓虹对：品红强调 + 青色细边。边只做 3:1 的描边，不承载正文。
        stroke: Color32::from_rgba_unmultiplied(0x3D, 0xE1, 0xFF, 96),
        ink: Color32::from_rgb(0xED, 0xEF, 0xF5),
        ink2: Color32::from_rgb(0xB7, 0xBE, 0xCC),
        ink3: Color32::from_rgb(0x8E, 0x96, 0xA6),
        accent: Color32::from_rgb(0xFF, 0x3D, 0x9E),
        ok: Color32::from_rgb(0x4A, 0xDE, 0x9B),
        danger: Color32::from_rgb(0xFF, 0x5C, 0x6E),
    }
}

/// 梦核：浅色柔和、朦胧，蓝/绿/粉/白都在。
///
/// 梦核（dreamcore）的核心色按美学 wiki 是"彩虹色、蓝色、绿色、粉色、白色"，
/// 配色浅而柔和——旧版整屏只有一个紫调，色相上就已经不像梦核了。更要命的是
/// 文字对比度：ink3 只有 2.58:1、accent 2.42:1、ok 2.08:1，而 ok↔danger 的
/// 可区分度只有 1.18——「成功」和「失败」在界面上几乎同色。现在保留粉/蓝/
/// 薄荷/奶油的柔光，但文字压到 11.5 / 7.0 / 4.9，语义色换成真正的绿和红。
pub fn dreamcore() -> Theme {
    Theme {
        id: "dream",
        name: "梦核",
        dark: false,
        style: Style::Dream,
        paper: Color32::from_rgb(0xEF, 0xE7, 0xF6),
        paper2: Color32::from_rgb(0xE2, 0xF0, 0xEE),
        card: Color32::from_rgba_unmultiplied(0xFF, 0xFF, 0xFF, 205),
        stroke: Color32::from_rgba_unmultiplied(0xFF, 0xFF, 0xFF, 220),
        ink: Color32::from_rgb(0x3B, 0x32, 0x50),
        ink2: Color32::from_rgb(0x5A, 0x52, 0x70),
        ink3: Color32::from_rgb(0x72, 0x6A, 0x88),
        accent: Color32::from_rgb(0x73, 0x46, 0xB8),
        ok: Color32::from_rgb(0x2C, 0x78, 0x60),
        danger: Color32::from_rgb(0x9E, 0x2F, 0x4C),
    }
}

pub fn presets() -> Vec<Theme> {
    vec![
        washi(),
        millennium(),
        neon(),
        dreamcore(),
        ink_night(),
        plain(),
    ]
}

/// Apply a theme's visuals to the context. `accent` overrides the theme accent.
pub fn apply(ctx: &Context, t: &Theme, accent: Color32) {
    let accent_hover = accent.gamma_multiply(0.8);

    let mut v = if t.dark { Visuals::dark() } else { Visuals::light() };
    v.override_text_color = Some(t.ink);
    v.panel_fill = t.paper;
    v.window_fill = t.card;
    v.extreme_bg_color = t.paper2;
    v.faint_bg_color = Color32::from_rgba_premultiplied(0x80, 0x80, 0x80, 0x10);
    v.window_corner_radius = CornerRadius::same(16);
    v.window_stroke = Stroke::new(1.0, t.stroke);
    v.window_shadow = if t.layered() {
        Shadow {
            offset: [0, 10],
            blur: 34,
            spread: 0,
            color: Color32::from_rgba_premultiplied(0, 0, 0, 90),
        }
    } else {
        Shadow {
            offset: [0, 4],
            blur: 20,
            spread: 0,
            color: Color32::from_rgba_premultiplied(0x40, 0x30, 0x20, 0x14),
        }
    };
    v.selection.bg_fill = accent.gamma_multiply(0.28);
    v.selection.stroke = Stroke::new(1.0, accent);

    let w = &mut v.widgets;
    w.noninteractive.bg_fill = t.card;
    w.noninteractive.weak_bg_fill = t.card;
    w.noninteractive.bg_stroke = Stroke::new(1.0, t.stroke);
    w.noninteractive.fg_stroke = Stroke::new(1.0, t.ink2);
    w.noninteractive.corner_radius = CornerRadius::same(12);

    w.inactive.bg_fill = if t.layered() {
        Color32::from_rgba_unmultiplied(0xFF, 0xFF, 0xFF, 34)
    } else {
        t.card
    };
    w.inactive.weak_bg_fill = if t.layered() {
        Color32::from_rgba_unmultiplied(0xFF, 0xFF, 0xFF, 22)
    } else {
        t.paper2
    };
    w.inactive.bg_stroke = Stroke::new(1.0, t.stroke);
    w.inactive.fg_stroke = Stroke::new(1.0, t.ink);
    w.inactive.corner_radius = CornerRadius::same(12);

    w.hovered.bg_fill = accent.gamma_multiply(0.18);
    w.hovered.weak_bg_fill = accent.gamma_multiply(0.18);
    w.hovered.bg_stroke = Stroke::new(1.0, accent_hover);
    w.hovered.fg_stroke = Stroke::new(1.0, t.ink);
    w.hovered.corner_radius = CornerRadius::same(12);

    w.active.bg_fill = accent.gamma_multiply(0.30);
    w.active.weak_bg_fill = accent.gamma_multiply(0.30);
    w.active.bg_stroke = Stroke::new(1.0, accent);
    w.active.fg_stroke = Stroke::new(1.0, t.ink);
    w.active.corner_radius = CornerRadius::same(12);

    w.open.bg_fill = t.card;
    w.open.bg_stroke = Stroke::new(1.0, t.stroke);

    ctx.set_visuals(v);

    ctx.all_styles_mut(|s| {
        s.spacing.item_spacing = egui::vec2(10.0, 10.0);
        s.spacing.button_padding = egui::vec2(14.0, 8.0);
        s.spacing.interact_size.y = 30.0;
        s.spacing.slider_width = 210.0;

        use egui::{FontFamily, FontId, TextStyle};
        s.text_styles = [
            (TextStyle::Heading, FontId::new(26.0, FontFamily::Proportional)),
            (TextStyle::Body, FontId::new(15.0, FontFamily::Proportional)),
            (TextStyle::Monospace, FontId::new(14.0, FontFamily::Monospace)),
            (TextStyle::Button, FontId::new(15.0, FontFamily::Proportional)),
            (TextStyle::Small, FontId::new(12.5, FontFamily::Proportional)),
        ]
        .into();
    });
}

/// A card surface styled for the current theme.
///
/// Use as `theme::card(&t).show(ui, |ui| { ... })`; for layered themes it also
/// paints the material effect (chrome sheen / neon rim / dreamy halo) on top.
pub struct Card {
    frame: Frame,
    style: Style,
    corner: f32,
    accent: Color32,
    dark: bool,
}

impl Card {
    pub fn show<R>(
        self,
        ui: &mut egui::Ui,
        add: impl FnOnce(&mut egui::Ui) -> R,
    ) -> egui::InnerResponse<R> {
        let resp = self.frame.show(ui, add);
        let rect = resp.response.rect;
        let p = ui.painter();
        match self.style {
            Style::Flat => {}
            Style::Metal => {
                // 千禧：白玻璃卡片 + 铬扫光 + 亮银镜面边（金属要亮）。
                metal_sheen(p, rect);
                edge_highlight(p, rect, self.corner, 235, 110);
            }
            Style::Neon => {
                // 霓虹：无边框光幕（中间亮、上下淡出）+ 顶边亮线。
                neon_curtain(p, rect, self.accent);
            }
            Style::Dream => {
                dream_halo(p, rect, self.corner, self.dark);
                top_sheen(p, rect, self.corner);
            }
        }
        resp
    }
}

pub fn card(t: &Theme) -> Card {
    Card {
        frame: card_frame(t),
        style: t.style,
        corner: if t.layered() { 24.0 } else { 16.0 },
        accent: t.accent,
        dark: t.dark,
    }
}

fn card_frame(t: &Theme) -> Frame {
    if t.style == Style::Neon {
        // 霓虹：给一层实底面板当文字遮罩。旧版这里是空 Frame（内容直接浮在
        // 背景上），再叠每卡片的发光光幕，正文对比度随光幕起伏——霓虹指南
        // 明确要求纹理上的文字背后压一层实底或遮罩。
        return Frame::new()
            .fill(t.card)
            .corner_radius(CornerRadius::same(16))
            .stroke(Stroke::new(1.0, t.stroke))
            .inner_margin(Margin::symmetric(16, 14));
    }
    let shadow = if t.layered() {
        Shadow {
            offset: [0, 12],
            blur: 40,
            spread: 0,
            color: Color32::from_rgba_premultiplied(0, 0, 0, 110),
        }
    } else if t.dark {
        Shadow {
            offset: [0, 3],
            blur: 16,
            spread: 0,
            color: Color32::from_rgba_premultiplied(0, 0, 0, 60),
        }
    } else {
        Shadow {
            offset: [0, 2],
            blur: 14,
            spread: 0,
            color: Color32::from_rgba_premultiplied(0x40, 0x30, 0x20, 0x0E),
        }
    };
    Frame::new()
        .fill(t.card)
        .corner_radius(CornerRadius::same(if t.layered() { 24 } else { 16 }))
        .stroke(Stroke::new(1.0, t.stroke))
        .inner_margin(Margin::symmetric(16, 14))
        .shadow(shadow)
}

/// Backdrop: solid for flat themes, a soft vertical gradient for glass/dark.
/// 壁纸：铺满视口，上面盖一层薄雾，保证玻璃卡片和文字在任何图上都能读。
pub fn paint_wallpaper(ui: &egui::Ui, t: &Theme, tex: &egui::TextureHandle) {
    let rect = ui.ctx().viewport_rect();
    let painter = ui.painter();
    painter.image(
        tex.id(),
        rect,
        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
        Color32::WHITE,
    );
    // 薄雾：壁纸越花，越需要这层。
    let veil = if t.dark { 120 } else { 96 };
    let base = t.paper;
    painter.rect_filled(
        rect,
        0.0,
        Color32::from_rgba_unmultiplied(base.r(), base.g(), base.b(), veil),
    );
}

pub fn paint_backdrop(ui: &egui::Ui, t: &Theme) {
    let rect = ui.ctx().viewport_rect();
    let painter = ui.painter();
    if t.paper == t.paper2 {
        painter.rect_filled(rect, 0.0, t.paper);
    } else {
        let steps = 48;
        for i in 0..steps {
            let f0 = i as f32 / steps as f32;
            let f1 = (i + 1) as f32 / steps as f32;
            let y0 = rect.top() + rect.height() * f0;
            let y1 = rect.top() + rect.height() * f1;
            let c = lerp_color(t.paper, t.paper2, f0);
            painter.rect_filled(
                egui::Rect::from_min_max(egui::pos2(rect.left(), y0), egui::pos2(rect.right(), y1)),
                0.0,
                c,
            );
        }
    }
    // 按风格叠材质层。
    let w = rect.width();
    let h = rect.height();
    match t.style {
        Style::Flat => {}
        Style::Metal => {
            // 千禧：整屏铬金属镜面反射带（金属感的全部来源）。
            chrome_bands(painter, rect);
        }
        Style::Neon => {
            // 霓虹：底色走上方的午夜蓝渐变。旧版这里硬盖一层 #010004 纯黑，
            // 等于把 paper/paper2 作废，而纯黑会加重 halation（发光字在暗底上
            // 的晕染）。扫描线也压到很淡：指南提醒纹理上的文字对比度不稳定，
            // 这里只留一点点 CRT 味。
            scanlines(painter, rect, 4.0, Color32::from_rgba_unmultiplied(0, 0, 0, 26));
        }
        Style::Dream => {
            // 梦核：粉 / 蓝 / 薄荷 / 奶油的大片柔光，低对比不刺眼。
            glow(painter, rect.left_top() + egui::vec2(w * 0.10, h * 0.06), w * 1.00,
                 Color32::from_rgba_unmultiplied(0xF4, 0xC8, 0xE8, 126));
            glow(painter, rect.right_top() + egui::vec2(-w * 0.05, h * 0.22), w * 0.92,
                 Color32::from_rgba_unmultiplied(0xC6, 0xD8, 0xF4, 116));
            glow(painter, rect.center_bottom() + egui::vec2(w * 0.06, h * 0.10), w * 0.95,
                 Color32::from_rgba_unmultiplied(0xC0, 0xEE, 0xDC, 110));
            glow(painter, rect.left_bottom() + egui::vec2(0.0, -h * 0.05), w * 0.70,
                 Color32::from_rgba_unmultiplied(0xF4, 0xE4, 0xB8, 104));
        }
    }
}

/// 霓虹面板：顶边一条灯管 + 往下淡出的很淡光晕。
///
/// 旧版给**每一张卡片**都铺满光幕 + 内部扫描线 + 发光描边 + 外发光，整屏还
/// 再叠一层扫描线。霓虹美学指南的原话是"過多的霓虹燈會消弭光影之間戲劇性的
/// 相互作用"——发光是强调手段，用满了就没有强调可言，正文也跟着难读。现在
/// 只保留顶边这一条灯管，其余交给卡片的实底和青色细边。
fn neon_curtain(painter: &egui::Painter, rect: egui::Rect, tint: Color32) {
    let p = painter.with_clip_rect(rect);

    // 顶边灯管：整张卡片唯一的发光元素
    p.hline(
        rect.left()..=rect.right(),
        rect.top() + 0.5,
        Stroke::new(1.5, Color32::from_rgba_unmultiplied(tint.r(), tint.g(), tint.b(), 170)),
    );

    // 从顶边往下淡出的光晕，只占卡片上部最多 48px
    let h = rect.height().min(48.0);
    if h <= 0.0 {
        return;
    }
    let steps = 24;
    for i in 0..steps {
        let f = (i as f32 + 0.5) / steps as f32;
        let a = ((1.0 - f) * (1.0 - f) * 46.0) as u8;
        if a == 0 {
            continue;
        }
        let y0 = rect.top() + h * (i as f32 / steps as f32);
        let y1 = rect.top() + h * ((i as f32 + 1.0) / steps as f32);
        p.rect_filled(
            egui::Rect::from_min_max(egui::pos2(rect.left(), y0), egui::pos2(rect.right(), y1)),
            0.0,
            Color32::from_rgba_unmultiplied(tint.r(), tint.g(), tint.b(), a),
        );
    }
}

/// 横向扫描线：cybercore / CRT 的味道。
fn scanlines(painter: &egui::Painter, rect: egui::Rect, step: f32, color: Color32) {
    let mut y = rect.top();
    while y < rect.bottom() {
        painter.hline(rect.x_range(), y, Stroke::new(1.0, color));
        y += step;
    }
}

/// 铬金属的镜面反射带：近黑↔纯白的高速交替，金属感全在这个对比里。
fn chrome_bands(painter: &egui::Painter, rect: egui::Rect) {
    const STOPS: &[(f32, [u8; 3])] = &[
        (0.00, [0x14, 0x14, 0x16]),
        (0.05, [0xBA, 0xBA, 0xBE]),
        (0.10, [0xFF, 0xFF, 0xFF]),
        (0.18, [0x70, 0x70, 0x76]),
        (0.32, [0xFF, 0xFF, 0xFF]),
        (0.44, [0x9C, 0x9C, 0xA2]),
        (0.56, [0xFF, 0xFF, 0xFF]),
        (0.68, [0x50, 0x50, 0x56]),
        (0.80, [0xE4, 0xE4, 0xE8]),
        (0.90, [0x26, 0x26, 0x2A]),
        (1.00, [0x0C, 0x0C, 0x0E]),
    ];
    let steps = 160;
    for i in 0..steps {
        let f0 = i as f32 / steps as f32;
        let f1 = (i + 1) as f32 / steps as f32;
        let c = sample_stops(STOPS, f0);
        let y0 = rect.top() + rect.height() * f0;
        let y1 = rect.top() + rect.height() * f1;
        painter.rect_filled(
            egui::Rect::from_min_max(egui::pos2(rect.left(), y0), egui::pos2(rect.right(), y1)),
            0.0,
            Color32::from_rgb(c[0], c[1], c[2]),
        );
    }
}

fn sample_stops(stops: &[(f32, [u8; 3])], f: f32) -> [u8; 3] {
    let f = f.clamp(0.0, 1.0);
    for w in stops.windows(2) {
        let (f0, c0) = w[0];
        let (f1, c1) = w[1];
        if f >= f0 && f <= f1 {
            let t = if (f1 - f0).abs() < 1e-6 { 0.0 } else { (f - f0) / (f1 - f0) };
            let l = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * t).round() as u8;
            return [l(c0[0], c1[0]), l(c0[1], c1[1]), l(c0[2], c1[2])];
        }
    }
    stops.last().map(|s| s.1).unwrap_or([0, 0, 0])
}

/// 千禧金属：卡片上的铬色扫光。
fn metal_sheen(painter: &egui::Painter, rect: egui::Rect) {
    let p = painter.with_clip_rect(rect);
    glow(
        &p,
        egui::pos2(rect.center().x, rect.top() + 2.0),
        rect.width() * 0.62,
        Color32::from_rgba_unmultiplied(0xFF, 0xFF, 0xFF, 64),
    );
    glow(
        &p,
        egui::pos2(rect.left() + rect.width() * 0.28, rect.bottom() + rect.height() * 0.12),
        rect.width() * 0.58,
        Color32::from_rgba_unmultiplied(0xC4, 0xD2, 0xF2, 96),
    );
}

/// 梦核：卡片边缘的柔和光晕，像蒙了一层雾。
fn dream_halo(painter: &egui::Painter, rect: egui::Rect, corner: f32, dark: bool) {
    let p = painter.with_clip_rect(rect);
    let cr = egui::CornerRadius::same(corner as u8);
    let r = rect.shrink(0.9);
    let a = if dark { 70 } else { 130 };
    p.rect_stroke(
        r,
        cr,
        Stroke::new(2.0, Color32::from_rgba_unmultiplied(0xFF, 0xFF, 0xFF, a)),
        egui::StrokeKind::Inside,
    );
    glow(
        &p,
        egui::pos2(rect.left() + rect.width() * 0.22, rect.top() + 2.0),
        rect.width() * 0.80,
        Color32::from_rgba_unmultiplied(0xFF, 0xFF, 0xFF, 120),
    );
    glow(
        &p,
        egui::pos2(rect.center().x, rect.bottom()),
        rect.width() * 0.72,
        Color32::from_rgba_unmultiplied(0xD8, 0xC8, 0xF0, 78),
    );
}

/// 一个从中心向外线性衰减的色斑（用三角扇近似柔和模糊）。
fn glow(painter: &egui::Painter, center: egui::Pos2, radius: f32, color: Color32) {
    let a = color.a() as f32;
    let inner = (radius * 0.42).max(1.0);
    let n = 72u32;
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(center, color);
    for i in 0..=n {
        let ang = i as f32 / n as f32 * std::f32::consts::TAU;
        let dir = egui::vec2(ang.cos(), ang.sin());
        mesh.colored_vertex(
            center + dir * inner,
            Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), (a * 0.62) as u8),
        );
    }
    for i in 0..=n {
        let ang = i as f32 / n as f32 * std::f32::consts::TAU;
        let dir = egui::vec2(ang.cos(), ang.sin());
        mesh.colored_vertex(center + dir * radius, Color32::TRANSPARENT);
    }
    for i in 0..n {
        // 内圈扇形
        mesh.add_triangle(0, 1 + i, 2 + i);
        // 外圈环带
        let o = 1 + i;
        let o2 = 2 + i;
        let b = n + 2;
        mesh.add_triangle(o, b + i, o2);
        mesh.add_triangle(o2, b + i, b + 1 + i);
    }
    painter.add(mesh);
}

/// 玻璃卡片顶部的镜面高光：两端淡、中间亮的一条细线。
fn top_sheen(painter: &egui::Painter, rect: egui::Rect, corner: f32) {
    let inset = corner + 8.0;
    let y0 = rect.top() + 0.9;
    let y1 = y0 + 1.3;
    let xl = rect.left() + inset;
    let xr = rect.right() - inset;
    let xm = (xl + xr) * 0.5;
    let clear = Color32::TRANSPARENT;
    let bright = Color32::from_rgba_unmultiplied(0xFF, 0xFF, 0xFF, 150);
    let mid = Color32::from_rgba_unmultiplied(0xFF, 0xFF, 0xFF, 46);
    let mut mesh = egui::Mesh::default();
    for (x, c) in [(xl, clear), (xm, bright), (xr, clear)] {
        mesh.colored_vertex(egui::pos2(x, y0), c);
        mesh.colored_vertex(egui::pos2(x, y1), c);
    }
    // 列0(0,1) 列1(2,3) 列2(4,5)
    mesh.add_triangle(0, 2, 1);
    mesh.add_triangle(2, 3, 1);
    mesh.add_triangle(2, 4, 3);
    mesh.add_triangle(4, 5, 3);
    // 两侧的柔和头，避免高光线突兀收尾
    let _ = mid;
    painter.add(mesh);
}

/// 玻璃/金属边缘的镜面高光：上半圈亮、下半圈淡，营造厚度与折射感。
fn edge_highlight(painter: &egui::Painter, rect: egui::Rect, corner: f32, bright_a: u8, soft_a: u8) {
    if rect.width() < 4.0 || rect.height() < 4.0 {
        return;
    }
    let r = rect.shrink(0.9);
    let cr = egui::CornerRadius::same(corner as u8);
    let mid = rect.center().y;
    let top_half = egui::Rect::from_min_max(rect.min, egui::pos2(rect.max.x, mid));
    let bot_half = egui::Rect::from_min_max(egui::pos2(rect.min.x, mid), rect.max);

    let bright = Color32::from_rgba_unmultiplied(0xFF, 0xFF, 0xFF, bright_a);
    let soft = Color32::from_rgba_unmultiplied(0xFF, 0xFF, 0xFF, soft_a);
    let outer = painter.with_clip_rect(top_half);
    outer.rect_stroke(r, cr, egui::Stroke::new(1.5, bright), egui::StrokeKind::Inside);
    let lower = painter.with_clip_rect(bot_half);
    lower.rect_stroke(r, cr, egui::Stroke::new(1.5, soft), egui::StrokeKind::Inside);
}

/// 读取一张图片，缩小 + 高斯模糊后传为纹理。模糊是关键：失焦的壁纸才像 iOS。
pub fn load_wallpaper(ctx: &egui::Context, path: &std::path::Path) -> Result<egui::TextureHandle, String> {
    let img = image::open(path).map_err(|e| format!("打不开图片：{e}"))?;
    // 先缩到长边 640，再高斯模糊；egui 线性放大后就是柔和的磨砂背景。
    let long = img.width().max(img.height());
    let img = if long > 640 {
        img.resize(640, 640, image::imageops::FilterType::Triangle)
    } else {
        img
    };
    let blurred = img.blur(14.0);
    let rgba = blurred.to_rgba8();
    let (w, h) = rgba.dimensions();
    let color_image =
        egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], rgba.as_raw());
    Ok(ctx.load_texture(
        "upmix_wallpaper",
        color_image,
        egui::TextureOptions::LINEAR,
    ))
}

fn lerp_color(a: Color32, b: Color32, t: f32) -> Color32 {
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()))
}

pub fn hairline(ui: &mut egui::Ui, t: &Theme) {
    let w = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(egui::vec2(w, 1.0), egui::Sense::hover());
    ui.painter()
        .hline(rect.x_range(), rect.center().y, Stroke::new(1.0, t.stroke));
}

pub fn install_fonts(ctx: &Context) {
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert(
        "eb_garamond".to_owned(),
        std::sync::Arc::new(egui::FontData::from_static(include_bytes!(
            "../assets/EBGaramond.ttf"
        ))),
    );
    fonts.font_data.insert(
        "noto_serif_sc".to_owned(),
        std::sync::Arc::new(egui::FontData::from_static(include_bytes!(
            "../assets/NotoSerifSC-subset.otf"
        ))),
    );
    let prop = fonts
        .families
        .entry(egui::FontFamily::Proportional)
        .or_default();
    prop.insert(0, "noto_serif_sc".to_owned());
    prop.insert(0, "eb_garamond".to_owned());
    ctx.set_fonts(fonts);
}
