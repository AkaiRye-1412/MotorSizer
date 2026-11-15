use egui::{
    scroll_area::ScrollBarVisibility, Align, Button, Color32, Layout, RichText, ScrollArea,
    Stroke, Ui,
};

use crate::app::{construction_label, kind_label, AppState, Mode, MotionMode, Tab};

use super::charts;
use super::components as c;
use super::theme as t;

pub fn input_panel(ui: &mut Ui, state: &mut AppState, height: f32) {
    c::card_sized(ui, height, |ui| {
        ScrollArea::vertical()
            .id_salt("input_scroll")
            .scroll_bar_visibility(ScrollBarVisibility::AlwaysVisible)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                // 为滚动条预留宽度，防止右对齐的输入值被覆盖。
                ui.set_width((ui.available_width() - 8.0).max(160.0));
                ui.spacing_mut().item_spacing.y = 2.0;
                c::section_title(ui, "工况输入", "实时计算");
                ui.label(
                    RichText::new(state.tab.subtitle())
                        .size(10.5)
                        .color(t::MUTED),
                );
                c::hairline(ui, t::BORDER);
                mode_controls(ui, state);
                c::hairline(ui, t::BORDER);

                let mut dirty = false;
                c::group_label(ui, "轴信息");
                dirty |= c::text_field(ui, "机器名称", &mut state.inputs.machine);
                dirty |= c::text_field(ui, "轴名称", &mut state.inputs.axis);
                dirty |= c::num_field(ui, "环境温度", "°C", &mut state.inputs.ambient, true, 0.5, 0);

                c::group_label(ui, "运动模式");
                ui.horizontal(|ui| {
                    dirty |= c::choice(
                        ui,
                        &mut state.motion_mode,
                        MotionMode::PointToPoint,
                        "点对点",
                    );
                    dirty |= c::choice(ui, &mut state.motion_mode, MotionMode::Custom, "自定义");
                });
                ui.horizontal(|ui| {
                    dirty |= c::choice(ui, &mut state.shape, crate::domain::motion::Shape::Trapezoid, "梯形");
                    dirty |= c::choice(ui, &mut state.shape, crate::domain::motion::Shape::Triangle, "三角形");
                });
                dirty |= c::check(ui, "S-曲线", &mut state.inputs.scurve);
                if state.inputs.scurve {
                    dirty |= c::num_field(
                        ui,
                        "S-曲线比例",
                        "%",
                        &mut state.inputs.scurve_pct,
                        true,
                        1.0,
                        0,
                    );
                }
                if state.shape == crate::domain::motion::Shape::Trapezoid {
                    dirty |= c::num_field(
                        ui,
                        "加速:运动 = 1 : N",
                        "",
                        &mut state.inputs.ratio_n,
                        true,
                        0.2,
                        1,
                    );
                }

                c::group_label(ui, "运动参数");
                let rotary = state.tab == Tab::Rotary;
                // 旋转轴的界面单位：角度用度、速度用 rpm、加速度用 rpm/s；
                // 内部仍保留 rev / rpm / rad/s²，避免影响求解与报表换算。
                let (mass_unit, distance_unit, speed_unit, accel_unit, force_unit) = if rotary {
                    ("kg·m²", "°", "rpm", "rpm/s", "N·m")
                } else {
                    ("kg", "m", "m/s", "m/s²", "N")
                };
                let (distance_scale, accel_scale) = if rotary {
                    (360.0, 60.0 / std::f64::consts::TAU)
                } else {
                    (1.0, 1.0)
                };
                let custom = state.motion_mode == MotionMode::Custom;
                // 点对点只输入行程/时间，速度与加速度由求解器反算；
                // calc_* 保持内部单位（rev、s、rpm、rad/s²），显示时再乘 scale。
                let (calc_s, calc_t, calc_v, calc_a) = match state.computed.profile.as_ref() {
                    Some(profile) => {
                        let distance = if rotary {
                            profile.kin.s / std::f64::consts::TAU
                        } else {
                            profile.kin.s
                        };
                        let speed = if rotary {
                            profile.kin.v / std::f64::consts::TAU * 60.0
                        } else {
                            profile.kin.v
                        };
                        (
                            Some(distance),
                            Some(profile.kin.t_run),
                            Some(speed),
                            Some(profile.kin.a),
                        )
                    }
                    None => (None, None, None, None),
                };
                // 自定义模式下未填写的项直接显示计算值；点对点下行程/时间本来就是必填。
                let (auto_s, auto_t, auto_v, auto_a) = if custom {
                    (calc_s, calc_t, calc_v, calc_a)
                } else {
                    (None, None, None, None)
                };

                dirty |= c::num_field(ui, if rotary { "负载转动惯量" } else { "总移动质量" }, mass_unit, &mut state.inputs.mass, true, 0.05, 4);
                dirty |= c::auto_value_field(ui, if rotary { "运动角度" } else { "运动距离" }, distance_unit, &mut state.inputs.distance, auto_s, distance_scale, if rotary { 0.5 } else { 0.01 }, if rotary { 2 } else { 4 });
                dirty |= c::auto_value_field(ui, "运动时间", "s", &mut state.inputs.time, auto_t, 1.0, 0.01, 3);

                if custom {
                    dirty |= c::auto_value_field(ui, "最大加速度", accel_unit, &mut state.inputs.accel, auto_a, accel_scale, if rotary { 10.0 } else { 0.01 }, if rotary { 1 } else { 3 });
                    dirty |= c::auto_value_field(ui, "最大速度", speed_unit, &mut state.inputs.speed, auto_v, 1.0, if rotary { 0.1 } else { 0.01 }, if rotary { 1 } else { 3 });
                    let any_auto = [
                        (state.inputs.distance, auto_s),
                        (state.inputs.time, auto_t),
                        (state.inputs.accel, auto_a),
                        (state.inputs.speed, auto_v),
                    ]
                    .iter()
                    .any(|(value, calc)| *value <= 0.0 && calc.is_some_and(|v| v > 0.0));
                    if any_auto {
                        ui.label(
                            RichText::new("◈ 蓝底数值由其余条件自动计算，可直接编辑")
                                .size(9.5)
                                .color(t::ACCENT),
                        );
                    }
                } else {
                    c::computed_field(ui, "最大加速度", accel_unit, calc_a, accel_scale, if rotary { 1 } else { 3 });
                    c::computed_field(ui, "最大速度", speed_unit, calc_v, 1.0, if rotary { 1 } else { 3 });
                }
                dirty |= c::num_field(ui, "停顿时间", "s", &mut state.inputs.dwell, true, 0.01, 2);

                c::group_label(ui, "负载");
                dirty |= c::num_field(ui, if rotary { "摩擦力矩" } else { "摩擦力" }, force_unit, &mut state.inputs.friction, true, 0.1, 2);
                dirty |= c::num_field(ui, if rotary { "外力矩" } else { "对向外力" }, force_unit, &mut state.inputs.ext_force, true, 0.1, 2);
                if !rotary {
                    dirty |= c::num_field(ui, "安装倾角", "°", &mut state.inputs.angle, true, 0.5, 1);
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("方向").size(11.0).color(t::TEXT_2));
                        dirty |= c::choice(ui, &mut state.inputs.up, true, "向上");
                        dirty |= c::choice(ui, &mut state.inputs.up, false, "向下");
                    });
                }

                c::group_label(ui, "选型设置");
                dirty |= c::num_field(ui, "安全系数余量", "%", &mut state.inputs.safety_pct, true, 1.0, 0);
                ui.label(
                    RichText::new(format!(
                        "实际倍率：{:.2}×（0% = 1.0×，100% = 2.0×）",
                        state.inputs.safety_factor()
                    ))
                    .size(9.5)
                    .color(t::MUTED),
                );
                dirty |= c::num_field(ui, "余量下限", "%", &mut state.window.low_pct, true, 1.0, 0);
                dirty |= c::num_field(ui, "余量上限", "%", &mut state.window.high_pct, true, 1.0, 0);
                dirty |= c::check(ui, "启用上限", &mut state.window.use_upper);

                c::group_label(ui, "驱动器匹配");
                // 母线电压与相数只影响驱动器候选，不需要重算选型结果。
                let _ = c::num_field(
                    ui,
                    "母线电压",
                    "V",
                    &mut state.drive_settings.bus_voltage,
                    true,
                    1.0,
                    0,
                );
                let mut phase = state.drive_settings.phase as f64;
                if c::num_field(ui, "驱动器相数", "相", &mut phase, true, 1.0, 0) {
                    state.drive_settings.phase = phase.round().clamp(1.0, 3.0) as u32;
                }

                if dirty {
                    state.touch();
                }
            });
    });
}

fn mode_controls(ui: &mut Ui, state: &mut AppState) {
    match state.mode {
        Mode::Independent => {}
        Mode::Combined => {
            c::group_label(ui, "综合曲线集合");
            let mut dirty = false;
            dirty |= c::num_field(
                ui,
                "添加数量",
                "条",
                &mut state.add_count,
                true,
                0.2,
                0,
            );
            let can_add = state.computed.profile.is_some();
            if c::action_button(ui, "添加当前曲线", true, can_add) {
                let count = state.add_count.round().clamp(1.0, 20.0) as usize;
                state.add_current_curves(count);
            }
            if c::action_button(ui, "清空全部曲线", false, state.curve_count() > 0) {
                state.clear_curves();
            }
            let count = state.curve_count();
            let info = ui.label(
                RichText::new(if count == 0 {
                    "尚未保存曲线，暂按当前曲线汇总".to_owned()
                } else {
                    format!("已保存 {count} 条曲线，汇总结果实时更新")
                })
                .size(9.5)
                .color(t::MUTED),
            );
            if !state.computed.curves.is_empty() {
                let labels = state
                    .computed
                    .curves
                    .iter()
                    .map(|curve| curve.label.as_str())
                    .collect::<Vec<_>>()
                    .join("、");
                info.on_hover_text(labels);
            }
            if dirty {
                state.touch();
            }
        }
        Mode::Solution => {
            c::group_label(ui, "方案库");
            let can_save = state.computed.profile.is_some();
            if c::action_button(ui, "保存当前方案", true, can_save) {
                state.save_scheme();
            }
            if c::action_button(ui, "清空全部方案", false, !state.schemes.is_empty()) {
                state.schemes.clear();
                state.status = "已清空方案库".to_owned();
            }
            ui.label(
                RichText::new(format!(
                    "已保存 {} 个方案；在结果区点击方案即可载入",
                    state.schemes.len()
                ))
                .size(9.5)
                .color(t::MUTED),
            );
        }
    }
}

pub fn result_panel(ui: &mut Ui, state: &mut AppState) {
    let rotary = state.tab == Tab::Rotary;
    let (force_unit, velocity_unit) = match state.tab {
        Tab::Linear => ("N", "m/s"),
        Tab::Rotary => ("N·m", "rpm"),
    };

    let combined = state.computed.combined;
    let profile = state.computed.profile.as_ref();
    let is_combined = state.mode == Mode::Combined;
    // 阶段分界要读取整条曲线集合，提前算好，避免与下方可变借用冲突。
    let marks = chart_marks(state, is_combined, profile);
    let (rms, peak, cycle) = if is_combined {
        combined
            .map(|c| (c.rms, c.peak, c.cycle_time))
            .unwrap_or((0.0, 0.0, 0.0))
    } else {
        (
            profile.map(|p| p.f_rms).unwrap_or(0.0),
            profile.map(|p| p.f_peak).unwrap_or(0.0),
            profile.map(|p| p.t_cycle).unwrap_or(0.0),
        )
    };
    let margin = state
        .computed
        .selected
        .as_ref()
        .map(|s| s.margin_pct)
        .unwrap_or(0.0);

    let labels = if is_combined {
        ("综合持续值", "综合峰值", "总周期", "安全余量")
    } else {
        ("持续推力/力矩", "峰值推力/力矩", "循环周期", "安全余量")
    };
    ui.columns(4, |columns| {
        c::kpi(&mut columns[0], labels.0, &fmt(rms), force_unit, t::ACCENT);
        c::kpi(&mut columns[1], labels.1, &fmt(peak), force_unit, t::ACCENT_CYAN);
        c::kpi(&mut columns[2], labels.2, &fmt(cycle), "s", t::SUCCESS);
        c::kpi(&mut columns[3], labels.3, &fmt(margin), "%", t::WARNING);
    });

    if !state.computed.errors.is_empty() || !state.computed.warnings.is_empty() {
        ui.add_space(6.0);
        for error in &state.computed.errors {
            c::note_line(ui, error, t::ERROR, t::ERROR_SOFT);
        }
        for warning in &state.computed.warnings {
            c::note_line(ui, warning, t::WARNING, t::WARNING_SOFT);
        }
    }

    if is_combined {
        if let Some(c) = combined {
            // 旋转：rad → 度，rad/s → rpm。
            let (distance, max_speed, distance_unit) = if rotary {
                (
                    c.distance / std::f64::consts::TAU * 360.0,
                    c.v_max / std::f64::consts::TAU * 60.0,
                    "°",
                )
            } else {
                (c.distance, c.v_max, "m")
            };
            ui.add_space(6.0);
            ui.label(
                RichText::new(format!(
                    "综合 {} 条 · 总行程 {distance:.3} {distance_unit} · 总运动 {:.2} s · 最大速度 {max_speed:.2} {velocity_unit}",
                    c.count, c.run_time
                ))
                .size(10.5)
                .color(t::MUTED),
            );
        }
    } else if let Some(p) = profile {
        let segment = if p.kin.tc > 0.0 {
            format!("加速 {:.2}s · 匀速 {:.2}s", p.kin.t1, p.kin.tc)
        } else {
            format!("加速 {:.2}s · 无匀速段", p.kin.t1)
        };
        ui.add_space(6.0);
        ui.label(
            RichText::new(format!(
                "{} · {} · 理论功率 {:.1} W（峰值 {:.1} W）",
                p.kin.shape.label(),
                segment,
                p.p_rated,
                p.p_max
            ))
            .size(10.5)
            .color(t::MUTED),
        );
    }

    ui.add_space(8.0);
    let rec_height = 228.0;
    if state.mode == Mode::Solution {
        schemes_card(ui, state, rec_height);
    } else {
    c::card_sized(ui, rec_height, |ui| {
        let count = state.computed.recs.len();
        let title = if is_combined { "推荐型号 · 综合" } else { "推荐型号" };
        c::section_title(ui, title, &format!("{count} 个匹配"));
        c::hairline(ui, t::BORDER);
        ui.add_space(3.0);

        if state.tab == Tab::Linear {
            let mut changed = false;
            ui.horizontal_wrapped(|ui| {
                changed |= c::check(ui, "无铁芯", &mut state.filters.ironless);
                changed |= c::check(ui, "有铁芯", &mut state.filters.ironcore);
                changed |= c::check(ui, "音圈", &mut state.filters.voice_coil);
            });
            ui.horizontal_wrapped(|ui| {
                changed |= c::check(ui, "普通冷却", &mut state.filters.standard_cooled);
                changed |= c::check(ui, "水冷", &mut state.filters.water_cooled);
            });
            if changed {
                state.touch();
            }
        }

        if state.computed.recs.is_empty() {
            c::empty_state(ui, "没有匹配型号", "放宽安全系数或余量窗口后再试");
        } else {
            ScrollArea::vertical()
                .id_salt("recommendation_scroll")
                .scroll_bar_visibility(ScrollBarVisibility::AlwaysVisible)
                .auto_shrink([false, false])
                .max_height(130.0)
                .show(ui, |ui| {
                    let selected = state.selected_name.clone();
                    let mut clicked = None;
                    for row in &state.computed.recs {
                        let active = selected.as_deref() == Some(row.name.as_str());
                        let fill = if active { t::ACCENT_SOFT } else { t::SURFACE_ALT };
                        let text = format!(
                            "{:<18}  余量 {:>7.1}%   利用率 {:>4.0}%",
                            row.name,
                            row.margin_pct,
                            row.utilization * 100.0
                        );
                        let response = ui.add_sized(
                            [ui.available_width(), 24.0],
                            Button::new(RichText::new(text).size(11.0).color(t::TEXT))
                                .fill(fill)
                                .stroke(Stroke::new(1.0, if active { t::ACCENT } else { t::BORDER }))
                                .corner_radius(6),
                        );
                        let response = if active {
                            response.on_hover_text(format!("安全余量 {:.1}%", row.margin_pct))
                        } else {
                            response
                        };
                        if response.clicked() {
                            clicked = Some(row.name.clone());
                        }
                    }
                    if let Some(name) = clicked {
                        state.select_motor(&name);
                    }
                });
        }
    });
    }

    ui.add_space(10.0);
    // 用布局实际剩余高度，避免把 KPI 与提示行的高度重复算进图表。
    let chart_height = ui.available_height().max(140.0);
    // 旋转轴的速度曲线内部是 rad/s，图上按 rpm 显示。
    let velocity: Vec<(f64, f64)> = if rotary {
        state
            .computed
            .v_samples
            .iter()
            .map(|(t, v)| (*t, *v / std::f64::consts::TAU * 60.0))
            .collect()
    } else {
        state.computed.v_samples.clone()
    };
    let force = &state.computed.f_samples;
    ui.columns(2, |columns| {
        chart_card(
            &mut columns[0],
            chart_height,
            "速度 / 时间",
            velocity_unit,
            &velocity,
            t::ACCENT,
            &marks,
        );
        chart_card(
            &mut columns[1],
            chart_height,
            "推力 / 时间",
            force_unit,
            force,
            t::ACCENT_CYAN,
            &marks,
        );
    });
}

/// 图上的阶段分界：单曲线标出匀速/减速起点，综合曲线标出各条曲线的周期分界。
fn chart_marks(
    state: &AppState,
    is_combined: bool,
    profile: Option<&crate::domain::profile::Profile>,
) -> Vec<charts::ChartMark> {
    if is_combined {
        let mut marks = Vec::new();
        let mut accumulated = 0.0;
        let last = state.computed.curves.len().saturating_sub(1);
        for (index, segment) in state.computed.curves.iter().enumerate() {
            accumulated += segment.profile.t_cycle;
            if index < last {
                marks.push(charts::ChartMark {
                    time: accumulated,
                    label: String::new(),
                });
            }
        }
        return marks;
    }

    let Some(profile) = profile else {
        return Vec::new();
    };
    let kin = &profile.kin;
    let mut marks = Vec::new();
    if kin.t1 > 0.0 {
        marks.push(charts::ChartMark {
            time: kin.t1,
            label: if kin.tc > 0.0 { "匀速" } else { "减速" }.to_owned(),
        });
    }
    if kin.tc > 0.0 && kin.t1 + kin.tc > 0.0 {
        marks.push(charts::ChartMark {
            time: kin.t1 + kin.tc,
            label: "减速".to_owned(),
        });
    }
    marks
}

fn schemes_card(ui: &mut Ui, state: &mut AppState, height: f32) {
    c::card_sized(ui, height, |ui| {
        let count = state.schemes.len();
        c::section_title(ui, "方案对比", &format!("{count} 个 · 点击载入"));
        c::hairline(ui, t::BORDER);
        if state.schemes.is_empty() {
            c::empty_state(
                ui,
                "方案库为空",
                "在左侧点击“保存当前方案”，再回到这里对比",
            );
            return;
        }

        ui.label(
            RichText::new("方案 / 电机 / 持续 / 峰值 / 余量")
                .size(9.5)
                .color(t::MUTED),
        );
        ScrollArea::vertical()
            .id_salt("scheme_scroll")
            .scroll_bar_visibility(ScrollBarVisibility::AlwaysVisible)
            .auto_shrink([false, false])
            .max_height(150.0)
            .show(ui, |ui| {
                let mut load = None;
                let mut delete = None;
                for (index, scheme) in state.schemes.iter().enumerate() {
                    let motor = scheme
                        .motor
                        .as_ref()
                        .map(|m| m.name.as_str())
                        .unwrap_or("未选型");
                    let text = format!(
                        "{}  {}  {:.2} / {:.2}  {:.1}%",
                        scheme.name, motor, scheme.rms, scheme.peak, scheme.margin
                    );
                    ui.horizontal(|ui| {
                        let row_width = (ui.available_width() - 30.0).max(80.0);
                        let response = ui
                            .add_sized(
                                [row_width, 24.0],
                                Button::new(RichText::new(text).size(10.5).color(t::TEXT))
                                    .fill(t::SURFACE_ALT)
                                    .stroke(Stroke::new(1.0, t::BORDER))
                                    .corner_radius(6),
                            )
                            .on_hover_text(format!("周期 {:.2} s", scheme.cycle));
                        if response.clicked() {
                            load = Some(index);
                        }
                        if ui.small_button("×").on_hover_text("删除该方案").clicked() {
                            delete = Some(index);
                        }
                    });
                }
                if let Some(index) = delete {
                    state.delete_scheme(index);
                } else if let Some(index) = load {
                    state.apply_scheme(index);
                }
            });
    });
}

fn chart_card(
    ui: &mut Ui,
    height: f32,
    title: &str,
    unit: &str,
    samples: &[(f64, f64)],
    color: Color32,
    marks: &[charts::ChartMark],
) {
    c::card_sized(ui, height, |ui| {
        c::section_title(ui, title, "实时");
        c::hairline(ui, t::BORDER);
        ui.add_space(4.0);
        charts::line_chart(ui, unit, samples, color, marks);
    });
}

pub fn detail_panel(ui: &mut Ui, state: &AppState, height: f32) {
    let selected = state.computed.selected.clone();
    let Some(sel) = selected else {
        c::card_scroll(ui, height, "detail_scroll", |ui| {
            ui.set_width((ui.available_width() - 12.0).max(180.0));
            c::section_title(ui, "电机详情", "未选择");
            ui.label(
                RichText::new(state.tab.label())
                    .size(10.5)
                    .color(t::MUTED),
            );
            c::hairline(ui, t::BORDER);
            ui.add_space(4.0);
            c::empty_state(ui, "没有可显示的型号", "先让工况产生推荐结果");
        });
        return;
    };

    // 规格与应用值拆成上下两张独立卡片：中间留出间距，两块边界一眼就能分开；
    // 各自独立滚动，规格表再长也不会把应用值挤到看不见的位置。
    let gap = 12.0;
    let top_height = ((height - gap) * 0.52).max(0.0);
    let bottom_height = (height - gap - top_height).max(0.0);

    c::card_scroll(ui, top_height, "detail_scroll_specs", |ui| {
        // 详情列使用固定右留白，保证规格值完整显示。
        ui.set_width((ui.available_width() - 12.0).max(180.0));
        ui.spacing_mut().item_spacing.y = 1.0;
        let motor = &sel.motor;
        let force_unit = if state.tab == Tab::Rotary { "N·m" } else { "N" };
        c::section_title(ui, "电机详情", &motor.name);
        ui.label(
            RichText::new(format!(
                "{} · {}",
                kind_label(motor.kind),
                construction_label(motor.construction)
            ))
            .size(10.5)
            .color(t::MUTED),
        );
        c::hairline(ui, t::BORDER);

        c::group_label(ui, "规格");
        c::spec_row(ui, "型号", &motor.name);
        c::spec_row(ui, "类型", kind_label(motor.kind));
        c::spec_row(ui, "冷却", construction_label(motor.construction));
        c::spec_row(ui, "持续能力", &format!("{} {force_unit}", fmt(motor.cont)));
        c::spec_row(ui, "峰值能力", &format!("{} {force_unit}", fmt(motor.peak)));
        c::spec_row(ui, "电机常数", &format!("{:.3}", motor.km));
        c::spec_row(ui, "力/力矩常数", &format!("{:.3}", motor.kf));
        c::spec_row(ui, "反电动势常数", &format!("{:.3}", motor.ke));
        // 电流统一用 Arms（安培有效值）：样册的 Icn/Ipk 均以 Arms
        // 给出；若数据源是 A（峰值安培），按 Arms = A/1.414 换算。
        c::spec_row(ui, "连续电流", &format!("{:.2} Arms", motor.i_cont));
        c::spec_row(ui, "峰值电流", &format!("{:.2} Arms", motor.i_peak));
        c::spec_row(ui, "相电阻", &format!("{:.3} Ω", motor.resist));
        c::spec_row(ui, "相电感", &format!("{:.3} mH", motor.induct));
        c::spec_row(ui, "额定功率", &format!("{:.1} W", motor.rated_w));
        c::spec_row(ui, "最大功率", &format!("{:.1} W", motor.max_w));
        if state.tab == Tab::Rotary {
            c::spec_row(ui, "转子惯量", &format!("{:.5} kg·m²", motor.inertia));
            c::spec_row(ui, "最高转速", &format!("{:.0} rpm", motor.max_speed));
        } else {
            c::spec_row(ui, "动子质量", &format!("{:.3} kg", motor.mover_mass));
            c::spec_row(ui, "线圈长度", &format!("{:.1} mm", motor.length));
            c::spec_row(ui, "最高速度", &format!("{:.2} m/s", motor.max_speed));
        }
    });

    ui.add_space(gap);

    c::card_scroll(ui, bottom_height, "detail_scroll_app", |ui| {
        ui.set_width((ui.available_width() - 12.0).max(180.0));
        ui.spacing_mut().item_spacing.y = 1.0;
        let motor = &sel.motor;
        c::section_title(ui, "应用值", "");
        c::hairline(ui, t::BORDER);
        ui.add_space(4.0);

        c::spec_row(ui, "持续利用率", &format!("{:.1} %", sel.util_cont * 100.0));
        c::spec_row(ui, "峰值利用率", &format!("{:.1} %", sel.util_peak * 100.0));
        c::spec_row(
            ui,
            "安全系数",
            &format!("{:.2}×", state.inputs.safety_factor()),
        );
        if motor.kf.abs() > 1e-9 {
            c::spec_row(
                ui,
                "估算持续电流",
                &format!("{:.2} Arms", sel.need_rms / motor.kf),
            );
            c::spec_row(
                ui,
                "估算峰值电流",
                &format!("{:.2} Arms", sel.need_peak / motor.kf),
            );
        }

        drive_section(ui, state);
    });
}

/// 驱动器候选：按应用电流、母线电压与相数筛选，余量从大到小。
fn drive_section(ui: &mut Ui, state: &AppState) {
    let matches = state.matched_drives();
    c::group_label(ui, "驱动器匹配");
    ui.label(
        RichText::new(format!(
            "母线 {:.0} V · {} 相 · 按应用电流匹配",
            state.drive_settings.bus_voltage, state.drive_settings.phase
        ))
        .size(9.5)
        .color(t::MUTED),
    );
    ui.add_space(3.0);

    if matches.is_empty() {
        ui.label(
            RichText::new("没有满足电流 / 电压 / 相数的驱动器")
                .size(10.5)
                .color(t::WARNING),
        );
        ui.label(
            RichText::new("可提高母线电压或切换相数后重试")
                .size(9.5)
                .color(t::MUTED),
        );
        return;
    }

    // 详情列高度有限，只列余量最大的前 6 个，其余在悬停提示中查看。
    let shown = matches.len().min(6);
    for item in &matches[..shown] {
        drive_row(ui, item);
    }
    if matches.len() > shown {
        let rest = matches[shown..]
            .iter()
            .map(|item| item.drive.name.as_str())
            .collect::<Vec<_>>()
            .join("、");
        ui.label(
            RichText::new(format!("另有 {} 个候选（按电流余量排序）", matches.len() - shown))
                .size(9.5)
                .color(t::MUTED),
        )
        .on_hover_text(rest);
    }
}

fn drive_row(ui: &mut Ui, item: &crate::domain::drive::DriveMatch) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(&item.drive.name).size(11.5).color(t::TEXT));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.add_space(6.0);
            // 余量 ≥ 50% 视为充裕，0–50% 为可用但偏紧。
            let color = if item.margin >= 0.5 {
                t::SUCCESS
            } else {
                t::WARNING
            };
            ui.label(
                RichText::new(format!("余量 {:.0}%", item.margin * 100.0))
                    .size(10.0)
                    .color(color),
            );
        });
    });
    ui.label(
        RichText::new(format!(
            "持续 {:.1} Arms / 峰值 {:.1} Arms · {:.0} V · {} 相",
            item.drive.cont, item.drive.peak, item.drive.voltage, item.drive.phase
        ))
        .size(9.5)
        .color(t::MUTED),
    );
    ui.add_space(4.0);
}

fn fmt(x: f64) -> String {
    if x.abs() >= 100.0 {
        format!("{x:.1}")
    } else if x.abs() >= 10.0 {
        format!("{x:.2}")
    } else {
        format!("{x:.3}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_state() -> AppState {
        AppState::from_catalog(crate::data::repo::load_in_memory().expect("内存数据库"))
    }

    #[test]
    fn single_curve_marks_show_phase_boundaries() {
        let state = default_state();
        let marks = chart_marks(&state, false, state.computed.profile.as_ref());
        assert_eq!(marks.len(), 2);
        assert_eq!(marks[0].label, "匀速");
        assert!(marks[0].time > 0.0);
        assert_eq!(marks[1].label, "减速");
        assert!(marks[1].time > marks[0].time);
    }

    #[test]
    fn combined_marks_split_cycles() {
        let mut state = default_state();
        state.set_mode(Mode::Combined);
        state.add_current_curves(2);
        state.recompute();
        let marks = chart_marks(&state, true, None);
        assert_eq!(marks.len(), 1);
        assert!(marks[0].label.is_empty());
        let cycle = state.computed.curves[0].profile.t_cycle;
        assert!((marks[0].time - cycle).abs() < 1e-9);
    }
}
