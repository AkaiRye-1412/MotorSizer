use std::collections::BTreeMap;

use egui::{
    Color32, CornerRadius, FontFamily, FontId, Shadow, Stroke, TextStyle,
};

// ---------- Token ----------

pub const BG: Color32 = Color32::from_rgb(0xF4, 0xF7, 0xFB);
pub const SURFACE: Color32 = Color32::from_rgb(0xFF, 0xFF, 0xFF);
pub const SURFACE_ALT: Color32 = Color32::from_rgb(0xF8, 0xFA, 0xFC);
pub const TOPBAR: Color32 = Color32::from_rgb(0x0B, 0x12, 0x20);
pub const SIDEBAR: Color32 = Color32::from_rgb(0x11, 0x1C, 0x2E);
pub const BORDER: Color32 = Color32::from_rgb(0xE2, 0xE8, 0xF0);
pub const BORDER_STRONG: Color32 = Color32::from_rgb(0xCB, 0xD5, 0xE1);

pub const TEXT: Color32 = Color32::from_rgb(0x0F, 0x17, 0x2A);
pub const TEXT_2: Color32 = Color32::from_rgb(0x47, 0x55, 0x69);
pub const MUTED: Color32 = Color32::from_rgb(0x64, 0x74, 0x8B);

pub const ACCENT: Color32 = Color32::from_rgb(0x25, 0x63, 0xEB);
pub const ACCENT_CYAN: Color32 = Color32::from_rgb(0x06, 0xB6, 0xD4);
pub const ACCENT_SOFT: Color32 = Color32::from_rgb(0xDB, 0xEA, 0xFE);
pub const SUCCESS: Color32 = Color32::from_rgb(0x04, 0x78, 0x57);
pub const WARNING: Color32 = Color32::from_rgb(0xB4, 0x53, 0x09);
pub const ERROR: Color32 = Color32::from_rgb(0xB9, 0x1C, 0x1C);
pub const ERROR_SOFT: Color32 = Color32::from_rgb(0xFE, 0xE2, 0xE2);
pub const WARNING_SOFT: Color32 = Color32::from_rgb(0xFE, 0xF3, 0xC7);

pub const SIDE_TEXT: Color32 = Color32::from_rgb(0xCB, 0xD5, 0xE1);
pub const SIDE_MUTED: Color32 = Color32::from_rgb(0x78, 0x89, 0xA2);
pub const SIDE_LINE: Color32 = Color32::from_rgb(0x1E, 0x2D, 0x45);
pub const WHITE_10: Color32 = Color32::from_rgba_unmultiplied_const(255, 255, 255, 26);

pub const SHADOW: Color32 = Color32::from_black_alpha(20);

// ---------- 全局样式 ----------

pub fn configure(ctx: &egui::Context) {
    ctx.set_theme(egui::Theme::Light);
    ctx.all_styles_mut(|style| {
        style.text_styles = text_styles();
        style.spacing.item_spacing = egui::vec2(8.0, 8.0);
        style.spacing.button_padding = egui::vec2(12.0, 7.0);
        style.spacing.interact_size = egui::vec2(40.0, 34.0);
        style.animation_time = 0.12;
        // 非浮动滚动条会为自身预留宽度，避免覆盖右对齐的数值；
        // 深色手柄保证在白色卡片上清晰可见。
        style.spacing.scroll.floating = false;
        style.spacing.scroll.foreground_color = true;
        style.spacing.scroll.bar_width = 6.0;
        style.spacing.scroll.bar_outer_margin = 3.0;
        style.spacing.scroll.bar_inner_margin = 3.0;

        let visuals = &mut style.visuals;
        visuals.dark_mode = false;
        visuals.panel_fill = BG;
        visuals.window_fill = SURFACE;
        visuals.faint_bg_color = SURFACE_ALT;
        visuals.extreme_bg_color = SURFACE;
        visuals.window_corner_radius = CornerRadius::same(12);
        visuals.window_shadow = Shadow {
            offset: [0, 8],
            blur: 24,
            spread: 0,
            color: SHADOW,
        };
        visuals.selection.bg_fill = ACCENT;
        visuals.selection.stroke = Stroke::new(1.0, Color32::WHITE);
        visuals.hyperlink_color = ACCENT;

        widgets(&mut visuals.widgets.noninteractive, SURFACE, SURFACE, TEXT);
        // inactive 前景色同时决定滚动条手柄颜色；使用次级文本色而不是纯黑。
        widgets(&mut visuals.widgets.inactive, SURFACE, SURFACE_ALT, TEXT_2);
        widgets(&mut visuals.widgets.hovered, ACCENT_SOFT, ACCENT_SOFT, TEXT);
        widgets(&mut visuals.widgets.active, ACCENT, ACCENT, Color32::WHITE);
        widgets(&mut visuals.widgets.open, ACCENT_SOFT, ACCENT_SOFT, TEXT);
    });
}

fn widgets(
    visuals: &mut egui::style::WidgetVisuals,
    bg: Color32,
    weak_bg: Color32,
    fg: Color32,
) {
    visuals.bg_fill = bg;
    visuals.weak_bg_fill = weak_bg;
    visuals.bg_stroke = Stroke::new(1.0, BORDER);
    visuals.fg_stroke = Stroke::new(1.0, fg);
    visuals.corner_radius = CornerRadius::same(8);
    visuals.expansion = 0.0;
}

fn text_styles() -> BTreeMap<TextStyle, FontId> {
    BTreeMap::from([
        (TextStyle::Small, FontId::new(11.0, FontFamily::Proportional)),
        (TextStyle::Body, FontId::new(13.5, FontFamily::Proportional)),
        (TextStyle::Button, FontId::new(13.0, FontFamily::Proportional)),
        (TextStyle::Heading, FontId::new(20.0, FontFamily::Proportional)),
        (TextStyle::Monospace, FontId::new(13.0, FontFamily::Monospace)),
    ])
}
