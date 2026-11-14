//! 运动学求解：梯形/三角形曲线，按任意两个及以上条件求解，并支持 S-曲线近似模型。
//!
//! 约定：
//! - `s` 为行程，`t` 为单次运动时间（不含停顿），`v` 为匀速速度，`a` 为峰值加速度。
//! - 梯形：`s = v*(t1 + tc)`，`T = 2*t1 + tc`，其中 `t1` 为加速段总时间。
//! - 三角形：`tc = 0`，`T = 2*t1`，`s = v*t1`。
//! - S-曲线：把加速段等效为 `t1_eff = (1 + p/4)*v/a`；距离/周期公式与梯形一致，
//!   峰值加速度由 `a = (1 + p/4)*v/t1_eff` 反算。

const EPS: f64 = 1e-9;
const TOL: f64 = 5e-5;

/// 允许的最大峰值加速度（m/s² 或 rad/s²）。
pub const MAX_ACCELERATION: f64 = 100_000.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    Trapezoid,
    Triangle,
}

impl Shape {
    pub fn label(self) -> &'static str {
        match self {
            Shape::Trapezoid => "梯形",
            Shape::Triangle => "三角形",
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct MotionInput {
    pub s: f64,
    pub t: f64,
    pub v: f64,
    pub a: f64,
    pub shape: Shape,
    pub scurve: bool,
    /// S-曲线百分比：0 接近梯形，100 为完全 S-曲线。
    pub scurve_percent: f64,
    /// 梯形加速段（或减速段）时间与单次运动时间的比 1:N。
    ///
    /// 与原软件“加速时间比运行时间”一致：N=3 时加速、匀速、减速各占 1/3；
    /// N=2 时没有匀速段，退化为三角形。N 小于 2 无解，按 2 处理。
    pub time_ratio: f64,
}

#[derive(Clone, Copy, Debug)]
pub struct Kinematics {
    pub s: f64,
    pub t_run: f64,
    pub v: f64,
    pub a: f64,
    /// 加速段总时间（S-曲线时为等效时间，包含 jerk 过渡）。
    pub t1: f64,
    pub tc: f64,
    pub shape: Shape,
    pub scurve: bool,
    /// 峰值 jerk；非 S-曲线时为 0。
    pub jerk: f64,
    /// jerk 过渡时间；非 S-曲线时为 0。
    pub t_jerk: f64,
}

#[derive(Clone, Debug, Default)]
pub struct SolveOutcome {
    pub kin: Option<Kinematics>,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
}

struct Parts {
    s: f64,
    t_run: f64,
    v: f64,
    a: f64,
    t1: f64,
    tc: f64,
}

fn part(s: f64, v: f64, a: f64, t1: f64, tc: f64) -> Parts {
    Parts {
        s,
        t_run: 2.0 * t1 + tc,
        v,
        a,
        t1,
        tc,
    }
}

fn positive(name: &str, value: f64, errors: &mut Vec<String>) -> bool {
    if !value.is_finite() {
        errors.push(format!("{name} 必须是有效数值"));
        false
    } else if value < 0.0 {
        errors.push(format!("{name} 不能为负数"));
        false
    } else {
        value > EPS
    }
}

/// 求解运动学。至少给出 S/T/V/A 中的两个量；给出三个或四个时会做一致性校验。
pub fn solve(input: MotionInput) -> SolveOutcome {
    let mut errors = Vec::new();
    let mut warnings = Vec::new();

    let hs = positive("行程", input.s, &mut errors);
    let ht = positive("运动时间", input.t, &mut errors);
    let hv = positive("速度", input.v, &mut errors);
    let ha = positive("加速度", input.a, &mut errors);

    // 加速度上限：先检查显式输入，求解后再检查计算值。
    if ha && input.a > MAX_ACCELERATION {
        errors.push(format!(
            "加速度 {} 超过上限 {}",
            fmt_num(input.a),
            fmt_num(MAX_ACCELERATION)
        ));
    }

    let count = [hs, ht, hv, ha].iter().filter(|x| **x).count();
    if count < 2 {
        errors.push("至少需要输入两个条件：行程、时间、速度、加速度".into());
        return SolveOutcome {
            kin: None,
            errors,
            warnings,
        };
    }

    let p = if input.scurve {
        let percent = input.scurve_percent;
        if !percent.is_finite() || !(0.0..=100.0).contains(&percent) {
            warnings.push("S-曲线百分比应在 0–100 之间，已按有效范围处理".into());
        }
        percent.clamp(0.0, 100.0) / 100.0
    } else {
        0.0
    };

    if input.scurve && p <= EPS {
        warnings.push(format!(
            "S-曲线百分比为 0，已按{}计算",
            input.shape.label()
        ));
    }

    // 有 S-曲线时，所有由 V/A 求 t1 的分支都需要这个系数。
    let k = if p > 0.0 { 1.0 + p / 4.0 } else { 1.0 };
    // 加速段（或减速段）最多占运动时间的一半：N<2 时曲线无解。
    let time_ratio = if input.time_ratio.is_finite() && input.time_ratio >= 2.0 {
        input.time_ratio
    } else {
        2.0
    };

    let result = match input.shape {
        Shape::Trapezoid => {
            solve_trapezoid(&input, hs, ht, hv, ha, k, time_ratio, &mut warnings)
        }
        Shape::Triangle => solve_triangle(&input, hs, ht, hv, ha, k, &mut warnings),
    };

    let parts = match result {
        Ok(x) => x,
        Err(err) => {
            errors.push(err);
            return SolveOutcome {
                kin: None,
                errors,
                warnings,
            };
        }
    };

    let mut kin = Kinematics {
        s: parts.s,
        t_run: parts.t_run,
        v: parts.v,
        a: parts.a,
        t1: parts.t1,
        tc: parts.tc,
        shape: input.shape,
        scurve: false,
        jerk: 0.0,
        t_jerk: 0.0,
    };

    if kin.tc < 0.0 && kin.tc > -TOL {
        kin.tc = 0.0;
    }
    if p > 0.0 && kin.v > EPS && kin.a > EPS {
        kin.scurve = true;
        kin.jerk = 4.0 * kin.a * kin.a / (p * kin.v);
        kin.t_jerk = if kin.jerk > 0.0 {
            kin.a / kin.jerk
        } else {
            0.0
        };
    }

    if !kin.s.is_finite()
        || !kin.t_run.is_finite()
        || !kin.v.is_finite()
        || !kin.a.is_finite()
        || !kin.t1.is_finite()
        || !kin.tc.is_finite()
    {
        errors.push("求解结果不是有效数值，请检查输入".into());
    } else if kin.s <= EPS || kin.v <= EPS || kin.a <= EPS || kin.t_run <= EPS {
        errors.push("输入条件无法形成有效运动：结果为零或负数".into());
    } else if kin.tc < -TOL {
        errors.push("给定距离不足以完成加减速：巡航时间为负".into());
    } else if kin.a > MAX_ACCELERATION + TOL {
        errors.push(format!(
            "计算得到的加速度 {} 超过上限 {}",
            fmt_num(kin.a),
            fmt_num(MAX_ACCELERATION)
        ));
    }

    let consistency = check_consistency(&input, &kin, hs, ht, hv, ha);
    if !consistency.is_empty() {
        errors.extend(consistency);
    }

    if errors.is_empty() {
        SolveOutcome {
            kin: Some(kin),
            errors,
            warnings,
        }
    } else {
        SolveOutcome {
            kin: None,
            errors,
            warnings,
        }
    }
}

fn solve_trapezoid(
    input: &MotionInput,
    hs: bool,
    ht: bool,
    hv: bool,
    ha: bool,
    k: f64,
    time_ratio: f64,
    warnings: &mut Vec<String>,
) -> Result<Parts, String> {
    let s = input.s;
    let t = input.t;
    let v = input.v;
    let a = input.a;

    match (hs, ht, hv, ha) {
        // 两个已知量
        (true, true, false, false) => {
            // 加速（减速）段占运动时间 1/N：T=2*t1+tc，t1=T/N。
            let t1 = t / time_ratio;
            let tc = t - 2.0 * t1;
            let v = s / (t1 + tc);
            let a = k * v / t1;
            let ratio_text = if (time_ratio - time_ratio.round()).abs() < 1e-6 {
                format!("{}", time_ratio.round() as i64)
            } else {
                format!("{time_ratio:.2}")
            };
            warnings.push(format!(
                "仅给出行程与时间，按加速:运动 = 1:{ratio_text} 估算速度和加速度"
            ));
            Ok(part(s, v, a, t1, tc))
        }
        (true, false, true, false) => {
            // 由 S=V·(t1+tc) 且 t1:T=1:N 反算：t1=S/(V·(N−1))，tc=(N−2)·t1。
            let t1 = s / (v * (time_ratio - 1.0));
            let tc = (time_ratio - 2.0) * t1;
            let a = k * v / t1;
            warnings.push(format!(
                "仅给出行程与速度，按加速:运动 = 1:{} 估算时间和加速度",
                fmt_num(time_ratio)
            ));
            Ok(part(s, v, a, t1, tc))
        }
        (true, false, false, true) => {
            let t1 = (k * s / a).sqrt();
            let v = s / t1;
            warnings.push("仅给出行程与加速度时，结果退化为三角形曲线（无匀速段）".into());
            Ok(part(s, v, a, t1, 0.0))
        }
        (false, true, true, false) => {
            let t1 = t / time_ratio;
            let tc = t - 2.0 * t1;
            let a = k * v / t1;
            let s = v * (t1 + tc);
            warnings.push(format!(
                "仅给出时间与速度，按加速:运动 = 1:{} 估算行程和加速度",
                fmt_num(time_ratio)
            ));
            Ok(part(s, v, a, t1, tc))
        }
        (false, true, false, true) => {
            let t1 = t / 2.0;
            let v = a * t / (2.0 * k);
            let s = v * t / 2.0;
            warnings.push("仅给出时间与加速度时，结果退化为三角形曲线（无匀速段）".into());
            Ok(part(s, v, a, t1, 0.0))
        }
        (false, false, true, true) => {
            let t1 = k * v / a;
            let s = v * t1;
            warnings.push("仅给出速度与加速度时，结果退化为三角形曲线（无匀速段）".into());
            Ok(part(s, v, a, t1, 0.0))
        }

        // 三个或四个已知量：用能直接封闭求解的组合，随后由一致性校验检查其余条件。
        (true, true, true, _) => {
            let t1 = t - s / v;
            if t1 <= EPS {
                return Err("给定时间过短：行程除以速度已超过总时间".into());
            }
            let tc = t - 2.0 * t1;
            if tc < -TOL {
                return Err("给定距离不足以完成加减速：巡航时间为负".into());
            }
            let a = k * v / t1;
            Ok(part(s, v, a, t1, tc.max(0.0)))
        }
        (true, true, false, true) => {
            let disc = t * t - 4.0 * k * s / a;
            if disc < -TOL {
                return Err("行程、时间与加速度组合无解（判别式为负）".into());
            }
            let t1 = (t - disc.max(0.0).sqrt()) / 2.0;
            if t1 <= EPS {
                return Err("行程、时间与加速度组合无解：加速时间为零".into());
            }
            let tc = t - 2.0 * t1;
            if tc < -TOL {
                return Err("给定条件无法形成梯形曲线：巡航时间为负".into());
            }
            let v = a * t1 / k;
            Ok(part(s, v, a, t1, tc.max(0.0)))
        }
        (true, false, true, true) => {
            let t1 = k * v / a;
            let tc = s / v - t1;
            if tc < -TOL {
                return Err("给定距离不足以达到该速度：巡航时间为负".into());
            }
            Ok(part(s, v, a, t1, tc.max(0.0)))
        }
        (false, true, true, true) => {
            let t1 = k * v / a;
            let tc = t - 2.0 * t1;
            if tc < -TOL {
                return Err("给定时间不足以完成加减速：巡航时间为负".into());
            }
            let s = v * (t1 + tc);
            Ok(part(s, v, a, t1, tc.max(0.0)))
        }
        _ => unreachable!("至少两个条件已经在上层校验"),
    }
}

fn solve_triangle(
    input: &MotionInput,
    hs: bool,
    ht: bool,
    hv: bool,
    ha: bool,
    k: f64,
    _warnings: &mut Vec<String>,
) -> Result<Parts, String> {
    let s = input.s;
    let t = input.t;
    let v = input.v;
    let a = input.a;

    // 三角形只有加速/减速两段；任意组合都用最直接的一组公式，
    // 多余条件交给一致性校验。四个条件时优先使用 S/T。
    let pair = if hs && ht {
        (true, true, false, false)
    } else if hs && hv {
        (true, false, true, false)
    } else if hs && ha {
        (true, false, false, true)
    } else if ht && hv {
        (false, true, true, false)
    } else if ht && ha {
        (false, true, false, true)
    } else {
        (false, false, true, true)
    };

    match pair {
        (true, true, false, false) => {
            let t1 = t / 2.0;
            let v = 2.0 * s / t;
            let a = k * v / t1;
            Ok(part(s, v, a, t1, 0.0))
        }
        (true, false, true, false) => {
            let t1 = s / v;
            let a = k * v / t1;
            Ok(part(s, v, a, t1, 0.0))
        }
        (true, false, false, true) => {
            let t1 = (k * s / a).sqrt();
            let v = s / t1;
            Ok(part(s, v, a, t1, 0.0))
        }
        (false, true, true, false) => {
            let t1 = t / 2.0;
            let a = k * v / t1;
            let s = v * t / 2.0;
            Ok(part(s, v, a, t1, 0.0))
        }
        (false, true, false, true) => {
            let t1 = t / 2.0;
            let v = a * t / (2.0 * k);
            let s = v * t / 2.0;
            Ok(part(s, v, a, t1, 0.0))
        }
        (false, false, true, true) => {
            let t1 = k * v / a;
            let s = v * t1;
            Ok(part(s, v, a, t1, 0.0))
        }
        _ => Err("内部错误：三角形求解条件不足".into()),
    }
}

fn check_consistency(
    input: &MotionInput,
    kin: &Kinematics,
    hs: bool,
    ht: bool,
    hv: bool,
    ha: bool,
) -> Vec<String> {
    let mut errors = Vec::new();
    if hs && (kin.s - input.s).abs() > TOL {
        errors.push(format!(
            "行程不一致：输入 {}，计算 {}",
            fmt_num(input.s),
            fmt_num(kin.s)
        ));
    }
    if ht && (kin.t_run - input.t).abs() > TOL {
        errors.push(format!(
            "运动时间不一致：输入 {}，计算 {}",
            fmt_num(input.t),
            fmt_num(kin.t_run)
        ));
    }
    if hv && (kin.v - input.v).abs() > TOL {
        errors.push(format!(
            "速度不一致：输入 {}，计算 {}",
            fmt_num(input.v),
            fmt_num(kin.v)
        ));
    }
    if ha && (kin.a - input.a).abs() > TOL {
        errors.push(format!(
            "加速度不一致：输入 {}，计算 {}",
            fmt_num(input.a),
            fmt_num(kin.a)
        ));
    }
    errors
}

fn fmt_num(x: f64) -> String {
    if x.abs() >= 1.0 {
        format!("{x:.4}")
    } else {
        format!("{x:.6}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(shape: Shape, s: f64, t: f64, v: f64, a: f64) -> MotionInput {
        MotionInput {
            s,
            t,
            v,
            a,
            shape,
            scurve: false,
            scurve_percent: 100.0,
            time_ratio: 3.0,
        }
    }

    fn ok(input: MotionInput) -> Kinematics {
        let out = solve(input);
        assert!(out.errors.is_empty(), "errors: {:?}", out.errors);
        out.kin.expect("should solve")
    }

    fn err(input: MotionInput) -> Vec<String> {
        let out = solve(input);
        assert!(out.kin.is_none(), "expected failure");
        out.errors
    }

    #[test]
    fn trapezoid_sv() {
        let k = ok(input(Shape::Trapezoid, 2.0, 0.0, 1.0, 0.0));
        assert!((k.t1 - 1.0).abs() < 1e-9);
        assert!((k.tc - 1.0).abs() < 1e-9);
        assert!((k.t_run - 3.0).abs() < 1e-9);
        assert!((k.a - 1.0).abs() < 1e-9);
        assert!((k.s - 2.0).abs() < 1e-9);
    }

    #[test]
    fn trapezoid_sva() {
        let k = ok(input(Shape::Trapezoid, 2.0, 0.0, 1.0, 1.0));
        assert!((k.t1 - 1.0).abs() < 1e-9);
        assert!((k.tc - 1.0).abs() < 1e-9);
        assert!((k.t_run - 3.0).abs() < 1e-9);
        assert!((k.a - 1.0).abs() < 1e-9);
    }

    #[test]
    fn trapezoid_sva_infeasible() {
        let errors = err(input(Shape::Trapezoid, 0.5, 0.0, 1.0, 1.0));
        assert!(!errors.is_empty());
    }

    #[test]
    fn trapezoid_st() {
        let k = ok(input(Shape::Trapezoid, 3.0, 3.0, 0.0, 0.0));
        assert!((k.t1 - 1.0).abs() < 1e-9);
        assert!((k.tc - 1.0).abs() < 1e-9);
        assert!((k.v - 1.5).abs() < 1e-9);
        assert!((k.a - 1.5).abs() < 1e-9);
    }

    #[test]
    fn trapezoid_time_ratio() {
        // 与原软件一致：加速段占运动时间 1/N（N=3 → 三段等时）。
        let mut i = input(Shape::Trapezoid, 1.0, 2.0, 0.0, 0.0);
        i.time_ratio = 3.0;
        let k = ok(i);
        assert!((k.t1 - 2.0 / 3.0).abs() < 1e-9);
        assert!((k.tc - 2.0 / 3.0).abs() < 1e-9);
        assert!((k.v - 0.75).abs() < 1e-9);
        assert!((k.a - 1.125).abs() < 1e-9);
        assert!((k.t_run - 2.0).abs() < 1e-9);
    }

    #[test]
    fn legacy_example_matches_reference_profile() {
        // 对照原软件：m=20 kg、s=0.8 m、t=0.5 s、比例 1:3 → 峰值 288 N。
        let mut i = input(Shape::Trapezoid, 0.8, 0.5, 0.0, 0.0);
        i.time_ratio = 3.0;
        let k = ok(i);
        assert!((k.t1 - 1.0 / 6.0).abs() < 1e-9);
        assert!((k.tc - 1.0 / 6.0).abs() < 1e-9);
        assert!((k.v - 2.4).abs() < 1e-9);
        assert!((k.a - 14.4).abs() < 1e-9);
        assert!((k.t_run - 0.5).abs() < 1e-9);
    }

    #[test]
    fn trapezoid_sv_honours_time_ratio() {
        // S+V 也要遵守 1:N：N=5 → t1 占 1/5，匀速段占 3/5。
        let mut i = input(Shape::Trapezoid, 2.0, 0.0, 1.0, 0.0);
        i.time_ratio = 5.0;
        let k = ok(i);
        assert!((k.t1 - 0.5).abs() < 1e-9);
        assert!((k.tc - 1.5).abs() < 1e-9);
        assert!((k.t_run - 2.5).abs() < 1e-9);
        assert!((k.a - 2.0).abs() < 1e-9);
        assert!((k.s - 2.0).abs() < 1e-9);
    }

    #[test]
    fn trapezoid_tv() {
        let k = ok(input(Shape::Trapezoid, 0.0, 3.0, 3.0, 0.0));
        assert!((k.t1 - 1.0).abs() < 1e-9);
        assert!((k.tc - 1.0).abs() < 1e-9);
        assert!((k.a - 3.0).abs() < 1e-9);
        assert!((k.s - 6.0).abs() < 1e-9);
    }

    #[test]
    fn trapezoid_sta() {
        let k = ok(input(Shape::Trapezoid, 6.0, 5.0, 0.0, 1.0));
        assert!((k.t1 - 2.0).abs() < 1e-9);
        assert!((k.tc - 1.0).abs() < 1e-9);
        assert!((k.v - 2.0).abs() < 1e-9);
        assert!((k.t_run - 5.0).abs() < 1e-9);
    }

    #[test]
    fn trapezoid_stv() {
        let k = ok(input(Shape::Trapezoid, 6.0, 5.0, 2.0, 0.0));
        assert!((k.t1 - 2.0).abs() < 1e-9);
        assert!((k.tc - 1.0).abs() < 1e-9);
        assert!((k.a - 1.0).abs() < 1e-9);
    }

    #[test]
    fn inconsistent_conditions_rejected() {
        let errors = err(input(Shape::Trapezoid, 6.0, 5.0, 2.0, 2.0));
        assert!(errors.iter().any(|e| e.contains("加速度不一致")));
    }

    #[test]
    fn too_few_conditions_rejected() {
        let errors = err(input(Shape::Trapezoid, 2.0, 0.0, 0.0, 0.0));
        assert!(!errors.is_empty());
    }

    #[test]
    fn excessive_acceleration_rejected() {
        let errors = err(input(Shape::Trapezoid, 0.0, 0.0, 1.0, 100_001.0));
        assert!(!errors.is_empty());
    }

    #[test]
    fn triangle_st() {
        let k = ok(input(Shape::Triangle, 2.0, 1.0, 0.0, 0.0));
        assert!((k.t1 - 0.5).abs() < 1e-9);
        assert!((k.v - 4.0).abs() < 1e-9);
        assert!((k.a - 8.0).abs() < 1e-9);
        assert!((k.tc - 0.0).abs() < 1e-9);
    }

    #[test]
    fn triangle_all_pairs() {
        // S+V
        let k = ok(input(Shape::Triangle, 2.0, 0.0, 2.0, 0.0));
        assert!((k.t1 - 1.0).abs() < 1e-9);
        assert!((k.a - 2.0).abs() < 1e-9);
        assert!((k.t_run - 2.0).abs() < 1e-9);
        // S+A
        let k = ok(input(Shape::Triangle, 4.0, 0.0, 0.0, 1.0));
        assert!((k.t1 - 2.0).abs() < 1e-9);
        assert!((k.v - 2.0).abs() < 1e-9);
        // T+V
        let k = ok(input(Shape::Triangle, 0.0, 2.0, 2.0, 0.0));
        assert!((k.t1 - 1.0).abs() < 1e-9);
        assert!((k.a - 2.0).abs() < 1e-9);
        assert!((k.s - 2.0).abs() < 1e-9);
        // T+A
        let k = ok(input(Shape::Triangle, 0.0, 2.0, 0.0, 2.0));
        assert!((k.v - 2.0).abs() < 1e-9);
        assert!((k.s - 2.0).abs() < 1e-9);
        // V+A
        let k = ok(input(Shape::Triangle, 0.0, 0.0, 2.0, 2.0));
        assert!((k.t1 - 1.0).abs() < 1e-9);
        assert!((k.s - 2.0).abs() < 1e-9);
        assert!((k.t_run - 2.0).abs() < 1e-9);
    }

    #[test]
    fn triangle_inconsistent_rejected() {
        let errors = err(input(Shape::Triangle, 2.0, 1.0, 1.0, 0.0));
        assert!(errors.iter().any(|e| e.contains("速度不一致")));
    }

    #[test]
    fn scurve_sv() {
        let mut i = input(Shape::Trapezoid, 2.0, 0.0, 1.0, 0.0);
        i.scurve = true;
        let k = ok(i);
        assert!(k.scurve);
        assert!((k.t1 - 1.0).abs() < 1e-9);
        assert!((k.tc - 1.0).abs() < 1e-9);
        assert!((k.a - 1.25).abs() < 1e-9);
        assert!((k.jerk - 6.25).abs() < 1e-9);
        assert!((k.t_jerk - 0.2).abs() < 1e-9);
    }

    #[test]
    fn scurve_sva() {
        let mut i = input(Shape::Trapezoid, 2.0, 0.0, 1.0, 1.0);
        i.scurve = true;
        let k = ok(i);
        assert!((k.t1 - 1.25).abs() < 1e-9);
        assert!((k.tc - 0.75).abs() < 1e-9);
        assert!((k.t_run - 3.25).abs() < 1e-9);
        assert!((k.jerk - 4.0).abs() < 1e-9);
        assert!((k.t_jerk - 0.25).abs() < 1e-9);
    }

    #[test]
    fn scurve_zero_percent_is_normal_curve() {
        let mut i = input(Shape::Trapezoid, 2.0, 0.0, 1.0, 0.0);
        i.scurve = true;
        i.scurve_percent = 0.0;
        let k = ok(i);
        assert!(!k.scurve);
        assert_eq!(k.jerk, 0.0);
        assert!((k.a - 1.0).abs() < 1e-9);
    }
}
