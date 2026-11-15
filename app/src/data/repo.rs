//! SQLite 数据仓库：负责建库、导入内置资料、一次性载入内存。
//!
//! 数据文件位置：
//! - 调试构建：`CARGO_MANIFEST_DIR/data/motors.db`
//! - 发布构建：可执行文件同目录 `data/motors.db`
//! - 环境变量 `MOTORSIZER_DB` 可覆盖以上路径
//!
//! 数据库不存在时会从内嵌的 `motors.json` / `drives.json` 自动建库导入；
//! 存在时只读载入，之后所有界面查询都使用内存中的数据。

use std::path::{Path, PathBuf};

use rusqlite::{params, params_from_iter, types::Value as SqlValue, Connection};
use serde_json::Value;

use super::motor::{classify, family_of, Construction, Motor, MotorKind};
use crate::domain::drive::Drive;

/// 数据库结构版本；修改表结构时递增。
const SCHEMA_VERSION: i64 = 2;

/// 历史电机类型修正的完成标记。
///
/// 归类规则修正前导入的 AJM/AQM/AKM/AKH 记录被写成了“无铁芯”，
/// 参数本身正确，只是 `kind` 列过期。用 meta 标记保证只修复一次。
const KIND_REPAIR_KEY: &str = "kind_repair_ironcore_v1";

const MOTORS_JSON: &str = include_str!("../../data/motors.json");
const DRIVES_JSON: &str = include_str!("../../data/drives.json");

const SCHEMA_SQL: &str = "
CREATE TABLE motors (
  id INTEGER PRIMARY KEY,
  name TEXT NOT NULL UNIQUE,
  family TEXT NOT NULL DEFAULT '',
  motor_type TEXT NOT NULL,
  kind TEXT NOT NULL,
  construction TEXT NOT NULL,
  frame_shape TEXT NOT NULL DEFAULT 'standard',
  cont REAL NOT NULL DEFAULT 0,
  peak REAL NOT NULL DEFAULT 0,
  km REAL NOT NULL DEFAULT 0,
  kf REAL NOT NULL DEFAULT 0,
  ke REAL NOT NULL DEFAULT 0,
  i_cont REAL NOT NULL DEFAULT 0,
  i_peak REAL NOT NULL DEFAULT 0,
  induct REAL NOT NULL DEFAULT 0,
  resist REAL NOT NULL DEFAULT 0,
  rated_w REAL NOT NULL DEFAULT 0,
  max_w REAL NOT NULL DEFAULT 0,
  mover_mass REAL NOT NULL DEFAULT 0,
  coil_weight REAL NOT NULL DEFAULT 0,
  length REAL NOT NULL DEFAULT 0,
  inertia REAL NOT NULL DEFAULT 0,
  thermal_resistance REAL NOT NULL DEFAULT 0,
  stroke REAL NOT NULL DEFAULT 0,
  max_speed REAL NOT NULL DEFAULT 0
);
CREATE INDEX idx_motors_kind ON motors(kind);
CREATE INDEX idx_motors_family ON motors(family);

CREATE TABLE drives (
  id INTEGER PRIMARY KEY,
  family TEXT NOT NULL DEFAULT '',
  name TEXT NOT NULL,
  cont REAL NOT NULL DEFAULT 0,
  peak REAL NOT NULL DEFAULT 0,
  voltage REAL NOT NULL DEFAULT 0,
  phase INTEGER NOT NULL DEFAULT 0,
  UNIQUE(name, voltage, phase)
);

CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
";

/// 一次载入的全部数据；界面运行期间不再访问数据库。
#[derive(Debug)]
pub struct Catalog {
    pub motors: Vec<Motor>,
    pub drives: Vec<Drive>,
    pub db_path: String,
    /// 本次启动是否新建并导入了数据库。
    pub seeded: bool,
}

/// 默认数据库路径（可用环境变量覆盖）。
pub fn default_db_path() -> PathBuf {
    if let Ok(custom) = std::env::var("MOTORSIZER_DB") {
        if !custom.trim().is_empty() {
            return PathBuf::from(custom);
        }
    }
    let base = if cfg!(debug_assertions) {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
    } else {
        std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(|parent| parent.to_path_buf()))
            .unwrap_or_else(|| PathBuf::from("."))
    };
    base.join("data").join("motors.db")
}

/// 打开默认数据库并载入全部数据。
pub fn load_default() -> Result<Catalog, String> {
    let path = default_db_path();
    load_at(&path)
}

/// 从指定路径载入；不存在时自动建库。
pub fn load_at(path: &Path) -> Result<Catalog, String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("无法创建数据目录 {}：{error}", parent.display()))?;
    }
    let mut conn = Connection::open(path)
        .map_err(|error| format!("无法打开数据库 {}：{error}", path.display()))?;
    let seeded = ensure_schema(&mut conn, path)?;
    // 归类规则变更前写入的记录不会自己更新；载入时顺手校正一次。
    repair_motor_kinds(&mut conn)?;
    let motors = read_motors(&conn)?;
    let drives = read_drives(&conn)?;
    Ok(Catalog {
        motors,
        drives,
        db_path: path.display().to_string(),
        seeded,
    })
}

/// 按型号名写入/覆盖电机参数（导入功能使用）。
///
/// - 新名字：插入记录，资料库专用字段（线圈重量、热阻、行程等）取默认值；
/// - 已有名字：只更新 `Motor` 能表达的参数，资料库中更细的列保持不变，
///   因此 OCR 未识别的字段不会把原有数据清零。
///
/// 整体在一个事务中执行，返回写入条数。
pub fn upsert_motors(path: &Path, motors: &[Motor]) -> Result<usize, String> {
    let mut conn = Connection::open(path)
        .map_err(|error| format!("无法打开数据库 {}：{error}", path.display()))?;
    let tx = conn.transaction().map_err(sql_error)?;
    for motor in motors {
        let motor_type = if motor.kind == MotorKind::Rotary { "R" } else { "L" };
        tx.execute(
            "INSERT INTO motors (
                name, family, motor_type, kind, construction, frame_shape,
                cont, peak, km, kf, ke, i_cont, i_peak, induct, resist,
                rated_w, max_w, mover_mass, coil_weight, length, inertia,
                thermal_resistance, stroke, max_speed
             ) VALUES (?1, ?2, ?3, ?4, ?5, 'standard', ?6, ?7, ?8, ?9, ?10,
                ?11, ?12, ?13, ?14, ?15, ?16, ?17, 0, ?18, ?19, 0, 0, ?20)
             ON CONFLICT(name) DO UPDATE SET
                family = excluded.family,
                motor_type = excluded.motor_type,
                kind = excluded.kind,
                construction = excluded.construction,
                cont = excluded.cont,
                peak = excluded.peak,
                km = excluded.km,
                kf = excluded.kf,
                ke = excluded.ke,
                i_cont = excluded.i_cont,
                i_peak = excluded.i_peak,
                induct = excluded.induct,
                resist = excluded.resist,
                rated_w = excluded.rated_w,
                max_w = excluded.max_w,
                mover_mass = excluded.mover_mass,
                length = excluded.length,
                inertia = excluded.inertia,
                max_speed = excluded.max_speed",
            params![
                motor.name,
                family_of(&motor.name),
                motor_type,
                motor.kind.as_str(),
                motor.construction.as_str(),
                motor.cont,
                motor.peak,
                motor.km,
                motor.kf,
                motor.ke,
                motor.i_cont,
                motor.i_peak,
                motor.induct,
                motor.resist,
                motor.rated_w,
                motor.max_w,
                motor.mover_mass,
                motor.length,
                motor.inertia,
                motor.max_speed,
            ],
        )
        .map_err(sql_error)?;
    }
    tx.commit().map_err(sql_error)?;
    Ok(motors.len())
}

/// 仅供测试：在内存中建库并导入同一份内置资料。
#[cfg(test)]
pub fn load_in_memory() -> Result<Catalog, String> {
    let mut conn =
        Connection::open_in_memory().map_err(|error| format!("无法创建内存数据库：{error}"))?;
    ensure_schema(&mut conn, Path::new(":memory:"))?;
    let motors = read_motors(&conn)?;
    let drives = read_drives(&conn)?;
    Ok(Catalog {
        motors,
        drives,
        db_path: "内存数据库".to_owned(),
        seeded: true,
    })
}

/// 检查库结构；缺失则建库导入，版本不符则返回可操作的错误。
fn ensure_schema(conn: &mut Connection, path: &Path) -> Result<bool, String> {
    let version: i64 = conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .map_err(sql_error)?;
    let tables: i64 = conn
        .query_row(
            "SELECT count(*) FROM sqlite_master
             WHERE type = 'table' AND name IN ('motors', 'drives')",
            [],
            |row| row.get(0),
        )
        .map_err(sql_error)?;

    if tables == 2 && version == SCHEMA_VERSION {
        return Ok(false);
    }
    // v1 的 SOLWHI-2.5/60 把电压与相数写反了（3 V / 48 相）；就地修正，不重建用户库。
    if tables == 2 && version == 1 {
        conn.execute(
            "UPDATE drives SET voltage = 48.0, phase = 3
             WHERE name = 'SOLWHI-2.5/60' AND voltage = 3.0 AND phase = 48",
            [],
        )
        .map_err(sql_error)?;
        conn.pragma_update(None, "user_version", SCHEMA_VERSION)
            .map_err(sql_error)?;
        return Ok(false);
    }
    if version != 0 {
        return Err(format!(
            "数据库版本 {version} 与程序要求 {SCHEMA_VERSION} 不一致（{}）。\
             请删除该文件后重新启动，程序会自动重建。",
            path.display()
        ));
    }

    // 空库或只建了一半表的空文件：重新建表并导入，不覆盖任何已存在的有效数据。
    conn.execute_batch(
        "DROP TABLE IF EXISTS motors;
         DROP TABLE IF EXISTS drives;
         DROP TABLE IF EXISTS meta;",
    )
    .map_err(sql_error)?;
    conn.execute_batch(SCHEMA_SQL).map_err(sql_error)?;
    seed(conn)?;
    conn.pragma_update(None, "user_version", SCHEMA_VERSION)
        .map_err(sql_error)?;
    Ok(true)
}

/// 按当前归类规则修正老库里的直线电机类型，返回改动的记录数。
///
/// 导入功能早期版本把 AJM/AQM/AKM/AKH 等有铁芯家族归到了无铁芯，
/// 但这些记录可能还带着用户手工校对过的参数值，重新 OCR 导入既慢又可能覆盖它们。
/// 这里只重算 `kind` 列：直线电机的类型完全由型号家族决定，预览中也不能单独选择
/// 有铁芯/无铁芯；旋转/直线是用户在预览里明确选过的，因此 `kind = 'rotary'` 的行不动。
///
/// 修复结果记在 meta 表里，后续启动不再全表扫描。
fn repair_motor_kinds(conn: &mut Connection) -> Result<usize, String> {
    let done: i64 = conn
        .query_row(
            "SELECT count(*) FROM meta WHERE key = ?1",
            params![KIND_REPAIR_KEY],
            |row| row.get(0),
        )
        .map_err(sql_error)?;
    if done > 0 {
        return Ok(0);
    }

    let tx = conn.transaction().map_err(sql_error)?;
    let mut records = Vec::new();
    {
        let mut stmt = tx
            .prepare("SELECT name, kind FROM motors WHERE kind != 'rotary'")
            .map_err(sql_error)?;
        let rows = stmt
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(sql_error)?;
        for row in rows {
            records.push(row.map_err(sql_error)?);
        }
    }

    let mut fixed = 0usize;
    for (name, stored) in records {
        let (kind, _) = classify(&name);
        if kind.as_str() != stored {
            tx.execute(
                "UPDATE motors SET kind = ?1 WHERE name = ?2",
                params![kind.as_str(), name],
            )
            .map_err(sql_error)?;
            fixed += 1;
        }
    }
    tx.execute(
        "INSERT INTO meta (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![KIND_REPAIR_KEY, format!("fixed {fixed} rows")],
    )
    .map_err(sql_error)?;
    tx.commit().map_err(sql_error)?;
    Ok(fixed)
}

/// 写入内置资料；整体放在一个事务里，任一条失败都不会留下半套数据。
fn seed(conn: &mut Connection) -> Result<(), String> {
    let motors_json: Vec<Value> =
        serde_json::from_str(MOTORS_JSON).map_err(|error| format!("内置电机资料解析失败：{error}"))?;
    let drives_json: Vec<Value> =
        serde_json::from_str(DRIVES_JSON).map_err(|error| format!("内置驱动器资料解析失败：{error}"))?;

    let tx = conn.transaction().map_err(sql_error)?;
    for value in &motors_json {
        let row = motor_row(value)?;
        let values = row.into_sql();
        tx.execute(
            "INSERT INTO motors (
                name, family, motor_type, kind, construction, frame_shape,
                cont, peak, km, kf, ke, i_cont, i_peak, induct, resist,
                rated_w, max_w, mover_mass, coil_weight, length, inertia,
                thermal_resistance, stroke, max_speed
             ) VALUES (?1, ?2, ?3, ?4, ?5, 'standard', ?6, ?7, ?8, ?9, ?10,
                ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23)",
            params_from_iter(values),
        )
        .map_err(sql_error)?;
    }

    // 资料库中有重复驱动器记录，按「名称 + 电压 + 相数」去重。
    let mut inserted: Vec<(String, i64, u32)> = Vec::new();
    for value in &drives_json {
        let name = text(value, "name");
        if name.is_empty() {
            continue;
        }
        let voltage = number(value, &["voltage"]);
        let phase = number(value, &["phase"]).round() as u32;
        let key = (name.clone(), (voltage * 100.0).round() as i64, phase);
        if inserted.contains(&key) {
            continue;
        }
        inserted.push(key);
        tx.execute(
            "INSERT INTO drives (family, name, cont, peak, voltage, phase)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                text(value, "family"),
                name,
                number(value, &["cont"]),
                number(value, &["peak"]),
                voltage,
                phase,
            ],
        )
        .map_err(sql_error)?;
    }

    tx.execute(
        "INSERT INTO meta (key, value) VALUES ('seed_source', 'embedded motors.json + drives.json')",
        [],
    )
    .map_err(sql_error)?;
    tx.commit().map_err(sql_error)?;
    Ok(())
}

fn read_motors(conn: &Connection) -> Result<Vec<Motor>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT name, cont, peak, km, kf, ke, i_cont, i_peak, induct, resist,
                    rated_w, max_w, mover_mass, length, inertia, max_speed,
                    kind, construction
             FROM motors ORDER BY id",
        )
        .map_err(sql_error)?;
    let rows = stmt.query_map([], |row| {
        let kind_text: String = row.get(16)?;
        let construction_text: String = row.get(17)?;
        let kind = MotorKind::parse(&kind_text).ok_or_else(|| invalid_text(16, &kind_text))?;
        let construction =
            Construction::parse(&construction_text).ok_or_else(|| invalid_text(17, &construction_text))?;
        Ok(Motor {
            name: row.get(0)?,
            cont: row.get(1)?,
            peak: row.get(2)?,
            km: row.get(3)?,
            kf: row.get(4)?,
            ke: row.get(5)?,
            i_cont: row.get(6)?,
            i_peak: row.get(7)?,
            induct: row.get(8)?,
            resist: row.get(9)?,
            rated_w: row.get(10)?,
            max_w: row.get(11)?,
            mover_mass: row.get(12)?,
            length: row.get(13)?,
            inertia: row.get(14)?,
            max_speed: row.get(15)?,
            kind,
            construction,
        })
    })
    .map_err(sql_error)?;
    let motors = rows
        .collect::<Result<Vec<_>, _>>()
        .map_err(sql_error)?;
    // 允许空库：用户可以把电机全部删除后再通过导入功能逐条添加。
    // 界面用“没有匹配型号”的空状态提示，不应因此拒绝载入整个数据库。
    Ok(motors)
}

fn read_drives(conn: &Connection) -> Result<Vec<Drive>, String> {
    let mut stmt = conn
        .prepare("SELECT name, cont, peak, voltage, phase FROM drives ORDER BY id")
        .map_err(sql_error)?;
    let rows = stmt
        .query_map([], |row| {
            Ok(Drive {
                name: row.get(0)?,
                cont: row.get(1)?,
                peak: row.get(2)?,
                voltage: row.get(3)?,
                phase: row.get(4)?,
            })
        })
        .map_err(sql_error)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(sql_error)
}

/// 电机导入行：先用同一套字段承接 JSON，再在写入时序列化。
struct MotorRow {
    name: String,
    family: String,
    motor_type: String,
    kind: String,
    construction: String,
    cont: f64,
    peak: f64,
    km: f64,
    kf: f64,
    ke: f64,
    i_cont: f64,
    i_peak: f64,
    induct: f64,
    resist: f64,
    rated_w: f64,
    max_w: f64,
    mover_mass: f64,
    coil_weight: f64,
    length: f64,
    inertia: f64,
    thermal_resistance: f64,
    stroke: f64,
    max_speed: f64,
}

impl Default for MotorRow {
    fn default() -> Self {
        Self {
            name: String::new(),
            family: String::new(),
            motor_type: "L".to_owned(),
            kind: "ironless".to_owned(),
            construction: "standard".to_owned(),
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
            coil_weight: 0.0,
            length: 0.0,
            inertia: 0.0,
            thermal_resistance: 0.0,
            stroke: 0.0,
            max_speed: 0.0,
        }
    }
}

impl MotorRow {
    fn into_sql(self) -> Vec<SqlValue> {
        vec![
            SqlValue::Text(self.name),
            SqlValue::Text(self.family),
            SqlValue::Text(self.motor_type),
            SqlValue::Text(self.kind),
            SqlValue::Text(self.construction),
            SqlValue::Real(self.cont),
            SqlValue::Real(self.peak),
            SqlValue::Real(self.km),
            SqlValue::Real(self.kf),
            SqlValue::Real(self.ke),
            SqlValue::Real(self.i_cont),
            SqlValue::Real(self.i_peak),
            SqlValue::Real(self.induct),
            SqlValue::Real(self.resist),
            SqlValue::Real(self.rated_w),
            SqlValue::Real(self.max_w),
            SqlValue::Real(self.mover_mass),
            SqlValue::Real(self.coil_weight),
            SqlValue::Real(self.length),
            SqlValue::Real(self.inertia),
            SqlValue::Real(self.thermal_resistance),
            SqlValue::Real(self.stroke),
            SqlValue::Real(self.max_speed),
        ]
    }
}

fn motor_row(value: &Value) -> Result<MotorRow, String> {
    let name = text(value, "name");
    if name.is_empty() {
        return Err("内置电机资料中存在没有名称的记录".to_owned());
    }
    // 资料里个别记录把 Motor Type 写成了 MoMotor Type（如 ACD120-80-Y）。
    let motor_type = match value.get("Motor Type").and_then(|x| x.as_str()) {
        Some(t) => t.to_owned(),
        None => match value.get("MoMotor Type").and_then(|x| x.as_str()) {
            Some(t) => t.to_owned(),
            None => {
                // 两个字段都缺失时按数据内容推断：有转矩字段即为旋转电机。
                if value.get("Peak Torque").is_some() || value.get("Continuous Torque").is_some() {
                    "R".to_owned()
                } else {
                    "L".to_owned()
                }
            }
        },
    };
    let rotary = motor_type == "R";
    let mut row = MotorRow::default();
    row.name = name.clone();
    row.family = family_of(&name);
    row.motor_type = motor_type;
    row.thermal_resistance = number(value, &["Thermal Resistance"]);
    row.coil_weight = number(value, &["Coil WeightB", "Coil Assembly Mass"]);

    if rotary {
        let (_, construction) = classify(&name);
        row.kind = MotorKind::Rotary.as_str().to_owned();
        row.construction = construction.as_str().to_owned();
        row.cont = number(value, &["Continuous Torque"]);
        row.peak = number(value, &["Peak Torque"]);
        row.km = number(value, &["Motor Constant"]);
        row.kf = number(value, &["Torque Constant"]);
        row.ke = number(value, &["Back EMF Constant"]);
        row.i_cont = number(value, &["Continuous Current"]);
        row.i_peak = number(value, &["Peak Current"]);
        row.induct = number(value, &["Inductance", "Phase Inductance"]);
        row.resist = number(value, &["Terminal Resistance", "Phase Resistance 25"]);
        row.rated_w = number(value, &["Continuous Power"]);
        row.max_w = number(value, &["Peak Power"]);
        row.mover_mass = 0.0;
        row.length = number(value, &["Coil LengthB"]);
        row.inertia = number(value, &["Rotor InertiaB"]);
        row.max_speed = 3000.0;
    } else {
        let (kind, construction) = classify(&name);
        row.kind = kind.as_str().to_owned();
        row.construction = construction.as_str().to_owned();
        row.cont = number(value, &["Continuous Force"]);
        row.peak = number(value, &["Peak Force"]);
        row.km = number(value, &["Motor Constant"]);
        row.kf = number(value, &["Force Constant"]);
        row.ke = number(value, &["Back EMF Constant"]);
        row.i_cont = number(value, &["Continuous Current"]);
        row.i_peak = number(value, &["Peak Current"]);
        row.induct = number(value, &["Phase Inductance", "Inductance"]);
        row.resist = number(value, &["Terminal Resistance", "Phase Resistance 25"]);
        row.rated_w = number(value, &["Continuous Power"]);
        row.max_w = number(value, &["Peak Power"]);
        row.mover_mass = number(value, &["Coil WeightB", "Coil Assembly Mass", "Total Mass"]);
        row.length = number(value, &["Coil LengthB", "Stroke"]);
        row.inertia = 0.0;
        row.max_speed = 5.0;
    }
    row.stroke = number(value, &["Stroke"]);
    Ok(row)
}

fn number(value: &Value, keys: &[&str]) -> f64 {
    for key in keys {
        if let Some(x) = value.get(*key).and_then(|x| x.as_f64()) {
            return x;
        }
    }
    0.0
}

fn text(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_owned()
}

fn sql_error(error: rusqlite::Error) -> String {
    format!("SQLite 错误：{error}")
}

fn invalid_text(column: usize, text: &str) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(
        column,
        rusqlite::types::Type::Text,
        Box::new(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("数据库中存在未知分类值：{text}"),
        )),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_db(tag: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("系统时间")
            .as_nanos();
        std::env::temp_dir().join(format!("motorsizer-{tag}-{nanos}.db"))
    }

    #[test]
    fn in_memory_seed_has_expected_counts() {
        let catalog = load_in_memory().expect("内存建库");
        assert_eq!(catalog.motors.len(), 302, "电机总数");
        let linear = catalog
            .motors
            .iter()
            .filter(|m| m.kind != MotorKind::Rotary)
            .count();
        let rotary = catalog
            .motors
            .iter()
            .filter(|m| m.kind == MotorKind::Rotary)
            .count();
        assert_eq!(linear, 232, "直线电机数量");
        assert_eq!(rotary, 70, "旋转电机数量");
        assert_eq!(catalog.drives.len(), 41, "驱动器去重后数量");
        assert_eq!(
            catalog
                .drives
                .iter()
                .filter(|d| d.name == "MT-4/12-DC")
                .count(),
            1,
            "重复驱动器记录应被去重"
        );

        // 原始资料把该型号的电压与相数写反了；导入后必须是 48 V / 3 相。
        let solwhi = catalog
            .drives
            .iter()
            .find(|d| d.name == "SOLWHI-2.5/60")
            .expect("SOLWHI-2.5/60");
        assert_eq!(solwhi.voltage, 48.0);
        assert_eq!(solwhi.phase, 3);

        // ACD120-80-Y 的 Motor Type 写成了 MoMotor Type，应识别为旋转电机。
        let acd = catalog
            .motors
            .iter()
            .find(|m| m.name == "ACD120-80-Y")
            .expect("ACD120-80-Y");
        assert_eq!(acd.kind, MotorKind::Rotary);
    }

    #[test]
    fn file_database_is_seeded_once_and_reused() {
        let path = temp_db("reuse");
        let first = load_at(&path).expect("首次建库");
        assert!(first.seeded);
        assert_eq!(first.motors.len(), 302);

        let second = load_at(&path).expect("再次载入");
        assert!(!second.seeded);
        assert_eq!(second.motors.len(), first.motors.len());
        assert_eq!(second.drives.len(), first.drives.len());

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn schema_version_mismatch_reports_actionable_error() {
        let path = temp_db("version");
        load_at(&path).expect("首次建库");
        {
            let conn = Connection::open(&path).expect("打开测试库");
            conn.execute_batch("PRAGMA user_version = 99")
                .expect("修改版本");
        }
        let error = load_at(&path).expect_err("版本不符应报错");
        assert!(error.contains("版本 99"), "{error}");
        assert!(error.contains("删除"), "{error}");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn version_one_database_repairs_swapped_drive_fields() {
        let path = temp_db("migrate");
        load_at(&path).expect("首次建库");
        {
            let conn = Connection::open(&path).expect("打开测试库");
            conn.execute_batch(
                "PRAGMA user_version = 1;
                 UPDATE drives SET voltage = 3.0, phase = 48
                 WHERE name = 'SOLWHI-2.5/60';",
            )
            .expect("还原 v1 数据");
        }
        let catalog = load_at(&path).expect("迁移后载入");
        assert!(!catalog.seeded, "迁移不应重建数据库");
        let drive = catalog
            .drives
            .iter()
            .find(|d| d.name == "SOLWHI-2.5/60")
            .expect("SOLWHI-2.5/60");
        assert_eq!(drive.voltage, 48.0);
        assert_eq!(drive.phase, 3);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn repair_motor_kinds_updates_stale_linear_records_once() {
        let path = temp_db("kind-repair");
        load_at(&path).expect("首次建库");

        // 模拟旧版导入：AJM 被写成无铁芯；另有一条用户明确选为旋转的记录。
        let mut stale = Motor::blank("AJM30-S-B1");
        stale.kind = MotorKind::Ironless;
        stale.cont = 42.0;
        let mut user_rotary = Motor::blank("CUSTOM-ROTARY");
        user_rotary.kind = MotorKind::Rotary;
        upsert_motors(&path, &[stale, user_rotary]).expect("写入旧记录");
        {
            let conn = Connection::open(&path).expect("打开测试库");
            conn.execute("DELETE FROM meta WHERE key = ?1", params![KIND_REPAIR_KEY])
                .expect("清除修复标记");
        }

        let catalog = load_at(&path).expect("修复后载入");
        let repaired = catalog
            .motors
            .iter()
            .find(|m| m.name == "AJM30-S-B1")
            .expect("AJM30-S-B1");
        assert_eq!(repaired.kind, MotorKind::IronCore);
        assert_eq!(repaired.cont, 42.0, "只修正类型，不动参数");
        let rotary = catalog
            .motors
            .iter()
            .find(|m| m.name == "CUSTOM-ROTARY")
            .expect("CUSTOM-ROTARY");
        assert_eq!(rotary.kind, MotorKind::Rotary, "用户选为旋转的记录不改写");

        // 修复标记写入后，再次载入不重复处理，结果保持稳定。
        let again = load_at(&path).expect("再次载入");
        assert_eq!(again.motors.len(), catalog.motors.len());
        assert_eq!(
            again
                .motors
                .iter()
                .find(|m| m.name == "AJM30-S-B1")
                .expect("AJM30-S-B1")
                .kind,
            MotorKind::IronCore
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn water_cooled_acm_keeps_iron_core_classification() {
        let catalog = load_in_memory().expect("内存建库");
        let motor = catalog
            .motors
            .iter()
            .find(|m| m.name == "ACM2-D-W-S1")
            .expect("ACM2-D-W-S1");
        assert_eq!(motor.kind, MotorKind::IronCore);
        assert_eq!(motor.construction, Construction::WaterCooled);
    }

    #[test]
    fn upsert_inserts_then_overwrites_without_touching_private_columns() {
        let path = temp_db("upsert");
        load_at(&path).expect("首次建库");

        let mut motor = Motor {
            name: "IMPORT-TEST-S1".to_owned(),
            cont: 3.0,
            peak: 9.0,
            km: 1.7,
            kf: 1.8,
            ke: 1.4,
            i_cont: 1.7,
            i_peak: 6.8,
            induct: 0.11,
            resist: 1.1,
            rated_w: 3.2,
            max_w: 28.0,
            mover_mass: 0.03,
            length: 22.0,
            inertia: 0.0,
            max_speed: 5.0,
            kind: MotorKind::Ironless,
            construction: Construction::Standard,
        };
        assert_eq!(upsert_motors(&path, &[motor.clone()]).expect("插入"), 1);
        let catalog = load_at(&path).expect("插入后载入");
        assert_eq!(catalog.motors.len(), 303, "应新增一条记录");

        // 模拟资料库更细的列：导入覆盖时必须保留。
        {
            let conn = Connection::open(&path).expect("打开测试库");
            conn.execute(
                "UPDATE motors SET coil_weight = 12.5 WHERE name = 'IMPORT-TEST-S1'",
                [],
            )
            .expect("写入私有序");
        }

        motor.cont = 4.5;
        upsert_motors(&path, &[motor]).expect("覆盖");
        let catalog = load_at(&path).expect("覆盖后载入");
        assert_eq!(catalog.motors.len(), 303, "同名不应新增");
        let stored = catalog
            .motors
            .iter()
            .find(|m| m.name == "IMPORT-TEST-S1")
            .expect("导入型号");
        assert_eq!(stored.cont, 4.5, "同名记录应使用新参数");

        let conn = Connection::open(&path).expect("打开测试库");
        let coil_weight: f64 = conn
            .query_row(
                "SELECT coil_weight FROM motors WHERE name = 'IMPORT-TEST-S1'",
                [],
                |row| row.get(0),
            )
            .expect("读取私有序");
        assert_eq!(coil_weight, 12.5, "覆盖时不应清空资料库专用字段");
        let _ = std::fs::remove_file(&path);
    }
}
