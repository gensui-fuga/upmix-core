//! Theme system: several presets (warm paper, liquid glass, ink night, plain),
//! plus a user-selectable accent colour.

use egui::{Color32, CornerRadius, Context, Frame, Margin, Shadow, Stroke, Visuals};

#[derive(Clone, Debug)]
pub struct Theme {
    pub id: &'static str,
    pub name: &'static str,
    pub dark: bool,
    /// Liquid-glass: translucent cards + highlight stroke + deep backdrop.
    pub glass: bool,
    pub paper: Color32,
    /// Backdrop gradient end (glass/dark use a subtle vertical gradient).
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

pub fn washi() -> Theme {
    Theme {
        id: "washi",
        name: "和纸",
        dark: false,
        glass: false,
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
        glass: false,
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
        glass: false,
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

/// Liquid glass: deep twilight backdrop + translucent frosted cards with a
/// bright top highlight. egui cannot blur the backdrop itself, so the "glass"
/// is simulated with translucency, a light rim and a strong soft shadow.
pub fn glass() -> Theme {
    Theme {
        id: "glass",
        name: "液态玻璃",
        dark: true,
        glass: true,
        paper: Color32::from_rgb(0x23, 0x2A, 0x3A),
        paper2: Color32::from_rgb(0x3A, 0x2E, 0x45),
        card: Color32::from_rgba_premultiplied(0xFF, 0xFF, 0xFF, 26),
        stroke: Color32::from_rgba_premultiplied(0xFF, 0xFF, 0xFF, 60),
        ink: Color32::from_rgb(0xF2, 0xF4, 0xF8),
        ink2: Color32::from_rgb(0xC4, 0xC9, 0xD4),
        ink3: Color32::from_rgb(0x97, 0x9E, 0xAC),
        accent: Color32::from_rgb(0x8F, 0xC7, 0xE8),
        ok: Color32::from_rgb(0x9A, 0xD6, 0xA6),
        danger: Color32::from_rgb(0xE0, 0x8A, 0x8A),
    }
}

pub fn presets() -> Vec<Theme> {
    vec![washi(), glass(), ink_night(), plain()]
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
    v.window_shadow = if t.glass {
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

    w.inactive.bg_fill = if t.glass {
        Color32::from_rgba_premultiplied(0xFF, 0xFF, 0xFF, 34)
    } else {
        t.card
    };
    w.inactive.weak_bg_fill = if t.glass {
        Color32::from_rgba_premultiplied(0xFF, 0xFF, 0xFF, 22)
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
pub fn card(t: &Theme) -> Frame {
    let shadow = if t.glass {
        Shadow {
            offset: [0, 8],
            blur: 28,
            spread: 0,
            color: Color32::from_rgba_premultiplied(0, 0, 0, 70),
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
        .corner_radius(CornerRadius::same(16))
        .stroke(Stroke::new(if t.glass { 1.2 } else { 1.0 }, t.stroke))
        .inner_margin(Margin::symmetric(16, 14))
        .shadow(shadow)
}

/// Backdrop: solid for flat themes, a soft vertical gradient for glass/dark.
pub fn paint_backdrop(ui: &egui::Ui, t: &Theme) {
    let rect = ui.ctx().viewport_rect();
    if t.paper == t.paper2 {
        ui.painter().rect_filled(rect, 0.0, t.paper);
        return;
    }
    let steps = 48;
    for i in 0..steps {
        let f0 = i as f32 / steps as f32;
        let f1 = (i + 1) as f32 / steps as f32;
        let y0 = rect.top() + rect.height() * f0;
        let y1 = rect.top() + rect.height() * f1;
        let c = lerp_color(t.paper, t.paper2, f0);
        ui.painter().rect_filled(
            egui::Rect::from_min_max(egui::pos2(rect.left(), y0), egui::pos2(rect.right(), y1)),
            0.0,
            c,
        );
    }
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
