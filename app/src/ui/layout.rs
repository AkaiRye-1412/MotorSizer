use egui::{
    pos2, Align, Color32, CornerRadius, Frame, Layout, Margin, Panel, Rect, RichText, Sense,
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
        .show(ui, |ui| {
            // 内容区用极淡的竖向渐变，白色卡片浮在上面时更有层次。
            let page = ui.clip_rect();
            t::vgrad(ui.painter(), page, t::BG_TOP, t::BG_BOTTOM);
            content(state, ui);
        });

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
            // 顶栏用竖向渐变，避免整块死黑；底部再压一条细线收口。
            let bar = ui.clip_rect();
            t::vgrad(ui.painter(), bar, t::TOPBAR_TOP, t::TOPBAR_BOTTOM);
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
            ui.painter().line_segment(
                [bar.left_bottom(), bar.right_bottom()],
                Stroke::new(1.0, t::SIDE_LINE),
            );
        });
}

fn brand(ui: &mut egui::Ui) {
    // Akribis logo 是深蓝字 + 青色三角，直接放在深色顶栏上文字会看不清，
    // 因此外面套一块白色圆角底板，logo 本身保持品牌原色不做改色。
    let texture = logo_texture(ui.ctx());
    let size = texture.size();
    let logo = Vec2::new(size[0] as f32, size[1] as f32);
    // 按固定高度缩放，宽度由原图比例得出，避免底板被拉伸。
    let logo_size = logo * (26.0 / logo.y.max(1.0));
    let margin = Vec2::new(8.0, 6.0);
    let (rect, _) = ui.allocate_exact_size(logo_size + margin * 2.0, Sense::hover());
    let painter = ui.painter();
    let radius = CornerRadius::same(10);
    // 底板下沿垫一层浅投影，让 logo 从深色顶栏里浮起来而不是贴在上面。
    painter.rect_filled(
        rect.translate(Vec2::new(0.0, 2.0)).expand(1.0),
        radius,
        Color32::from_black_alpha(90),
    );
    painter.rect_filled(rect, radius, Color32::WHITE);
    painter.rect_stroke(
        rect,
        radius,
        Stroke::new(1.0, Color32::from_rgba_unmultiplied(0xFF, 0xFF, 0xFF, 30)),
        egui::StrokeKind::Inside,
    );
    painter.image(
        texture.id(),
        Rect::from_min_size(rect.min + margin, logo_size),
        Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
        Color32::WHITE,
    );
}

/// 内嵌的 Akribis logo；首次绘制时解码成 egui 纹理，之后复用缓存。
fn logo_texture(ctx: &egui::Context) -> egui::TextureHandle {
    let cache_id = egui::Id::unique("akribis_logo_texture");
    if let Some(handle) = ctx.data(|data| data.get_temp::<egui::TextureHandle>(cache_id)) {
        return handle;
    }
    // PNG 已经裁好并去掉背景，解码失败说明打包资源损坏，直接让程序暴露问题。
    let decoded = image::load_from_memory(include_bytes!("../../assets/akribis-logo.png"))
        .expect("内置 Akribis logo 无法解码")
        .to_rgba8();
    let size = [decoded.width() as usize, decoded.height() as usize];
    let color_image = egui::ColorImage::from_rgba_unmultiplied(size, decoded.as_raw());
    let handle = ctx.load_texture("akribis_logo", color_image, egui::TextureOptions::LINEAR);
    ctx.data_mut(|data| data.insert_temp(cache_id, handle.clone()));
    handle
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
            // 侧栏同样用渐变，与顶栏的明暗过渡衔接。
            let nav = ui.clip_rect();
            t::vgrad(ui.painter(), nav, t::SIDEBAR_TOP, t::SIDEBAR_BOTTOM);
            ui.painter()
                .line_segment([nav.right_top(), nav.right_bottom()], Stroke::new(1.0, t::SIDE_LINE));
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
            // 状态栏与内容区之间用一条细线分隔，避免白色连成一片。
            let bar = ui.clip_rect();
            ui.painter().line_segment(
                [bar.left_top(), bar.right_top()],
                Stroke::new(1.0, t::BORDER),
            );
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
