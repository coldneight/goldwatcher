//! Visual language for GoldWatcher: deep navy surfaces, warm gold accents and
//! restrained financial data colours.

use eframe::egui::{
    self, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Frame, Margin,
    RichText, Stroke, TextStyle, Vec2, Visuals,
};

pub const CJK_FONT: &[u8] = include_bytes!("../assets/GoldWatcherSans-Regular.otf");

pub const BG: Color32 = Color32::from_rgb(8, 11, 16);
pub const SURFACE: Color32 = Color32::from_rgb(15, 20, 28);
pub const SURFACE_2: Color32 = Color32::from_rgb(20, 27, 37);
pub const SURFACE_3: Color32 = Color32::from_rgb(25, 33, 45);
pub const BORDER: Color32 = Color32::from_rgb(35, 45, 59);
pub const BORDER_SOFT: Color32 = Color32::from_rgb(27, 35, 46);
pub const GOLD: Color32 = Color32::from_rgb(232, 185, 90);
pub const GOLD_BRIGHT: Color32 = Color32::from_rgb(248, 217, 139);
pub const GOLD_DIM: Color32 = Color32::from_rgb(158, 120, 48);
pub const TEXT: Color32 = Color32::from_rgb(232, 236, 242);
pub const MUTED: Color32 = Color32::from_rgb(137, 148, 163);
pub const FAINT: Color32 = Color32::from_rgb(91, 101, 115);
pub const GREEN: Color32 = Color32::from_rgb(72, 199, 142);
pub const RED: Color32 = Color32::from_rgb(242, 111, 112);
pub const BLUE: Color32 = Color32::from_rgb(100, 164, 232);

pub fn alpha(color: Color32, a: u8) -> Color32 {
    Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), a)
}

pub fn install(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    fonts.font_data.insert(
        "noto-cjk".to_owned(),
        FontData::from_static(CJK_FONT).into(),
    );
    if let Some(family) = fonts.families.get_mut(&FontFamily::Proportional) {
        family.push("noto-cjk".to_owned());
    }
    if let Some(family) = fonts.families.get_mut(&FontFamily::Monospace) {
        family.push("noto-cjk".to_owned());
    }
    if let Some((bytes, index)) = system_cjk_font() {
        let mut data = FontData::from_owned(bytes);
        data.index = index;
        fonts.font_data.insert("system-cjk".to_owned(), data.into());
        for family in [FontFamily::Proportional, FontFamily::Monospace] {
            if let Some(list) = fonts.families.get_mut(&family) {
                list.push("system-cjk".to_owned());
            }
        }
    }
    ctx.set_fonts(fonts);

    let mut style = (*ctx.style()).clone();
    let mut visuals = Visuals::dark();
    visuals.panel_fill = BG;
    visuals.window_fill = SURFACE;
    visuals.extreme_bg_color = Color32::from_rgb(6, 9, 13);
    visuals.faint_bg_color = SURFACE_2;
    visuals.code_bg_color = SURFACE_2;
    visuals.selection.bg_fill = alpha(GOLD, 70);
    visuals.selection.stroke = Stroke::new(1.0_f32, GOLD_BRIGHT);
    visuals.window_stroke = Stroke::new(1.0_f32, BORDER);
    visuals.window_corner_radius = CornerRadius::same(16);
    visuals.menu_corner_radius = CornerRadius::same(10);
    visuals.hyperlink_color = GOLD;
    visuals.warn_fg_color = GOLD;
    visuals.error_fg_color = RED;

    visuals.widgets.noninteractive.bg_fill = SURFACE;
    visuals.widgets.noninteractive.weak_bg_fill = SURFACE;
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, BORDER_SOFT);
    visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0_f32, TEXT);
    visuals.widgets.noninteractive.corner_radius = CornerRadius::same(10);

    visuals.widgets.inactive.bg_fill = SURFACE_2;
    visuals.widgets.inactive.weak_bg_fill = SURFACE_2;
    visuals.widgets.inactive.bg_stroke = Stroke::new(1.0_f32, BORDER);
    visuals.widgets.inactive.fg_stroke = Stroke::new(1.0_f32, TEXT);
    visuals.widgets.inactive.corner_radius = CornerRadius::same(9);

    visuals.widgets.hovered.bg_fill = SURFACE_3;
    visuals.widgets.hovered.weak_bg_fill = SURFACE_3;
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, GOLD_DIM);
    visuals.widgets.hovered.fg_stroke = Stroke::new(1.0_f32, GOLD_BRIGHT);
    visuals.widgets.hovered.corner_radius = CornerRadius::same(9);

    visuals.widgets.active.bg_fill = SURFACE_3;
    visuals.widgets.active.weak_bg_fill = SURFACE_3;
    visuals.widgets.active.bg_stroke = Stroke::new(1.0_f32, GOLD);
    visuals.widgets.active.fg_stroke = Stroke::new(1.0_f32, GOLD_BRIGHT);
    visuals.widgets.active.corner_radius = CornerRadius::same(9);

    visuals.widgets.open.bg_fill = SURFACE_3;
    visuals.widgets.open.weak_bg_fill = SURFACE_3;
    visuals.widgets.open.bg_stroke = Stroke::new(1.0_f32, GOLD_DIM);
    visuals.widgets.open.fg_stroke = Stroke::new(1.0_f32, GOLD_BRIGHT);

    style.visuals = visuals;
    style.spacing.item_spacing = Vec2::new(10.0, 10.0);
    style.spacing.button_padding = Vec2::new(13.0, 7.0);
    style.spacing.interact_size.y = 30.0;
    style.spacing.slider_width = 160.0;
    style.text_styles.insert(
        TextStyle::Heading,
        FontId::new(23.0, FontFamily::Proportional),
    );
    style
        .text_styles
        .insert(TextStyle::Body, FontId::new(14.0, FontFamily::Proportional));
    style.text_styles.insert(
        TextStyle::Button,
        FontId::new(13.5, FontFamily::Proportional),
    );
    style.text_styles.insert(
        TextStyle::Small,
        FontId::new(11.5, FontFamily::Proportional),
    );
    style.text_styles.insert(
        TextStyle::Monospace,
        FontId::new(13.0, FontFamily::Monospace),
    );
    ctx.set_style(style);
}

/// Load a system CJK font as a fallback for user-entered names containing
/// characters outside the bundled common-Chinese subset.
fn system_cjk_font() -> Option<(Vec<u8>, u32)> {
    #[cfg(target_os = "linux")]
    let candidates: &[(&str, u32)] = &[
        ("/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc", 2),
        ("/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc", 2),
        ("/usr/share/fonts/wenquanyi/wqy-zenhei/wqy-zenhei.ttc", 0),
        ("/usr/share/fonts/truetype/wqy/wqy-zenhei.ttc", 0),
    ];
    #[cfg(target_os = "windows")]
    let candidates: &[(&str, u32)] = &[
        ("C:\\Windows\\Fonts\\msyh.ttc", 0),
        ("C:\\Windows\\Fonts\\msyh.ttf", 0),
        ("C:\\Windows\\Fonts\\simhei.ttf", 0),
        ("C:\\Windows\\Fonts\\simsun.ttc", 0),
    ];
    #[cfg(target_os = "macos")]
    let candidates: &[(&str, u32)] = &[
        ("/System/Library/Fonts/PingFang.ttc", 0),
        ("/System/Library/Fonts/Hiragino Sans GB.ttc", 0),
    ];
    #[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
    let candidates: &[(&str, u32)] = &[];

    for (path, index) in candidates {
        if let Ok(bytes) = std::fs::read(path) {
            return Some((bytes, *index));
        }
    }
    None
}

pub fn card() -> Frame {
    Frame::new()
        .fill(SURFACE)
        .stroke(Stroke::new(1.0_f32, BORDER_SOFT))
        .corner_radius(16)
        .inner_margin(Margin::symmetric(18, 16))
}

pub fn card_tight() -> Frame {
    Frame::new()
        .fill(SURFACE)
        .stroke(Stroke::new(1.0_f32, BORDER_SOFT))
        .corner_radius(14)
        .inner_margin(Margin::symmetric(14, 12))
}

pub fn title(text: impl Into<String>) -> RichText {
    RichText::new(text).size(15.0).color(TEXT).strong()
}

pub fn label(text: impl Into<String>) -> RichText {
    RichText::new(text).size(12.0).color(MUTED)
}

pub fn tiny(text: impl Into<String>) -> RichText {
    RichText::new(text).size(10.5).color(FAINT)
}

pub fn gold_label(text: impl Into<String>) -> RichText {
    RichText::new(text).size(11.0).color(GOLD).strong()
}

pub fn big_money(text: impl Into<String>) -> RichText {
    RichText::new(text)
        .size(40.0)
        .color(TEXT)
        .family(FontFamily::Proportional)
}

pub fn primary_button(text: impl Into<String>) -> egui::Button<'static> {
    egui::Button::new(
        RichText::new(text.into())
            .size(13.0)
            .color(Color32::from_rgb(26, 20, 8))
            .strong(),
    )
    .fill(GOLD)
    .stroke(Stroke::new(1.0_f32, GOLD_BRIGHT))
    .corner_radius(9)
}

pub fn ghost_button(text: impl Into<String>) -> egui::Button<'static> {
    egui::Button::new(RichText::new(text.into()).size(12.0).color(MUTED))
        .fill(Color32::TRANSPARENT)
        .stroke(Stroke::new(1.0_f32, BORDER))
        .corner_radius(8)
}

pub fn subtle_button(text: impl Into<String>) -> egui::Button<'static> {
    egui::Button::new(RichText::new(text.into()).size(12.0).color(MUTED))
        .fill(SURFACE_2)
        .stroke(Stroke::new(1.0_f32, BORDER_SOFT))
        .corner_radius(8)
}

pub fn direction_color(value: f64) -> Color32 {
    if value > 0.0 {
        GREEN
    } else if value < 0.0 {
        RED
    } else {
        MUTED
    }
}

pub fn status_pill(
    ui: &mut egui::Ui,
    text: impl Into<String>,
    color: Color32,
    dot: bool,
) -> egui::Response {
    let text = text.into();
    Frame::new()
        .fill(alpha(color, 20))
        .stroke(Stroke::new(1.0_f32, alpha(color, 70)))
        .corner_radius(20)
        .inner_margin(Margin::symmetric(10, 5))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                if dot {
                    let (rect, _) = ui.allocate_exact_size(Vec2::splat(6.0), egui::Sense::hover());
                    ui.painter().circle_filled(rect.center(), 3.0, color);
                    ui.add_space(1.0);
                }
                ui.label(RichText::new(text).size(11.0).color(color).strong());
            });
        })
        .response
}

pub fn ambient(ui: &egui::Ui) {
    let rect = ui.max_rect();
    let painter = ui.painter();
    let center = egui::pos2(rect.right() - 140.0, rect.top() + 10.0);
    for i in (0..9).rev() {
        let radius = 80.0 + i as f32 * 36.0;
        let a = (3.0 + (9 - i) as f32 * 0.9) as u8;
        painter.circle_filled(center, radius, alpha(GOLD, a));
    }
    let center2 = egui::pos2(rect.left() + 40.0, rect.bottom() - 60.0);
    for i in (0..6).rev() {
        let radius = 100.0 + i as f32 * 46.0;
        painter.circle_filled(center2, radius, alpha(BLUE, 2));
    }
}

pub fn divider(ui: &mut egui::Ui) {
    ui.add_space(2.0);
    let (rect, _) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), 1.0), egui::Sense::hover());
    ui.painter().rect_filled(rect, 0.0, BORDER_SOFT);
    ui.add_space(2.0);
}

pub fn change_chip(ui: &mut egui::Ui, value: f64, text: impl Into<String>) {
    let color = direction_color(value);
    Frame::new()
        .fill(alpha(color, 18))
        .stroke(Stroke::new(1.0_f32, alpha(color, 55)))
        .corner_radius(7)
        .inner_margin(Margin::symmetric(8, 4))
        .show(ui, |ui| {
            ui.label(RichText::new(text.into()).size(12.0).color(color).strong());
        });
}
