//! 从 OCR 结果重建参数表：识别表头、串联/并联列、行标签，归一化数字，
//! 合并多次识别（整页 + 分列裁剪 + 去网格）的候选值，最后生成型号名。
//!
//! 所有几何阈值都以页面像素为单位，随 PDF 渲染宽度缩放；样例页取 1800 宽。

use crate::data::motor::{classify, Construction, Motor, MotorKind};
use crate::import::ocr::OcrSheet;

/// 可导入的电机参数字段。未列出的字段（热阻、行程等）保持数据库原值。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Field {
    Cont,
    Peak,
    Kf,
    Ke,
    Km,
    Resist,
    Induct,
    ICont,
    IPeak,
    RatedW,
    Mass,
    Length,
    Inertia,
    MaxSpeed,
}

impl Field {
    pub const COUNT: usize = 14;
    pub const ALL: [Field; Field::COUNT] = [
        Field::Cont,
        Field::Peak,
        Field::Kf,
        Field::Ke,
        Field::Km,
        Field::Resist,
        Field::Induct,
        Field::ICont,
        Field::IPeak,
        Field::RatedW,
        Field::Mass,
        Field::Length,
        Field::Inertia,
        Field::MaxSpeed,
    ];

    fn index(self) -> usize {
        self as usize
    }

    /// 界面展示用的中文名。
    pub fn label(self) -> &'static str {
        match self {
            Field::Cont => "持续推力/转矩",
            Field::Peak => "峰值推力/转矩",
            Field::Kf => "力/转矩常数",
            Field::Ke => "反电势常数",
            Field::Km => "电机常数",
            Field::Resist => "相间电阻",
            Field::Induct => "相间电感",
            Field::ICont => "持续电流",
            Field::IPeak => "峰值电流",
            Field::RatedW => "持续热功率",
            Field::Mass => "线圈/转子质量",
            Field::Length => "线圈长度",
            Field::Inertia => "转动惯量",
            Field::MaxSpeed => "最高转速",
        }
    }
}

/// 行标签别名。顺序即优先级：更具体的标签必须排在前面，
/// 例如“最高转速@持续转矩”同时包含两个别名，应归为转速。
const ALIASES: &[(&str, Field)] = &[
    ("最高转速", Field::MaxSpeed),
    ("最高速度", Field::MaxSpeed),
    ("持续转矩", Field::Cont),
    ("峰值转矩", Field::Peak),
    ("转矩常数", Field::Kf),
    ("反电势常数", Field::Ke),
    ("电机常数", Field::Km),
    ("相间电阻", Field::Resist),
    ("相间电感", Field::Induct),
    ("持续电流", Field::ICont),
    ("峰值电流", Field::IPeak),
    ("持续热功率", Field::RatedW),
    ("线圈质量", Field::Mass),
    ("动子质量", Field::Mass),
    ("转子质量", Field::Mass),
    // 旋转直驱样册的机械参数表用“总质量（自冷）”描述整机质量。
    ("总质量", Field::Mass),
    // OCR 常把“总质量”识成“灬质量/灬质里/灬人里”；具体的“线圈质量”
    // 等别名排在上面，因此这些宽松别名只作为兜底。
    ("质量", Field::Mass),
    ("质里", Field::Mass),
    ("人里", Field::Mass),
    ("线圈长度", Field::Length),
    ("转动惯量", Field::Inertia),
    ("转子惯量", Field::Inertia),
    ("持续推力", Field::Cont),
    ("峰值推力", Field::Peak),
    ("力常数", Field::Kf),
];

pub fn field_of_label(text: &str) -> Option<Field> {
    let joined: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    ALIASES
        .iter()
        .find(|(alias, _)| joined.contains(alias))
        .map(|(_, field)| *field)
}

/// 识别串联/并联子表头。子表头是短标签（“串联”“并联”“串联接法”），
/// 而图题“转矩-转速曲线 AXD120-71 串联接法”是一整句；只按文字长度
/// 区分二者，避免把图题当成参数列标题。
fn connection_label(text: &str) -> Option<&'static str> {
    let joined: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    if joined.chars().count() > 6 {
        return None;
    }
    match (joined.contains('串'), joined.contains('并')) {
        (true, false) => Some("S"),
        (false, true) => Some("P"),
        _ => None,
    }
}

/// 型号标题里允许出现的非字母数字符号；注意中文破折号“一”也可能是
/// 连字符误识别，其余汉字则说明这是一句图题而不是型号。
fn is_model_punctuation(ch: char) -> bool {
    matches!(
        ch,
        '一' | '－' | '–' | '—' | '−' | '-' | '·' | '•' | '.' | '．' | '/' | '／' | ' '
    )
}

fn contains_chinese(text: &str) -> bool {
    text.chars().any(|ch| {
        if is_model_punctuation(ch) {
            return false;
        }
        matches!(ch, '\u{3400}'..='\u{9FFF}' | '\u{F900}'..='\u{FAFF}')
    })
}

#[derive(Clone, Debug)]
pub struct TableColumn {
    pub name: String,
    pub center_x: f64,
}

#[derive(Clone, Debug)]
pub struct TableRow {
    pub field: Field,
    pub center_y: f64,
}

#[derive(Clone, Debug)]
pub struct PageTable {
    pub page: u32,
    pub rotary: bool,
    pub columns: Vec<TableColumn>,
    pub rows: Vec<TableRow>,
    pub header_bottom: f64,
    pub bottom: f64,
    pub column_tolerance: f64,
    pub row_tolerance: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PassKind {
    /// 整页或清理后的整页，按 x 坐标定位列；`scale` 是识别图相对页面
    /// （表头坐标系）的缩放，通常为 1。
    Full { weight: u32, scale: f64 },
    /// 单列裁剪图，按已知列索引收集；`top` 是裁剪图在页面中的纵坐标，
    /// `scale` 是裁剪图相对页面的缩放。
    Column {
        column: usize,
        top: f64,
        scale: f64,
        weight: u32,
    },
    /// 单行条带裁剪图，用于补扫整列识别漏掉的个别数值行。
    /// `x0`/`top` 是裁剪图左上角在页面中的坐标，`scale_x`/`scale_y`
    /// 是裁剪图相对页面的缩放（裁剪可能同时放大，宽高缩放相同）。
    Band {
        x0: f64,
        top: f64,
        scale_x: f64,
        scale_y: f64,
        weight: u32,
    },
    /// 单个单元格的高倍放大裁剪图。整列/整行裁剪时 Windows OCR 偶尔会
    /// 丢掉小数点甚至整段数字（例如把 3.3 读成 33），单独裁剪单元格并
    /// 留白后识别率最高；因此该通道不参与坐标匹配，只取离 `y` 最近且
    /// 在 `tolerance` 内的那一行，命中即视为该单元格的独立读数，优先
    /// 于其他通道。`y`/`tolerance` 是裁剪图（放大后）中的像素坐标。
    Cell {
        column: usize,
        row: usize,
        y: f64,
        tolerance: f64,
        weight: u32,
    },
    /// PDF 文本层整页。数值位置来自文本层，本身是精确的；但列中心
    /// 来自 OCR 识别出的子表头，宽表格里可能和数值实际位置有几十像素
    /// 偏差（按最近距离会把左列数值分到右列）。因此该通道按“同一行内
    /// 按 x 从左到右”顺序分配给各列，而不是逐行找最近列。
    Text { weight: u32 },
}

pub struct Pass<'a> {
    pub sheet: &'a OcrSheet,
    pub kind: PassKind,
}

/// 一次识别得到的候选值。除数值和权重外，记录原文中有多少位有效数字、
/// 是否带小数点：单元格通道的读数不一定可靠（Windows OCR 会把
/// 18.5 读成“1 & 5”、把 150.3 读成“1503”），用这两项质量指标来判断
/// 该信哪一个通道。
#[derive(Clone, Copy)]
struct Observation {
    value: f64,
    weight: u32,
    digits: u32,
    dotted: bool,
}

impl Observation {
    fn new(value: f64, weight: u32, text: &str) -> Self {
        let mut digits = 0;
        let mut dotted = false;
        for ch in text.chars() {
            match ch {
                '0'..='9'
                | '\u{FF10}'..='\u{FF19}'
                | 'O'
                | 'o'
                | 'Q'
                | 'I'
                | 'l'
                | '|' => digits += 1,
                '.' | '\u{FF0E}' | ',' | '\u{FF0C}' => dotted = true,
                _ => {}
            }
        }
        Self {
            value,
            weight,
            digits,
            dotted,
        }
    }

    /// 质量排序键：有效数字更多者优先；数量相同则带小数点者优先。
    fn rank(&self) -> (u32, bool) {
        (self.digits, self.dotted)
    }
}

/// 两个数值是否落在 `merge` 使用的同一容差内。
fn same_value(left: f64, right: f64) -> bool {
    (left - right).abs() <= (left.abs() * 0.006).max(0.02)
}

#[derive(Clone, Debug, Default)]
pub struct Recognized {
    values: [Option<f64>; Field::COUNT],
    conflict: [bool; Field::COUNT],
}

impl Recognized {
    pub fn value(&self, field: Field) -> Option<f64> {
        self.values[field.index()]
    }

    pub fn has_conflict(&self, field: Field) -> bool {
        self.conflict[field.index()]
    }

    pub fn recognized_fields(&self) -> Vec<Field> {
        Field::ALL
            .into_iter()
            .filter(|field| self.values[field.index()].is_some())
            .collect()
    }

    /// 把识别值写入电机；未识别字段保留 `motor` 的原值（若有）。
    pub fn apply(&self, motor: &mut Motor, rotary: bool) {
        macro_rules! set {
            ($field:expr, $target:ident) => {
                if let Some(value) = self.value($field) {
                    motor.$target = value;
                }
            };
        }
        set!(Field::Cont, cont);
        set!(Field::Peak, peak);
        set!(Field::Kf, kf);
        set!(Field::Ke, ke);
        set!(Field::Km, km);
        set!(Field::Resist, resist);
        set!(Field::Induct, induct);
        set!(Field::ICont, i_cont);
        set!(Field::IPeak, i_peak);
        set!(Field::RatedW, rated_w);
        set!(Field::Mass, mover_mass);
        set!(Field::Length, length);
        set!(Field::Inertia, inertia);
        set!(Field::MaxSpeed, max_speed);

        if rotary {
            motor.kind = MotorKind::Rotary;
            motor.construction = Construction::Standard;
        } else {
            let (kind, construction) = classify(&motor.name);
            motor.kind = kind;
            motor.construction = construction;
        }
        if motor.rated_w <= 0.0 && motor.km > 0.0 {
            motor.rated_w = (motor.cont / motor.km).powi(2);
        }
        if motor.max_w <= 0.0 && motor.km > 0.0 {
            motor.max_w = (motor.peak / motor.km).powi(2);
        }
        if motor.max_speed <= 0.0 {
            motor.max_speed = if rotary { 3000.0 } else { 5.0 };
        }
    }
}

/// 识别页面中的参数表。失败时返回 `None`，由调用方汇总为可读错误。
pub fn detect_table(sheet: &OcrSheet) -> Option<PageTable> {
    if sheet.width <= 0.0 || sheet.height <= 0.0 {
        return None;
    }
    let width = sheet.width;
    let height = sheet.height;

    // 1. 行标签：左侧、正文区域、可匹配别名。
    let mut rows: Vec<TableRow> = Vec::new();
    let mut rotary_hint = false;
    for line in &sheet.lines {
        if line.words.is_empty() || line.min_x() > width * 0.32 {
            continue;
        }
        let y = line.center_y();
        if y < height * 0.08 {
            continue;
        }
        let Some(field) = field_of_label(&line.joined()) else {
            continue;
        };
        if line.joined().contains("转矩") {
            rotary_hint = true;
        }
        if rows.iter().all(|row| row.field != field) {
            rows.push(TableRow {
                field,
                center_y: y,
            });
        }
    }
    if rows.len() < 3 {
        return None;
    }
    rows.sort_by(|a, b| a.center_y.total_cmp(&b.center_y));
    if rows.iter().any(|row| matches!(row.field, Field::Inertia | Field::MaxSpeed)) {
        rotary_hint = true;
    }

    // 2. 串联/并联子表头。只接受“串联/并联/串联接法”这类短标签；
    //    右侧转矩-转速图题里也含“串联接法”，但那是整句，不能当列标题，
    //    否则会凭空多出列并把 header_bottom 拉到图题位置，所有数据行被过滤。
    let mut subs: Vec<(f64, f64, String)> = Vec::new();
    for line in &sheet.lines {
        if line.words.is_empty() || line.min_x() < width * 0.28 {
            continue;
        }
        let Some(connection) = connection_label(&line.joined()) else {
            continue;
        };
        let x = line.center_x();
        let y = line.center_y();
        if subs
            .iter()
            .all(|(sx, sy, _)| (sx - x).abs() > 40.0 || (sy - y).abs() > 12.0)
        {
            subs.push((x, y, connection.to_owned()));
        }
    }
    subs.sort_by(|a, b| a.0.total_cmp(&b.0));

    let first_value_y = rows.first().map(|row| row.center_y).unwrap_or(height * 0.2);

    // 3. 列：优先使用子表头上方的型号标题（每列一个），否则退化为
    //    “没有串联/并联标记的单列/多列”。
    let mut columns = Vec::new();
    if !subs.is_empty() {
        let sub_top = subs.iter().map(|(_, y, _)| *y).fold(f64::INFINITY, f64::min);
        let headers = header_candidates(sheet, sub_top - 4.0, 160.0);
        let closest_y = headers
            .iter()
            .filter(|(_, y, _)| *y < sub_top - 4.0)
            .map(|(_, y, _)| *y)
            .fold(f64::NEG_INFINITY, f64::max);
        let mut group: Vec<(String, f64)> = headers
            .iter()
            .filter(|(_, y, _)| (*y - closest_y).abs() <= 8.0)
            .map(|(name, _, x)| (name.clone(), *x))
            .collect();
        group.sort_by(|a, b| a.1.total_cmp(&b.1));

        let paired: Option<Vec<(String, f64)>> = if group.len() == subs.len() {
            Some(group.clone())
        } else {
            None
        };
        for (index, (x, _, connection)) in subs.iter().enumerate() {
            let name = match &paired {
                Some(names) => names[index].0.clone(),
                None => {
                    let nearest = group
                        .iter()
                        .map(|(name, hx)| (name, (*hx - x).abs()))
                        .filter(|(_, dx)| *dx <= 150.0)
                        .min_by(|a, b| a.1.total_cmp(&b.1));
                    match nearest {
                        Some((name, _)) => name.clone(),
                        None => match group.first() {
                            Some((name, _)) => name.clone(),
                            None => continue,
                        },
                    }
                }
            };
            if let Some(composed) = compose_name(&name, connection, rotary_hint) {
                columns.push(TableColumn {
                    name: composed,
                    center_x: *x,
                });
            }
        }
    }

    if columns.is_empty() {
        // 没有串联/并联标记：把最低一组型号标题当成列名，不补连接字母。
        let headers = header_candidates(sheet, first_value_y - 4.0, 220.0);
        let closest_y = headers
            .iter()
            .map(|(_, y, _)| *y)
            .filter(|y| *y < first_value_y - 4.0)
            .fold(f64::NEG_INFINITY, f64::max);
        for (name, y, x) in &headers {
            if (*y - closest_y).abs() <= 8.0 {
                columns.push(TableColumn {
                    name: name.clone(),
                    center_x: *x,
                });
            }
        }
        columns.sort_by(|a, b| a.center_x.total_cmp(&b.center_x));
    }

    if columns.is_empty() {
        return None;
    }

    let min_gap = columns
        .windows(2)
        .map(|pair| (pair[1].center_x - pair[0].center_x).abs())
        .fold(f64::INFINITY, f64::min);
    let column_tolerance = (min_gap * 0.75).clamp(40.0, 120.0);
    let row_gap = rows
        .windows(2)
        .map(|pair| pair[1].center_y - pair[0].center_y)
        .fold(f64::INFINITY, f64::min);
    let row_tolerance = (row_gap * 0.55).clamp(10.0, 24.0);
    let header_bottom = if subs.is_empty() {
        first_value_y - 8.0
    } else {
        subs.iter().map(|(_, y, _)| *y).fold(f64::NEG_INFINITY, f64::max) + 10.0
    };
    let bottom = rows.iter().map(|row| row.center_y).fold(0.0, f64::max) + row_gap;

    Some(PageTable {
        page: sheet.page,
        rotary: rotary_hint,
        columns,
        rows,
        header_bottom,
        bottom,
        column_tolerance,
        row_tolerance,
        width,
        height,
    })
}

/// 在给定纵向范围内查找型号码标题，返回 (清洗后的名称, 中心 y, 中心 x)。
fn header_candidates(sheet: &OcrSheet, bottom: f64, band: f64) -> Vec<(String, f64, f64)> {
    let mut out = Vec::new();
    for line in &sheet.lines {
        if line.words.is_empty() {
            continue;
        }
        let y = line.center_y();
        if y > bottom || y < bottom - band {
            continue;
        }
        let text = line.joined();
        // 图题“转矩-转速曲线 AXD120-71 串联接法”清洗后也能得到
        // AXD120-71，但它不是列标题；含汉字的整句直接排除。
        if contains_chinese(&text) {
            continue;
        }
        let name = clean_header(&text);
        if !is_model_like(&name) {
            continue;
        }
        if out
            .iter()
            .any(|(other, oy, ox): &(String, f64, f64)| {
                other == &name && (*oy - y).abs() < 8.0 && (*ox - line.center_x()).abs() < 30.0
            })
        {
            continue;
        }
        out.push((name, y, line.center_x()));
    }
    out
}

/// 清洗 OCR 型号：全角/西里尔连字符归一，字母数字形近字纠正。
pub fn clean_header(text: &str) -> String {
    let mut out = String::new();
    for ch in text.chars() {
        // 全角 ASCII（Ａ、－ 等）先折回半角。
        let half = if ('\u{FF01}'..='\u{FF5E}').contains(&ch) {
            char::from_u32(ch as u32 - 0xFEE0).unwrap_or(ch)
        } else {
            ch
        };
        let mapped = match half {
            // 型号里的分隔符常被识别成破折号、中点或句点（AXD280·100），
            // 统一折成连字符，才能匹配数据库中的 AXD280-100。
            'һ' | '－' | '–' | '—' | '−' | '一' | '·' | '•' | '.' | '．' | '/' | '／' => '-',
            other => other,
        };
        if mapped.is_ascii_alphanumeric() || mapped == '-' {
            out.push(mapped.to_ascii_uppercase());
        }
    }
    let corrected: String = out
        .chars()
        .map(|ch| match ch {
            'I' | 'L' | '|' => '1',
            'O' | 'Q' => '0',
            other => other,
        })
        .collect();
    corrected.trim_matches('-').to_owned()
}

fn is_model_like(name: &str) -> bool {
    if name.len() < 4 || name.len() > 28 {
        return false;
    }
    let letters = name.chars().take_while(|ch| ch.is_ascii_alphabetic()).count();
    letters >= 2
        && name.contains('-')
        && name.chars().any(|ch| ch.is_ascii_digit())
        && name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-')
}

/// 由型号码标题和连接方式组合数据库中的型号名。
///
/// 直线：在最后一个 `-` 之前插入连接字母（`AUM1-S1` → `AUM1-S-S1`）；
/// 旋转：在末尾追加（`ADR110-A75` → `ADR110-A75-S`）。若标题中已经含有
/// 中间的 S/P 段，则替换而不是重复插入。
pub fn compose_name(header: &str, connection: &str, rotary: bool) -> Option<String> {
    if header.is_empty() || connection.is_empty() {
        return None;
    }
    let segments: Vec<&str> = header.split('-').filter(|part| !part.is_empty()).collect();
    if segments.len() < 2 {
        return None;
    }
    let mut parts: Vec<String> = segments.iter().map(|part| (*part).to_owned()).collect();
    if rotary {
        if matches!(parts.last().map(String::as_str), Some("S") | Some("P")) {
            *parts.last_mut()? = connection.to_owned();
        } else {
            parts.push(connection.to_owned());
        }
    } else if parts.len() >= 3
        && parts[1..parts.len() - 1]
            .iter()
            .any(|part| part == "S" || part == "P")
    {
        let end = parts.len() - 1;
        for part in &mut parts[1..end] {
            if part == "S" || part == "P" {
                *part = connection.to_owned();
            }
        }
    } else {
        let last = parts.len() - 1;
        parts.insert(last, connection.to_owned());
    }
    Some(parts.join("-"))
}

/// 收集所有识别通道的候选值。每个候选带权重，合并时按簇取最高权重。
pub fn collect(table: &PageTable, passes: &[Pass]) -> Vec<Recognized> {
    let mut candidates: Vec<Vec<Observation>> =
        (0..table.columns.len() * Field::COUNT).map(|_| Vec::new()).collect();
    // 单元格通道的读数单独存放：它只包含一个单元格，行列直接已知，
    // 不受整列扫描丢字/丢小数点的影响。
    let mut cell_candidates: Vec<Vec<Observation>> =
        (0..table.columns.len() * Field::COUNT).map(|_| Vec::new()).collect();

    for pass in passes {
        if let PassKind::Text { weight } = pass.kind {
            collect_text(pass.sheet, table, weight, &mut candidates);
            continue;
        }
        // 单元格通道：只取该格预期位置附近最近的一行数值。裁剪窗口两侧
        // 可能夹带相邻行（尤其是未被表头识别的行），按坐标取最近一行
        // 可以避免把邻居的数值记到这一格。
        if let PassKind::Cell {
            column,
            row,
            y,
            tolerance,
            weight,
        } = pass.kind
        {
            if column >= table.columns.len() || row >= table.rows.len() {
                continue;
            }
            let mut nearest: Option<(f64, Observation)> = None;
            for line in &pass.sheet.lines {
                if line.words.is_empty() {
                    continue;
                }
                let distance = (line.center_y() - y).abs();
                if distance > tolerance {
                    continue;
                }
                let text = line.joined();
                let Some(value) = parse_number(&text) else {
                    continue;
                };
                if nearest.map(|(best, _)| distance < best).unwrap_or(true) {
                    nearest = Some((distance, Observation::new(value, weight, &text)));
                }
            }
            if let Some((_, observation)) = nearest {
                let field = table.rows[row].field;
                cell_candidates[field.index() * table.columns.len() + column]
                    .push(observation);
            }
            continue;
        }
        for line in &pass.sheet.lines {
            if line.words.is_empty() {
                continue;
            }
            let text = line.joined();
            let Some(value) = parse_number(&text) else {
                continue;
            };
            let (y, x) = match pass.kind {
                PassKind::Column { top, scale, .. } => {
                    (line.center_y() / scale.max(0.01) + top, line.center_x())
                }
                PassKind::Full { scale, .. } => {
                    let scale = scale.max(0.01);
                    (line.center_y() / scale, line.center_x() / scale)
                }
                PassKind::Band {
                    x0,
                    top,
                    scale_x,
                    scale_y,
                    ..
                } => {
                    let scale_x = scale_x.max(0.01);
                    let scale_y = scale_y.max(0.01);
                    (
                        line.center_y() / scale_y + top,
                        line.center_x() / scale_x + x0,
                    )
                }
                // 单元格与文本层通道已在上面单独处理，不会走到这里。
                PassKind::Cell { .. } | PassKind::Text { .. } => continue,
            };
            if y < table.header_bottom + 4.0 || y > table.bottom + 12.0 {
                continue;
            }
            let Some(row_index) = nearest_row(table, y) else {
                continue;
            };
            // 结果按字段枚举下标存放；行在页面上的顺序与字段枚举顺序
            // 不一定一致（“最高转速”排在“转动惯量”前后都可能），
            // 用 row_index 存会让 MaxSpeed/Inertia 等字段永远取不到值。
            let field = table.rows[row_index].field;
            let index = match pass.kind {
                PassKind::Column { column, weight, .. } => {
                    if column >= table.columns.len() {
                        continue;
                    }
                    Some((column, weight))
                }
                // 整页和单行条带都按 x 坐标落到最近的列；条带的 x 坐标
                // 已经映射回页面坐标系，因此两者可以共用同一套逻辑。
                PassKind::Full { weight, .. } | PassKind::Band { weight, .. } => {
                    nearest_column(table, x).map(|col| (col, weight))
                }
                // 单元格与文本层通道不会走到这里。
                PassKind::Cell { .. } | PassKind::Text { .. } => continue,
            };
            let Some((column, weight)) = index else {
                continue;
            };
            candidates[field.index() * table.columns.len() + column]
                .push(Observation::new(value, weight, &text));
        }
    }

    let mut out = Vec::with_capacity(table.columns.len());
    for column in 0..table.columns.len() {
        let mut recognized = Recognized::default();
        for field in Field::ALL {
            let cell = field.index() * table.columns.len() + column;
            let (value, conflict) = decide(&candidates[cell], &cell_candidates[cell]);
            recognized.values[field.index()] = value;
            recognized.conflict[field.index()] = conflict;
        }
        out.push(recognized);
    }
    out
}

/// 文本层整页收集：按行分组，行内按 x 顺序对应到各列。
fn collect_text(
    sheet: &OcrSheet,
    table: &PageTable,
    weight: u32,
    candidates: &mut [Vec<Observation>],
) {
    let mut rows: Vec<Vec<(f64, Observation)>> = vec![Vec::new(); table.rows.len()];
    for line in &sheet.lines {
        if line.words.is_empty() {
            continue;
        }
        let text = line.joined();
        let Some(value) = parse_number(&text) else {
            continue;
        };
        let y = line.center_y();
        let x = line.center_x();
        if y < table.header_bottom + 4.0 || y > table.bottom + 12.0 {
            continue;
        }
        let Some(row_index) = nearest_row(table, y) else {
            continue;
        };
        // 只接收贴近数值列的读数：左侧标签（如“转矩常数±10%”）和
        // 右侧曲线/图纸标注里的数字不能混进表格。
        let limit = (table.column_tolerance * 1.5).max(60.0);
        if table
            .columns
            .iter()
            .all(|column| (column.center_x - x).abs() > limit)
        {
            continue;
        }
        rows[row_index].push((x, Observation::new(value, weight, &text)));
    }

    let mut order: Vec<usize> = (0..table.columns.len()).collect();
    order.sort_by(|a, b| table.columns[*a].center_x.total_cmp(&table.columns[*b].center_x));

    for (row_index, mut values) in rows.into_iter().enumerate() {
        if values.is_empty() {
            continue;
        }
        values.sort_by(|a, b| a.0.total_cmp(&b.0));
        let field = table.rows[row_index].field;
        if values.len() == order.len() {
            // 每一列都恰好一个数值：按从左到右的顺序一一对应，不受
            // 列中心与数值实际位置偏差的影响。
            for (slot, (_, observation)) in values.into_iter().enumerate() {
                candidates[field.index() * table.columns.len() + order[slot]].push(observation);
            }
            continue;
        }
        // 数量对不上（缺列或有多余读数）时只在距离足够近时按最近列
        // 归类，宁缺毋滥，避免把左列数值错记到右列。
        let near = table.column_tolerance * 0.5;
        for (x, observation) in values {
            if let Some(column) = nearest_column(table, x) {
                if (table.columns[column].center_x - x).abs() <= near {
                    candidates[field.index() * table.columns.len() + column].push(observation);
                }
            }
        }
    }
}

fn nearest_column(table: &PageTable, x: f64) -> Option<usize> {
    let mut best: Option<(usize, f64)> = None;
    for (col, column) in table.columns.iter().enumerate() {
        let distance = (column.center_x - x).abs();
        if distance <= table.column_tolerance
            && best.map(|(_, d)| distance < d).unwrap_or(true)
        {
            best = Some((col, distance));
        }
    }
    best.map(|(col, _)| col)
}

fn nearest_row(table: &PageTable, y: f64) -> Option<usize> {
    let mut best: Option<(usize, f64)> = None;
    for (index, row) in table.rows.iter().enumerate() {
        let distance = (row.center_y - y).abs();
        if distance <= table.row_tolerance && best.map(|(_, d)| distance < d).unwrap_or(true) {
            best = Some((index, distance));
        }
    }
    best.map(|(index, _)| index)
}

/// 把候选值按 0.6% 相对容差聚类，取总权重最高的簇；出现两个以上
/// 有分量的簇时标记冲突，提示用户核对。
pub fn merge(candidates: &[(f64, u32)]) -> (Option<f64>, bool) {
    let mut clusters: Vec<(f64, u32, u32)> = Vec::new(); // 均值、总权重、数量
    for &(value, weight) in candidates {
        let mut found = false;
        for cluster in &mut clusters {
            let tolerance = (cluster.0.abs() * 0.006).max(0.02);
            if (cluster.0 - value).abs() <= tolerance {
                cluster.0 = (cluster.0 * cluster.1 as f64 + value * weight as f64)
                    / (cluster.1 + weight) as f64;
                cluster.1 += weight;
                cluster.2 += 1;
                found = true;
                break;
            }
        }
        if !found {
            clusters.push((value, weight, 1));
        }
    }
    let best = clusters
        .iter()
        .max_by_key(|(_, weight, _)| *weight)
        .copied();
    let conflict = clusters.iter().filter(|(_, weight, _)| *weight >= 2).count() > 1;
    (best.map(|(value, _, _)| value), conflict)
}

/// 合并普通通道（整页/整列/条带）与单元格通道的读数。
///
/// 两个通道各有所长：整列裁剪给出的数字通常完整，但会丢小数点
/// （3.3 被读成 33、150.3 被读成 1503）；单元格裁剪留白后小数点稳定，
/// 但偶尔会把 18.5 读成“1 & 5”。因此不按通道权重硬性取舍，而是先让
/// 每个通道各自选出质量最高的读数，再比较质量（有效数字位数、是否带
/// 小数点）：质量高者胜出；质量相同时单元格通道优先，若数值仍不同
/// 则标记冲突提示用户核对。
fn decide(other: &[Observation], cell: &[Observation]) -> (Option<f64>, bool) {
    let pairs: Vec<(f64, u32)> = other.iter().map(|o| (o.value, o.weight)).collect();
    let (other_value, other_conflict) = merge(&pairs);

    let best_cell = cell
        .iter()
        .max_by(|a, b| a.rank().cmp(&b.rank()).then(a.weight.cmp(&b.weight)));
    let cell_conflict = best_cell
        .map(|best| {
            cell.iter()
                .any(|o| !same_value(o.value, best.value) && o.rank() == best.rank())
        })
        .unwrap_or(false);

    let Some(best) = best_cell else {
        return (other_value, other_conflict);
    };
    let Some(other_value) = other_value else {
        return (Some(best.value), cell_conflict);
    };
    if same_value(best.value, other_value) {
        return (Some(best.value), cell_conflict || other_conflict);
    }

    // 普通通道里支持 `other_value` 的读数中质量最高者；若一个都对不上
    // （聚类均值偏离所有成员），视为质量最低。
    let other_quality = other
        .iter()
        .filter(|o| same_value(o.value, other_value))
        .max_by(|a, b| a.rank().cmp(&b.rank()).then(a.weight.cmp(&b.weight)));
    let ordering = match other_quality {
        Some(quality) => best
            .rank()
            .cmp(&quality.rank())
            .then(best.weight.cmp(&quality.weight)),
        None => std::cmp::Ordering::Greater,
    };
    match ordering {
        std::cmp::Ordering::Greater => (Some(best.value), cell_conflict),
        std::cmp::Ordering::Less => (Some(other_value), other_conflict),
        std::cmp::Ordering::Equal => (Some(best.value), true),
    }
}

/// OCR 数字归一化：全角标点、误识别的“口/丿/t”、千位空格、单位后缀。
pub fn parse_number(text: &str) -> Option<f64> {
    let mut source: String = text
        .chars()
        .filter(|ch| !ch.is_whitespace())
        .map(|ch| match ch {
            '\u{FF10}'..='\u{FF19}' => char::from_u32(ch as u32 - 0xFEE0).unwrap_or(ch),
            '\u{FF0E}' => '.',
            '\u{FF0C}' => ',',
            other => other,
        })
        .collect();

    // 前缀“s/S”常是“5.”的误识别（样例中的 s18 应为 5.18）。
    if let Some(rest) = source.strip_prefix('s').or_else(|| source.strip_prefix('S')) {
        if rest.len() >= 2
            && rest
                .chars()
                .next()
                .map(|ch| ch.is_ascii_digit())
                .unwrap_or(false)
        {
            source = format!("5.{rest}");
        }
    }
    // 前缀“L”常是“1.”的误识别（“L24”应为 1.24）。
    if let Some(rest) = source.strip_prefix('L') {
        if rest.len() >= 2
            && rest
                .chars()
                .next()
                .map(|ch| ch.is_ascii_digit())
                .unwrap_or(false)
        {
            source = format!("1.{rest}");
        }
    }

    // 值必须以数字或已知形近字开头；型号文本（AUM1-S1）不能变成“1”。
    let first = source.chars().next()?;
    let first_ok = first.is_ascii_digit()
        || matches!(first, '-' | '−' | '一' | 'O' | 'o' | 'Q' | 'I' | 'l' | '|');
    if !first_ok {
        return None;
    }

    let chars: Vec<char> = source.chars().collect();
    let mut buffer = String::new();
    let mut seen_digit = false;
    let mut last_digit = false;
    let mut after_exponent = false;
    for (index, &ch) in chars.iter().enumerate() {
        let next = chars.get(index + 1).copied();
        // “2t0”“1L9”“5L1”常见于“21.0”“11.9”“51.1”中
        // 小数点与数字 1 粘连的误识别。
        if (ch == 't' || ch == 'L')
            && last_digit
            && next.map(|n| n.is_ascii_digit()).unwrap_or(false)
        {
            buffer.push_str("1.");
            last_digit = false;
            continue;
        }
        let mapped_digit = match ch {
            '0'..='9' => Some(ch),
            'O' | 'o' | 'Q' => Some('0'),
            'I' | 'l' | '|' => Some('1'),
            _ => None,
        };
        if let Some(digit) = mapped_digit {
            buffer.push(digit);
            seen_digit = true;
            last_digit = true;
            after_exponent = false;
            continue;
        }
        // 科学记数法里的 E 常被识成“巳/已”（4.15E-03 写成
        // “4.15巳03”）。只有后面紧跟符号时才可还原；若后面直接是
        // 指数数字，说明负号也丢了，无法判断正负——宁可放弃该行，
        // 也不要得到 4150 这种离谱值。
        if last_digit && matches!(ch, '巳' | '已') {
            if next.map(|n| n.is_ascii_digit()).unwrap_or(false) {
                return None;
            }
            buffer.push('e');
            last_digit = false;
            after_exponent = true;
            continue;
        }
        // “彐/彑”无法可靠归一化：它可能是小数点，也可能整体替换了
        // “15”中的字符（“4.15E-03”被识成“4彐5E-03”）。此时若按
        // 普通结束符截断，会得到 4.0 这类错误值；宁可放弃该行，留给
        // 更高分辨率的补扫通道。
        if matches!(ch, '彐' | '彑') {
            return None;
        }
        // 指数符号也经常被 OCR 成小数点、间隔号或破折号：E·03、E．03、
        // E一03 都应还原为 E-03。此时后面必须紧跟指数数字，否则按普通
        // 字符结束解析，避免“E”后无数字时产生半截浮点数。
        if after_exponent
            && !last_digit
            && next.map(|n| n.is_ascii_digit()).unwrap_or(false)
        {
            if ch != '+' {
                buffer.push('-');
            }
            continue;
        }
        let separator = matches!(ch, '.' | ',' | '，' | '、' | '．' | '。' | '·' | ':' | '；' | ';');
        let ocr_dot = matches!(ch, '口' | '丿' | '/')
            && last_digit
            && next.map(|n| n.is_ascii_digit()).unwrap_or(false);
        if (separator && last_digit) || ocr_dot {
            buffer.push('.');
            last_digit = false;
            continue;
        }
        if (ch == 'e' || ch == 'E') && last_digit {
            buffer.push('e');
            last_digit = false;
            after_exponent = true;
            continue;
        }
        if (ch == '-' || ch == '−' || ch == '一') && (buffer.is_empty() || after_exponent) {
            buffer.push('-');
            last_digit = false;
            continue;
        }
        // 单位、中文说明等：数字结束后立即停止。但如果剩余文本里还有
        // E/e 指数标记，说明整段科学记数法被 OCR 打散（5.15E-01 被识成
        // “5 xx xx E 一 01”），截断只会得到 5.0 这种错误值；放弃整行，
        // 交给逐格通道重新识别。
        if seen_digit {
            if chars[index..]
                .iter()
                .any(|rest| matches!(rest, 'e' | 'E' | '巳' | '已' | 'ｅ' | 'Ｅ'))
            {
                return None;
            }
            break;
        }
    }
    if !seen_digit {
        return None;
    }
    let value: f64 = buffer.parse().ok()?;
    if !value.is_finite() || value < 0.0 || value > 1.0e7 {
        return None;
    }
    Some(value)
}

/// 连接方式归一化后的比较键：`AUM1-S-S1` 与 `AUM1-P-S1` 都折叠为
/// `AUM1-S1`；旋转型号不受影响。
pub fn connection_collapsed(name: &str) -> String {
    let segments: Vec<&str> = name.split('-').filter(|part| !part.is_empty()).collect();
    if segments.len() >= 3 && (segments[1] == "S" || segments[1] == "P") {
        let mut parts = segments;
        parts.remove(1);
        parts.join("-")
    } else {
        segments.join("-")
    }
}

/// 在现有数据库中查找同名记录：先精确匹配，再回退到唯一的连接方式折叠匹配。
pub fn match_existing(name: &str, motors: &[Motor]) -> Option<usize> {
    if let Some(index) = motors
        .iter()
        .position(|motor| motor.name.eq_ignore_ascii_case(name))
    {
        return Some(index);
    }
    let key = connection_collapsed(&name.to_ascii_uppercase());
    let mut found = None;
    for (index, motor) in motors.iter().enumerate() {
        if connection_collapsed(&motor.name.to_ascii_uppercase()) == key {
            if found.is_some() {
                return None;
            }
            found = Some(index);
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::import::ocr::{OcrWord, OcrLine};

    fn word(text: &str, x: f64, y: f64, w: f64) -> OcrWord {
        OcrWord { text: text.to_owned(), x, y, w, h: 18.0 }
    }

    fn line(words: Vec<OcrWord>) -> OcrLine {
        let text = words.iter().map(|w| w.text.as_str()).collect::<Vec<_>>().join(" ");
        OcrLine { text, words }
    }

    fn sheet(width: f64, height: f64, lines: Vec<OcrLine>) -> OcrSheet {
        OcrSheet { id: "1".into(), page: 1, width, height, lines }
    }

    fn label(text: &str, y: f64) -> OcrLine {
        line(vec![word(text, 60.0, y - 9.0, 160.0)])
    }

    fn cell(text: &str, x: f64, y: f64) -> OcrLine {
        line(vec![word(text, x - 20.0, y - 9.0, 40.0)])
    }

    /// 两列直线参数表：AUM2-S1 串联/并联 各一行数据。
    fn linear_two_columns() -> OcrSheet {
        let mut lines = vec![
            line(vec![word("AUM2-S1", 410.0, 120.0, 110.0)]),
            line(vec![word("AUM2-S1", 610.0, 120.0, 110.0)]),
            line(vec![word("串联", 420.0, 152.0, 50.0)]),
            line(vec![word("并联", 620.0, 152.0, 50.0)]),
        ];
        for (i, (name, field)) in [
            ("持续推力", "3.0"),
            ("峰值推力", "11.9"),
            ("力常数", "1.75"),
            ("反电势常数", "0.15"),
            ("电机常数", "0.14"),
            ("相间电阻", "1.7"),
            ("相间电感", "6.2"),
            ("持续电流", "1.7"),
            ("峰值电流", "6.8"),
            ("持续热功率", "21.0"),
            ("线圈质量", "0.03"),
            ("线圈长度", "22.0"),
        ]
        .into_iter()
        .enumerate()
        {
            let y = 190.0 + i as f64 * 40.0;
            lines.push(label(name, y));
            lines.push(cell(field, 420.0, y));
            lines.push(cell(field, 620.0, y));
        }
        sheet(800.0, 800.0, lines)
    }

    /// AXD 样册页：左侧参数标签，右侧串联/并联两列数值，表格右侧另有
    /// 转矩-转速图题（同一行里含型号和“串联接法/并联接法”）。
    fn datasheet_with_chart_titles() -> OcrSheet {
        let mut lines = vec![
            // 型号标题中的“．”是印刷体中点，OCR 可能识别成句点。
            line(vec![
                word("AXD280", 646.0, 424.0, 110.0),
                word("．", 766.0, 424.0, 20.0),
                word("100", 790.0, 424.0, 60.0),
            ]),
            line(vec![word("串联", 1055.0, 473.0, 40.0)]),
            line(vec![word("并联", 1174.0, 473.0, 40.0)]),
            line(vec![
                word("转矩一转速曲线", 1633.0, 520.0, 600.0),
                word("AXD280一100", 2250.0, 520.0, 100.0),
                word("串联接法", 2360.0, 520.0, 60.0),
            ]),
            line(vec![
                word("转矩一转速曲线", 1633.0, 1010.0, 600.0),
                word("AXD280一100", 2250.0, 1010.0, 100.0),
                word("并联接法", 2360.0, 1010.0, 60.0),
            ]),
        ];
        for (i, (name, series, parallel)) in [
            ("持续转矩", "5.0", "2.5"),
            ("峰值转矩", "10.0", "5.0"),
            ("转矩常数", "0.26", "0.13"),
            ("反电势常数", "0.5", "0.25"),
            ("电机常数", "0.14", "0.07"),
            ("相间电阻", "1.7", "0.85"),
            ("相间电感", "6.2", "3.1"),
            ("持续电流", "1.7", "3.4"),
            ("峰值电流", "6.8", "13.6"),
            ("持续热功率", "21.0", "42.0"),
            ("最高转速@持续转矩", "1700", "2000"),
            ("转动惯量", "0.001", "0.001"),
        ]
        .into_iter()
        .enumerate()
        {
            let y = 509.0 + i as f64 * 38.0;
            lines.push(label(name, y));
            lines.push(cell(series, 1085.0, y));
            lines.push(cell(parallel, 1205.0, y));
        }
        sheet(2700.0, 3583.0, lines)
    }

    #[test]
    fn text_pass_uses_row_order_when_columns_are_shifted() {
        // 模拟宽列样册（如 324 页目录的 AXD 页）：子表头位置在列左缘，
        // 数值整体右移后更靠近右侧列的中心，按“最近列”会分错。
        let mut sample = datasheet_with_chart_titles();
        for line in &mut sample.lines {
            for word in &mut line.words {
                if (word.x - 1065.0).abs() < 0.5 {
                    word.x += 85.0;
                }
                if (word.x - 1185.0).abs() < 0.5 {
                    word.x += 105.0;
                }
            }
        }
        let table = detect_table(&sample).expect("应识别参数表");
        let recognized = collect(
            &table,
            &[Pass {
                sheet: &sample,
                kind: PassKind::Text { weight: 1 },
            }],
        );
        assert_eq!(recognized[0].value(Field::Cont), Some(5.0), "串联列");
        assert_eq!(recognized[1].value(Field::Cont), Some(2.5), "并联列");
        assert_eq!(recognized[0].value(Field::Kf), Some(0.26));
        assert_eq!(recognized[1].value(Field::ICont), Some(3.4));
    }

    #[test]
    fn chart_titles_do_not_become_columns_or_hide_values() {
        let sample = datasheet_with_chart_titles();
        let table = detect_table(&sample).expect("应识别参数表");
        assert!(table.rotary);
        assert_eq!(table.columns.len(), 2, "图题不能生成额外的串联/并联列");
        assert_eq!(table.columns[0].name, "AXD280-100-S");
        assert_eq!(table.columns[1].name, "AXD280-100-P");
        assert!(
            table.header_bottom < table.rows[0].center_y,
            "表头下沿必须在第一行数据之上"
        );

        let recognized = collect(
            &table,
            &[Pass {
                sheet: &sample,
                kind: PassKind::Full { weight: 1, scale: 1.0 },
            }],
        );
        assert_eq!(recognized[0].value(Field::Cont), Some(5.0));
        assert_eq!(recognized[0].value(Field::Peak), Some(10.0));
        assert_eq!(recognized[0].value(Field::Kf), Some(0.26));
        assert_eq!(recognized[1].value(Field::ICont), Some(3.4));
        // MaxSpeed/Inertia 在页面上排在其它字段之后，必须按字段下标归位。
        assert_eq!(recognized[1].value(Field::MaxSpeed), Some(2000.0));
        assert_eq!(recognized[0].value(Field::Inertia), Some(0.001));
    }

    #[test]
    fn detects_linear_two_connection_columns() {
        let table = detect_table(&linear_two_columns()).expect("应识别参数表");
        assert_eq!(table.columns.len(), 2);
        assert_eq!(table.columns[0].name, "AUM2-S-S1");
        assert_eq!(table.columns[1].name, "AUM2-P-S1");
        assert!(table.rows.len() >= 10);
        assert!(!table.rotary);
    }

    #[test]
    fn composes_rotary_and_linear_names() {
        assert_eq!(compose_name("ADR110-A75", "S", true), Some("ADR110-A75-S".into()));
        assert_eq!(compose_name("ADR110-A75-S", "P", true), Some("ADR110-A75-P".into()));
        assert_eq!(compose_name("AUM1-S1", "S", false), Some("AUM1-S-S1".into()));
        assert_eq!(compose_name("AUM2-S1", "P", false), Some("AUM2-P-S1".into()));
        assert_eq!(compose_name("AUM1-S-S1", "P", false), Some("AUM1-P-S1".into()));
        assert_eq!(compose_name("AUM1", "S", false), None);
    }

    #[test]
    fn cleans_ocr_model_names() {
        assert_eq!(clean_header("AUMI -SI"), "AUM1-S1");
        assert_eq!(clean_header("ADRI10һA75"), "ADR110-A75");
        assert_eq!(clean_header("ＡＤＲ110－Ａ75"), "ADR110-A75");
        // 样册常用中点分隔型号（AXD280·100），OCR 也可能输出句点。
        assert_eq!(clean_header("AXD280·100"), "AXD280-100");
        assert_eq!(clean_header("AXD280 ． 100"), "AXD280-100");
    }

    #[test]
    fn parses_messy_numbers() {
        assert_eq!(parse_number("11，9"), Some(11.9));
        assert_eq!(parse_number("12 口 8"), Some(12.8));
        assert_eq!(parse_number("s18"), Some(5.18));
        assert_eq!(parse_number("3．086E一04"), Some(0.0003086));
        // 科学记数法的负号/字母常被识成间隔号、句点或汉字“巳”。
        assert_eq!(parse_number("4，15E·03"), Some(0.00415));
        assert_eq!(parse_number("4．15E一03"), Some(0.00415));
        assert_eq!(parse_number("4 ． 15 巳 03"), None);
        assert_eq!(parse_number("1.18E．03"), Some(0.00118));
        // 符号保留时仍可还原汉字 E。
        assert_eq!(parse_number("4 ． 15 巳 一 03"), Some(0.00415));
        // 无法还原的“彐”会让数字半途截断，必须放弃整行而不是误报 4.0。
        assert_eq!(parse_number("4 彐 5E 一 03"), None);
        assert_eq!(parse_number("6.8 Arms"), Some(6.8));
        // 被 OCR 打散的科学记数法不能在首个乱码处截断成 5.0。
        assert_eq!(parse_number("5口口E一01"), None);
        assert_eq!(parse_number("2t0"), Some(21.0));
        assert_eq!(parse_number("1L9"), Some(11.9));
        assert_eq!(parse_number("5L1"), Some(51.1));
        assert_eq!(parse_number("L24"), Some(1.24));
        assert_eq!(parse_number("L90"), Some(1.90));
        assert_eq!(parse_number("100"), Some(100.0));
        assert_eq!(parse_number("N/Arms"), None);
        assert_eq!(parse_number("AUM1-S1"), None);
        assert_eq!(parse_number(""), None);
    }

    #[test]
    fn cell_pass_overrides_decimal_lost_by_column_crop() {
        let mut sample = datasheet_with_chart_titles();
        // 模拟整页/整列通道把并联电阻 0.85 读成 33（小数点和首位都丢失）。
        let index = sample
            .lines
            .iter()
            .position(|line| line.text == "0.85")
            .expect("样表应含并联电阻 0.85");
        sample.lines[index] = cell("33", 1205.0, 699.0);
        let table = detect_table(&sample).expect("应识别参数表");

        let whole_page = collect(
            &table,
            &[Pass {
                sheet: &sample,
                kind: PassKind::Full { weight: 1, scale: 1.0 },
            }],
        );
        assert_eq!(whole_page[1].value(Field::Resist), Some(33.0));

        // 单元格通道只给正确读数；它存在时应完全覆盖低分辨率通道，
        // 且不把“33”当成冲突。
        let cell_sheet = sheet(2700.0, 3583.0, vec![cell("3.3", 200.0, 100.0)]);
        let recognized = collect(
            &table,
            &[
                Pass {
                    sheet: &sample,
                    kind: PassKind::Full { weight: 1, scale: 1.0 },
                },
                Pass {
                    sheet: &cell_sheet,
                    kind: PassKind::Cell {
                        column: 1,
                        row: 5,
                        y: 100.0,
                        tolerance: 100.0,
                        weight: 2,
                    },
                },
            ],
        );
        assert_eq!(recognized[1].value(Field::Resist), Some(3.3));
        assert!(
            !recognized[1].has_conflict(Field::Resist),
            "单元格通道是权威读数，不应产生冲突"
        );
    }

    #[test]
    fn merges_repeated_passes_by_weight() {
        let (value, conflict) = merge(&[(5.18, 1), (5.18, 3), (5.18, 2)]);
        assert!((value.unwrap() - 5.18).abs() < 1e-9);
        assert!(!conflict);
        let (value, conflict) = merge(&[(18.0, 1), (5.18, 3), (5.18, 2)]);
        assert!((value.unwrap() - 5.18).abs() < 1e-9);
        assert!(!conflict, "低权重误识别不应形成冲突");
        let (_, conflict) = merge(&[(5.18, 2), (5.3, 2)]);
        assert!(conflict);
        assert_eq!(merge(&[]).0, None);
    }

    fn named(name: &str) -> Motor {
        Motor {
            name: name.to_owned(),
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

    #[test]
    fn matches_existing_with_connection_fallback() {
        let motors = vec![
            named("AUM1-S-S1"),
            named("AUM2-S-S1"),
            named("AUM2-P-S1"),
        ];
        assert_eq!(match_existing("AUM1-S-S1", &motors), Some(0));
        assert_eq!(match_existing("AUM1-S1", &motors), Some(0));
        assert_eq!(match_existing("AUM2-S1", &motors), None, "两个候选不应自动覆盖");
        assert_eq!(match_existing("ADR110-A75-S", &motors), None);
        assert_eq!(connection_collapsed("AUM1-P-S1"), "AUM1-S1");
        assert_eq!(connection_collapsed("ADR110-A75-S"), "ADR110-A75-S");
    }

    #[test]
    fn collects_values_from_full_and_column_passes() {
        let table = detect_table(&linear_two_columns()).expect("应识别参数表");
        let full = linear_two_columns();
        let crop = OcrSheet {
            id: "c1".into(),
            page: 1,
            width: 100.0,
            height: 500.0,
            lines: vec![cell("11，9", 50.0, 60.0)],
        };
        let recognized = collect(
            &table,
            &[
                Pass {
                    sheet: &full,
                    kind: PassKind::Full { weight: 1, scale: 1.0 },
                },
                Pass {
                    sheet: &crop,
                    kind: PassKind::Column { column: 1, top: 190.0, scale: 1.5, weight: 3 },
                },
            ],
        );
        assert_eq!(recognized.len(), 2);
        assert!((recognized[1].value(Field::Peak).unwrap() - 11.9).abs() < 0.001);
        assert!(recognized[0].value(Field::Cont).unwrap() > 0.0);
    }

    #[test]
    fn band_pass_recovers_row_dropped_by_column_passes() {
        let mut page = linear_two_columns();
        // 模拟 Windows OCR 整页/整列识别漏掉“持续推力”这一行。
        page.lines
            .retain(|line| !line.words.iter().any(|w| w.text == "3.0"));

        let table = detect_table(&page).expect("应识别参数表");
        let missing = collect(
            &table,
            &[Pass {
                sheet: &page,
                kind: PassKind::Full { weight: 1, scale: 1.0 },
            }],
        );
        assert!(missing[0].value(Field::Cont).is_none());
        assert!(missing[1].value(Field::Cont).is_none());

        // 4 倍放大的单行条带：裁剪左上角在页面 (360, 175)，宽 320、高 30。
        // OCR 坐标映射回页面后应分别落到串联列 (x≈420) 和并联列 (x≈620)。
        let band = sheet(
            1280.0,
            120.0,
            vec![
                cell("3.0", 240.0, 69.0),
                cell("11.9", 1040.0, 69.0),
            ],
        );
        let recognized = collect(
            &table,
            &[
                Pass {
                    sheet: &page,
                    kind: PassKind::Full { weight: 1, scale: 1.0 },
                },
                Pass {
                    sheet: &band,
                    kind: PassKind::Band {
                        x0: 360.0,
                        top: 175.0,
                        scale_x: 4.0,
                        scale_y: 4.0,
                        weight: 3,
                    },
                },
            ],
        );
        assert!((recognized[0].value(Field::Cont).unwrap() - 3.0).abs() < 0.001);
        assert!((recognized[1].value(Field::Cont).unwrap() - 11.9).abs() < 0.001);
    }
}
