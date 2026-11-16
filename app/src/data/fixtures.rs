//! 测试用示例资料。
//!
//! 内置 `data/motors.json` 已清空（产品不预置电机），依赖具体型号与数量的
//! 测试不能再读种子数据；这里提供一份确定性的最小样本。

use crate::data::motor::{Construction, Motor, MotorKind};
use crate::data::repo::Catalog;

/// 一台直线电机 + 一台旋转电机，足以覆盖直线/旋转两种选型路径。
pub fn sample_catalog() -> Catalog {
    Catalog {
        motors: vec![linear_motor(), rotary_motor()],
        drives: Vec::new(),
        db_path: "示例资料".to_owned(),
        seeded: true,
    }
}

fn linear_motor() -> Motor {
    Motor {
        name: "SAMPLE-L-01".to_owned(),
        // 默认直线工况需求：持续 6.89 N / 峰值 8.44 N（含安全系数），
        // 这里取 9.5 / 12.0，使余量约 38%，落在默认余量窗口 30–300% 内。
        cont: 9.5,
        peak: 12.0,
        km: 5.0,
        kf: 10.0,
        ke: 10.0,
        i_cont: 5.0,
        i_peak: 15.0,
        induct: 2.0,
        resist: 1.0,
        rated_w: 250.0,
        max_w: 900.0,
        mover_mass: 0.5,
        length: 100.0,
        inertia: 0.0,
        max_speed: 5.0,
        kind: MotorKind::Ironless,
        construction: Construction::Standard,
    }
}

fn rotary_motor() -> Motor {
    Motor {
        name: "SAMPLE-R-01".to_owned(),
        // 默认旋转工况需求：持续 43.29 N·m / 峰值 53.01 N·m（含安全系数）。
        // 取 63 / 77 使余量约 45%，落在默认余量窗口 30–300% 内。
        cont: 63.0,
        peak: 77.0,
        km: 0.5,
        kf: 1.0,
        ke: 1.0,
        i_cont: 2.0,
        i_peak: 6.0,
        induct: 5.0,
        resist: 2.0,
        rated_w: 100.0,
        max_w: 300.0,
        mover_mass: 0.0,
        length: 80.0,
        inertia: 0.0001,
        max_speed: 3000.0,
        kind: MotorKind::Rotary,
        construction: Construction::Standard,
    }
}
