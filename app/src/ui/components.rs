use egui::{
    Align, Button, Color32, CornerRadius, Frame, Layout, Margin, Response, RichText, Sense,
    Shadow, Stroke, Ui, Vec2,
};

use super::theme as t;

/// 固定高度卡片；用于并排的输入、结果和图表列。
pub fn card_sized(ui: &mut Ui, height: f32, add: impl FnOnce(&mut Ui)) {
    Frame::NONE
        .fill(t::SURFACE)
        .stroke(Stroke::new(1.0, t::BORDER))
        .corner_radius(CornerRadius::same(14))
        .inner_margin(Margin::same(16))
        .shadow(Shadow {
            offset: [0, 6],
            blur: 22,
            spread: 0,
            color: Color32::from_black_alpha(26),
        })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.set_min_height((height - 32.0).max(0.0));
            add(ui);
        });
}

/// 带独立滚动区的定高卡片。
///
/// 与 `card_sized` 的区别是给内部 ScrollArea 设了 `max_height`：
/// 滚动区默认会向上撑开到父容器剩余高度，多张卡片叠放时会把下面的卡片挤出可视区。
pub fn card_scroll(ui: &mut Ui, height: f32, id: &str, add: impl FnOnce(&mut Ui)) {
    Frame::NONE
        .fill(t::SURFACE)
        .stroke(Stroke::new(1.0, t::BORDER))
        .corner_radius(CornerRadius::same(14))
        .inner_margin(Margin::same(16))
        .shadow(Shadow {
            offset: [0, 6],
            blur: 22,
            spread: 0,
            color: Color32::from_black_alpha(26),
        })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            let inner_height = (height - 32.0).max(60.0);
            egui::ScrollArea::vertical()
                .id_salt(id)
                .max_height(inner_height)
                .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible)
                .auto_shrink([false, false])
                .show(ui, add);
        });
}

pub fn section_title(ui: &mut Ui, title: &str, hint: &str) {
    ui.horizontal(|ui| {
        // 标题前加一小段强调色竖条，替代了单纯的文字标题，视觉上更像卡片头。
        let (bar, _) = ui.allocate_exact_size(Vec2::new(3.0, 14.0), Sense::hover());
        ui.painter().rect_filled(bar, CornerRadius::same(2), t::ACCENT);
        ui.add_space(3.0);
        ui.label(RichText::new(title).size(15.0).strong().color(t::TEXT));
        if !hint.is_empty() {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(RichText::new(hint).size(11.0).color(t::MUTED));
            });
        }
    });
}

pub fn group_label(ui: &mut Ui, text: &str) {
    ui.add_space(6.0);
    ui.label(RichText::new(text).size(11.0).strong().color(t::TEXT_2));
    ui.add_space(2.0);
}

pub fn hairline(ui: &mut Ui, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(
        Vec2::new(ui.available_width(), 1.0),
        Sense::hover(),
    );
    ui.painter().rect_filled(rect, 0.0, color);
}

pub fn kpi(ui: &mut Ui, label: &str, value: &str, unit: &str, accent: Color32) {
    let response = Frame::NONE
        // 底色是白底极淡地混入该项的强调色，四项并排时各自有辨识度又不刺眼。
        .fill(t::tint(t::SURFACE, accent, 0.05))
        .stroke(Stroke::new(1.0, t::tint(t::BORDER, accent, 0.35)))
        .corner_radius(CornerRadius::same(12))
        .inner_margin(Margin::symmetric(14, 11))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(label).size(11.0).color(t::MUTED));
            ui.add_space(2.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new(value).size(24.0).strong().color(t::TEXT));
                ui.label(RichText::new(unit).size(10.0).color(accent));
            });
        })
        .response;
    // 左侧 3px 强调条，让指标卡有“标签页”一样的识别度。
    let rect = response.rect;
    ui.painter().rect_filled(
        egui::Rect::from_center_size(egui::pos2(rect.left() + 3.0, rect.center().y), Vec2::new(3.0, 22.0)),
        CornerRadius::same(2),
        accent,
    );
}

pub fn badge(ui: &mut Ui, text: &str, fg: Color32, bg: Color32) {
    Frame::NONE
        .fill(bg)
        .corner_radius(CornerRadius::same(6))
        .inner_margin(Margin::symmetric(7, 3))
        .show(ui, |ui| {
            ui.set_height(20.0);
            ui.label(RichText::new(text).size(10.0).strong().color(fg));
        });
}

pub fn top_tab(ui: &mut Ui, text: &str, active: bool) -> Response {
    let fill = if active { t::ACCENT } else { Color32::TRANSPARENT };
    let color = if active { Color32::WHITE } else { t::SIDE_TEXT };
    ui.add(
        Button::new(RichText::new(text).size(13.0).strong().color(color))
            .fill(fill)
            .stroke(Stroke::NONE)
            .corner_radius(8)
            .min_size(Vec2::new(96.0, 32.0)),
    )
}

/// 深色顶栏上的次要操作按钮（如导出报告）。
pub fn top_action(ui: &mut Ui, text: &str) -> Response {
    ui.add(
        Button::new(RichText::new(text).size(12.0).color(t::SIDE_TEXT))
            .fill(Color32::TRANSPARENT)
            .stroke(Stroke::new(1.0, t::WHITE_10))
            .corner_radius(8)
            .min_size(Vec2::new(88.0, 30.0)),
    )
}

pub fn nav_item(ui: &mut Ui, text: &str, active: bool, enabled: bool) -> Response {
    let fill = if active { t::ACCENT } else { Color32::TRANSPARENT };
    let color = if active {
        Color32::WHITE
    } else if enabled {
        t::SIDE_TEXT
    } else {
        t::SIDE_MUTED
    };
    ui.add_enabled(
        enabled,
        Button::new(RichText::new(text).size(13.0).color(color))
            .fill(fill)
            .stroke(Stroke::NONE)
            .corner_radius(8)
            .min_size(Vec2::new(ui.available_width(), 42.0)),
    )
}

pub fn num_field(
    ui: &mut Ui,
    label: &str,
    unit: &str,
    value: &mut f64,
    enabled: bool,
    speed: f64,
    decimals: usize,
) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.label(RichText::new(label).size(11.0).color(t::TEXT_2));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let suffix = if unit.is_empty() {
                String::new()
            } else {
                format!(" {unit}")
            };
            let response = ui.add_enabled(
                enabled,
                egui::DragValue::new(value)
                    .speed(speed)
                    .fixed_decimals(decimals)
                    .suffix(suffix),
            );
            changed |= response.changed();
        });
    });
    changed
}

/// 可编辑的“自动计算值”字段。
///
/// 自定义模式下行程、时间、速度、加速度只需给出其中两项，其余由求解器补算。
/// 未填写（≤0）而存在计算值时，输入框直接显示计算值（蓝底蓝字），并且仍可拖动/点选编辑；
/// 用户一旦改动就写回真实输入，改用普通样式，表示这一项成为用户指定的约束。
///
/// `scale` 用于界面单位与内部单位的换算：显示值 = 内部值 × scale，编辑后按 scale 写回。
pub fn auto_value_field(
    ui: &mut Ui,
    label: &str,
    unit: &str,
    value: &mut f64,
    auto: Option<f64>,
    scale: f64,
    speed: f64,
    decimals: usize,
) -> bool {
    let auto = auto.filter(|v| v.is_finite() && *v > 0.0);
    let auto_active = *value <= 0.0 && auto.is_some();
    let scale = if scale.is_finite() && scale > 0.0 { scale } else { 1.0 };
    let mut proxy = (if auto_active { auto.unwrap() } else { *value }) * scale;
    let mut changed = false;

    ui.horizontal(|ui| {
        ui.label(RichText::new(label).size(11.0).color(t::TEXT_2));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let suffix = if unit.is_empty() {
                String::new()
            } else {
                format!(" {unit}")
            };
            let widget = egui::DragValue::new(&mut proxy)
                .speed(speed)
                .fixed_decimals(decimals)
                .suffix(suffix);
            let response = if auto_active {
                let response = ui
                    .scope(|ui| {
                        {
                            let visuals = &mut ui.style_mut().visuals;
                            // DragValue 未编辑时是 Button，编辑时是 TextEdit；
                            // override_text_color 对两者都生效，保证蓝字一致。
                            visuals.override_text_color = Some(t::ACCENT);
                            for state in [
                                &mut visuals.widgets.inactive,
                                &mut visuals.widgets.hovered,
                                &mut visuals.widgets.active,
                            ] {
                                state.weak_bg_fill = t::ACCENT_SOFT;
                                state.bg_fill = t::ACCENT_SOFT;
                                state.bg_stroke = Stroke::new(1.0, t::BORDER_STRONG);
                            }
                        }
                        ui.add(widget)
                    })
                    .inner;
                response.on_hover_text("由其余条件自动计算；拖动或输入后改为手动值")
            } else {
                ui.add(widget)
            };

            if response.changed() {
                *value = proxy / scale;
                changed = true;
            }
        });
    });
    changed
}

/// 只读的“计算值”字段。
///
/// 点对点模式下速度/加速度由行程、时间和加速比反算，用户不需要输入；
/// 这里用蓝底蓝字与白色输入框区分，悬停时给出说明。
/// `scale` 把内部单位换算成界面单位（与 `auto_value_field` 一致）。
pub fn computed_field(
    ui: &mut Ui,
    label: &str,
    unit: &str,
    value: Option<f64>,
    scale: f64,
    decimals: usize,
) {
    let scale = if scale.is_finite() && scale > 0.0 { scale } else { 1.0 };
    ui.horizontal(|ui| {
        ui.label(RichText::new(label).size(11.0).color(t::TEXT_2));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let suffix = if unit.is_empty() {
                String::new()
            } else {
                format!(" {unit}")
            };
            let (text, color) = match value {
                Some(number) => (
                    format!("{:.decimals$}{suffix}", number * scale),
                    t::ACCENT,
                ),
                None => ("—".to_owned(), t::MUTED),
            };
            let response = Frame::new()
                .fill(t::ACCENT_SOFT)
                .stroke(Stroke::new(1.0, t::BORDER_STRONG))
                .corner_radius(CornerRadius::same(6))
                .inner_margin(Margin::symmetric(8, 4))
                .show(ui, |ui| {
                    ui.set_min_width(92.0);
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.label(RichText::new(text).size(12.0).color(color).strong());
                    });
                })
                .response;
            response.on_hover_text("由行程、时间与加速比自动计算，无需输入");
        });
    });
}

pub fn text_field(ui: &mut Ui, label: &str, value: &mut String) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.label(RichText::new(label).size(11.0).color(t::TEXT_2));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let response = ui.add_sized([150.0, 24.0], egui::TextEdit::singleline(value));
            changed |= response.changed();
        });
    });
    changed
}

pub fn check(ui: &mut Ui, label: &str, value: &mut bool) -> bool {
    let response = ui.checkbox(value, RichText::new(label).size(11.5).color(t::TEXT_2));
    response.changed()
}

pub fn action_button(ui: &mut Ui, text: &str, primary: bool, enabled: bool) -> bool {
    let fill = if primary { t::ACCENT } else { t::SURFACE_ALT };
    let color = if primary { Color32::WHITE } else { t::TEXT_2 };
    ui.add_enabled(
        enabled,
        Button::new(RichText::new(text).size(11.5).strong().color(color))
            .fill(fill)
            .stroke(Stroke::new(1.0, if primary { t::ACCENT } else { t::BORDER }))
            .corner_radius(7)
            .min_size(Vec2::new(ui.available_width(), 28.0)),
    )
    .clicked()
}

pub fn choice<T: Copy + PartialEq>(ui: &mut Ui, value: &mut T, item: T, label: &str) -> bool {
    let active = *value == item;
    let fill = if active { t::ACCENT } else { t::SURFACE_ALT };
    let color = if active { Color32::WHITE } else { t::TEXT_2 };
    let response = ui.add(
        Button::new(RichText::new(label).size(11.5).strong().color(color))
            .fill(fill)
            .stroke(Stroke::new(1.0, if active { t::ACCENT } else { t::BORDER }))
            .corner_radius(7)
            .min_size(Vec2::new(0.0, 26.0)),
    );
    if response.clicked() && !active {
        *value = item;
        true
    } else {
        false
    }
}

pub fn note_line(ui: &mut Ui, text: &str, color: Color32, bg: Color32) {
    Frame::NONE
        .fill(bg)
        .stroke(Stroke::new(1.0, t::BORDER))
        .corner_radius(CornerRadius::same(7))
        .inner_margin(Margin::symmetric(8, 5))
        .show(ui, |ui| {
            ui.label(RichText::new(text).size(10.5).color(color));
        });
}

pub fn spec_row(ui: &mut Ui, label: &str, value: &str) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(label).size(12.0).color(t::MUTED));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            // 右侧留白，避免数值被滚动条覆盖或贴边裁切。
            ui.add_space(6.0);
            ui.label(RichText::new(value).size(12.0).strong().color(t::TEXT));
        });
    });
    ui.add_space(2.0);
}

pub fn empty_state(ui: &mut Ui, title: &str, note: &str) {
    Frame::NONE
        .fill(t::SURFACE_ALT)
        .corner_radius(CornerRadius::same(10))
        .inner_margin(Margin::symmetric(16, 18))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.set_height(60.0);
            ui.vertical_centered(|ui| {
                ui.label(RichText::new("◇").size(24.0).color(t::BORDER_STRONG));
                ui.add_space(2.0);
                ui.label(RichText::new(title).size(12.5).color(t::TEXT_2));
                ui.label(RichText::new(note).size(10.5).color(t::MUTED));
            });
        });
}
