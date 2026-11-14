//! 驱动器匹配：按应用电流需求、母线电压与相数在已载入的驱动器资料中筛选。
//! 本模块只做计算，不直接读取文件或数据库。

#[derive(Clone, Debug)]
pub struct Drive {
    pub name: String,
    /// 持续电流 A
    pub cont: f64,
    /// 峰值电流 A
    pub peak: f64,
    /// 额定母线电压 V
    pub voltage: f64,
    /// 相数
    pub phase: u32,
}

/// 用户在界面选择的电气条件。
#[derive(Clone, Copy, Debug)]
pub struct DriveSettings {
    /// 母线电压 V；驱动器额定电压必须能覆盖该值。
    pub bus_voltage: f64,
    pub phase: u32,
}

impl Default for DriveSettings {
    fn default() -> Self {
        // 资料库中 310 V / 2 相条目最多，作为默认工况。
        Self {
            bus_voltage: 310.0,
            phase: 2,
        }
    }
}

/// 一条匹配结果。`margin` 为 0 表示刚好满足，负值表示电流不足。
#[derive(Clone, Debug)]
pub struct DriveMatch {
    pub drive: Drive,
    /// min(持续余量, 峰值余量) - 1
    pub margin: f64,
}

/// 按应用电流需求匹配驱动器。
///
/// - `need_cont` / `need_peak` 为已乘安全系数的应用电流（A）；
/// - 驱动器必须在持续与峰值两段都不低于需求；
/// - 额定母线电压需不低于所选母线电压，相数必须一致；
/// - 结果按电流余量降序，余量相同时取电流更小的型号（更经济）。
pub fn match_drives(
    drives: &[Drive],
    need_cont: f64,
    need_peak: f64,
    settings: DriveSettings,
) -> Vec<DriveMatch> {
    if need_cont <= 0.0 && need_peak <= 0.0 {
        return Vec::new();
    }
    let phase = settings.phase.max(1);
    let bus = settings.bus_voltage.max(0.0);
    let mut out: Vec<DriveMatch> = drives
        .iter()
        .filter(|drive| {
            drive.phase == phase
                && drive.voltage + 1e-6 >= bus
                && drive.cont + 1e-9 >= need_cont
                && drive.peak + 1e-9 >= need_peak
        })
        .map(|drive| {
            // 两段电流都要满足，取利用率更高的一段决定余量。
            let util_cont = if need_cont > 0.0 {
                need_cont / drive.cont
            } else {
                0.0
            };
            let util_peak = if need_peak > 0.0 {
                need_peak / drive.peak
            } else {
                0.0
            };
            let util = util_cont.max(util_peak);
            DriveMatch {
                drive: drive.clone(),
                margin: 1.0 / util - 1.0,
            }
        })
        .collect();
    out.sort_by(|a, b| {
        b.margin
            .total_cmp(&a.margin)
            .then_with(|| a.drive.cont.total_cmp(&b.drive.cont))
            .then_with(|| a.drive.name.cmp(&b.drive.name))
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn drive(name: &str, cont: f64, peak: f64, voltage: f64, phase: u32) -> Drive {
        Drive {
            name: name.to_owned(),
            cont,
            peak,
            voltage,
            phase,
        }
    }

    #[test]
    fn filters_by_phase_and_bus_voltage() {
        let drives = vec![
            drive("D-48-2", 6.0, 18.0, 48.0, 2),
            drive("D-48-3", 6.0, 18.0, 48.0, 3),
            drive("D-310-2", 6.0, 18.0, 310.0, 2),
        ];
        let settings = DriveSettings {
            bus_voltage: 310.0,
            phase: 2,
        };
        let matches = match_drives(&drives, 4.0, 10.0, settings);
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].drive.name, "D-310-2");
    }

    #[test]
    fn rejects_insufficient_current() {
        let drives = vec![drive("D-5", 5.0, 10.0, 310.0, 2)];
        // 峰值不足
        assert!(match_drives(&drives, 4.0, 12.0, DriveSettings::default()).is_empty());
        // 持续不足
        assert!(match_drives(&drives, 6.0, 8.0, DriveSettings::default()).is_empty());
        // 刚好满足
        assert_eq!(
            match_drives(&drives, 5.0, 10.0, DriveSettings::default()).len(),
            1
        );
    }

    #[test]
    fn sorts_by_current_margin_descending() {
        let drives = vec![
            drive("D-tight", 5.0, 11.0, 310.0, 2),
            drive("D-loose", 20.0, 40.0, 310.0, 2),
            drive("D-mid", 10.0, 20.0, 310.0, 2),
        ];
        let matches = match_drives(&drives, 4.0, 10.0, DriveSettings::default());
        let names: Vec<_> = matches.iter().map(|m| m.drive.name.as_str()).collect();
        assert_eq!(names, ["D-loose", "D-mid", "D-tight"]);
        assert!(matches[0].margin > matches[1].margin);
    }

    #[test]
    fn zero_demand_returns_empty() {
        let drives = vec![drive("D", 5.0, 10.0, 310.0, 2)];
        assert!(match_drives(&drives, 0.0, 0.0, DriveSettings::default()).is_empty());
    }
}
