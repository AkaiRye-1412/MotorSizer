//! 电机与驱动器的纯数据类型，以及型号家族归类规则。
//! 本模块不访问数据库，供 data::repo 与 domain 层共用。

impl Motor {
    /// 参数全零的空记录；供导入预览在没有同名旧数据时作为底稿。
    pub fn blank(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            cont: 0.0,
            peak: 0.0,
            km: 0.0,
            kf: 0.0,
            ke: 0.0,
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
            kind: MotorKind::Ironless,
            construction: Construction::Standard,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Motor {
    pub name: String,
    /// 直线：持续/峰值推力 N；旋转：持续/峰值转矩 N·m
    pub cont: f64,
    pub peak: f64,
    /// 电机常数：直线 N/√W；旋转 N·m/√W
    pub km: f64,
    /// 力常数 N/Arms（直线）或转矩常数 N·m/Arms（旋转）
    pub kf: f64,
    /// 反电动势常数 V/(m/s)（直线）或 V/(rad/s)（旋转）
    pub ke: f64,
    /// 持续/峰值电流，单位 Arms（安培有效值）；若资料给的是 A（峰值安培），
    /// 按 Arms = A / 1.414 换算后再录入。
    pub i_cont: f64,
    pub i_peak: f64,
    pub induct: f64, // mH
    pub resist: f64, // ohm
    pub rated_w: f64,
    pub max_w: f64,
    pub mover_mass: f64, // kg（旋转：转子惯量见 inertia）
    pub length: f64,     // 线圈长度 mm（旋转：电机长度）
    pub inertia: f64,    // kg·m²（旋转）
    /// 最高速度 m/s（旋转：最高转速 rpm）
    pub max_speed: f64,
    pub kind: MotorKind,
    pub construction: Construction,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MotorKind {
    /// 无铁芯直线电机
    Ironless,
    /// 有铁芯直线电机
    IronCore,
    /// 音圈电机
    VoiceCoil,
    /// 旋转直驱电机
    Rotary,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Construction {
    /// 普通/自然冷却
    Standard,
    WaterCooled,
    AirCooled,
}

impl MotorKind {
    /// 与 SQLite 中 kind 列的字符串表示对应。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ironless => "ironless",
            Self::IronCore => "ironcore",
            Self::VoiceCoil => "voice_coil",
            Self::Rotary => "rotary",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "ironless" => Some(Self::Ironless),
            "ironcore" => Some(Self::IronCore),
            "voice_coil" => Some(Self::VoiceCoil),
            "rotary" => Some(Self::Rotary),
            _ => None,
        }
    }
}

impl Construction {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Standard => "standard",
            Self::WaterCooled => "water",
            Self::AirCooled => "air",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "standard" => Some(Self::Standard),
            "water" => Some(Self::WaterCooled),
            "air" => Some(Self::AirCooled),
            _ => None,
        }
    }
}

/// 型号家族：取名称开头的连续字母，例如 AUM2-P-S3 → AUM。
pub fn family_of(name: &str) -> String {
    name.chars()
        .take_while(|c| c.is_ascii_alphabetic())
        .collect()
}

/// 直线电机家族归类（依据 Akribis 样册“规格一览表”）：
/// - 有铁芯：ACM / AJM / AQM / AKM / AKH
/// - 无铁芯：AUM / AWM / AHM / ALM / ACW / ACD / ACR / RDM
/// - 音圈：AVA / AVM / UserVC
/// - 数据集未收录的家族默认按无铁芯处理，导入后请在预览中核对类型。
pub fn classify(name: &str) -> (MotorKind, Construction) {
    let upper = name.to_ascii_uppercase();
    let water =
        upper.contains("-W-") || upper.contains("-W") || upper.contains("W-S") || upper.contains("W-P");
    let air = !water && upper.contains("-A-");
    let constr = if water {
        Construction::WaterCooled
    } else if air {
        Construction::AirCooled
    } else {
        Construction::Standard
    };
    let kind = if upper.starts_with("AVA") || upper.starts_with("AVM") || upper == "USERVC" {
        MotorKind::VoiceCoil
    } else if upper.starts_with("ACM")
        || upper.starts_with("AJM")
        || upper.starts_with("AQM")
        || upper.starts_with("AKM")
        || upper.starts_with("AKH")
    {
        MotorKind::IronCore
    } else if upper.starts_with("AUM")
        || upper.starts_with("AWM")
        || upper.starts_with("AHM")
        || upper.starts_with("ALM")
        || upper.starts_with("ACW")
        || upper.starts_with("ACD")
        || upper.starts_with("ACR")
        || upper.starts_with("RDM")
    {
        MotorKind::Ironless
    } else {
        MotorKind::Ironless
    };
    (kind, constr)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn family_classification_matches_catalog() {
        // 无铁芯系列
        assert_eq!(classify("AUM2-P-S3").0, MotorKind::Ironless);
        assert_eq!(classify("AWM1-S1").0, MotorKind::Ironless);
        assert_eq!(classify("AHM-S80-S1").0, MotorKind::Ironless);
        // 有铁芯系列
        assert_eq!(classify("ACM1-S30").0, MotorKind::IronCore);
        // 样册规格一览表：AJM/AQM/AKM/AKH 同样属于有铁芯
        assert_eq!(classify("AJM30-B1").0, MotorKind::IronCore);
        assert_eq!(classify("AQM24-B1").0, MotorKind::IronCore);
        assert_eq!(classify("AKM100-W-B2").0, MotorKind::IronCore);
        assert_eq!(classify("AKM100-W-B2").1, Construction::WaterCooled);
        assert_eq!(classify("AKH100-L/H-B2").0, MotorKind::IronCore);
        // 无铁芯系列（ACW 是扁平无铁芯，容易被“水冷”后缀混淆）
        assert_eq!(classify("ALM-T015").0, MotorKind::Ironless);
        assert_eq!(classify("ACW120").0, MotorKind::Ironless);
        // 音圈系列
        assert_eq!(classify("AVM60-25").0, MotorKind::VoiceCoil);
        // 冷却方式不改变铁芯归类
        assert_eq!(classify("ACM2-D-W-S1").0, MotorKind::IronCore);
        assert_eq!(classify("ACM2-D-W-S1").1, Construction::WaterCooled);
    }

    #[test]
    fn family_prefix_is_extracted() {
        assert_eq!(family_of("AUM2-P-S3"), "AUM");
        assert_eq!(family_of("ACM2-D-W-S1"), "ACM");
        assert_eq!(family_of("UserVC"), "UserVC");
        assert_eq!(family_of("SM8-10"), "SM");
    }

    #[test]
    fn kind_and_construction_round_trip_through_strings() {
        for kind in [
            MotorKind::Ironless,
            MotorKind::IronCore,
            MotorKind::VoiceCoil,
            MotorKind::Rotary,
        ] {
            assert_eq!(MotorKind::parse(kind.as_str()), Some(kind));
        }
        for construction in [
            Construction::Standard,
            Construction::WaterCooled,
            Construction::AirCooled,
        ] {
            assert_eq!(Construction::parse(construction.as_str()), Some(construction));
        }
    }
}
