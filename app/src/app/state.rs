use std::f64::consts::TAU;
use std::path::PathBuf;
use std::sync::mpsc::Receiver;
use std::time::Duration;

use eframe::egui::Context;

use crate::data::motor::{Construction, Motor, MotorKind};
use crate::data::repo::{self, Catalog};
use crate::import::{ocr, workflow};
use crate::domain::curve::{self, Combined, CurveSegment};
use crate::domain::drive::{self, Drive, DriveMatch, DriveSettings};
use crate::domain::motion::{self, MotionInput, Shape};
use crate::domain::profile::Profile;
use crate::domain::select::{self, Filters, MarginWindow, Requirement, DEFAULT_SAFETY_FACTOR};

const GRAVITY: f64 = 9.81;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tab {
    Linear,
    Rotary,
}

impl Tab {
    pub fn label(self) -> &'static str {
        match self {
            Self::Linear => "直线电机",
            Self::Rotary => "旋转电机",
        }
    }

    pub fn subtitle(self) -> &'static str {
        match self {
            Self::Linear => "平移轴 · N / m / m/s",
            Self::Rotary => "旋转轴 · N·m / ° / rpm",
        }
    }

    fn is_rotary(self) -> bool {
        matches!(self, Self::Rotary)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Independent,
    Combined,
    Solution,
}

impl Mode {
    pub fn label(self) -> &'static str {
        match self {
            Self::Independent => "独立运动曲线",
            Self::Combined => "综合曲线汇总",
            Self::Solution => "方案对比",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MotionMode {
    PointToPoint,
    Custom,
}



#[derive(Clone, Debug, PartialEq)]
pub struct Inputs {
    pub machine: String,
    pub axis: String,
    /// 直线：kg；旋转：kg·m²
    pub mass: f64,
    /// 直线：m；旋转：rev（界面按度显示，×360 换算）
    pub distance: f64,
    /// s
    pub time: f64,
    /// 直线：m/s；旋转：rpm
    pub speed: f64,
    /// 直线：m/s²；旋转：rad/s²（界面按 rpm/s 显示，×60/2π 换算）
    pub accel: f64,
    pub dwell: f64,
    /// 直线：N；旋转：N·m
    pub friction: f64,
    pub ext_force: f64,
    /// 度，仅直线
    pub angle: f64,
    /// 向上/向下，仅直线
    pub up: bool,
    pub ambient: f64,
    /// 0–100%，倍率 = 1 + p/100
    pub safety_pct: f64,
    pub ratio_n: f64,
    pub scurve: bool,
    pub scurve_pct: f64,
}

impl Default for Inputs {
    fn default() -> Self {
        Self {
            machine: "General Machine".to_owned(),
            axis: "Axis 1".to_owned(),
            mass: 5.0,
            distance: 1.0,
            time: 2.0,
            speed: 0.0,
            accel: 0.0,
            // 与原软件默认一致：新工况默认没有停顿；需要保压/上下料时再填。
            dwell: 0.0,
            friction: 0.0,
            ext_force: 0.0,
            angle: 0.0,
            up: true,
            ambient: 22.0,
            safety_pct: (DEFAULT_SAFETY_FACTOR - 1.0) * 100.0,
            ratio_n: 3.0,
            scurve: false,
            scurve_pct: 100.0,
        }
    }
}

impl Inputs {
    pub fn safety_factor(&self) -> f64 {
        1.0 + self.safety_pct.clamp(0.0, 100.0) / 100.0
    }
}

#[derive(Clone, Debug)]
pub struct RecommendationRow {
    pub name: String,
    pub margin_pct: f64,
    pub utilization: f64,
    pub util_cont: f64,
    pub util_peak: f64,
}

#[derive(Clone, Debug)]
pub struct SelectedMotor {
    pub motor: Motor,
    pub margin_pct: f64,
    pub util_cont: f64,
    pub util_peak: f64,
    pub need_rms: f64,
    pub need_peak: f64,
}

#[derive(Clone, Debug, Default)]
pub struct Computed {
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
    pub profile: Option<Profile>,
    pub requirement: Option<Requirement>,
    pub recs: Vec<RecommendationRow>,
    pub selected: Option<SelectedMotor>,
    /// 综合模式下的汇总结果与参与汇总的曲线快照。
    pub combined: Option<Combined>,
    pub curves: Vec<CurveSegment>,
    /// 已按时间轴拼接好的曲线采样，由重算阶段生成。
    pub v_samples: Vec<(f64, f64)>,
    pub f_samples: Vec<(f64, f64)>,
}

/// 保存下来的方案快照；用于方案对比与载入。
#[derive(Clone, Debug)]
pub struct SchemeSnapshot {
    pub name: String,
    pub tab: Tab,
    pub motion_mode: MotionMode,
    pub shape: Shape,
    pub inputs: Inputs,
    pub motor: Option<Motor>,
    pub rms: f64,
    pub peak: f64,
    pub cycle: f64,
    pub margin: f64,
}

/// 一次后台识别任务的运行状态。
pub struct ImportRun {
    pub receiver: Receiver<workflow::Event>,
    pub cancel: PathBuf,
    pub stage: String,
    pub done: u32,
    pub total: u32,
}

pub enum ImportPhase {
    Closed,
    /// 已选择文件，等待用户确认页码范围。
    Ready {
        path: PathBuf,
        start_page: u32,
        end_page: u32,
        is_pdf: bool,
    },
    Running(ImportRun),
    Preview(workflow::Preview),
    Failed(String),
}

impl Default for ImportPhase {
    fn default() -> Self {
        Self::Closed
    }
}

#[derive(Default)]
pub struct ImportUi {
    pub phase: ImportPhase,
    /// 预览中当前展开编辑的候选下标。
    pub selected: usize,
}

pub struct AppState {
    pub tab: Tab,
    pub mode: Mode,
    pub motion_mode: MotionMode,
    pub shape: Shape,
    pub inputs: Inputs,
    pub filters: Filters,
    pub window: MarginWindow,
    pub status: String,
    pub db_status: String,
    pub computed: Computed,
    pub selected_name: Option<String>,
    /// 综合模式每条曲线的添加数量。
    pub add_count: f64,
    /// 方案对比中保存的方案（仅内存，阶段 4 后可扩展持久化）。
    pub schemes: Vec<SchemeSnapshot>,
    /// 驱动器资料库与用户选择的电气条件。
    pub drives: Vec<Drive>,
    pub drive_settings: DriveSettings,
    pub import: ImportUi,
    combined_linear: Vec<CurveSegment>,
    combined_rotary: Vec<CurveSegment>,
    linear_motors: Vec<Motor>,
    rotary_motors: Vec<Motor>,
    dirty: bool,
}

impl AppState {
    /// 从默认 SQLite 库载入；失败时把错误交给界面显示错误页，不在此处 panic。
    pub fn new() -> Result<Self, String> {
        let catalog = repo::load_default()?;
        Ok(Self::from_catalog(catalog))
    }

    /// 用一次性载入的资料构建状态；界面运行期间不再访问数据库。
    pub(crate) fn from_catalog(catalog: Catalog) -> Self {
        let mut state = Self {
            tab: Tab::Linear,
            mode: Mode::Independent,
            motion_mode: MotionMode::PointToPoint,
            shape: Shape::Trapezoid,
            inputs: Inputs::default(),
            // 默认不勾选筛选条件，两类都不勾选即显示全部型号。
            filters: Filters::default(),
            window: MarginWindow {
                low_pct: 30.0,
                high_pct: 300.0,
                use_upper: true,
            },
            status: "正在计算…".to_owned(),
            db_status: String::new(),
            computed: Computed::default(),
            selected_name: None,
            add_count: 1.0,
            schemes: Vec::new(),
            drives: Vec::new(),
            drive_settings: DriveSettings::default(),
            import: ImportUi::default(),
            combined_linear: Vec::new(),
            combined_rotary: Vec::new(),
            linear_motors: Vec::new(),
            rotary_motors: Vec::new(),
            dirty: true,
        };
        state.apply_catalog(catalog);
        state
    }

    /// 用新的数据库快照替换内存资料，保持当前工况与界面状态不变。
    fn apply_catalog(&mut self, catalog: Catalog) {
        let Catalog {
            motors,
            drives,
            db_path,
            seeded,
        } = catalog;
        self.linear_motors.clear();
        self.rotary_motors.clear();
        for motor in motors {
            if motor.kind == MotorKind::Rotary {
                self.rotary_motors.push(motor);
            } else {
                self.linear_motors.push(motor);
            }
        }
        self.drives = drives;
        self.db_status = format!(
            "SQLite · {db_path} · 直线 {} / 旋转 {} / 驱动器 {}{}",
            self.linear_motors.len(),
            self.rotary_motors.len(),
            self.drives.len(),
            if seeded { "（已自动建库）" } else { "" }
        );
        let selection_exists = self
            .selected_name
            .as_ref()
            .map(|name| {
                self.linear_motors
                    .iter()
                    .chain(self.rotary_motors.iter())
                    .any(|motor| &motor.name == name)
            })
            .unwrap_or(true);
        if !selection_exists {
            self.selected_name = None;
        }
        self.dirty = true;
        self.recompute();
    }

    /// 导入落库后重新载入数据库并刷新推荐列表。
    pub fn reload_from_db(&mut self) -> Result<(), String> {
        let catalog = repo::load_default()?;
        self.apply_catalog(catalog);
        Ok(())
    }

    /// 当前内存中的全部型号（供导入时判断同名覆盖）。
    pub fn all_motors(&self) -> Vec<Motor> {
        self.linear_motors
            .iter()
            .chain(self.rotary_motors.iter())
            .cloned()
            .collect()
    }

    /// 用户通过“打开文件”对话框选中了文档。
    pub fn select_import_document(&mut self, path: PathBuf) {
        let is_pdf = path
            .extension()
            .map(|extension| extension.to_string_lossy().eq_ignore_ascii_case("pdf"))
            .unwrap_or(false);
        self.import.selected = 0;
        self.import.phase = ImportPhase::Ready {
            path,
            start_page: 1,
            end_page: 0,
            is_pdf,
        };
    }

    /// 从“准备”阶段启动后台识别。
    pub fn begin_import(&mut self, ctx: &Context) {
        let ImportPhase::Ready {
            path,
            start_page,
            end_page,
            ..
        } = std::mem::replace(&mut self.import.phase, ImportPhase::Closed)
        else {
            return;
        };
        match workflow::spawn(
            workflow::Setup {
                path,
                start_page,
                end_page,
            },
            self.all_motors(),
            ctx.clone(),
        ) {
            Ok(running) => {
                self.import.phase = ImportPhase::Running(ImportRun {
                    receiver: running.receiver,
                    cancel: running.cancel,
                    stage: "准备识别…".to_owned(),
                    done: 0,
                    total: 0,
                });
            }
            Err(error) => self.import.phase = ImportPhase::Failed(error),
        }
    }

    /// 每帧轮询后台任务；完成时切到预览或错误状态。
    pub fn poll_import(&mut self, ctx: &Context) {
        let mut finished = None;
        if let ImportPhase::Running(run) = &mut self.import.phase {
            while let Ok(event) = run.receiver.try_recv() {
                match event {
                    workflow::Event::Progress(progress) => {
                        run.stage = progress.stage;
                        run.done = progress.done;
                        run.total = progress.total;
                    }
                    workflow::Event::Finished(result) => {
                        finished = Some(result);
                        break;
                    }
                }
            }
        }
        if let Some(result) = finished {
            match result {
                Ok(preview) => {
                    self.import.selected = 0;
                    self.status = format!("识别完成：{} 个候选型号", preview.candidates.len());
                    self.import.phase = ImportPhase::Preview(preview);
                }
                Err(message) if message == ocr::CANCELLED => {
                    self.status = "已取消导入".to_owned();
                    self.import.phase = ImportPhase::Closed;
                }
                Err(message) => {
                    self.import.phase = ImportPhase::Failed(message);
                }
            }
        }
        if matches!(self.import.phase, ImportPhase::Running(_)) {
            ctx.request_repaint_after(Duration::from_millis(120));
        }
    }

    /// 写入取消标记；脚本与工作线程会在下一个检查点退出。
    pub fn cancel_import(&mut self) {
        if let ImportPhase::Running(run) = &mut self.import.phase {
            run.stage = "正在取消…".to_owned();
            let _ = std::fs::write(&run.cancel, b"cancel");
        }
    }

    pub fn close_import(&mut self) {
        self.import.phase = ImportPhase::Closed;
        self.import.selected = 0;
    }

    /// 把预览中勾选的行写入 SQLite，并刷新内存资料。
    /// 返回（导入数量，覆盖数量）。
    pub fn import_selected(&mut self) -> Result<(usize, usize), String> {
        let (selected, overwrites) = {
            let ImportPhase::Preview(preview) = &self.import.phase else {
                return Err("当前没有可导入的预览数据".to_owned());
            };
            let selected: Vec<Motor> = preview
                .candidates
                .iter()
                .filter(|candidate| candidate.include)
                .map(|candidate| candidate.motor.clone())
                .collect();
            if selected.is_empty() {
                return Err("请至少勾选一个型号".to_owned());
            }
            let overwrites = preview.overwrite_count();
            (selected, overwrites)
        };
        repo::upsert_motors(&repo::default_db_path(), &selected)?;
        self.reload_from_db()?;
        self.close_import();
        self.status = format!(
            "已导入 {} 个型号（覆盖 {} 个同名记录）",
            selected.len(),
            overwrites
        );
        Ok((selected.len(), overwrites))
    }

    pub fn touch(&mut self) {
        self.dirty = true;
    }

    pub fn recompute_if_dirty(&mut self) {
        if self.dirty {
            self.recompute();
        }
    }

    pub fn set_tab(&mut self, tab: Tab) {
        if self.tab != tab {
            self.tab = tab;
            self.dirty = true;
        }
    }

    pub fn set_mode(&mut self, mode: Mode) {
        if self.mode != mode {
            self.mode = mode;
            self.dirty = true;
        }
    }

    pub fn select_motor(&mut self, name: &str) {
        self.selected_name = Some(name.to_owned());
        self.sync_selected();
    }

    pub fn curve_count(&self) -> usize {
        if self.tab.is_rotary() {
            self.combined_rotary.len()
        } else {
            self.combined_linear.len()
        }
    }

    /// 把当前工况按指定数量加入综合曲线集合。
    pub fn add_current_curves(&mut self, count: usize) {
        let Some(profile) = self.computed.profile.clone() else {
            return;
        };
        let rotary = self.tab.is_rotary();
        let store = if rotary {
            &mut self.combined_rotary
        } else {
            &mut self.combined_linear
        };
        let start = store.len() + 1;
        let total = count.clamp(1, 20);
        for index in 0..total {
            store.push(CurveSegment {
                label: format!("曲线 {}", start + index),
                profile: profile.clone(),
            });
        }
        self.dirty = true;
    }

    pub fn clear_curves(&mut self) {
        let rotary = self.tab.is_rotary();
        if rotary {
            self.combined_rotary.clear();
        } else {
            self.combined_linear.clear();
        }
        self.dirty = true;
    }

    /// 保存当前工况与选中型号，供方案对比使用。
    pub fn save_scheme(&mut self) {
        let Some(profile) = self.computed.profile.clone() else {
            return;
        };
        let selected = self.computed.selected.clone();
        let motor = selected.as_ref().map(|s| s.motor.clone());
        let (rms, peak, margin) = selected
            .as_ref()
            .map(|s| (s.need_rms, s.need_peak, s.margin_pct))
            .unwrap_or((profile.f_rms, profile.f_peak, 0.0));
        let name = format!("方案 {}", self.schemes.len() + 1);
        self.schemes.push(SchemeSnapshot {
            name: name.clone(),
            tab: self.tab,
            motion_mode: self.motion_mode,
            shape: self.shape,
            inputs: self.inputs.clone(),
            motor,
            rms,
            peak,
            cycle: profile.t_cycle,
            margin,
        });
        self.status = format!("已保存 {name}");
    }

    pub fn apply_scheme(&mut self, index: usize) {
        let Some(scheme) = self.schemes.get(index).cloned() else {
            return;
        };
        self.tab = scheme.tab;
        self.motion_mode = scheme.motion_mode;
        self.shape = scheme.shape;
        self.inputs = scheme.inputs;
        self.selected_name = scheme.motor.map(|motor| motor.name);
        self.dirty = true;
        self.recompute();
        self.status = format!("已载入 {} · {}", scheme.name, scheme.tab.label());
    }

    pub fn delete_scheme(&mut self, index: usize) {
        if index < self.schemes.len() {
            self.schemes.remove(index);
            self.status = "已删除方案".to_owned();
        }
    }

    fn motors(&self) -> &[Motor] {
        if self.tab.is_rotary() {
            &self.rotary_motors
        } else {
            &self.linear_motors
        }
    }

    fn build_profile(&self, kin: motion::Kinematics) -> Profile {
        let rotary = self.tab.is_rotary();
        let ext_force = self.inputs.friction + self.inputs.ext_force;
        let (inertia, gravity) = if rotary {
            (self.inputs.mass, 0.0)
        } else {
            let angle = self.inputs.angle * std::f64::consts::PI / 180.0;
            let sign = if self.inputs.up { 1.0 } else { -1.0 };
            (
                self.inputs.mass,
                self.inputs.mass * GRAVITY * angle.sin() * sign,
            )
        };
        Profile::new(kin, inertia, ext_force, gravity, self.inputs.dwell)
    }

    pub fn recompute(&mut self) {
        let mut computed = Computed::default();
        let rotary = self.tab.is_rotary();

        // 直线用 m / m/s / m/s²；旋转用 rad / rad/s / rad/s²。
        let (s, t, v, a) = if rotary {
            (
                self.inputs.distance * TAU,
                self.inputs.time,
                self.inputs.speed * TAU / 60.0,
                self.inputs.accel,
            )
        } else {
            (
                self.inputs.distance,
                self.inputs.time,
                self.inputs.speed,
                self.inputs.accel,
            )
        };

        let (s, t, v, a) = match self.motion_mode {
            MotionMode::PointToPoint => (s, t, 0.0, 0.0),
            MotionMode::Custom => (s, t, v, a),
        };

        let input = MotionInput {
            s,
            t,
            v,
            a,
            shape: self.shape,
            scurve: self.inputs.scurve,
            scurve_percent: self.inputs.scurve_pct,
            time_ratio: self.inputs.ratio_n,
        };
        let outcome = motion::solve(input);
        computed.errors = outcome.errors;
        computed.warnings = outcome.warnings;

        let current = outcome.kin.map(|kin| self.build_profile(kin));
        computed.profile = current.clone();
        let safety = self.inputs.safety_factor();

        // 综合模式使用曲线集合；集合为空时暂按当前曲线作为一条。
        let requirement = match self.mode {
            Mode::Combined => {
                let store = if rotary {
                    &self.combined_rotary
                } else {
                    &self.combined_linear
                };
                let segments = if store.is_empty() {
                    current
                        .as_ref()
                        .map(|profile| {
                            vec![CurveSegment {
                                label: "当前曲线".to_owned(),
                                profile: profile.clone(),
                            }]
                        })
                        .unwrap_or_default()
                } else {
                    store.clone()
                };
                let combined = curve::combine(&segments);
                let (v_samples, f_samples) = curve::timeline(&segments, 60);
                computed.combined = combined;
                computed.curves = segments;
                computed.v_samples = v_samples;
                computed.f_samples = f_samples;
                combined.map(|c| Requirement::new(c.rms, c.peak, safety))
            }
            Mode::Independent | Mode::Solution => {
                if let Some(profile) = &current {
                    computed.v_samples = profile.sample_velocity(160);
                    computed.f_samples = profile.sample_force(160);
                    Some(Requirement::from_profile(profile, safety))
                } else {
                    None
                }
            }
        };

        computed.requirement = requirement;
        if let Some(requirement) = requirement {
            if requirement.needed_rms() > 0.0 || requirement.needed_peak() > 0.0 {
                let filters = if rotary { Filters::all() } else { self.filters };
                let recs = select::recommend(
                    self.motors(),
                    &requirement,
                    &filters,
                    &self.window,
                );
                computed.recs = recs
                    .iter()
                    .map(|r| RecommendationRow {
                        name: r.motor.name.clone(),
                        margin_pct: r.margin_pct,
                        utilization: r.utilization,
                        util_cont: if r.needed_rms > 0.0 && r.motor.cont > 0.0 {
                            r.needed_rms / r.motor.cont
                        } else {
                            0.0
                        },
                        util_peak: if r.needed_peak > 0.0 && r.motor.peak > 0.0 {
                            r.needed_peak / r.motor.peak
                        } else {
                            0.0
                        },
                    })
                    .collect();
            }
        }

        self.status = match self.mode {
            Mode::Combined => {
                let count = computed.curves.len();
                if computed.combined.is_none() {
                    "综合曲线：请先添加有效曲线".to_owned()
                } else if computed.recs.is_empty() {
                    format!("综合计算完成 · {count} 条曲线 · 没有匹配型号")
                } else {
                    format!(
                        "综合计算完成 · {count} 条曲线 · {} 个型号可匹配",
                        computed.recs.len()
                    )
                }
            }
            Mode::Solution => {
                if let Some(error) = computed.errors.first() {
                    error.clone()
                } else {
                    format!(
                        "方案对比 · 已保存 {} 个方案 · 当前 {} 个型号可匹配",
                        self.schemes.len(),
                        computed.recs.len()
                    )
                }
            }
            Mode::Independent => {
                if let Some(error) = computed.errors.first() {
                    error.clone()
                } else if computed.recs.is_empty() {
                    "计算完成 · 没有型号满足当前安全系数与余量窗口".to_owned()
                } else {
                    format!("计算完成 · {} 个型号可匹配", computed.recs.len())
                }
            }
        };

        self.computed = computed;
        self.sync_selected();
        self.dirty = false;
    }

    /// 当前选中电机的驱动器候选。
    /// 界面每帧调用一次，数据量只有几十条，无需缓存。
    pub fn matched_drives(&self) -> Vec<DriveMatch> {
        let Some(selected) = &self.computed.selected else {
            return Vec::new();
        };
        let motor = &selected.motor;
        // 应用电流 = 需求力/力矩 ÷ 力/力矩常数（已包含安全系数）。
        let (app_cont, app_peak) = if motor.kf.abs() > 1e-9 {
            (selected.need_rms / motor.kf, selected.need_peak / motor.kf)
        } else {
            (0.0, 0.0)
        };
        // 驱动器至少要覆盖电机自身的额定持续/峰值电流；
        // 应用需求高于额定时，再按实际需求抬高标准。
        let need_cont = motor.i_cont.max(app_cont);
        let need_peak = motor.i_peak.max(app_peak);
        drive::match_drives(&self.drives, need_cont, need_peak, self.drive_settings)
    }

    fn sync_selected(&mut self) {
        let row = self
            .selected_name
            .as_ref()
            .and_then(|name| self.computed.recs.iter().find(|r| &r.name == name))
            .or_else(|| self.computed.recs.first())
            .cloned();

        let selected = row.as_ref().and_then(|row| {
            let motor = self
                .motors()
                .iter()
                .find(|motor| motor.name == row.name)?
                .clone();
            let requirement = self.computed.requirement?;
            Some(SelectedMotor {
                motor,
                margin_pct: row.margin_pct,
                util_cont: row.util_cont,
                util_peak: row.util_peak,
                need_rms: requirement.needed_rms(),
                need_peak: requirement.needed_peak(),
            })
        });
        self.computed.selected = selected;

        if let Some(selected) = &self.computed.selected {
            self.selected_name = Some(selected.motor.name.clone());
        } else if self.computed.recs.is_empty() {
            self.selected_name = None;
        }
    }
}

pub fn kind_label(kind: MotorKind) -> &'static str {
    match kind {
        MotorKind::Ironless => "无铁芯",
        MotorKind::IronCore => "有铁芯",
        MotorKind::VoiceCoil => "音圈",
        MotorKind::Rotary => "旋转",
    }
}

pub fn construction_label(construction: Construction) -> &'static str {
    match construction {
        Construction::Standard => "普通冷却",
        Construction::WaterCooled => "水冷",
        Construction::AirCooled => "风冷",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 测试统一使用内存数据库，避免触碰真实的 data/motors.db。
    fn state() -> AppState {
        AppState::from_catalog(crate::data::repo::load_in_memory().expect("内存数据库"))
    }

    #[test]
    fn default_linear_state_has_recommendations() {
        let state = state();
        assert!(state.computed.errors.is_empty(), "{:?}", state.computed.errors);
        assert!(!state.computed.recs.is_empty());
        assert!(state.computed.selected.is_some());
        assert!(state.computed.profile.is_some());
    }

    #[test]
    fn switching_to_rotary_recomputes() {
        let mut state = state();
        state.set_tab(Tab::Rotary);
        state.recompute_if_dirty();
        assert!(state.computed.errors.is_empty(), "{:?}", state.computed.errors);
        assert!(!state.computed.recs.is_empty());
        assert!(state
            .computed
            .selected
            .as_ref()
            .is_some_and(|s| s.motor.kind == MotorKind::Rotary));
    }

    #[test]
    fn custom_triangle_is_solved() {
        let mut state = state();
        state.motion_mode = MotionMode::Custom;
        state.shape = Shape::Triangle;
        state.inputs.distance = 1.0;
        state.inputs.time = 1.0;
        // S=1、T=1 的三角形曲线对应 V=2、A=4；自定义模式需给出一致条件。
        state.inputs.speed = 2.0;
        state.inputs.accel = 4.0;
        state.touch();
        state.recompute();
        assert!(state.computed.errors.is_empty(), "{:?}", state.computed.errors);
        let kin = state.computed.profile.as_ref().unwrap().kin;
        assert_eq!(kin.shape, Shape::Triangle);
        assert!((kin.tc - 0.0).abs() < 1e-9);
    }

    #[test]
    fn scurve_flag_reaches_kinematics() {
        let mut state = state();
        state.inputs.scurve = true;
        state.inputs.scurve_pct = 100.0;
        state.touch();
        state.recompute();
        assert!(state.computed.errors.is_empty(), "{:?}", state.computed.errors);
        let kin = state.computed.profile.as_ref().unwrap().kin;
        assert!(kin.scurve);
        assert!(kin.jerk > 0.0);
    }

    #[test]
    fn combined_mode_aggregates_saved_curves() {
        let mut state = state();
        state.set_mode(Mode::Combined);
        state.add_current_curves(2);
        state.recompute();
        let combined = state.computed.combined.unwrap();
        assert_eq!(combined.count, 2);
        let single = state.computed.profile.as_ref().unwrap().t_cycle;
        assert!((combined.cycle_time - single * 2.0).abs() < 1e-9);
        assert!(combined.peak > 0.0);
        assert!(!state.computed.recs.is_empty());
    }

    #[test]
    fn combined_mode_without_saved_curves_uses_current_preview() {
        let mut state = state();
        state.set_mode(Mode::Combined);
        state.recompute();
        let combined = state.computed.combined.unwrap();
        assert_eq!(combined.count, 1);
    }

    #[test]
    fn scheme_save_and_apply_restores_inputs() {
        let mut state = state();
        state.save_scheme();
        assert_eq!(state.schemes.len(), 1);
        let saved_distance = state.schemes[0].inputs.distance;
        let saved_motor = state.schemes[0].motor.as_ref().map(|m| m.name.clone());

        state.inputs.distance = 3.0;
        state.recompute();
        state.apply_scheme(0);

        assert!((state.inputs.distance - saved_distance).abs() < 1e-9);
        assert_eq!(state.selected_name, saved_motor);
        assert!(state.computed.profile.is_some());
    }

    #[test]
    fn default_selection_has_drive_candidates() {
        let state = state();
        let matches = state.matched_drives();
        assert!(!matches.is_empty(), "默认工况应有可匹配驱动器");
        // 默认母线 310 V / 2 相，候选必须同时满足电压与相数。
        assert!(matches
            .iter()
            .all(|m| m.drive.voltage >= 310.0 - 1e-6 && m.drive.phase == 2));

        // 余量必须按“应用电流与电机额定电流取大”计算，而不是只看瞬时峰值；
        // 不同工况可能选到很小的电机，余量数值本身不做硬性上限。
        let selected = state.computed.selected.as_ref().expect("应有选中电机");
        let motor = &selected.motor;
        let app_cont = if motor.kf.abs() > 1e-9 {
            selected.need_rms / motor.kf
        } else {
            0.0
        };
        let app_peak = if motor.kf.abs() > 1e-9 {
            selected.need_peak / motor.kf
        } else {
            0.0
        };
        let need_cont = motor.i_cont.max(app_cont);
        let need_peak = motor.i_peak.max(app_peak);
        let util = (need_cont / matches[0].drive.cont).max(need_peak / matches[0].drive.peak);
        let expected = 1.0 / util - 1.0;
        assert!(
            (matches[0].margin - expected).abs() < 1e-9,
            "驱动器余量应基于额定/应用电流的较大值：{} vs {}",
            matches[0].margin,
            expected
        );
    }

    #[test]
    fn drive_phase_setting_filters_candidates() {
        let mut state = state();
        state.drive_settings.phase = 3;
        let matches = state.matched_drives();
        assert!(matches.iter().all(|m| m.drive.phase == 3));
    }

    #[test]
    fn invalid_ratio_is_clamped_not_panicking() {
        let mut state = state();
        state.inputs.ratio_n = 0.0;
        state.touch();
        state.recompute();
        assert!(state.computed.errors.is_empty(), "{:?}", state.computed.errors);
    }
}
