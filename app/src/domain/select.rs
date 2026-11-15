//! 选型：把单曲线需求乘以安全系数，按持续/峰值能力计算余量并排序。

use std::cmp::Ordering;

use crate::data::motor::{Construction, Motor, MotorKind};

use super::profile::Profile;

pub const DEFAULT_SAFETY_FACTOR: f64 = 1.5;

const EPS: f64 = 1e-9;

#[derive(Clone, Copy, Debug)]
pub struct Requirement {
    /// 未乘安全系数的持续均方根需求。
    pub rms: f64,
    /// 未乘安全系数的峰值需求。
    pub peak: f64,
    pub safety_factor: f64,
}

impl Requirement {
    pub fn new(rms: f64, peak: f64, safety_factor: f64) -> Self {
        let rms = if rms.is_finite() { rms.max(0.0) } else { 0.0 };
        let peak = if peak.is_finite() { peak.max(0.0) } else { 0.0 };
        let safety_factor = if safety_factor.is_finite() && safety_factor > 0.0 {
            safety_factor
        } else {
            1.0
        };
        Self {
            rms,
            peak,
            safety_factor,
        }
    }

    pub fn from_profile(profile: &Profile, safety_factor: f64) -> Self {
        Self::new(profile.f_rms, profile.f_peak, safety_factor)
    }

    pub fn needed_rms(&self) -> f64 {
        self.rms * self.safety_factor
    }

    pub fn needed_peak(&self) -> f64 {
        self.peak * self.safety_factor
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Filters {
    pub ironless: bool,
    pub ironcore: bool,
    pub voice_coil: bool,
    pub standard_cooled: bool,
    pub water_cooled: bool,
}

impl Filters {
    pub fn all() -> Self {
        Self {
            ironless: true,
            ironcore: true,
            voice_coil: true,
            standard_cooled: true,
            water_cooled: true,
        }
    }

    pub fn matches(&self, motor: &Motor) -> bool {
        // 旋转电机没有直线电机的结构/冷却分类。
        if motor.kind == MotorKind::Rotary {
            return true;
        }
        // 每组复选框的含义是“只显示勾选的类别”；整组都不勾选表示该组不加限制。
        let kind_enabled = self.ironless || self.ironcore || self.voice_coil;
        let kind_ok = !kind_enabled
            || match motor.kind {
                MotorKind::Ironless => self.ironless,
                MotorKind::IronCore => self.ironcore,
                MotorKind::VoiceCoil => self.voice_coil,
                MotorKind::Rotary => true,
            };
        // 风冷即自然冷却，不再单独筛选，与普通冷却合并。
        let cooling_enabled = self.standard_cooled || self.water_cooled;
        let cooling_ok = !cooling_enabled
            || match motor.construction {
                Construction::Standard | Construction::AirCooled => self.standard_cooled,
                Construction::WaterCooled => self.water_cooled,
            };
        kind_ok && cooling_ok
    }
}

impl Default for Filters {
    fn default() -> Self {
        // 默认不勾选任何筛选条件：两组都不勾选即显示全部型号。
        Self {
            ironless: false,
            ironcore: false,
            voice_coil: false,
            standard_cooled: false,
            water_cooled: false,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct MarginWindow {
    pub low_pct: f64,
    pub high_pct: f64,
    pub use_upper: bool,
}

impl Default for MarginWindow {
    fn default() -> Self {
        Self {
            low_pct: 0.0,
            high_pct: 200.0,
            use_upper: false,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Recommendation<'a> {
    pub motor: &'a Motor,
    /// 相对需求能力的余量百分比：`(min(持续能力/需求, 峰值能力/需求)-1)*100`。
    pub margin_pct: f64,
    /// 实际使用率 0–1，超过 1 表示能力不足（通常已被过滤）。
    pub utilization: f64,
    pub needed_rms: f64,
    pub needed_peak: f64,
}

fn capacity(available: f64, needed: f64) -> f64 {
    if needed <= EPS {
        f64::INFINITY
    } else if available > 0.0 {
        available / needed
    } else {
        0.0
    }
}

/// 返回满足安全系数与余量窗口的电机，按余量从小到大（即利用率从高到低）排序。
pub fn recommend<'a>(
    motors: &'a [Motor],
    requirement: &Requirement,
    filters: &Filters,
    window: &MarginWindow,
) -> Vec<Recommendation<'a>> {
    let needed_rms = requirement.needed_rms();
    let needed_peak = requirement.needed_peak();
    if needed_rms <= EPS && needed_peak <= EPS {
        return Vec::new();
    }

    let mut out = Vec::new();
    for motor in motors {
        if !filters.matches(motor) {
            continue;
        }

        let cap_rms = capacity(motor.cont, needed_rms);
        let cap_peak = capacity(motor.peak, needed_peak);
        let cap = cap_rms.min(cap_peak);
        // 持续或峰值能力不足的型号不进入推荐列表。
        if cap < 1.0 - EPS {
            continue;
        }

        let margin_pct = (cap - 1.0) * 100.0;
        if margin_pct < window.low_pct - EPS {
            continue;
        }
        if window.use_upper && margin_pct > window.high_pct + EPS {
            continue;
        }

        let util_rms = if needed_rms <= EPS {
            0.0
        } else if motor.cont > 0.0 {
            needed_rms / motor.cont
        } else {
            f64::INFINITY
        };
        let util_peak = if needed_peak <= EPS {
            0.0
        } else if motor.peak > 0.0 {
            needed_peak / motor.peak
        } else {
            f64::INFINITY
        };
        let utilization = util_rms.max(util_peak);

        out.push(Recommendation {
            motor,
            margin_pct,
            utilization,
            needed_rms,
            needed_peak,
        });
    }

    // 余量越小越贴近需求，排在越前面；利用率与余量互补
    // （利用率 = 1/(1+余量)），所以余量升序同时也是利用率降序。
    // 余量相同时按型号名排序，保证列表顺序稳定可复现。
    out.sort_by(|a, b| {
        a.margin_pct
            .partial_cmp(&b.margin_pct)
            .unwrap_or(Ordering::Equal)
            .then_with(|| a.motor.name.cmp(&b.motor.name))
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn motor(
        name: &str,
        cont: f64,
        peak: f64,
        kind: MotorKind,
        construction: Construction,
    ) -> Motor {
        Motor {
            name: name.into(),
            cont,
            peak,
            km: 0.0,
            kf: 1.0,
            ke: 1.0,
            i_cont: 0.0,
            i_peak: 0.0,
            induct: 0.0,
            resist: 0.0,
            rated_w: 0.0,
            max_w: 0.0,
            mover_mass: 0.0,
            length: 0.0,
            inertia: 0.0,
            max_speed: 0.0,
            kind,
            construction,
        }
    }

    #[test]
    fn margin_is_limited_by_peak_capacity() {
        let ms = vec![motor(
            "M",
            10.0,
            20.0,
            MotorKind::Ironless,
            Construction::Standard,
        )];
        let recs = recommend(
            &ms,
            &Requirement::new(4.0, 10.0, 1.5),
            &Filters::all(),
            &MarginWindow::default(),
        );
        assert_eq!(recs.len(), 1);
        assert!((recs[0].needed_rms - 6.0).abs() < 1e-9);
        assert!((recs[0].needed_peak - 15.0).abs() < 1e-9);
        assert!((recs[0].margin_pct - 33.3333333333333).abs() < 1e-6);
        assert!((recs[0].utilization - 0.75).abs() < 1e-9);
    }

    #[test]
    fn insufficient_motor_is_excluded() {
        let ms = vec![motor(
            "M",
            5.0,
            30.0,
            MotorKind::Ironless,
            Construction::Standard,
        )];
        let recs = recommend(
            &ms,
            &Requirement::new(4.0, 10.0, 1.5),
            &Filters::all(),
            &MarginWindow::default(),
        );
        assert!(recs.is_empty());
    }

    #[test]
    fn exact_capacity_has_zero_margin() {
        let ms = vec![motor(
            "M",
            6.0,
            15.0,
            MotorKind::Ironless,
            Construction::Standard,
        )];
        let recs = recommend(
            &ms,
            &Requirement::new(4.0, 10.0, 1.5),
            &Filters::all(),
            &MarginWindow::default(),
        );
        assert_eq!(recs.len(), 1);
        assert!(recs[0].margin_pct.abs() < 1e-9);
        assert!((recs[0].utilization - 1.0).abs() < 1e-9);
    }

    #[test]
    fn margin_window_applies_lower_and_optional_upper() {
        let ms = vec![
            motor("A", 6.0, 100.0, MotorKind::Ironless, Construction::Standard),
            motor("B", 7.8, 100.0, MotorKind::Ironless, Construction::Standard),
            motor("C", 12.0, 100.0, MotorKind::Ironless, Construction::Standard),
            motor("D", 30.0, 100.0, MotorKind::Ironless, Construction::Standard),
        ];
        let recs = recommend(
            &ms,
            &Requirement::new(4.0, 10.0, 1.5),
            &Filters::all(),
            &MarginWindow {
                low_pct: 30.0,
                high_pct: 100.0,
                use_upper: true,
            },
        );
        assert_eq!(recs.len(), 2, "{:?}", recs.iter().map(|r| (&r.motor.name, r.margin_pct)).collect::<Vec<_>>());
        // 余量升序：先出现 30% 的贴近型号，再是 100% 的富余型号。
        assert!((recs[0].margin_pct - 30.0).abs() < 1e-6);
        assert!((recs[1].margin_pct - 100.0).abs() < 1e-6);
    }

    #[test]
    fn results_are_sorted_by_margin_ascending() {
        // 持续能力都够用，峰值能力决定余量：small 更贴近需求，应排在前面。
        let ms = vec![
            motor("small", 8.0, 16.0, MotorKind::Ironless, Construction::Standard),
            motor("large", 12.0, 20.0, MotorKind::Ironless, Construction::Standard),
        ];
        let recs = recommend(
            &ms,
            &Requirement::new(4.0, 10.0, 1.5),
            &Filters::all(),
            &MarginWindow::default(),
        );
        assert_eq!(recs.len(), 2);
        assert_eq!(recs[0].motor.name, "small");
        assert_eq!(recs[1].motor.name, "large");
        // 余量升序 ⇔ 利用率降序
        assert!(recs[0].utilization > recs[1].utilization);
    }

    #[test]
    fn filters_are_applied_to_linear_motors() {
        let ms = vec![
            motor("ironless", 10.0, 30.0, MotorKind::Ironless, Construction::Standard),
            motor("ironcore", 10.0, 30.0, MotorKind::IronCore, Construction::WaterCooled),
            motor("voice", 10.0, 30.0, MotorKind::VoiceCoil, Construction::AirCooled),
        ];
        let filters = Filters {
            ironless: true,
            ironcore: false,
            voice_coil: false,
            standard_cooled: true,
            water_cooled: false,
        };
        let recs = recommend(
            &ms,
            &Requirement::new(4.0, 10.0, 1.5),
            &filters,
            &MarginWindow::default(),
        );
        assert_eq!(recs.len(), 1);
        assert_eq!(recs[0].motor.name, "ironless");
    }

    #[test]
    fn empty_filters_keep_every_linear_kind() {
        let ms = vec![
            motor("ironless", 10.0, 30.0, MotorKind::Ironless, Construction::Standard),
            motor("ironcore", 10.0, 30.0, MotorKind::IronCore, Construction::WaterCooled),
            motor("voice", 10.0, 30.0, MotorKind::VoiceCoil, Construction::AirCooled),
        ];
        let recs = recommend(
            &ms,
            &Requirement::new(4.0, 10.0, 1.5),
            &Filters::default(),
            &MarginWindow::default(),
        );
        assert_eq!(recs.len(), 3);
    }

    #[test]
    fn air_cooled_counts_as_natural_cooling() {
        let ms = vec![
            motor("air", 10.0, 30.0, MotorKind::Ironless, Construction::AirCooled),
            motor("water", 10.0, 30.0, MotorKind::Ironless, Construction::WaterCooled),
        ];
        let filters = Filters {
            ironless: true,
            ironcore: false,
            voice_coil: false,
            standard_cooled: true,
            water_cooled: false,
        };
        let recs = recommend(
            &ms,
            &Requirement::new(4.0, 10.0, 1.5),
            &filters,
            &MarginWindow::default(),
        );
        assert_eq!(recs.len(), 1);
        assert_eq!(recs[0].motor.name, "air");
    }

    #[test]
    fn zero_requirement_returns_empty() {
        let ms = vec![motor("M", 10.0, 30.0, MotorKind::Ironless, Construction::Standard)];
        let recs = recommend(
            &ms,
            &Requirement::new(0.0, 0.0, 1.5),
            &Filters::all(),
            &MarginWindow::default(),
        );
        assert!(recs.is_empty());
    }

    #[test]
    fn invalid_safety_factor_falls_back_to_one() {
        let r = Requirement::new(4.0, 10.0, -1.0);
        assert!((r.safety_factor - 1.0).abs() < 1e-9);
    }
}
