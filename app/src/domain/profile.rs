//! 单曲线力学：推力/转矩、RMS、峰值、周期与速度/力采样。
//!
//! 符号约定：向上运动为正；`gravity` 为重力沿运动方向的分量（向下运动时为负），
//! `ext_force` 为外部负载力（与运动方向相反时为正）。

use super::motion::Kinematics;

#[derive(Clone, Debug)]
pub struct Profile {
    pub kin: Kinematics,

    pub f_acc: f64,
    pub f_cruise: f64,
    pub f_dec: f64,
    pub f_dwell: f64,
    pub f_peak: f64,
    pub f_rms: f64,

    /// 匀速段功率 W。
    pub p_rated: f64,
    /// 峰值功率 W（按峰值力与最大速度估算）。
    pub p_max: f64,
    /// 单周期时间：运动时间 + 停顿时间。
    pub t_cycle: f64,
}

impl Profile {
    pub fn new(kin: Kinematics, inertia: f64, ext_force: f64, gravity: f64, dwell: f64) -> Self {
        let inertia = if inertia.is_finite() {
            inertia.max(0.0)
        } else {
            0.0
        };
        let ext_force = if ext_force.is_finite() { ext_force } else { 0.0 };
        let gravity = if gravity.is_finite() { gravity } else { 0.0 };
        let dwell = if dwell.is_finite() { dwell.max(0.0) } else { 0.0 };

        let f_acc = ext_force + gravity + inertia * kin.a;
        let f_cruise = ext_force + gravity;
        let f_dec = ext_force + gravity - inertia * kin.a;
        // 停顿期间速度为 0，但外部力/重力仍然作用于电机。
        let f_dwell = ext_force + gravity;

        let f_peak = f_acc
            .abs()
            .max(f_cruise.abs())
            .max(f_dec.abs())
            .max(f_dwell.abs());

        let t_cycle = kin.t_run + dwell;
        let energy = f_acc * f_acc * kin.t1
            + f_cruise * f_cruise * kin.tc
            + f_dec * f_dec * kin.t1
            + f_dwell * f_dwell * dwell;
        let f_rms = if t_cycle > 0.0 {
            (energy / t_cycle).sqrt()
        } else {
            0.0
        };

        let p_rated = f_cruise.abs() * kin.v;
        // 停顿段速度为 0，不产生机械功率；峰值功率只取三个运动段中最大的力。
        let moving_peak = f_acc.abs().max(f_cruise.abs()).max(f_dec.abs());
        let p_max = moving_peak * kin.v;

        Self {
            kin,
            f_acc,
            f_cruise,
            f_dec,
            f_dwell,
            f_peak,
            f_rms,
            p_rated,
            p_max,
            t_cycle,
        }
    }

    /// 速度（直线 m/s，旋转 rad/s）。`t` 从运动开始计时。
    pub fn velocity_at(&self, t: f64) -> f64 {
        let k = &self.kin;
        if t <= 0.0 || t >= k.t_run {
            return 0.0;
        }

        if k.scurve && k.jerk > 0.0 && k.t_jerk > 0.0 {
            if t < k.t1 {
                return self.accel_velocity(t);
            }
            if t < k.t1 + k.tc {
                return k.v;
            }
            // 减速段是加速段的时间镜像。
            return self.accel_velocity(k.t_run - t);
        }

        if t < k.t1 {
            k.v * t / k.t1
        } else if t < k.t1 + k.tc {
            k.v
        } else {
            k.v * (1.0 - (t - k.t1 - k.tc) / k.t1)
        }
        .clamp(0.0, k.v)
    }

    fn accel_velocity(&self, t: f64) -> f64 {
        let k = &self.kin;
        let tj = k.t_jerk;
        if tj <= 0.0 || k.jerk <= 0.0 {
            return (k.v * t / k.t1).clamp(0.0, k.v);
        }
        if t <= 0.0 {
            return 0.0;
        }
        if t < tj {
            return (0.5 * k.jerk * t * t).clamp(0.0, k.v);
        }
        let v_after_jerk = 0.5 * k.jerk * tj * tj;
        if t < k.t1 - tj {
            return (v_after_jerk + k.a * (t - tj)).clamp(0.0, k.v);
        }
        if t >= k.t1 {
            return k.v;
        }
        // 最后一段 jerk 下降：从 v1 开始，加速度线性降到 0。
        let tau = t - (k.t1 - tj);
        let v1 = k.v - 0.5 * k.jerk * tj * tj;
        (v1 + k.a * tau - 0.5 * k.jerk * tau * tau).clamp(0.0, k.v)
    }

    /// 推力/转矩（N 或 N·m）。按阶段取对应值；S-曲线沿用四段近似。
    pub fn force_at(&self, t: f64) -> f64 {
        let k = &self.kin;
        if t < 0.0 || t >= self.t_cycle {
            0.0
        } else if t < k.t1 {
            self.f_acc
        } else if t < k.t1 + k.tc {
            self.f_cruise
        } else if t < k.t_run {
            self.f_dec
        } else {
            self.f_dwell
        }
    }

    pub fn sample_velocity(&self, samples: usize) -> Vec<(f64, f64)> {
        let n = samples.max(2);
        let total = self.t_cycle;
        if total <= 0.0 {
            return vec![(0.0, 0.0)];
        }
        (0..n)
            .map(|i| {
                let t = total * i as f64 / (n - 1) as f64;
                (t, self.velocity_at(t))
            })
            .collect()
    }

    pub fn sample_force(&self, samples: usize) -> Vec<(f64, f64)> {
        let n = samples.max(2);
        let total = self.t_cycle;
        if total <= 0.0 {
            return vec![(0.0, 0.0)];
        }
        (0..n)
            .map(|i| {
                let t = total * i as f64 / (n - 1) as f64;
                (t, self.force_at(t))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::motion::{solve, MotionInput, Shape};

    fn kin(s: f64, v: f64, a: f64) -> Kinematics {
        let out = solve(MotionInput {
            s,
            t: 0.0,
            v,
            a,
            shape: Shape::Trapezoid,
            scurve: false,
            scurve_percent: 100.0,
            time_ratio: 3.0,
        });
        assert!(out.errors.is_empty(), "{:?}", out.errors);
        out.kin.unwrap()
    }

    #[test]
    fn profile_forces_and_rms() {
        let k = kin(2.0, 1.0, 1.0);
        let p = Profile::new(k, 1.0, 5.0, 0.0, 1.0);
        assert!((p.f_acc - 6.0).abs() < 1e-9);
        assert!((p.f_cruise - 5.0).abs() < 1e-9);
        assert!((p.f_dec - 4.0).abs() < 1e-9);
        assert!((p.f_dwell - 5.0).abs() < 1e-9);
        assert!((p.f_peak - 6.0).abs() < 1e-9);
        let expected = (102.0f64 / 4.0).sqrt();
        assert!((p.f_rms - expected).abs() < 1e-9);
        assert!((p.t_cycle - 4.0).abs() < 1e-9);
        assert!((p.p_rated - 5.0).abs() < 1e-9);
        assert!((p.p_max - 6.0).abs() < 1e-9);
    }

    #[test]
    fn legacy_example_force_and_rms_match() {
        // 原软件：20 kg、0.8 m、0.5 s、加速:总时间=1:3、无停顿。
        // F_peak = m·a = 20×14.4 = 288 N；
        // F_rms = 288·√(2×(1/6)/0.5) = 235.151 N。
        let out = solve(MotionInput {
            s: 0.8,
            t: 0.5,
            v: 0.0,
            a: 0.0,
            shape: Shape::Trapezoid,
            scurve: false,
            scurve_percent: 100.0,
            time_ratio: 3.0,
        });
        assert!(out.errors.is_empty(), "{:?}", out.errors);
        let p = Profile::new(out.kin.unwrap(), 20.0, 0.0, 0.0, 0.0);
        assert!((p.f_peak - 288.0).abs() < 1e-6);
        let expected_rms = (288.0f64.powi(2) * (1.0 / 3.0) / 0.5).sqrt();
        assert!((p.f_rms - expected_rms).abs() < 1e-6);

        // 停顿也计入周期后，持续值按整周期均方根下降。
        let dwell = Profile::new(
            out.kin.unwrap(),
            20.0,
            0.0,
            0.0,
            0.5,
        );
        assert!((dwell.t_cycle - 1.0).abs() < 1e-9);
        assert!((dwell.f_rms - 288.0 / 3.0f64.sqrt()).abs() < 1e-6);
    }

    #[test]
    fn dwell_force_includes_external_load() {
        let k = kin(2.0, 1.0, 1.0);
        let p = Profile::new(k, 1.0, 5.0, 0.0, 2.0);
        assert!((p.f_dwell - 5.0).abs() < 1e-9);
        assert!((p.f_peak - 6.0).abs() < 1e-9);
    }

    #[test]
    fn dwell_force_includes_gravity() {
        let k = kin(2.0, 1.0, 1.0);
        let p = Profile::new(k, 1.0, 5.0, 9.81, 0.0);
        assert!((p.f_dwell - 14.81).abs() < 1e-9);
        assert!((p.f_acc - 15.81).abs() < 1e-9);
    }

    #[test]
    fn deceleration_force_can_be_negative() {
        let k = kin(2.0, 1.0, 1.0);
        let p = Profile::new(k, 10.0, 0.0, 0.0, 0.0);
        assert!((p.f_dec + 10.0).abs() < 1e-9);
        assert!((p.f_peak - 10.0).abs() < 1e-9);
    }

    #[test]
    fn trapezoid_velocity_shape() {
        let k = kin(2.0, 1.0, 1.0);
        let p = Profile::new(k, 1.0, 5.0, 0.0, 1.0);
        assert!((p.velocity_at(0.0) - 0.0).abs() < 1e-9);
        assert!((p.velocity_at(0.5) - 0.5).abs() < 1e-9);
        assert!((p.velocity_at(1.0) - 1.0).abs() < 1e-9);
        assert!((p.velocity_at(1.5) - 1.0).abs() < 1e-9);
        assert!((p.velocity_at(2.5) - 0.5).abs() < 1e-9);
        assert!((p.velocity_at(3.0) - 0.0).abs() < 1e-9);
    }

    #[test]
    fn scurve_velocity_shape() {
        let out = solve(MotionInput {
            s: 2.0,
            t: 0.0,
            v: 1.0,
            a: 1.0,
            shape: Shape::Trapezoid,
            scurve: true,
            scurve_percent: 100.0,
            time_ratio: 3.0,
        });
        assert!(out.errors.is_empty(), "{:?}", out.errors);
        let k = out.kin.unwrap();
        let p = Profile::new(k, 1.0, 5.0, 0.0, 0.0);
        assert!((p.velocity_at(0.0) - 0.0).abs() < 1e-9);
        assert!((p.velocity_at(0.25) - 0.125).abs() < 1e-9);
        assert!((p.velocity_at(1.0) - 0.875).abs() < 1e-9);
        assert!((p.velocity_at(1.25) - 1.0).abs() < 1e-9);
        assert!((p.velocity_at(2.0) - 1.0).abs() < 1e-9);
        assert!((p.velocity_at(2.25) - 0.875).abs() < 1e-9);
        assert!((p.velocity_at(3.0) - 0.125).abs() < 1e-9);
        assert!((p.velocity_at(3.25) - 0.0).abs() < 1e-9);
    }
}
