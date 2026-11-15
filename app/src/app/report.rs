//! 选型报告导出：把当前工况、计算结果与推荐型号写成 Markdown 文件。
//!
//! 报告只写入本地磁盘（默认位于数据库同目录的 `reports/`），不上传任何数据。

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use super::state::{construction_label, kind_label, AppState, Mode, MotionMode, Tab};

/// 默认报告目录：与数据库同级的 `reports/`，供文件对话框作为起始位置。
pub fn default_report_dir() -> PathBuf {
    // 报告与数据库放在同一数据目录，便于用户找到和备份。
    let db = crate::data::repo::default_db_path();
    db.parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("reports")
}

/// 保存对话框默认显示的文件名（带时间戳，避免覆盖旧报告）。
pub fn suggested_file_name() -> String {
    format!("选型报告-{}.md", timestamp())
}

/// 导出到指定目录；目录不存在时自动创建。
pub fn export_to(state: &AppState, dir: &Path) -> Result<PathBuf, String> {
    export_to_path(state, &dir.join(suggested_file_name()))
}

/// 导出到用户选择的完整路径；父目录不存在时自动创建。
pub fn export_to_path(state: &AppState, path: &Path) -> Result<PathBuf, String> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .map_err(|error| format!("无法创建报告目录 {}：{error}", parent.display()))?;
        }
    }
    std::fs::write(path, render(state))
        .map_err(|error| format!("无法写入报告 {}：{error}", path.display()))?;
    Ok(path.to_path_buf())
}

/// 生成报告文本，便于测试与后续导出为其他格式。
fn render(state: &AppState) -> String {
    let mut out = String::new();
    let rotary = state.tab == Tab::Rotary;
    let force_unit = if rotary { "N·m" } else { "N" };

    macro_rules! line {
        ($($arg:tt)*) => {{
            out.push_str(&format!($($arg)*));
            out.push('\n');
        }};
    }

    let mode = match state.mode {
        Mode::Independent => "独立运动曲线",
        Mode::Combined => "综合曲线汇总",
        Mode::Solution => "方案对比",
    };
    let motion = match state.motion_mode {
        MotionMode::PointToPoint => "点对点（仅给出行程与时间）",
        MotionMode::Custom => "自定义（速度/加速度参与求解）",
    };

    line!("# MotionSizer Studio 选型报告");
    line!("");
    line!("| 项目 | 值 |");
    line!("| --- | --- |");
    line!("| 生成时间 | {} UTC |", timestamp_display());
    line!(
        "| 机器 / 轴 | {} / {} |",
        state.inputs.machine,
        state.inputs.axis
    );
    line!("| 页面 | {} |", state.tab.label());
    line!("| 工作模式 | {mode} |");
    line!("| 数据源 | {} |", state.db_status);
    line!("");

    // ---- 1. 工况输入 ----
    line!("## 1. 工况输入");
    line!("");
    line!("| 项目 | 值 |");
    line!("| --- | --- |");
    line!("| 运动模式 | {motion} |");
    let shape = if state.inputs.scurve {
        format!(
            "{} + S-曲线（比例 {:.0}%）",
            state.shape.label(),
            state.inputs.scurve_pct
        )
    } else {
        state.shape.label().to_owned()
    };
    line!("| 曲线形状 | {shape} |");
    if state.shape == crate::domain::motion::Shape::Trapezoid {
        line!("| 加速:运动 | 1 : {:.2} |", state.inputs.ratio_n);
    }
    if rotary {
        line!("| 负载转动惯量 | {:.4} kg·m² |", state.inputs.mass);
        // 内部角度为 rev、加速度为 rad/s²；报表与界面一致按度和 rpm/s 输出。
        line!(
            "| 运动角度 | {:.2} ° |",
            state.inputs.distance * 360.0
        );
        line!("| 运动时间 | {:.3} s |", state.inputs.time);
        if state.motion_mode == MotionMode::Custom {
            line!("| 最大速度 | {:.1} rpm |", state.inputs.speed);
            line!(
                "| 最大加速度 | {:.1} rpm/s |",
                state.inputs.accel / std::f64::consts::TAU * 60.0
            );
        }
        line!("| 摩擦力矩 | {:.3} N·m |", state.inputs.friction);
        line!("| 外力矩 | {:.3} N·m |", state.inputs.ext_force);
    } else {
        line!("| 总移动质量 | {:.4} kg |", state.inputs.mass);
        line!("| 运动距离 | {:.4} m |", state.inputs.distance);
        line!("| 运动时间 | {:.3} s |", state.inputs.time);
        if state.motion_mode == MotionMode::Custom {
            line!("| 最大速度 | {:.4} m/s |", state.inputs.speed);
            line!("| 最大加速度 | {:.3} m/s² |", state.inputs.accel);
        }
        line!("| 摩擦力 | {:.3} N |", state.inputs.friction);
        line!("| 对向外力 | {:.3} N |", state.inputs.ext_force);
        line!(
            "| 安装倾角 / 方向 | {:.1}° / {} |",
            state.inputs.angle,
            if state.inputs.up { "向上" } else { "向下" }
        );
    }
    line!("| 停顿时间 | {:.3} s |", state.inputs.dwell);
    line!("| 环境温度 | {:.1} °C |", state.inputs.ambient);
    line!(
        "| 安全系数余量 | {:.0}%（倍率 {:.2}×） |",
        state.inputs.safety_pct,
        state.inputs.safety_factor()
    );
    line!(
        "| 余量窗口 | 下限 {:.0}% · 上限 {} |",
        state.window.low_pct,
        if state.window.use_upper {
            format!("{:.0}%", state.window.high_pct)
        } else {
            "不启用".to_owned()
        }
    );
    line!("");

    // ---- 2. 结果 ----
    line!("## 2. 运动与力学结果");
    line!("");
    line!("| 项目 | 值 |");
    line!("| --- | --- |");
    if state.mode == Mode::Combined {
        if let Some(combined) = state.computed.combined {
            let distance = if rotary {
                combined.distance / std::f64::consts::TAU * 360.0
            } else {
                combined.distance
            };
            line!("| 参与汇总曲线 | {} 条 |", combined.count);
            line!(
                "| 总行程 | {:.4} {} |",
                distance,
                if rotary { "°" } else { "m" }
            );
            line!("| 总运动时间 | {:.3} s |", combined.run_time);
            line!("| 总周期（含停顿） | {:.3} s |", combined.cycle_time);
            line!(
                "| 综合持续值（RMS） | {:.4} {force_unit} |",
                combined.rms
            );
            line!("| 综合峰值 | {:.4} {force_unit} |", combined.peak);
            line!(
                "| 最大速度 | {} |",
                velocity_text(combined.v_max, rotary)
            );
            line!("");
            line!("### 曲线明细");
            line!("");
            line!("| 曲线 | 持续 {force_unit} | 峰值 {force_unit} | 周期 s |");
            line!("| --- | --- | --- | --- |");
            for segment in &state.computed.curves {
                line!(
                    "| {} | {:.4} | {:.4} | {:.3} |",
                    segment.label,
                    segment.profile.f_rms,
                    segment.profile.f_peak,
                    segment.profile.t_cycle
                );
            }
        } else {
            line!("| 状态 | 尚未形成有效的综合曲线 |");
        }
    } else if let Some(profile) = &state.computed.profile {
        let kin = &profile.kin;
        let distance = if rotary {
            kin.s / std::f64::consts::TAU * 360.0
        } else {
            kin.s
        };
        line!(
            "| 行程 | {:.4} {} |",
            distance,
            if rotary { "°" } else { "m" }
        );
        line!("| 运动时间 | {:.3} s |", kin.t_run);
        line!("| 循环周期 | {:.3} s |", profile.t_cycle);
        line!("| 加速时间 | {:.3} s |", kin.t1);
        line!("| 匀速时间 | {:.3} s |", kin.tc);
        line!("| 最大速度 | {} |", velocity_text(kin.v, rotary));
        line!(
            "| 最大加速度 | {:.3} {} |",
            // 内部加速度是 rad/s²；换算到 rpm/s：rad/s² × (60/2π)。
            if rotary {
                kin.a / std::f64::consts::TAU * 60.0
            } else {
                kin.a
            },
            if rotary { "rpm/s".to_owned() } else { "m/s²".to_owned() }
        );
        line!("| 持续值（RMS） | {:.4} {force_unit} |", profile.f_rms);
        line!("| 峰值 | {:.4} {force_unit} |", profile.f_peak);
        line!("| 加速段力 | {:.4} {force_unit} |", profile.f_acc);
        line!("| 匀速段力 | {:.4} {force_unit} |", profile.f_cruise);
        line!("| 减速段力 | {:.4} {force_unit} |", profile.f_dec);
        line!("| 停顿段力 | {:.4} {force_unit} |", profile.f_dwell);
        line!("| 匀速段功率 | {:.1} W |", profile.p_rated);
        line!("| 峰值功率 | {:.1} W |", profile.p_max);
    } else {
        line!("| 状态 | 工况未产生有效运动结果 |");
    }
    if let Some(requirement) = state.computed.requirement {
        line!(
            "| 选型需求（含安全系数） | 持续 {:.4} / 峰值 {:.4} {force_unit} |",
            requirement.needed_rms(),
            requirement.needed_peak()
        );
    }
    line!("");

    // ---- 3. 推荐型号 ----
    let count = state.computed.recs.len();
    line!("## 3. 推荐型号（共 {count} 个，按余量排序）");
    line!("");
    if count == 0 {
        line!("当前安全系数或余量窗口下没有匹配型号。");
    } else {
        line!("| # | 型号 | 余量 | 利用率 |");
        line!("| --- | --- | --- | --- |");
        for (index, row) in state.computed.recs.iter().take(10).enumerate() {
            line!(
                "| {} | {} | {:.1}% | {:.0}% |",
                index + 1,
                row.name,
                row.margin_pct,
                row.utilization * 100.0
            );
        }
        if count > 10 {
            line!("");
            line!("仅列出前 10 个，完整列表请在程序中查看。");
        }
    }
    line!("");

    // ---- 4. 选中电机 ----
    line!("## 4. 选中电机");
    line!("");
    if let Some(selected) = &state.computed.selected {
        let motor = &selected.motor;
        line!("### 规格");
        line!("");
        line!("| 项目 | 值 |");
        line!("| --- | --- |");
        line!("| 型号 | {} |", motor.name);
        line!("| 类型 | {} |", kind_label(motor.kind));
        line!("| 冷却 | {} |", construction_label(motor.construction));
        line!("| 持续能力 | {:.3} {force_unit} |", motor.cont);
        line!("| 峰值能力 | {:.3} {force_unit} |", motor.peak);
        line!("| 电机常数 | {:.3} |", motor.km);
        line!("| 力/力矩常数 | {:.3} |", motor.kf);
        line!("| 反电动势常数 | {:.3} |", motor.ke);
        line!("| 连续电流 | {:.2} Arms |", motor.i_cont);
        line!("| 峰值电流 | {:.2} Arms |", motor.i_peak);
        line!("| 相电阻 | {:.3} Ω |", motor.resist);
        line!("| 相电感 | {:.3} mH |", motor.induct);
        line!("| 额定功率 | {:.1} W |", motor.rated_w);
        line!("| 最大功率 | {:.1} W |", motor.max_w);
        if rotary {
            line!("| 转子惯量 | {:.6} kg·m² |", motor.inertia);
            line!("| 最高转速 | {:.0} rpm |", motor.max_speed);
        } else {
            line!("| 动子质量 | {:.3} kg |", motor.mover_mass);
            line!("| 线圈长度 | {:.1} mm |", motor.length);
            line!("| 最高速度 | {:.2} m/s |", motor.max_speed);
        }
        line!("");
        line!("### 应用值");
        line!("");
        line!("| 项目 | 值 |");
        line!("| --- | --- |");
        line!("| 需求持续值 | {:.4} {force_unit} |", selected.need_rms);
        line!("| 需求峰值 | {:.4} {force_unit} |", selected.need_peak);
        line!("| 持续利用率 | {:.1}% |", selected.util_cont * 100.0);
        line!("| 峰值利用率 | {:.1}% |", selected.util_peak * 100.0);
        line!("| 实际余量 | {:.1}% |", selected.margin_pct);
        line!("| 安全系数 | {:.2}× |", state.inputs.safety_factor());
        if motor.kf.abs() > 1e-9 {
            line!("| 估算持续电流 | {:.2} Arms |", selected.need_rms / motor.kf);
            line!("| 估算峰值电流 | {:.2} Arms |", selected.need_peak / motor.kf);
        }
    } else {
        line!("当前没有选中的型号。");
    }
    line!("");

    // ---- 5. 驱动器候选 ----
    let drives = state.matched_drives();
    line!("## 5. 驱动器候选");
    line!("");
    line!(
        "匹配条件：母线 {:.0} V · {} 相 · 按电机额定与应用电流。",
        state.drive_settings.bus_voltage, state.drive_settings.phase
    );
    line!("");
    if drives.is_empty() {
        line!("没有满足电流 / 电压 / 相数条件的驱动器。");
    } else {
        line!("| 型号 | 持续电流 | 峰值电流 | 额定电压 | 相数 | 余量 |");
        line!("| --- | --- | --- | --- | --- | --- |");
        for item in drives.iter().take(10) {
            line!(
                "| {} | {:.1} Arms | {:.1} Arms | {:.0} V | {} 相 | {:.0}% |",
                item.drive.name,
                item.drive.cont,
                item.drive.peak,
                item.drive.voltage,
                item.drive.phase,
                item.margin * 100.0
            );
        }
    }
    line!("");

    // ---- 6. 方案对比 ----
    if state.mode == Mode::Solution {
        line!("## 6. 方案对比");
        line!("");
        if state.schemes.is_empty() {
            line!("尚未保存方案。");
        } else {
            line!("| 方案 | 页面 | 电机 | 持续 | 峰值 | 周期 s | 余量 |");
            line!("| --- | --- | --- | --- | --- | --- | --- |");
            for scheme in &state.schemes {
                line!(
                    "| {} | {} | {} | {:.3} | {:.3} | {:.2} | {:.1}% |",
                    scheme.name,
                    scheme.tab.label(),
                    scheme
                        .motor
                        .as_ref()
                        .map(|motor| motor.name.as_str())
                        .unwrap_or("未选型"),
                    scheme.rms,
                    scheme.peak,
                    scheme.cycle,
                    scheme.margin
                );
            }
        }
        line!("");
    }

    // ---- 提示与警告；方案对比模式才有第 6 节，因此编号随之顺延 ----
    let warning_section = if state.mode == Mode::Solution { 7 } else { 6 };
    line!("## {warning_section}. 提示与警告");
    line!("");
    if state.computed.errors.is_empty() && state.computed.warnings.is_empty() {
        line!("无。");
    } else {
        for error in &state.computed.errors {
            line!("- 错误：{error}");
        }
        for warning in &state.computed.warnings {
            line!("- 提示：{warning}");
        }
    }
    line!("");

    // ---- 说明 ----
    line!("## 说明");
    line!("");
    line!("- 数据来源：程序内置的厂家型号资料库；正式选型请以厂家最新样本为准。");
    line!("- 持续值采用加速、匀速、减速、停顿四段的均方根；峰值取四段绝对值的最大值。");
    line!("- S-曲线按等效加速时间模型计算，属于工程近似。");
    line!("- 报告由本地程序生成，未上传任何数据。");
    out
}

fn velocity_text(v: f64, rotary: bool) -> String {
    if rotary {
        format!("{:.1} rpm", v / std::f64::consts::TAU * 60.0)
    } else {
        format!("{:.4} m/s", v)
    }
}

/// 文件名用 UTC 时间戳，避免同名覆盖。
fn timestamp() -> String {
    let (year, month, day, hour, minute, second) = utc_now();
    format!("{year:04}{month:02}{day:02}-{hour:02}{minute:02}{second:02}")
}

fn timestamp_display() -> String {
    let (year, month, day, hour, minute, second) = utc_now();
    format!("{year:04}-{month:02}-{day:02} {hour:02}:{minute:02}:{second:02}")
}

/// 当前 UTC 时间；不引入额外时间库，用日历换算得到年月日时分秒。
fn utc_now() -> (i64, u32, u32, u32, u32, u32) {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0) as i64;
    let days = seconds.div_euclid(86_400);
    let secs_of_day = seconds.rem_euclid(86_400) as u32;
    let (year, month, day) = civil_from_days(days);
    (
        year,
        month,
        day,
        secs_of_day / 3600,
        (secs_of_day % 3600) / 60,
        secs_of_day % 60,
    )
}

/// 把 1970-01-01 起的天数换算为公历日期（Howard Hinnant 算法）。
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = if month_prime < 10 {
        month_prime + 3
    } else {
        month_prime - 9
    };
    let year = if month <= 2 { year + 1 } else { year };
    (year, month as u32, day as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> AppState {
        AppState::from_catalog(crate::data::repo::load_in_memory().expect("内存数据库"))
    }

    #[test]
    fn report_contains_all_key_sections() {
        let state = state();
        let text = render(&state);
        assert!(text.contains("# MotionSizer Studio 选型报告"));
        assert!(text.contains("## 1. 工况输入"));
        assert!(text.contains("## 2. 运动与力学结果"));
        assert!(text.contains("## 3. 推荐型号"));
        assert!(text.contains("## 4. 选中电机"));
        assert!(text.contains("## 5. 驱动器候选"));
        assert!(text.contains("## 6. 提示与警告"));
        assert!(text.contains(&state.computed.selected.as_ref().unwrap().motor.name));
    }

    #[test]
    fn combined_report_lists_curve_details() {
        let mut state = state();
        state.set_mode(Mode::Combined);
        state.add_current_curves(2);
        state.recompute();
        let text = render(&state);
        assert!(text.contains("参与汇总曲线 | 2 条"));
        assert!(text.contains("### 曲线明细"));
    }

    #[test]
    fn solution_report_includes_schemes_and_numbering() {
        let mut state = state();
        state.set_mode(Mode::Solution);
        state.recompute();
        state.save_scheme();
        let text = render(&state);
        assert!(text.contains("## 6. 方案对比"));
        assert!(text.contains("## 7. 提示与警告"));
        assert!(text.contains("方案 1"));
    }

    #[test]
    fn export_to_selected_path_uses_that_location() {
        let state = state();
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("motorsizer-picked-{nanos}"));
        let path = dir.join("我的选型报告.md");
        let saved = export_to_path(&state, &path).expect("导出到指定路径");
        assert_eq!(saved, path);
        let text = std::fs::read_to_string(&path).expect("读取报告");
        assert!(text.contains("# MotionSizer Studio 选型报告"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rotary_report_converts_acceleration_to_rpm_per_second() {
        let mut state = state();
        state.set_tab(Tab::Rotary);
        state.recompute();
        let acceleration = state.computed.profile.as_ref().unwrap().kin.a;
        let expected = acceleration / std::f64::consts::TAU * 60.0;
        let text = render(&state);
        assert!(text.contains(&format!("{expected:.3} rpm/s")));
        // rad/s² → rpm/s 乘的是 60/2π；1 rev/s² = 2π rad/s² = 60 rpm/s。
        // 默认旋转工况（1 rev / 2 s / 加速占 1/3）→ a=9TAU/8 → 67.5 rpm/s。
        assert!((acceleration / std::f64::consts::TAU * 60.0 - 67.5).abs() < 1e-6);
    }

    #[test]
    fn export_writes_markdown_file() {
        let state = state();
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("motorsizer-report-{nanos}"));
        let path = export_to(&state, &dir).expect("导出报告");
        let text = std::fs::read_to_string(&path).expect("读取报告");
        assert!(text.contains("选型报告"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn utc_calendar_conversion_is_correct() {
        // 1970-01-01、2000-01-01、2026-11-12 三个已知日期。
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(10_957), (2000, 1, 1));
        assert_eq!(civil_from_days(20_769), (2026, 11, 12));
    }

    #[test]
    fn velocity_text_uses_rpm_for_rotary() {
        let v = std::f64::consts::TAU; // 1 rev/s
        assert_eq!(velocity_text(v, true), "60.0 rpm");
        assert_eq!(velocity_text(1.0, false), "1.0000 m/s");
    }
}
