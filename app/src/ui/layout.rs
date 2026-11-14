use egui::{
    pos2, Align, Color32, CornerRadius, Frame, Layout, Margin, Panel, RichText, Sense, Shape,
    Stroke, Vec2,
};

use crate::app::{report, AppState, Mode, Tab};

use super::components as c;
use super::file_dialog;
use super::panels;
use super::theme as t;

pub fn show(state: &mut AppState, ui: &mut egui::Ui) {
    top_bar(state, ui);
    side_nav(state, ui);
    status_bar(state, ui);

    egui::CentralPanel::default()
        .frame(
            Frame::NONE
                .fill(t::BG)
                .inner_margin(Margin::symmetric(16, 14)),
        )
        .show(ui, |ui| content(state, ui));

    let ctx = ui.ctx().clone();
    super::import_dialog::show(&ctx, state);
}

fn top_bar(state: &mut AppState, ui: &mut egui::Ui) {
    Panel::top("app_top")
        .exact_size(64.0)
        .frame(
            Frame::NONE
                .fill(t::TOPBAR)
                .inner_margin(Margin::symmetric(20, 0)),
        )
        .show(ui, |ui| {
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                brand(ui);
                ui.add_space(26.0);

                for tab in [Tab::Linear, Tab::Rotary] {
                    if c::top_tab(ui, tab.label(), state.tab == tab).clicked() {
                        state.set_tab(tab);
                    }
                    ui.add_space(4.0);
                }

                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    c::badge(ui, "LOCAL", Color32::WHITE, t::WHITE_10);
                    ui.add_space(8.0);
                    ui.label(
                        RichText::new("离线选型 · 数据本地")
                            .size(11.0)
                            .color(t::SIDE_MUTED),
                    );
                    ui.add_space(12.0);
                    let export = c::top_action(ui, "导出报告")
                        .on_hover_text("选择保存位置并导出 Markdown 报告");
                    if export.clicked() {
                        // 原生“另存为”对话框会阻塞当前线程，关闭后再更新状态栏。
                        match file_dialog::save_markdown(
                            &report::suggested_file_name(),
                            Some(&report::default_report_dir()),
                        ) {
                            Some(path) => {
                                state.status = match report::export_to_path(state, &path) {
                                    Ok(saved) => format!("报告已保存：{}", saved.display()),
                                    Err(error) => format!("报告导出失败：{error}"),
                                };
                            }
                            None => state.status = "已取消导出报告".to_owned(),
                        }
                    }
                });
            });
        });
}

fn brand(ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(Vec2::splat(32.0), Sense::hover());
        let painter = ui.painter_at(rect);
        let center = rect.center();
        painter.rect_filled(
            rect.shrink(1.0),
            CornerRadius::same(9),
            Color32::from_rgb(0x16, 0x27, 0x42),
        );
        painter.add(Shape::convex_polygon(
            vec![
                pos2(center.x, center.y - 9.0),
                pos2(center.x + 9.0, center.y),
                pos2(center.x, center.y + 9.0),
                pos2(center.x - 9.0, center.y),
            ],
            t::ACCENT,
            Stroke::NONE,
        ));
        painter.add(Shape::convex_polygon(
            vec![
                pos2(center.x, center.y - 3.5),
                pos2(center.x + 3.5, center.y),
                pos2(center.x, center.y + 3.5),
                pos2(center.x - 3.5, center.y),
            ],
            t::TOPBAR,
            Stroke::NONE,
        ));

        ui.add_space(8.0);
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            ui.label(
                RichText::new("MotionSizer")
                    .size(18.0)
                    .strong()
                    .color(Color32::WHITE),
            );
            ui.label(RichText::new("STUDIO").size(9.0).color(t::SIDE_MUTED));
        });
    });
}

fn side_nav(state: &mut AppState, ui: &mut egui::Ui) {
    Panel::left("app_nav")
        .exact_size(184.0)
        .frame(
            Frame::NONE
                .fill(t::SIDEBAR)
                .inner_margin(Margin::symmetric(12, 16)),
        )
        .show(ui, |ui| {
            ui.label(RichText::new("工作流").size(10.0).color(t::SIDE_MUTED));
            ui.add_space(8.0);

            for mode in [Mode::Independent, Mode::Combined, Mode::Solution] {
                let response = c::nav_item(ui, mode.label(), state.mode == mode, true);
                if response.clicked() {
                    state.set_mode(mode);
                }
                ui.add_space(4.0);
            }

            ui.add_space(10.0);
            c::hairline(ui, t::SIDE_LINE);
            ui.add_space(10.0);
            ui.label(RichText::new("数据").size(10.0).color(t::SIDE_MUTED));
            ui.add_space(6.0);
            let import = c::nav_item(ui, "导入电机参数", false, true)
                .on_hover_text("从 PDF 或图片识别电机参数，预览后写入数据库");
            if import.clicked() {
                // 原生打开对话框会阻塞当前线程；返回后再更新界面状态。
                match file_dialog::open_document() {
                    Some(path) => state.select_import_document(path),
                    None => state.status = "已取消选择导入文件".to_owned(),
                }
            }

            ui.with_layout(Layout::bottom_up(Align::LEFT), |ui| {
                ui.label(RichText::new("v2.1 · Studio").size(10.0).color(t::SIDE_MUTED));
                ui.add_space(8.0);
                c::hairline(ui, t::SIDE_LINE);
                ui.add_space(8.0);
                let shape = if state.inputs.scurve {
                    "S-曲线"
                } else {
                    state.shape.label()
                };
                c::badge(ui, shape, t::ACCENT_SOFT, t::ACCENT);
            });
        });
}

fn status_bar(state: &mut AppState, ui: &mut egui::Ui) {
    Panel::bottom("app_status")
        .exact_size(30.0)
        .frame(
            Frame::NONE
                .fill(t::SURFACE)
                .inner_margin(Margin::symmetric(16, 0)),
        )
        .show(ui, |ui| {
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                let color = if state.computed.errors.is_empty() {
                    t::SUCCESS
                } else {
                    t::ERROR
                };
                ui.label(RichText::new("●").size(9.0).color(color));
                ui.add_space(6.0);
                ui.label(RichText::new(&state.status).size(10.5).color(t::MUTED));
                ui.add_space(12.0);
                ui.label(RichText::new("|").size(10.5).color(t::BORDER_STRONG));
                ui.add_space(12.0);
                ui.label(RichText::new(&state.db_status).size(10.5).color(t::MUTED));

                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.label(
                        RichText::new("MotionSizer Studio · Rust + egui")
                            .size(10.5)
                            .color(t::MUTED),
                    );
                });
            });
        });
}

fn content(state: &mut AppState, ui: &mut egui::Ui) {
    let width = ui.available_width();
    let height = ui.available_height();
    let gap = 12.0;
    // 三栏总宽必须小于内容区；窄窗口时按比例收缩，避免右栏被挤出屏幕。
    let left_width = (width * 0.30).clamp(280.0, 340.0);
    let right_width = (width * 0.27).clamp(260.0, 320.0);
    let center_width = (width - left_width - right_width - 2.0 * gap).max(280.0);

    ui.horizontal_top(|ui| {
        ui.allocate_ui_with_layout(
            Vec2::new(left_width, height),
            Layout::top_down(Align::LEFT),
            |ui| panels::input_panel(ui, state, height),
        );
        ui.add_space(gap);
        ui.allocate_ui_with_layout(
            Vec2::new(center_width, height),
            Layout::top_down(Align::LEFT),
            |ui| panels::result_panel(ui, state),
        );
        ui.add_space(gap);
        ui.allocate_ui_with_layout(
            Vec2::new(right_width, height),
            Layout::top_down(Align::LEFT),
            |ui| panels::detail_panel(ui, state, height),
        );
    });
}

/// 数据库无法加载时的独立错误页：窗口仍可关闭，并给出可操作的恢复步骤。
pub fn error_page(ui: &mut egui::Ui, error: &str) {
    egui::CentralPanel::default()
        .frame(
            Frame::NONE
                .fill(t::BG)
                .inner_margin(Margin::symmetric(48, 40)),
        )
        .show(ui, |ui| {
            ui.set_max_width(820.0);
            ui.label(RichText::new("MotionSizer Studio").size(11.0).color(t::MUTED));
            ui.add_space(6.0);
            ui.label(RichText::new("数据加载失败").size(24.0).strong().color(t::TEXT));
            ui.add_space(6.0);
            ui.label(
                RichText::new("程序没有启动选型界面，以免在缺少数据时给出错误结果。")
                    .size(12.0)
                    .color(t::MUTED),
            );
            ui.add_space(18.0);
            Frame::NONE
                .fill(t::SURFACE)
                .stroke(Stroke::new(1.0, t::BORDER))
                .corner_radius(CornerRadius::same(12))
                .inner_margin(Margin::same(18))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(RichText::new(error).size(12.0).color(t::ERROR));
                    ui.add_space(10.0);
                    c::hairline(ui, t::BORDER);
                    ui.add_space(8.0);
                    ui.label(RichText::new("恢复方式").size(12.0).strong().color(t::TEXT_2));
                    ui.add_space(4.0);
                    for line in [
                        "1. 确认数据目录可写（默认位置：程序的 data/motors.db）。",
                        "2. 删除损坏的 motors.db 后重新启动，程序会自动从内置资料重建。",
                        "3. 也可以用环境变量 MOTORSIZER_DB 指向其他数据库路径。",
                    ] {
                        ui.label(RichText::new(line).size(11.0).color(t::MUTED));
                    }
                });
        });
}
