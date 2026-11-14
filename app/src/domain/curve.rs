//! 多曲线汇总：把多条独立曲线的持续时间、峰值与均方根需求合并。

use super::profile::Profile;

#[derive(Clone, Debug)]
pub struct CurveSegment {
    pub label: String,
    pub profile: Profile,
}

#[derive(Clone, Copy, Debug)]
pub struct Combined {
    pub count: usize,
    /// 各曲线行程之和（直线 m / 旋转 rev 的原始换算单位）。
    pub distance: f64,
    /// 各曲线运动时间之和（不含停顿）。
    pub run_time: f64,
    /// 各曲线周期之和（含停顿）。
    pub cycle_time: f64,
    pub peak: f64,
    pub rms: f64,
    pub v_max: f64,
}

impl Combined {
    /// 加权 RMS：`sqrt(Σ(F_rms² * T_cycle) / ΣT_cycle)`。
    pub fn from_profiles(profiles: &[&Profile]) -> Option<Self> {
        if profiles.is_empty() {
            return None;
        }
        let total_cycle: f64 = profiles.iter().map(|p| p.t_cycle).sum();
        if total_cycle <= 0.0 {
            return None;
        }
        let energy: f64 = profiles
            .iter()
            .map(|p| p.f_rms * p.f_rms * p.t_cycle)
            .sum();
        Some(Self {
            count: profiles.len(),
            distance: profiles.iter().map(|p| p.kin.s).sum(),
            run_time: profiles.iter().map(|p| p.kin.t_run).sum(),
            cycle_time: total_cycle,
            peak: profiles
                .iter()
                .map(|p| p.f_peak.abs())
                .fold(0.0, f64::max),
            rms: (energy / total_cycle).sqrt(),
            v_max: profiles
                .iter()
                .map(|p| p.kin.v)
                .fold(0.0, f64::max),
        })
    }
}

pub fn combine(segments: &[CurveSegment]) -> Option<Combined> {
    let profiles: Vec<&Profile> = segments.iter().map(|s| &s.profile).collect();
    Combined::from_profiles(&profiles)
}

/// 把多条曲线的单个周期首尾相接，生成用于绘图的连续时间轴。
pub fn timeline(
    segments: &[CurveSegment],
    samples_per_curve: usize,
) -> (Vec<(f64, f64)>, Vec<(f64, f64)>) {
    let n = samples_per_curve.max(2);
    let mut velocity = Vec::with_capacity(segments.len() * n);
    let mut force = Vec::with_capacity(segments.len() * n);
    let mut offset = 0.0;
    for (index, segment) in segments.iter().enumerate() {
        let profile = &segment.profile;
        if profile.t_cycle <= 0.0 {
            continue;
        }
        for i in 0..n {
            // 最后一条曲线保留终点，其余曲线之间不重复时间点。
            if i == 0 && index > 0 {
                continue;
            }
            let local = profile.t_cycle * i as f64 / (n - 1) as f64;
            velocity.push((offset + local, profile.velocity_at(local)));
            force.push((offset + local, profile.force_at(local)));
        }
        offset += profile.t_cycle;
    }
    (velocity, force)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::motion::{solve, MotionInput, Shape};

    fn profile(s: f64, t: f64, dwell: f64) -> Profile {
        let out = solve(MotionInput {
            s,
            t,
            v: 0.0,
            a: 0.0,
            shape: Shape::Trapezoid,
            scurve: false,
            scurve_percent: 100.0,
            time_ratio: 3.0,
        });
        Profile::new(out.kin.unwrap(), 5.0, 0.0, 0.0, dwell)
    }

    fn segment(label: &str, profile: Profile) -> CurveSegment {
        CurveSegment {
            label: label.into(),
            profile,
        }
    }

    #[test]
    fn identical_curves_double_time_and_distance() {
        let one = profile(1.0, 2.0, 0.5);
        let combined = combine(&[
            segment("a", one.clone()),
            segment("b", one.clone()),
        ])
        .unwrap();
        assert_eq!(combined.count, 2);
        assert!((combined.cycle_time - one.t_cycle * 2.0).abs() < 1e-9);
        assert!((combined.distance - 2.0).abs() < 1e-9);
        assert!((combined.rms - one.f_rms).abs() < 1e-9);
        assert!((combined.peak - one.f_peak).abs() < 1e-9);
    }

    #[test]
    fn mixed_curves_use_weighted_rms() {
        let a = profile(1.0, 2.0, 0.5);
        let b = profile(2.0, 2.0, 0.5);
        let expected = ((a.f_rms * a.f_rms * a.t_cycle + b.f_rms * b.f_rms * b.t_cycle)
            / (a.t_cycle + b.t_cycle))
            .sqrt();
        let combined = combine(&[segment("a", a.clone()), segment("b", b.clone())]).unwrap();
        assert!((combined.rms - expected).abs() < 1e-9);
        assert!(combined.peak >= b.f_peak.abs());
    }

    #[test]
    fn timeline_offsets_each_curve() {
        let a = profile(1.0, 2.0, 0.5);
        let b = profile(1.0, 2.0, 0.5);
        let (velocity, force) = timeline(
            &[segment("a", a.clone()), segment("b", b.clone())],
            10,
        );
        // 两条曲线各 10 点，第二条起点与第一条终点重合，因此是 19 点。
        assert_eq!(velocity.len(), 19);
        assert_eq!(force.len(), 19);
        let last = velocity.last().unwrap().0;
        assert!((last - (a.t_cycle + b.t_cycle)).abs() < 1e-9);
    }
}
