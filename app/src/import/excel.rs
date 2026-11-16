//! 电机参数 Excel 中间稿：识别结果导出为固定模板，人工校对后读回并校验。
//!
//! 模板约定见 `docs/IMPORT-EXCEL.md`：单工作表、首行表头、按表头名匹配列，
//! 直线与旋转共用一份模板，由「类型」列区分。

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use calamine::{open_workbook_auto, Data, DataType, Range, Reader};
use rust_xlsxwriter::{Format, Workbook};

use crate::data::motor::{classify, Construction, Motor, MotorKind};
use crate::import::parse::match_existing;

/// 模板工作表名。
pub const SHEET_NAME: &str = "电机参数";

const HEADER_MODEL: &str = "型号";
const HEADER_TYPE: &str = "类型";
const HEADER_COOLING: &str = "冷却方式";
const HEADER_CONT: &str = "持续值";
const HEADER_PEAK: &str = "峰值";
const HEADER_KM: &str = "电机常数";
const HEADER_KF: &str = "力/转矩常数";
const HEADER_KE: &str = "反电动势常数";
const HEADER_I_CONT: &str = "持续电流";
const HEADER_I_PEAK: &str = "峰值电流";
const HEADER_INDUCT: &str = "电感";
const HEADER_RESIST: &str = "电阻";
const HEADER_RATED_W: &str = "持续功率";
const HEADER_MAX_W: &str = "峰值功率";
const HEADER_MASS: &str = "动子质量 / 转子惯量";
const HEADER_LENGTH: &str = "长度";
const HEADER_SPEED: &str = "最高速度 / 最高转速";
const HEADER_PAGE: &str = "来源页码";

const HEADERS: &[&str] = &[
    HEADER_MODEL,
    HEADER_TYPE,
    HEADER_COOLING,
    HEADER_CONT,
    HEADER_PEAK,
    HEADER_KM,
    HEADER_KF,
    HEADER_KE,
    HEADER_I_CONT,
    HEADER_I_PEAK,
    HEADER_INDUCT,
    HEADER_RESIST,
    HEADER_RATED_W,
    HEADER_MAX_W,
    HEADER_MASS,
    HEADER_LENGTH,
    HEADER_SPEED,
    HEADER_PAGE,
];

const COLUMN_WIDTHS: &[f64] = &[
    20.0, 8.0, 12.0, 12.0, 12.0, 14.0, 14.0, 16.0, 12.0, 12.0, 10.0, 10.0, 12.0, 12.0, 20.0, 10.0,
    18.0, 12.0,
];

const COL_MODEL: u16 = 0;
const COL_TYPE: u16 = 1;
const COL_COOLING: u16 = 2;
const COL_CONT: u16 = 3;
const COL_PEAK: u16 = 4;
const COL_KM: u16 = 5;
const COL_KF: u16 = 6;
const COL_KE: u16 = 7;
const COL_I_CONT: u16 = 8;
const COL_I_PEAK: u16 = 9;
const COL_INDUCT: u16 = 10;
const COL_RESIST: u16 = 11;
const COL_RATED_W: u16 = 12;
const COL_MAX_W: u16 = 13;
const COL_MASS: u16 = 14;
const COL_LENGTH: u16 = 15;
const COL_SPEED: u16 = 16;
const COL_PAGE: u16 = 17;

/// 导出草稿的一行：识别得到的电机与来源页码。
pub struct DraftRow {
    pub page: u32,
    pub motor: Motor,
}

/// 校验后的一行；`motor` 为空表示该行无法构成有效记录。
pub struct ExcelRow {
    pub line: u32,
    pub name: String,
    pub motor: Option<Motor>,
    pub errors: Vec<String>,
}

/// 读取并校验后的 Excel 预览结果。
pub struct ExcelPreview {
    pub path: PathBuf,
    pub rows: Vec<ExcelRow>,
    /// 文件级错误：缺必需列、文件内型号重复、没有有效行等。
    pub errors: Vec<String>,
    /// 导入前的数据库快照，用于标注“覆盖/新增”。
    pub existing: Vec<Motor>,
}

impl ExcelPreview {
    /// 存在任何错误时整份文件都不允许提交。
    pub fn blocked(&self) -> bool {
        !self.errors.is_empty() || self.rows.iter().any(|row| !row.errors.is_empty())
    }

    pub fn error_count(&self) -> usize {
        self.errors.len() + self.rows.iter().map(|row| row.errors.len()).sum::<usize>()
    }

    pub fn valid_motors(&self) -> Vec<Motor> {
        self.rows
            .iter()
            .filter(|row| row.errors.is_empty())
            .filter_map(|row| row.motor.clone())
            .collect()
    }

    pub fn overwrite_count(&self) -> usize {
        self.rows
            .iter()
            .filter(|row| row.errors.is_empty())
            .filter(|row| {
                row.motor
                    .as_ref()
                    .is_some_and(|motor| match_existing(&motor.name, &self.existing).is_some())
            })
            .count()
    }
}

/// 把识别结果写成草稿工作簿。
pub fn write_draft(path: &Path, rows: &[DraftRow]) -> Result<(), String> {
    let mut workbook = Workbook::new();
    let sheet = workbook.add_worksheet();
    sheet
        .set_name(SHEET_NAME)
        .map_err(|error| format!("无法设置工作表名：{error}"))?;

    let header_format = Format::new().set_bold();
    sheet
        .write_row_with_format(0, 0, HEADERS.iter().copied(), &header_format)
        .map_err(|error| format!("无法写入表头：{error}"))?;

    for (index, row) in rows.iter().enumerate() {
        let excel_row = index as u32 + 1;
        let motor = &row.motor;
        write_text(sheet, excel_row, COL_MODEL, &motor.name)?;
        write_text(sheet, excel_row, COL_TYPE, type_text(motor.kind))?;
        write_text(sheet, excel_row, COL_COOLING, cooling_text(motor.construction))?;
        let mass = if motor.kind == MotorKind::Rotary {
            motor.inertia
        } else {
            motor.mover_mass
        };
        for (column, value) in [
            (COL_CONT, motor.cont),
            (COL_PEAK, motor.peak),
            (COL_KM, motor.km),
            (COL_KF, motor.kf),
            (COL_KE, motor.ke),
            (COL_I_CONT, motor.i_cont),
            (COL_I_PEAK, motor.i_peak),
            (COL_INDUCT, motor.induct),
            (COL_RESIST, motor.resist),
            (COL_RATED_W, motor.rated_w),
            (COL_MAX_W, motor.max_w),
            (COL_MASS, mass),
            (COL_LENGTH, motor.length),
            (COL_SPEED, motor.max_speed),
        ] {
            sheet
                .write_number(excel_row, column, value)
                .map_err(|error| format!("无法写入第 {} 行：{error}", excel_row + 1))?;
        }
        sheet
            .write_number(excel_row, COL_PAGE, row.page as f64)
            .map_err(|error| format!("无法写入第 {} 行：{error}", excel_row + 1))?;
    }

    sheet
        .set_freeze_panes(1, 0)
        .map_err(|error| format!("无法冻结表头：{error}"))?;
    for (index, width) in COLUMN_WIDTHS.iter().enumerate() {
        sheet
            .set_column_width(index as u16, *width)
            .map_err(|error| format!("无法设置列宽：{error}"))?;
    }

    workbook
        .save(path)
        .map_err(|error| format!("无法保存 Excel {}：{error}", path.display()))
}

fn write_text(
    sheet: &mut rust_xlsxwriter::Worksheet,
    row: u32,
    column: u16,
    text: &str,
) -> Result<(), String> {
    sheet
        .write_string(row, column, text)
        .map_err(|error| format!("无法写入第 {} 行：{error}", row + 1))?;
    Ok(())
}

fn type_text(kind: MotorKind) -> &'static str {
    if kind == MotorKind::Rotary {
        "旋转"
    } else {
        "直线"
    }
}

fn cooling_text(construction: Construction) -> &'static str {
    if construction == Construction::WaterCooled {
        "水冷"
    } else {
        "普通冷却"
    }
}

/// 生成只含表头的空白模板，供识别失败时人工对照样册填写。
pub fn write_template(path: &Path) -> Result<(), String> {
    write_draft(path, &[])
}

/// 读取工作簿并逐行校验；打开失败返回 `Err`，内容问题进入预览。
pub fn read_workbook(path: &Path, existing: &[Motor]) -> Result<ExcelPreview, String> {
    let mut workbook =
        open_workbook_auto(path).map_err(|error| format!("无法打开 Excel：{error}"))?;
    let names = workbook.sheet_names();
    let sheet_name = names
        .iter()
        .find(|name| normalize(name) == normalize(SHEET_NAME))
        .or_else(|| names.first())
        .cloned()
        .ok_or_else(|| "工作簿中没有工作表".to_owned())?;
    let range = workbook
        .worksheet_range(&sheet_name)
        .map_err(|error| format!("无法读取工作表「{sheet_name}」：{error}"))?;

    let mut preview = ExcelPreview {
        path: path.to_path_buf(),
        rows: Vec::new(),
        errors: Vec::new(),
        existing: existing.to_vec(),
    };

    if range.height() == 0 {
        preview.errors.push("工作表为空".to_owned());
        return Ok(preview);
    }

    let columns = header_columns(&range);
    for header in [HEADER_MODEL, HEADER_TYPE, HEADER_CONT, HEADER_PEAK] {
        if !columns.contains_key(&normalize(header)) {
            preview.errors.push(format!("缺少必需列：{header}"));
        }
    }
    if !preview.errors.is_empty() {
        return Ok(preview);
    }

    for row_index in 1..range.height() {
        if row_is_empty(&range, row_index) {
            continue;
        }
        preview.rows.push(parse_row(&range, &columns, row_index));
    }

    if preview.rows.is_empty() {
        preview.errors.push("没有数据行".to_owned());
    }

    // 文件内型号重复会让“谁覆盖谁”不确定，按文件级错误处理。
    let mut lines_by_name: HashMap<String, Vec<u32>> = HashMap::new();
    for row in &preview.rows {
        let key = normalize(&row.name).to_uppercase();
        if key.is_empty() {
            continue;
        }
        lines_by_name.entry(key).or_default().push(row.line);
    }
    for (name, lines) in lines_by_name {
        if lines.len() > 1 {
            let line_text = lines
                .iter()
                .map(|line| line.to_string())
                .collect::<Vec<_>>()
                .join("、");
            preview
                .errors
                .push(format!("文件内型号重复：{name}（第 {line_text} 行）"));
        }
    }

    Ok(preview)
}

fn header_columns(range: &Range<Data>) -> HashMap<String, usize> {
    let mut columns = HashMap::new();
    let Some(header_row) = range.rows().next() else {
        return columns;
    };
    for (index, cell) in header_row.iter().enumerate() {
        if let Some(text) = cell_text(cell) {
            if !text.is_empty() {
                columns.insert(normalize(&text), index);
            }
        }
    }
    columns
}

fn row_is_empty(range: &Range<Data>, row: usize) -> bool {
    range
        .rows()
        .nth(row)
        .map(|cells| {
            cells
                .iter()
                .all(|cell| cell_text(cell).map(|text| text.is_empty()).unwrap_or(true))
        })
        .unwrap_or(true)
}

fn parse_row(
    range: &Range<Data>,
    columns: &HashMap<String, usize>,
    row_index: usize,
) -> ExcelRow {
    let line = row_index as u32 + 1;
    let mut errors = Vec::new();
    let name = text_field(range, columns, row_index, HEADER_MODEL);
    if name.is_empty() {
        errors.push("型号为空".to_owned());
    }

    let type_value = text_field(range, columns, row_index, HEADER_TYPE);
    let Some(rotary) = parse_type(&type_value) else {
        errors.push(format!("类型无法识别：{type_value}"));
        return ExcelRow {
            line,
            name,
            motor: None,
            errors,
        };
    };

    let cooling_value = text_field(range, columns, row_index, HEADER_COOLING);
    let cooling = match parse_cooling(&cooling_value) {
        Ok(value) => value,
        Err(message) => {
            errors.push(message);
            None
        }
    };

    let cont = number_field(range, columns, row_index, HEADER_CONT, &mut errors);
    let peak = number_field(range, columns, row_index, HEADER_PEAK, &mut errors);
    if cont <= 0.0 {
        errors.push("持续值必须大于 0".to_owned());
    }
    if peak <= 0.0 {
        errors.push("峰值必须大于 0".to_owned());
    }

    let km = number_field(range, columns, row_index, HEADER_KM, &mut errors);
    let kf = number_field(range, columns, row_index, HEADER_KF, &mut errors);
    let ke = number_field(range, columns, row_index, HEADER_KE, &mut errors);
    let i_cont = number_field(range, columns, row_index, HEADER_I_CONT, &mut errors);
    let i_peak = number_field(range, columns, row_index, HEADER_I_PEAK, &mut errors);
    let induct = number_field(range, columns, row_index, HEADER_INDUCT, &mut errors);
    let resist = number_field(range, columns, row_index, HEADER_RESIST, &mut errors);
    let rated_w = number_field(range, columns, row_index, HEADER_RATED_W, &mut errors);
    let max_w = number_field(range, columns, row_index, HEADER_MAX_W, &mut errors);
    let mass = number_field(range, columns, row_index, HEADER_MASS, &mut errors);
    let length = number_field(range, columns, row_index, HEADER_LENGTH, &mut errors);
    let max_speed = number_field(range, columns, row_index, HEADER_SPEED, &mut errors);

    for (label, value) in [
        (HEADER_KM, km),
        (HEADER_KF, kf),
        (HEADER_KE, ke),
        (HEADER_I_CONT, i_cont),
        (HEADER_I_PEAK, i_peak),
        (HEADER_INDUCT, induct),
        (HEADER_RESIST, resist),
        (HEADER_RATED_W, rated_w),
        (HEADER_MAX_W, max_w),
        (HEADER_MASS, mass),
        (HEADER_LENGTH, length),
        (HEADER_SPEED, max_speed),
    ] {
        if !value.is_finite() || value < 0.0 {
            errors.push(format!("{label} 必须为非负数"));
        }
    }

    if !errors.is_empty() {
        return ExcelRow {
            line,
            name,
            motor: None,
            errors,
        };
    }

    // 直线电机的结构分类由型号家族推断；冷却方式优先使用表格里的显式值。
    let (kind, inferred) = if rotary {
        (MotorKind::Rotary, Construction::Standard)
    } else {
        classify(&name)
    };
    let construction = cooling.unwrap_or(inferred);
    let motor = Motor {
        name: name.clone(),
        cont,
        peak,
        km,
        kf,
        ke,
        i_cont,
        i_peak,
        induct,
        resist,
        rated_w,
        max_w,
        mover_mass: if rotary { 0.0 } else { mass },
        length,
        inertia: if rotary { mass } else { 0.0 },
        max_speed,
        kind,
        construction,
    };

    ExcelRow {
        line,
        name,
        motor: Some(motor),
        errors,
    }
}

fn text_field(
    range: &Range<Data>,
    columns: &HashMap<String, usize>,
    row: usize,
    header: &str,
) -> String {
    let Some(&column) = columns.get(&normalize(header)) else {
        return String::new();
    };
    range
        .get((row, column))
        .and_then(cell_text)
        .unwrap_or_default()
}

fn number_field(
    range: &Range<Data>,
    columns: &HashMap<String, usize>,
    row: usize,
    header: &str,
    errors: &mut Vec<String>,
) -> f64 {
    let Some(&column) = columns.get(&normalize(header)) else {
        return 0.0;
    };
    match range.get((row, column)) {
        None | Some(Data::Empty) => 0.0,
        Some(Data::Int(value)) => *value as f64,
        Some(Data::Float(value)) => *value,
        Some(Data::String(text)) => {
            let trimmed = text.trim();
            if trimmed.is_empty() {
                return 0.0;
            }
            match trimmed.parse::<f64>() {
                Ok(value) => value,
                Err(_) => {
                    errors.push(format!("{header} 不是有效数字：{trimmed}"));
                    0.0
                }
            }
        }
        Some(other) => {
            errors.push(format!("{header} 不是有效数字：{other:?}"));
            0.0
        }
    }
}

fn cell_text(cell: &Data) -> Option<String> {
    cell.as_string().map(|text| text.trim().to_owned())
}

fn parse_type(text: &str) -> Option<bool> {
    match normalize(text).to_lowercase().as_str() {
        "直线" | "l" | "linear" => Some(false),
        "旋转" | "r" | "rotary" => Some(true),
        _ => None,
    }
}

fn parse_cooling(text: &str) -> Result<Option<Construction>, String> {
    let normalized = normalize(text);
    if normalized.is_empty() {
        return Ok(None);
    }
    match normalized.as_str() {
        "普通冷却" | "自然冷却" | "风冷" | "普通" => Ok(Some(Construction::Standard)),
        "水冷" => Ok(Some(Construction::WaterCooled)),
        _ => Err(format!("冷却方式无法识别：{text}")),
    }
}

/// 去掉所有空白，避免用户改表头时多敲空格导致匹配失败。
fn normalize(text: &str) -> String {
    text.chars().filter(|c| !c.is_whitespace()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_motor(name: &str, rotary: bool, cont: f64, peak: f64) -> Motor {
        Motor {
            name: name.to_owned(),
            cont,
            peak,
            km: 2.0,
            kf: 3.0,
            ke: 4.0,
            i_cont: 5.0,
            i_peak: 6.0,
            induct: 7.0,
            resist: 8.0,
            rated_w: 9.0,
            max_w: 10.0,
            mover_mass: if rotary { 0.0 } else { 0.25 },
            length: 100.0,
            inertia: if rotary { 0.001 } else { 0.0 },
            max_speed: 3000.0,
            kind: if rotary {
                MotorKind::Rotary
            } else {
                MotorKind::Ironless
            },
            construction: Construction::Standard,
        }
    }

    fn temp_xlsx(tag: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("系统时间")
            .as_nanos();
        std::env::temp_dir().join(format!("motionsizer-excel-{tag}-{nanos}.xlsx"))
    }

    #[test]
    fn draft_round_trip_keeps_fields() {
        let path = temp_xlsx("roundtrip");
        let rows = vec![
            DraftRow {
                page: 3,
                motor: sample_motor("TEST-L-01", false, 12.5, 30.0),
            },
            DraftRow {
                page: 5,
                motor: sample_motor("TEST-R-01", true, 2.5, 8.0),
            },
        ];
        write_draft(&path, &rows).expect("写出草稿");
        let preview = read_workbook(&path, &[]).expect("读回草稿");
        assert!(preview.errors.is_empty(), "{:?}", preview.errors);
        assert_eq!(preview.rows.len(), 2);
        assert!(preview.rows.iter().all(|row| row.errors.is_empty()));

        let linear = preview.rows[0].motor.as_ref().expect("直线电机");
        assert_eq!(linear.name, "TEST-L-01");
        assert_eq!(linear.kind, MotorKind::Ironless);
        assert!((linear.cont - 12.5).abs() < 1e-9);
        assert!((linear.mover_mass - 0.25).abs() < 1e-9);

        let rotary = preview.rows[1].motor.as_ref().expect("旋转电机");
        assert_eq!(rotary.kind, MotorKind::Rotary);
        assert!((rotary.inertia - 0.001).abs() < 1e-12);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn template_contains_header_row_only() {
        let path = temp_xlsx("template");
        write_template(&path).expect("写模板");
        let mut workbook = open_workbook_auto(&path).expect("打开模板");
        let sheet_name = workbook.sheet_names().first().cloned().expect("工作表");
        assert_eq!(sheet_name, SHEET_NAME);
        let range = workbook.worksheet_range(&sheet_name).expect("读取表头");
        assert_eq!(range.height(), 1, "空白模板只应有表头一行");
        let header = range.rows().next().expect("表头");
        assert_eq!(cell_text(&header[0]).as_deref(), Some(HEADER_MODEL));
        assert_eq!(cell_text(&header[3]).as_deref(), Some(HEADER_CONT));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn blank_cells_read_as_zero() {
        let path = temp_xlsx("blank");
        let mut motor = sample_motor("TEST-L-02", false, 10.0, 20.0);
        motor.km = 0.0;
        motor.kf = 0.0;
        motor.ke = 0.0;
        write_draft(&path, &[DraftRow { page: 1, motor }]).expect("写出草稿");
        let preview = read_workbook(&path, &[]).expect("读回草稿");
        let row = preview.rows.first().expect("数据行");
        assert!(row.errors.is_empty(), "{:?}", row.errors);
        let motor = row.motor.as_ref().expect("电机");
        assert_eq!(motor.km, 0.0);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn unknown_type_blocks_row() {
        let path = temp_xlsx("badtype");
        let mut workbook = Workbook::new();
        let sheet = workbook.add_worksheet();
        sheet.set_name(SHEET_NAME).unwrap();
        sheet
            .write_row(0, 0, [HEADER_MODEL, HEADER_TYPE, HEADER_CONT, HEADER_PEAK])
            .unwrap();
        sheet.write_row(1, 0, ["TEST-X", "未知"]).unwrap();
        sheet.write_number(1, 2, 10.0).unwrap();
        sheet.write_number(1, 3, 20.0).unwrap();
        workbook.save(&path).expect("写出测试表");

        let preview = read_workbook(&path, &[]).expect("读取");
        assert!(preview.blocked());
        let row = preview.rows.first().expect("数据行");
        assert!(row.errors.iter().any(|error| error.contains("类型无法识别")));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn non_numeric_value_blocks_row() {
        let path = temp_xlsx("badnumber");
        let mut workbook = Workbook::new();
        let sheet = workbook.add_worksheet();
        sheet.set_name(SHEET_NAME).unwrap();
        sheet
            .write_row(0, 0, [HEADER_MODEL, HEADER_TYPE, HEADER_CONT, HEADER_PEAK])
            .unwrap();
        sheet.write_row(1, 0, ["TEST-X", "直线", "50 N"]).unwrap();
        sheet.write_number(1, 3, 20.0).unwrap();
        workbook.save(&path).expect("写出测试表");

        let preview = read_workbook(&path, &[]).expect("读取");
        let row = preview.rows.first().expect("数据行");
        assert!(row
            .errors
            .iter()
            .any(|error| error.contains("不是有效数字")));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn missing_required_columns_is_file_error() {
        let path = temp_xlsx("missing");
        let mut workbook = Workbook::new();
        let sheet = workbook.add_worksheet();
        sheet
            .write_row(0, 0, [HEADER_MODEL, HEADER_TYPE])
            .unwrap();
        sheet.write_row(1, 0, ["TEST-X", "直线"]).unwrap();
        workbook.save(&path).expect("写出测试表");

        let preview = read_workbook(&path, &[]).expect("读取");
        assert!(preview.rows.is_empty());
        assert!(preview
            .errors
            .iter()
            .any(|error| error.contains("缺少必需列")));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn duplicate_models_are_file_error() {
        let path = temp_xlsx("duplicate");
        let rows = vec![
            DraftRow {
                page: 1,
                motor: sample_motor("DUP-01", false, 10.0, 20.0),
            },
            DraftRow {
                page: 2,
                motor: sample_motor("DUP-01", false, 11.0, 21.0),
            },
        ];
        write_draft(&path, &rows).expect("写出草稿");
        let preview = read_workbook(&path, &[]).expect("读取");
        assert!(preview
            .errors
            .iter()
            .any(|error| error.contains("文件内型号重复")));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn overwrite_count_matches_existing_names() {
        let path = temp_xlsx("overwrite");
        let rows = vec![DraftRow {
            page: 1,
            motor: sample_motor("EXIST-01", false, 10.0, 20.0),
        }];
        write_draft(&path, &rows).expect("写出草稿");
        let existing = vec![sample_motor("EXIST-01", false, 1.0, 2.0)];
        let preview = read_workbook(&path, &existing).expect("读取");
        assert_eq!(preview.overwrite_count(), 1);
        assert_eq!(preview.valid_motors().len(), 1);
        let _ = std::fs::remove_file(&path);
    }
}
