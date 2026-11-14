//! 计算内核：运动学、力学与选型。
//! 本层不依赖界面与数据存储，便于单元测试和后续替换数据源。

pub mod curve;
pub mod drive;
pub mod motion;
pub mod profile;
pub mod select;
