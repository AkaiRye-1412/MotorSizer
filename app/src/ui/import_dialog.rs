//! 导入电机参数对话框：选择文件 → 本地 OCR 识别 → 可编辑预览 → 写入 SQLite。
//!
//! 对话框使用模态层，识别期间由后台线程工作，界面保持可重绘；
//! 关闭或取消不会留下半写数据，只有“导入”按钮会执行数据库写入。

use egui::{
    Align, Button, Color32, CornerRadius, Frame, Label, Layout, Margin, RichText, ScrollArea,
    Shadow, Stroke, TextEdit, Vec2,
};

use crate::app::state::{AppState, ImportPhase};
use crate::data::motor::{classify, MotorKind};
use crate::import::excel;
use crate::import::parse::{self, Field};
use crate::import::workflow;

use super::components as c;
use super::file_dialog;
use super::theme as t;

enum Action {
    None,
    Start,
    Cancel,
    Close,
    /// 把识别预览中勾选的行导出为 Excel 草稿。
    ExportExcel,
    /// 把校验通过的 Excel 行写入数据库。
    CommitExcel,
    /// 重新选择文件（Excel 校验失败后修正再导入）。
    Reselect,
    /// 导出空白模板，人工填写后再导入。
    ExportTemplate,
}

pub fn show(ctx: &egui::Context, state: &mut AppState) {
    if matches!(state.import.phase, ImportPhase::Closed) {
        return;
    }
    let modal = egui::Modal::new(egui::Id::unique("import_motor_params"))
        .frame(
            Frame::NONE
                .fill(t::SURFACE)
                .stroke(Stroke::new(1.0, t::BORDER))
                .corner_radius(CornerRadius::same(14))
                .shadow(Shadow {
                    offset: [0, 12],
                    blur: 36,
                    spread: 0,
                    color: t::SHADOW,
                })
                .inner_margin(Margin::same(20)),
        )
        .show(ctx, |ui| {
            ui.set_width(880.0);
            match &mut state.import.phase {
                ImportPhase::Closed => Action::None,
                ImportPhase::Ready {
                    path,
                    start_page,
                    end_page,
                    is_pdf,
                } => ready_page(ui, path, *is_pdf, start_page, end_page),
                ImportPhase::Running(run) => {
                    running_page(ui, &run.stage, run.done, run.total)
                }
                ImportPhase::Preview(preview) => {
                    preview_page(ui, preview, &mut state.import.selected)
                }
                ImportPhase::ExcelPreview(preview) => excel_preview_page(ui, preview),
                ImportPhase::Failed(error) => failed_page(ui, error),
            }
        });

    let close = modal.should_close();
    match modal.inner {
        Action::Start => state.begin_import(ctx),
        Action::Cancel => state.cancel_import(),
        Action::Close => state.close_import(),
        Action::ExportExcel => {
            let name = format!("电机参数-{}.xlsx", state.draft_source_name());
            if let Some(path) = file_dialog::save_excel(&name, None) {
                if let Err(error) = state.export_draft_to(&path) {
                    state.status = format!("导出失败：{error}");
                }
            }
        }
        Action::CommitExcel => {
            if let Err(error) = state.commit_excel_import() {
                state.status = format!("导入失败：{error}");
            }
        }
        Action::Reselect => {
            if let Some(path) = file_dialog::open_document() {
                state.select_import_document(path);
            }
        }
        Action::ExportTemplate => {
            if let Some(path) = file_dialog::save_excel("电机参数-空白模板.xlsx", None) {
                if let Err(error) = state.export_blank_template_to(&path) {
                    state.status = format!("导出失败：{error}");
                }
            }
        }
        Action::None => {}
    }
    if close {
        if matches!(state.import.phase, ImportPhase::Running(_)) {
            state.cancel_import();
        } else {
            state.close_import();
        }
    }
}

fn header(ui: &mut egui::Ui, title: &str, subtitle: &str) -> bool {
    let mut close = false;
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.label(RichText::new(title).size(18.0).strong().color(t::TEXT));
            ui.label(RichText::new(subtitle).size(11.0).color(t::MUTED));
        });
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            close = secondary(ui, "关闭");
        });
    });
    ui.add_space(10.0);
    c::hairline(ui, t::BORDER);
    ui.add_space(12.0);
    close
}

fn secondary(ui: &mut egui::Ui, text: &str) -> bool {
    ui.add(
        Button::new(RichText::new(text).size(11.5).color(t::TEXT_2))
            .fill(t::SURFACE_ALT)
            .stroke(Stroke::new(1.0, t::BORDER))
            .corner_radius(7)
            .min_size(Vec2::new(84.0, 30.0)),
    )
    .clicked()
}

fn primary(ui: &mut egui::Ui, text: &str, enabled: bool) -> bool {
    ui.add_enabled(
        enabled,
        Button::new(RichText::new(text).size(11.5).strong().color(Color32::WHITE))
            .fill(t::ACCENT)
            .stroke(Stroke::new(1.0, t::ACCENT))
            .corner_radius(7)
            .min_size(Vec2::new(140.0, 30.0)),
    )
    .clicked()
}

fn ready_page(
    ui: &mut egui::Ui,
    path: &std::path::Path,
    is_pdf: bool,
    start_page: &mut u32,
    end_page: &mut u32,
) -> Action {
    let mut action = Action::None;
    if header(ui, "导入电机参数", "从 PDF / 图片识别参数，先预览，确认后写入数据库") {
        action = Action::Close;
    }

    Frame::NONE
        .fill(t::SURFACE_ALT)
        .stroke(Stroke::new(1.0, t::BORDER))
        .corner_radius(CornerRadius::same(9))
        .inner_margin(Margin::same(12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(
                RichText::new(
                    path.file_name()
                        .map(|name| name.to_string_lossy().into_owned())
                        .unwrap_or_else(|| path.display().to_string()),
                )
                .size(13.0)
                .strong()
                .color(t::TEXT),
            );
            ui.add_space(2.0);
            ui.add(
                Label::new(
                    RichText::new(path.display().to_string())
                        .size(10.5)
                        .color(t::MUTED),
                )
                .wrap(),
            );
            ui.add_space(8.0);
            if is_pdf {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("识别页码").size(11.5).color(t::TEXT_2));
                    ui.add(
                        egui::DragValue::new(start_page)
                            .range(1..=1_000_000)
                            .speed(1.0)
                            .prefix("第 "),
                    );
                    ui.label(RichText::new("页 到").size(11.5).color(t::TEXT_2));
                    ui.add(
                        egui::DragValue::new(end_page)
                            .range(0..=1_000_000)
                            .speed(1.0)
                            .suffix(" 页"),
                    );
                    ui.label(RichText::new("（0 = 到最后一页）").size(10.5).color(t::MUTED));
                });
                ui.add_space(4.0);
                c::note_line(
                    ui,
                    "建议先识别包含参数表的 1–2 页确认效果；整本样册页数较多，识别会持续较久。",
                    t::WARNING,
                    t::WARNING_SOFT,
                );
            } else {
                ui.label(
                    RichText::new("图片将整张识别；请确保参数表区域清晰、方向正确。")
                        .size(10.5)
                        .color(t::MUTED),
                );
            }
        });

    ui.add_space(10.0);
    ui.label(
        RichText::new("识别使用 Windows 本地 OCR：文件与识别结果不会上传网络。")
            .size(10.5)
            .color(t::MUTED),
    );

    ui.add_space(14.0);
    ui.horizontal(|ui| {
        if secondary(ui, "取消") {
            action = Action::Close;
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if primary(ui, "开始识别", true) {
                action = Action::Start;
            }
        });
    });
    action
}

fn running_page(ui: &mut egui::Ui, stage: &str, done: u32, total: u32) -> Action {
    let mut action = Action::None;
    // 识别中点击右上角“关闭”等同取消：先让工作线程在检查点退出，再关闭对话框。
    if header(ui, "正在识别电机参数", "本地 OCR 处理中，请稍候") {
        action = Action::Cancel;
    }

    ui.horizontal(|ui| {
        ui.spinner();
        ui.add_space(4.0);
        ui.label(RichText::new(stage).size(13.0).strong().color(t::TEXT));
    });
    ui.add_space(10.0);
    let fraction = if total > 0 {
        done as f32 / total as f32
    } else {
        0.0
    };
    let text = if total > 0 {
        format!("{done} / {total}")
    } else {
        "准备中…".to_owned()
    };
    ui.add(
        egui::ProgressBar::new(fraction)
            .desired_width(ui.available_width())
            .text(text),
    );
    ui.add_space(8.0);
    ui.label(
        RichText::new("识别过程可能需要数秒到数分钟；取消会停止当前任务并清理临时文件。")
            .size(10.5)
            .color(t::MUTED),
    );
    ui.add_space(12.0);
    ui.horizontal(|ui| {
        if secondary(ui, "取消识别") {
            action = Action::Cancel;
        }
    });
    action
}

fn failed_page(ui: &mut egui::Ui, error: &str) -> Action {
    let mut action = Action::None;
    if header(ui, "识别未完成", "可以关闭对话框后重新选择文件或调整页码") {
        action = Action::Close;
    }
    c::note_line(ui, error, t::ERROR, t::ERROR_SOFT);
    ui.add_space(8.0);
    ui.label(
        RichText::new(
            "常见原因：页面中没有可识别的参数表、PDF 为扫描图片且分辨率过低、\
             或系统缺少中文 OCR 语言包。",
        )
        .size(10.5)
        .color(t::MUTED),
    );
    ui.add_space(12.0);
    ui.horizontal(|ui| {
        if secondary(ui, "关闭") {
            action = Action::Close;
        }
        if secondary(ui, "导出空白模板") {
            action = Action::ExportTemplate;
        }
    });
    action
}

fn preview_page(ui: &mut egui::Ui, preview: &mut workflow::Preview, selected: &mut usize) -> Action {
    let mut action = Action::None;
    let engine = if preview.engine.is_empty() {
        "本地 OCR"
    } else {
        preview.engine.as_str()
    };
    let subtitle = format!(
        "来源 {} · 共识别 {} 页 · OCR {engine}",
        preview
            .source
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| preview.source.display().to_string()),
        preview.pages
    );
    if header(ui, "导入预览", &subtitle) {
        action = Action::Close;
    }

    if preview.candidates.is_empty() {
        c::empty_state(
            ui,
            "没有候选型号",
            "没有从参数表中恢复出型号；请尝试其他页面或更清晰的扫描件。",
        );
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            if secondary(ui, "关闭") {
                action = Action::Close;
            }
            if secondary(ui, "导出空白模板") {
                action = Action::ExportTemplate;
            }
        });
        return action;
    }

    let total = preview.candidates.len();
    let mut current = (*selected).min(total - 1);
    let mut included = preview
        .candidates
        .iter()
        .filter(|candidate| candidate.include)
        .count();
    let overwrites = preview.overwrite_count();

    ui.horizontal(|ui| {
        ui.label(
            RichText::new(format!(
                "候选 {total} 个 · 勾选 {included} 个 · 同名覆盖 {overwrites} 个"
            ))
            .size(11.5)
            .color(t::TEXT_2),
        );
        ui.add_space(12.0);
        // 三态“全选”：部分勾选时显示 indeterminate，点击后整体
        // 切换为全选/全不选，便于一次导入整页型号。
        let all = included == total;
        let mut checked = all;
        let response = ui.add(
            egui::Checkbox::new(
                &mut checked,
                RichText::new("全选").size(11.5).color(t::TEXT_2),
            )
            .indeterminate(included > 0 && !all),
        );
        if response.clicked() {
            for candidate in &mut preview.candidates {
                candidate.include = checked;
            }
        }
    });
    // 同一帧内刷新数量，避免底部“导入 N 个型号”滞后一帧。
    included = preview
        .candidates
        .iter()
        .filter(|candidate| candidate.include)
        .count();
    ui.add_space(6.0);

    ui.horizontal_top(|ui| {
        // 左侧候选列表。
        ui.allocate_ui_with_layout(
            Vec2::new(330.0, 360.0),
            Layout::top_down(Align::LEFT),
            |ui| {
                ScrollArea::vertical()
                    .id_salt("import_candidates")
                    .max_height(360.0)
                    .show(ui, |ui| {
                        for (index, candidate) in preview.candidates.iter_mut().enumerate() {
                            ui.horizontal(|ui| {
                                let mut include = candidate.include;
                                if ui.checkbox(&mut include, "").changed() {
                                    candidate.include = include;
                                }
                                // 选中行由 selectable_label 填充强调色背景，
                                // 文字必须用白色，否则强调色文字会“消失”在背景上。
                                let label = RichText::new(&candidate.motor.name)
                                    .size(11.5)
                                    .color(if current == index { Color32::WHITE } else { t::TEXT });
                                if ui.selectable_label(current == index, label).clicked() {
                                    current = index;
                                }
                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    let (text, color, background) =
                                        if parse::match_existing(
                                            &candidate.motor.name,
                                            &preview.existing,
                                        )
                                        .is_some()
                                        {
                                            ("覆盖", t::WARNING, t::WARNING_SOFT)
                                        } else {
                                            ("新增", t::SUCCESS, t::ACCENT_SOFT)
                                        };
                                    c::badge(ui, text, color, background);
                                    if !candidate.conflicts.is_empty() {
                                        c::badge(ui, "待核对", t::ERROR, t::ERROR_SOFT);
                                    } else if !workflow::is_usable(&candidate.motor) {
                                        c::badge(ui, "缺参数", t::MUTED, t::SURFACE_ALT);
                                    }
                                    ui.label(
                                        RichText::new(format!("P{}", candidate.page))
                                            .size(10.0)
                                            .color(t::MUTED),
                                    );
                                });
                            });
                            ui.add_space(2.0);
                        }
                    });
            },
        );
        ui.add_space(14.0);

        // 右侧编辑区：名称、类型与核心参数。
        ui.allocate_ui_with_layout(
            Vec2::new(ui.available_width(), 360.0),
            Layout::top_down(Align::LEFT),
            |ui| {
                Frame::NONE
                    .fill(t::SURFACE_ALT)
                    .stroke(Stroke::new(1.0, t::BORDER))
                    .corner_radius(CornerRadius::same(10))
                    .inner_margin(Margin::same(12))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.set_min_height(330.0);
                        ScrollArea::vertical()
                            .id_salt("import_editor")
                            .max_height(330.0)
                            .show(ui, |ui| {
                                editor(ui, preview, current);
                            });
                    });
            },
        );
    });

    *selected = current;
    ui.add_space(12.0);
    c::note_line(
        ui,
        "识别结果只导出为 Excel：请在 Excel 中核对、补全后，通过「导入电机参数」重新选择 .xlsx 入库。",
        t::WARNING,
        t::WARNING_SOFT,
    );
    ui.add_space(10.0);
    c::hairline(ui, t::BORDER);
    ui.add_space(10.0);
    ui.horizontal(|ui| {
        if secondary(ui, "关闭") {
            action = Action::Close;
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if primary(ui, &format!("导出 Excel（{included} 个型号）"), included > 0) {
                action = Action::ExportExcel;
            }
        });
    });
    action
}

/// Excel 导入预览：逐行展示校验结果，存在任何错误时整份不允许提交。
fn excel_preview_page(ui: &mut egui::Ui, preview: &excel::ExcelPreview) -> Action {
    let mut action = Action::None;
    let overwrites = preview.overwrite_count();
    let valid = preview.valid_motors().len();
    let subtitle = format!(
        "{} · {} 行数据 · 新增 {} / 覆盖 {} / 错误 {}",
        preview
            .path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| preview.path.display().to_string()),
        preview.rows.len(),
        valid.saturating_sub(overwrites),
        overwrites,
        preview.error_count()
    );
    if header(ui, "Excel 导入预览", &subtitle) {
        action = Action::Close;
    }

    for error in &preview.errors {
        c::note_line(ui, error, t::ERROR, t::ERROR_SOFT);
        ui.add_space(4.0);
    }

    if preview.rows.is_empty() {
        c::empty_state(
            ui,
            "没有可导入的数据",
            "请确认表头包含「型号 / 类型 / 持续值 / 峰值」，且至少有一行数据。",
        );
    } else {
        ScrollArea::vertical()
            .id_salt("excel_import_rows")
            .max_height(430.0)
            .show(ui, |ui| {
                egui::Grid::new("excel_import_grid")
                    .num_columns(5)
                    .striped(true)
                    .spacing([16.0, 6.0])
                    .show(ui, |ui| {
                        for title in ["行", "型号", "类型", "持续 / 峰值", "状态"] {
                            ui.label(RichText::new(title).size(10.5).color(t::MUTED));
                        }
                        ui.end_row();

                        for row in &preview.rows {
                            ui.label(RichText::new(row.line.to_string()).size(10.5).color(t::MUTED));
                            ui.label(RichText::new(&row.name).size(11.5).color(t::TEXT));
                            let kind_text = row
                                .motor
                                .as_ref()
                                .map(|motor| {
                                    if motor.kind == MotorKind::Rotary {
                                        "旋转"
                                    } else {
                                        "直线"
                                    }
                                })
                                .unwrap_or("—");
                            ui.label(RichText::new(kind_text).size(11.0).color(t::TEXT_2));
                            let values = row
                                .motor
                                .as_ref()
                                .map(|motor| format!("{:.3} / {:.3}", motor.cont, motor.peak))
                                .unwrap_or_else(|| "—".to_owned());
                            ui.label(RichText::new(values).size(11.0).color(t::TEXT_2));
                            ui.horizontal(|ui| {
                                if !row.errors.is_empty() {
                                    c::badge(ui, "错误", t::ERROR, t::ERROR_SOFT);
                                } else if row.motor.as_ref().is_some_and(|motor| {
                                    parse::match_existing(&motor.name, &preview.existing).is_some()
                                }) {
                                    c::badge(ui, "覆盖", t::WARNING, t::WARNING_SOFT);
                                } else {
                                    c::badge(ui, "新增", t::SUCCESS, t::ACCENT_SOFT);
                                }
                            });
                            ui.end_row();

                            if !row.errors.is_empty() {
                                ui.label("");
                                ui.label(
                                    RichText::new(row.errors.join("；"))
                                        .size(10.5)
                                        .color(t::ERROR),
                                );
                                ui.label("");
                                ui.label("");
                                ui.label("");
                                ui.end_row();
                            }
                        }
                    });
            });
    }

    ui.add_space(12.0);
    if preview.blocked() {
        c::note_line(
            ui,
            "存在校验错误，整份 Excel 不能提交；请在 Excel 中修正后重新选择文件。",
            t::ERROR,
            t::ERROR_SOFT,
        );
        ui.add_space(6.0);
    }
    c::hairline(ui, t::BORDER);
    ui.add_space(10.0);
    ui.horizontal(|ui| {
        if secondary(ui, "关闭") {
            action = Action::Close;
        }
        if secondary(ui, "重新选择文件") {
            action = Action::Reselect;
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if primary(
                ui,
                &format!("导入 {valid} 个型号"),
                !preview.blocked() && valid > 0,
            ) {
                action = Action::CommitExcel;
            }
        });
    });
    action
}

fn editor(ui: &mut egui::Ui, preview: &mut workflow::Preview, index: usize) {
    let existing = parse::match_existing(
        &preview.candidates[index].motor.name,
        &preview.existing,
    );
    let candidate = &mut preview.candidates[index];

    ui.horizontal(|ui| {
        ui.label(RichText::new("型号名称").size(11.0).color(t::TEXT_2));
        ui.add(
            TextEdit::singleline(&mut candidate.motor.name)
                .desired_width(240.0)
                .font(egui::TextStyle::Body),
        );
        ui.add_space(8.0);
        match existing {
            Some(existing_index) => {
                let name = &preview.existing[existing_index].name;
                c::badge(ui, &format!("覆盖 {name}"), t::WARNING, t::WARNING_SOFT);
            }
            None => {
                c::badge(ui, "新增记录", t::SUCCESS, t::ACCENT_SOFT);
            }
        }
    });
    // `badge` 内部借用结束后再使用 candidate，避免重叠可变借用。
    let Some(candidate) = preview.candidates.get_mut(index) else {
        return;
    };

    ui.add_space(6.0);
    let mut rotary = candidate.motor.kind == MotorKind::Rotary;
    ui.horizontal(|ui| {
        ui.label(RichText::new("电机类型").size(11.0).color(t::TEXT_2));
        if c::choice(ui, &mut rotary, true, "旋转电机") {
            candidate.motor.kind = MotorKind::Rotary;
            candidate.motor.construction = crate::data::motor::Construction::Standard;
        }
        if c::choice(ui, &mut rotary, false, "直线电机") {
            let (kind, construction) = classify(&candidate.motor.name);
            candidate.motor.kind = kind;
            candidate.motor.construction = construction;
        }
        if candidate.motor.kind == MotorKind::Rotary {
            ui.label(RichText::new("N·m / rpm").size(10.0).color(t::MUTED));
        } else {
            ui.label(RichText::new("N / m/s").size(10.0).color(t::MUTED));
        }
    });

    ui.add_space(6.0);
    let mut changed = false;
    let motor = &mut candidate.motor;
    let rotary = motor.kind == MotorKind::Rotary;

    let cont_unit = if rotary { "N·m" } else { "N" };
    // 样册里的力/转矩常数以 N·m/Arms 给出，与电流字段的 Arms 单位一致。
    let kf_unit = if rotary { "N·m/Arms" } else { "N/Arms" };
    let ke_unit = if rotary { "V/(rad/s)" } else { "V/(m/s)" };
    let km_unit = if rotary { "N·m/√W" } else { "N/√W" };
    let speed_unit = if rotary { "rpm" } else { "m/s" };
    let mass_label = if rotary { "转子质量" } else { "线圈质量" };

    changed |= field_row(
        ui,
        ("持续推力/转矩", cont_unit, &mut motor.cont, 0.05, 3),
        ("峰值推力/转矩", cont_unit, &mut motor.peak, 0.1, 3),
    );
    changed |= field_row(
        ui,
        ("力/转矩常数", kf_unit, &mut motor.kf, 0.01, 4),
        ("反电势常数", ke_unit, &mut motor.ke, 0.001, 5),
    );
    changed |= field_row(
        ui,
        ("电机常数", km_unit, &mut motor.km, 0.01, 4),
        ("相间电阻", "Ω", &mut motor.resist, 0.01, 4),
    );
    changed |= field_row(
        ui,
        ("相间电感", "mH", &mut motor.induct, 0.01, 4),
        ("持续电流", "Arms", &mut motor.i_cont, 0.01, 3),
    );
    changed |= field_row(
        ui,
        ("峰值电流", "Arms", &mut motor.i_peak, 0.01, 3),
        ("持续热功率", "W", &mut motor.rated_w, 0.1, 3),
    );
    changed |= field_row(
        ui,
        (mass_label, "kg", &mut motor.mover_mass, 0.001, 5),
        ("线圈长度", "mm", &mut motor.length, 0.1, 2),
    );
    if rotary {
        changed |= field_row(
            ui,
            ("转动惯量", "kg·m²", &mut motor.inertia, 0.0001, 7),
            ("最高转速", speed_unit, &mut motor.max_speed, 1.0, 2),
        );
    } else {
        changed |= field_row(
            ui,
            ("最高速度", speed_unit, &mut motor.max_speed, 0.1, 2),
            ("", "", &mut motor.inertia, 0.0, 0),
        );
    }

    if changed {
        // 功率由推力与电机常数派生；不覆盖用户直接修改的功率以外的字段。
        if motor.km > 0.0 {
            motor.rated_w = (motor.cont / motor.km).powi(2);
            motor.max_w = (motor.peak / motor.km).powi(2);
        }
        if motor.max_speed <= 0.0 {
            motor.max_speed = if rotary { 3000.0 } else { 5.0 };
        }
    }

    ui.add_space(8.0);
    let detected = &candidate.detected;
    let conflicts = &candidate.conflicts;
    let missing: Vec<&str> = Field::ALL
        .into_iter()
        .filter(|field| !detected.contains(field))
        .map(Field::label)
        .collect();
    if !conflicts.is_empty() {
        let names: Vec<&str> = conflicts.iter().map(|field| field.label()).collect();
        c::note_line(
            ui,
            &format!("识别值存在分歧：{}。请对照原文档核对。", names.join("、")),
            t::ERROR,
            t::ERROR_SOFT,
        );
        ui.add_space(4.0);
    }
    if !missing.is_empty() {
        let text = if existing.is_some() {
            format!("未识别：{}。已沿用数据库中的原值。", missing.join("、"))
        } else {
            format!("未识别：{}。新增型号请手动补全。", missing.join("、"))
        };
        c::note_line(ui, &text, t::WARNING, t::WARNING_SOFT);
    } else {
        c::note_line(
            ui,
            "参数表字段已全部识别，导入后可在选型页直接使用。",
            t::SUCCESS,
            t::ACCENT_SOFT,
        );
    }
}

fn field_row(
    ui: &mut egui::Ui,
    left: (&str, &str, &mut f64, f64, usize),
    right: (&str, &str, &mut f64, f64, usize),
) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        let width = 224.0;
        ui.allocate_ui_with_layout(
            Vec2::new(width, 26.0),
            Layout::left_to_right(Align::Center),
            |ui| {
                ui.set_width(width);
                changed |= c::num_field(ui, left.0, left.1, left.2, true, left.3, left.4);
            },
        );
        ui.add_space(8.0);
        if !right.0.is_empty() {
            ui.allocate_ui_with_layout(
                Vec2::new(width, 26.0),
                Layout::left_to_right(Align::Center),
                |ui| {
                    ui.set_width(width);
                    changed |= c::num_field(ui, right.0, right.1, right.2, true, right.3, right.4);
                },
            );
        }
    });
    changed
}
