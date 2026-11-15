//! 从 PDF 文本层提取参数表文字（用于矢量型样册）。
//!
//! Windows OCR 对孤立的短小数几乎没有识别能力：同一份 AXD 样册里
//! “1.5”“18.5”“2.5”这类只有三个字符的单元格，无论放大多少倍、
//! 怎样留白或加标记，zh-Hans 引擎都会整行丢弃；而这类 PDF 本身带有
//! 完整文本层（字体 + ToUnicode 映射 + 排版坐标）。直接读取文本层
//! 既没有识别误差，也不需要渲染整页图像，速度快几个数量级。
//!
//! 扫描件（没有文本层）仍然走原有 OCR 流程；本模块返回空结果时由
//! `workflow` 自动回退。
//!
//! 实现范围：经典 xref/对象表、FlateDecode、Type0（Identity-H）与
//! 简单字体、ToUnicode CMap、横向排版文字。旋转文字（如竖排表头）
//! 不参与参数识别，直接跳过。

use std::collections::HashMap;
use std::fs;
use std::path::Path;

use flate2::read::ZlibDecoder;
use std::io::Read;

use crate::import::ocr::{OcrLine, OcrSheet, OcrWord};

/// 页面坐标换算：与 Windows 按 1800 宽渲染 A4 页（实际 2700 像素）
/// 的比例一致，使解析模块中的像素阈值保持原有含义。
const PIXELS_PER_POINT: f64 = 4.53;

/// 从 PDF 的文本层提取每一页的文字行。页码为 1 基；`end_page` 为 0
/// 表示到最后一页。没有文本层的页面会被跳过；整体失败时返回 Err，
/// 调用方据此回退到 OCR。
pub fn read_text_sheets(
    path: &Path,
    start_page: u32,
    end_page: u32,
) -> Result<(Vec<OcrSheet>, u32), String> {
    let data = fs::read(path).map_err(|error| format!("无法读取 PDF：{error}"))?;
    let pdf = Pdf::parse(&data)?;
    let pages = pdf.pages()?;
    let total = pages.len() as u32;
    let first = start_page.max(1) as usize;
    let last = if end_page == 0 {
        pages.len()
    } else {
        (end_page as usize).min(pages.len())
    };
    if first > last {
        return Ok((Vec::new(), total));
    }

    let mut sheets = Vec::new();
    for (index, page) in pages.iter().take(last).skip(first - 1).enumerate() {
        let number = (first + index) as u32;
        let Some((width, height)) = page.size else {
            continue;
        };
        let runs = pdf.page_runs(page).unwrap_or_default();
        let lines = group_runs(runs, height);
        if lines.is_empty() {
            continue;
        }
        sheets.push(OcrSheet {
            id: number.to_string(),
            page: number,
            width: width * PIXELS_PER_POINT,
            height: height * PIXELS_PER_POINT,
            lines,
        });
    }
    Ok((sheets, total))
}

/// 一个被绘制出来的文字片段（在 PDF 用户坐标中，单位磅）。
struct TextRun {
    text: String,
    x: f64,
    y: f64,
    size: f64,
    width: f64,
}

/// 把文字片段按纵向基线聚成行，再按较大的横向间距切成“单元格”，
/// 与 OCR 输出的一行一个数值保持一致。
fn group_runs(runs: Vec<TextRun>, page_height: f64) -> Vec<OcrLine> {
    let mut items: Vec<TextRun> = runs
        .into_iter()
        .filter(|run| !run.text.trim().is_empty())
        .collect();
    if items.is_empty() {
        return Vec::new();
    }
    items.sort_by(|a, b| {
        b.y.total_cmp(&a.y)
            .then_with(|| a.x.total_cmp(&b.x))
    });

    let mut lines: Vec<Vec<TextRun>> = Vec::new();
    for run in items {
        let size = run.size.max(1.0);
        let tolerance = (size * 0.45).max(1.5);
        match lines.iter_mut().find(|line| {
            let baseline = line.iter().map(|item| item.y).sum::<f64>() / line.len() as f64;
            (baseline - run.y).abs() <= tolerance
        }) {
            Some(line) => line.push(run),
            None => lines.push(vec![run]),
        }
    }

    let mut out = Vec::new();
    for mut line in lines {
        line.sort_by(|a, b| a.x.total_cmp(&b.x));
        let size = line.iter().map(|item| item.size).fold(0.0_f64, f64::max);
        // 表格单元格之间的间距由竖线决定，通常在 13 磅以上；同一单元格
        // 内的字符/片段间距更小。用固定阈值切分，避免字号不同（表头 8 磅、
        // 数值 5 磅）时把相邻两列合并成一个数值串。
        const CELL_GAP_POINTS: f64 = 3.5;
        let mut cell: Vec<TextRun> = Vec::new();
        let mut cells: Vec<Vec<TextRun>> = Vec::new();
        for run in line {
            let gap = cell
                .last()
                .map(|previous: &TextRun| run.x - (previous.x + previous.width))
                .unwrap_or(0.0);
            if !cell.is_empty() && gap > CELL_GAP_POINTS {
                cells.push(std::mem::take(&mut cell));
            }
            cell.push(run);
        }
        if !cell.is_empty() {
            cells.push(cell);
        }

        for cell in cells {
            let text: String = cell.iter().map(|run| run.text.as_str()).collect();
            if text.trim().is_empty() {
                continue;
            }
            let min_x = cell.iter().map(|run| run.x).fold(f64::INFINITY, f64::min);
            let max_x = cell
                .iter()
                .map(|run| run.x + run.width)
                .fold(f64::NEG_INFINITY, f64::max);
            let baseline = cell.iter().map(|run| run.y).sum::<f64>() / cell.len() as f64;
            let height = size * 1.15;
            // PDF 以左下角为原点；OCR 坐标系以左上角为原点。
            let top = (page_height - baseline - size * 0.82) * PIXELS_PER_POINT;
            let x = min_x * PIXELS_PER_POINT;
            let width = ((max_x - min_x).max(size * 0.3)) * PIXELS_PER_POINT;
            out.push(OcrLine {
                text,
                words: vec![OcrWord {
                    text: String::new(),
                    x,
                    y: top,
                    w: width,
                    h: height * PIXELS_PER_POINT,
                }],
            });
        }
    }
    out.sort_by(|a, b| a.center_y().total_cmp(&b.center_y()));
    out
}

#[derive(Clone, Copy)]
struct Matrix([f64; 6]);

impl Matrix {
    fn multiply(left: Matrix, right: Matrix) -> Matrix {
        let a = left.0;
        let b = right.0;
        Matrix([
            a[0] * b[0] + a[1] * b[2],
            a[0] * b[1] + a[1] * b[3],
            a[2] * b[0] + a[3] * b[2],
            a[2] * b[1] + a[3] * b[3],
            a[4] * b[0] + a[5] * b[2] + b[4],
            a[4] * b[1] + a[5] * b[3] + b[5],
        ])
    }

    fn translate(self, tx: f64, ty: f64) -> Matrix {
        Matrix::multiply(Matrix([1.0, 0.0, 0.0, 1.0, tx, ty]), self)
    }

    fn apply(self, x: f64, y: f64) -> (f64, f64) {
        (
            self.0[0] * x + self.0[2] * y + self.0[4],
            self.0[1] * x + self.0[3] * y + self.0[5],
        )
    }

    /// 线性部分的旋转角（弧度）；纵向排版文字可以据此排除。
    fn angle(self) -> f64 {
        self.0[1].atan2(self.0[0])
    }

    fn scale(self) -> f64 {
        self.0[0].hypot(self.0[1])
    }
}

impl Default for Matrix {
    fn default() -> Self {
        Matrix([1.0, 0.0, 0.0, 1.0, 0.0, 0.0])
    }
}

struct PdfObject {
    dict: String,
    stream: Option<Vec<u8>>,
}

struct PageInfo {
    number: u32,
    resources: Option<String>,
    size: Option<(f64, f64)>,
}

struct Pdf {
    objects: HashMap<u32, PdfObject>,
}

impl Pdf {
    fn parse(data: &[u8]) -> Result<Self, String> {
        let mut objects = HashMap::new();
        let mut index = 0usize;
        while let Some(position) = find_token(data, index, b"obj") {
            let boundary_before = position > 0 && is_space(data[position - 1]);
            let boundary_after = position + 3 < data.len() && is_space(data[position + 3]);
            if !boundary_before || !boundary_after {
                index = position + 3;
                continue;
            }

            // 关键字前依次是“对象号 生成号”。
            let mut cursor = position;
            while cursor > 0 && is_space(data[cursor - 1]) {
                cursor -= 1;
            }
            let generation_end = cursor;
            while cursor > 0 && data[cursor - 1].is_ascii_digit() {
                cursor -= 1;
            }
            let Some(generation) = parse_u32(&data[cursor..generation_end]) else {
                index = position + 3;
                continue;
            };
            while cursor > 0 && is_space(data[cursor - 1]) {
                cursor -= 1;
            }
            let number_end = cursor;
            while cursor > 0 && data[cursor - 1].is_ascii_digit() {
                cursor -= 1;
            }
            let Some(number) = parse_u32(&data[cursor..number_end]) else {
                index = position + 3;
                continue;
            };
            let _ = generation;

            let body_start = position + 3;
            let (dict, stream, next) = Self::read_body(data, body_start);
            objects.insert(number, PdfObject { dict, stream });
            index = next;
        }
        if objects.is_empty() {
            return Err("PDF 中没有可解析的对象".to_owned());
        }
        let mut pdf = Self { objects };
        // 现代 PDF 常把页面树、字体字典等放进 /ObjStm 压缩对象流；
        // 只扫描顶层对象会找不到它们，导致整份文件退回 OCR。
        pdf.load_object_streams();
        Ok(pdf)
    }

    /// 把 /ObjStm 里的压缩对象展开进对象表（压缩对象本身不能包含流）。
    fn load_object_streams(&mut self) {
        let mut pending: Vec<(Vec<u8>, String)> = Vec::new();
        for object in self.objects.values() {
            if !object.dict.contains("/ObjStm") {
                continue;
            }
            if let Some(stream) = &object.stream {
                pending.push((stream.clone(), object.dict.clone()));
            }
        }
        for (stream, dict) in pending {
            let data = self.decode_stream_bytes(&stream);
            let first = dict_value(&dict, "/First")
                .and_then(|value| value.parse::<usize>().ok())
                .unwrap_or(0);
            let count = dict_value(&dict, "/N")
                .and_then(|value| value.parse::<usize>().ok())
                .unwrap_or(0);
            let pairs = object_stream_pairs(&data, first, count);
            for (index, (number, start)) in pairs.iter().enumerate() {
                let end = pairs
                    .get(index + 1)
                    .map(|(_, next)| *next)
                    .unwrap_or(data.len());
                if *start >= data.len() || end <= *start || end > data.len() {
                    continue;
                }
                let body = String::from_utf8_lossy(&data[*start..end])
                    .trim()
                    .to_owned();
                self.objects.entry(*number).or_insert(PdfObject {
                    dict: body,
                    stream: None,
                });
            }
        }
    }

    /// 读取对象正文：字典 + 可选流数据；返回 (字典, 流, 下一个扫描位置)。
    fn read_body(data: &[u8], start: usize) -> (String, Option<Vec<u8>>, usize) {
        let stream_key = find_token(data, start, b"stream");
        let end_key = find_token(data, start, b"endobj");
        match (stream_key, end_key) {
            (Some(stream_pos), _) if stream_pos < end_key.unwrap_or(usize::MAX) => {
                let dict = String::from_utf8_lossy(&data[start..stream_pos]).into_owned();
                let mut cursor = stream_pos + 6;
                if cursor < data.len() && data[cursor] == b'\r' {
                    cursor += 1;
                }
                if cursor < data.len() && data[cursor] == b'\n' {
                    cursor += 1;
                }
                let end = find_token(data, cursor, b"endstream").unwrap_or(data.len());
                let mut stream = data[cursor..end].to_vec();
                while matches!(stream.last(), Some(b'\n') | Some(b'\r')) {
                    stream.pop();
                }
                let next = find_token(data, end, b"endobj")
                    .map(|position| position + 6)
                    .unwrap_or(end);
                (dict, Some(stream), next)
            }
            (_, Some(end_pos)) => (
                String::from_utf8_lossy(&data[start..end_pos]).into_owned(),
                None,
                end_pos + 6,
            ),
            _ => (
                String::from_utf8_lossy(&data[start..]).into_owned(),
                None,
                data.len(),
            ),
        }
    }

    fn get(&self, number: u32) -> Option<&PdfObject> {
        self.objects.get(&number)
    }

    fn object_for_value(&self, value: &str) -> Option<&PdfObject> {
        as_reference(value).and_then(|number| self.get(number))
    }

    /// 按页树顺序列出所有页面，并继承父节点的资源与页面尺寸。
    fn pages(&self) -> Result<Vec<PageInfo>, String> {
        let mut catalog = None;
        for (number, object) in &self.objects {
            let is_catalog = dict_value(&object.dict, "/Type")
                .map(|value| value.contains("Catalog"))
                .unwrap_or(false);
            if is_catalog {
                catalog = Some(*number);
                break;
            }
        }
        let Some(catalog) = catalog else {
            return Err("PDF 缺少 Catalog".to_owned());
        };
        let catalog = self.get(catalog).ok_or("Catalog 对象缺失")?;
        let pages_ref = dict_value(&catalog.dict, "/Pages")
            .and_then(as_reference)
            .ok_or("Catalog 缺少 /Pages")?;
        let mut list = Vec::new();
        self.walk_pages(pages_ref, None, None, &mut list, 0)?;
        if list.is_empty() {
            return Err("PDF 页树为空".to_owned());
        }
        Ok(list)
    }

    fn walk_pages(
        &self,
        number: u32,
        resources: Option<&str>,
        size: Option<(f64, f64)>,
        list: &mut Vec<PageInfo>,
        depth: usize,
    ) -> Result<(), String> {
        if depth > 32 {
            return Ok(());
        }
        let object = self.get(number).ok_or("页树引用无效")?;
        let resources = dict_value(&object.dict, "/Resources")
            .map(str::to_owned)
            .or_else(|| resources.map(str::to_owned));
        let size = page_size(&object.dict).or(size);

        let kind = dict_value(&object.dict, "/Type").unwrap_or("");
        if kind.contains("Pages") {
            let Some(kids) = dict_value(&object.dict, "/Kids") else {
                return Ok(());
            };
            for child in references_in(kids) {
                self.walk_pages(
                    child,
                    resources.as_deref(),
                    size,
                    list,
                    depth + 1,
                )?;
            }
        } else if kind.contains("Page") {
            list.push(PageInfo {
                number,
                resources,
                size,
            });
        }
        Ok(())
    }

    /// 解释一页的内容流，返回按绘制顺序排列的文字片段。
    fn page_runs(&self, page: &PageInfo) -> Result<Vec<TextRun>, String> {
        let Some(page_object) = self.get(page.number) else {
            return Ok(Vec::new());
        };
        let fonts = self.page_fonts(page.resources.as_deref());
        let mut runs = Vec::new();
        for content in self.page_contents(&page_object.dict) {
            let data = self.decode_stream(content);
            let tokens = tokenize(&data);
            interpret(&tokens, &fonts, &mut runs);
        }
        Ok(runs)
    }

    fn page_contents(&self, dict: &str) -> Vec<&PdfObject> {
        let Some(contents) = dict_value(dict, "/Contents") else {
            return Vec::new();
        };
        let mut out = Vec::new();
        if let Some(number) = as_reference(contents) {
            if let Some(object) = self.get(number) {
                out.push(object);
            }
        } else {
            for number in references_in(contents) {
                if let Some(object) = self.get(number) {
                    out.push(object);
                }
            }
        }
        out
    }

    fn page_fonts(&self, resources: Option<&str>) -> HashMap<String, Font> {
        let mut fonts = HashMap::new();
        let Some(resources) = resources else {
            return fonts;
        };
        let resources = self
            .object_for_value(resources)
            .map(|object| object.dict.as_str())
            .unwrap_or(resources);
        let Some(font_dict) = dict_value(resources, "/Font") else {
            return fonts;
        };
        let font_dict = self
            .object_for_value(font_dict)
            .map(|object| object.dict.clone())
            .unwrap_or_else(|| font_dict.to_owned());
        for (name, value) in dict_entries(&font_dict) {
            if !name.starts_with('/') {
                continue;
            }
            let Some(number) = as_reference(value) else {
                continue;
            };
            let Some(object) = self.get(number) else {
                continue;
            };
            let font = self.read_font(&object.dict);
            fonts.insert(name.trim_start_matches('/').to_owned(), font);
        }
        fonts
    }

    fn read_font(&self, dict: &str) -> Font {
        let subtype = dict_value(dict, "/Subtype").unwrap_or("");
        let two_byte = subtype.contains("Type0");
        let cmap = dict_value(dict, "/ToUnicode")
            .and_then(as_reference)
            .and_then(|number| self.get(number))
            .and_then(|object| object.stream.as_ref())
            .map(|stream| parse_cmap(&self.decode_stream_bytes(stream)))
            .unwrap_or_default();
        let (widths, default_width) = if two_byte {
            // /DescendantFonts 可能直接是数组，也可能是指向数组对象的引用；
            // 真正的 CIDFont 字典（含 /W、/DW）是数组里的第一个引用。
            let descendant = dict_value(dict, "/DescendantFonts")
                .and_then(|value| {
                    let array = self
                        .object_for_value(value)
                        .map(|object| object.dict.as_str())
                        .unwrap_or(value);
                    references_in(array).first().copied()
                })
                .and_then(|number| self.get(number));
            match descendant {
                Some(object) => {
                    let default_width = dict_value(&object.dict, "/DW")
                        .and_then(|value| value.parse::<f64>().ok())
                        .map(|width| width / 1000.0)
                        .unwrap_or(1.0);
                    (parse_cid_widths(&object.dict), default_width)
                }
                None => (HashMap::new(), 1.0),
            }
        } else {
            (parse_simple_widths(dict), 0.5)
        };
        Font {
            cmap,
            widths,
            two_byte,
            default_width,
        }
    }

    fn decode_stream(&self, object: &PdfObject) -> Vec<u8> {
        match &object.stream {
            Some(stream) => self.decode_stream_bytes(stream),
            None => Vec::new(),
        }
    }

    fn decode_stream_bytes(&self, bytes: &[u8]) -> Vec<u8> {
        let mut decoder = ZlibDecoder::new(bytes);
        let mut out = Vec::new();
        if decoder.read_to_end(&mut out).is_ok() && !out.is_empty() {
            out
        } else {
            bytes.to_vec()
        }
    }
}

/// 读取 /W（CID 字体）的宽度表：`[c [w…]]` 或 `[cfirst clast w]`。
fn parse_cid_widths(dict: &str) -> HashMap<u16, f64> {
    let mut widths = HashMap::new();
    let items = width_array(dict, "/W");
    let mut index = 0usize;
    while index < items.len() {
        let Some(start) = items[index].number() else {
            index += 1;
            continue;
        };
        match items.get(index + 1) {
            Some(Token::Array(list)) => {
                for (offset, item) in list.iter().enumerate() {
                    if let Some(width) = item.number() {
                        widths.insert(
                            (start as u16).saturating_add(offset as u16),
                            width / 1000.0,
                        );
                    }
                }
                index += 2;
            }
            Some(Token::Num(last)) => {
                let Some(width) = items.get(index + 2).and_then(Token::number) else {
                    index += 1;
                    continue;
                };
                let start = start as u16;
                let last = *last as u16;
                if last >= start && last - start < 4096 {
                    for code in start..=last {
                        widths.insert(code, width / 1000.0);
                    }
                }
                index += 3;
            }
            _ => index += 1,
        }
    }
    widths
}

/// 读取简单字体的 /Widths 数组（按 /FirstChar 连续编码）。
fn parse_simple_widths(dict: &str) -> HashMap<u16, f64> {
    let first = dict_value(dict, "/FirstChar")
        .and_then(|value| value.parse::<u16>().ok())
        .unwrap_or(0);
    let mut widths = HashMap::new();
    for (offset, item) in width_array(dict, "/Widths").iter().enumerate() {
        if let Some(width) = item.number() {
            widths.insert(first.saturating_add(offset as u16), width / 1000.0);
        }
    }
    widths
}

fn width_array(dict: &str, key: &str) -> Vec<Token> {
    let Some(value) = dict_value(dict, key) else {
        return Vec::new();
    };
    match tokenize(value.as_bytes()).into_iter().next() {
        Some(Token::Array(items)) => items,
        _ => Vec::new(),
    }
}

/// 字体：ToUnicode 映射 + 字宽（以 em 为单位）。
#[derive(Default)]
struct Font {
    cmap: HashMap<u16, String>,
    widths: HashMap<u16, f64>,
    two_byte: bool,
    default_width: f64,
}

impl Font {
    fn decode(&self, bytes: &[u8]) -> Vec<(u16, String, f64)> {
        let mut out = Vec::new();
        if self.two_byte {
            for pair in bytes.chunks(2) {
                if pair.len() < 2 {
                    break;
                }
                let code = u16::from_be_bytes([pair[0], pair[1]]);
                let text = self
                    .cmap
                    .get(&code)
                    .cloned()
                    .unwrap_or_else(|| "\u{FFFD}".to_owned());
                let width = self.width(&code, &text);
                out.push((code, text, width));
            }
        } else {
            for &byte in bytes {
                let code = byte as u16;
                let text = self
                    .cmap
                    .get(&code)
                    .cloned()
                    .unwrap_or_else(|| from_win_ansi(byte));
                let width = self.width(&code, &text);
                out.push((code, text, width));
            }
        }
        out
    }

    /// 字宽来自 /W（CID 字体）或 /Widths（简单字体）；缺失时按
    /// 中文全角 1em、西文半角 0.5em 兜底。
    fn width(&self, code: &u16, text: &str) -> f64 {
        self.widths
            .get(code)
            .copied()
            .unwrap_or_else(|| {
                if self.default_width > 0.0 {
                    self.default_width
                } else if is_wide(text) {
                    1.0
                } else {
                    0.5
                }
            })
    }
}

fn is_wide(text: &str) -> bool {
    text.chars().any(|ch| {
        matches!(ch, '\u{3000}'..='\u{9FFF}' | '\u{F900}'..='\u{FAFF}' | '\u{FF00}'..='\u{FFEF}')
    })
}

fn from_win_ansi(byte: u8) -> String {
    // 只覆盖参数表里会出现的字符：数字、字母和常见符号。
    if (0x20..=0x7E).contains(&byte) {
        return (byte as char).to_string();
    }
    match byte {
        0x80 => "€".to_owned(),
        0x91 => "‘".to_owned(),
        0x92 => "’".to_owned(),
        0x93 => "“".to_owned(),
        0x94 => "”".to_owned(),
        0xA0 => " ".to_owned(),
        0xB0 => "°".to_owned(),
        0xB5 => "µ".to_owned(),
        _ => "\u{FFFD}".to_owned(),
    }
}

/// 解释内容流中的文字操作符。
fn interpret(tokens: &[Token], fonts: &HashMap<String, Font>, runs: &mut Vec<TextRun>) {
    let mut operands: Vec<Token> = Vec::new();
    let mut ctm = Matrix::default();
    let mut stack: Vec<Matrix> = Vec::new();
    let mut text = Matrix::default();
    let mut font: Option<&Font> = None;
    let mut font_size = 1.0f64;
    let mut leading = 0.0f64;
    let mut char_spacing = 0.0f64;
    let mut word_spacing = 0.0f64;

    let show = |bytes: &[u8],
                    text: &mut Matrix,
                    font: Option<&Font>,
                    char_spacing: f64,
                    word_spacing: f64,
                    font_size: f64,
                    ctm: Matrix,
                    runs: &mut Vec<TextRun>| {
        let Some(current) = font else { return };
        let glyphs = current.decode(bytes);
        let (x, y) = text.apply(0.0, 0.0);
        let (ux, uy) = ctm.apply(x, y);
        let angle = text.angle();
        if angle.abs() < 0.08 {
            let size = text.scale().max(1.0) * font_size;
            let content: String = glyphs.iter().map(|(_, text, _)| text.as_str()).collect();
            if !content.trim().is_empty() {
                let width: f64 = glyphs
                    .iter()
                    .map(|(_, _, width)| width * font_size)
                    .sum::<f64>()
                    * text.scale().max(1.0);
                runs.push(TextRun {
                    text: content,
                    x: ux,
                    y: uy,
                    size,
                    width: width.max(1.0),
                });
            }
        }
        // 这份样册由排版软件导出：每个数值用一次 Tj 画出，紧接着用 Td
        // 把笔位挪到下一行/下一列，Td 的横向分量已按“整数串不累加字宽”
        // 计算。若这里再按字宽推进笔位，位置会逐行漂移（实测整列偏移
        // 可达上千像素）。因此以 Td 为准，不做字宽推进。
        let _ = (char_spacing, word_spacing, font_size);
    };

    for token in tokens {
        match token {
            Token::Op(operator) => {
                match operator.as_str() {
                    "q" => stack.push(ctm),
                    "Q" => {
                        if let Some(previous) = stack.pop() {
                            ctm = previous;
                        }
                    }
                    "cm" => {
                        if let Some(values) = numbers(&operands, 6) {
                            let matrix = Matrix([
                                values[0], values[1], values[2], values[3], values[4], values[5],
                            ]);
                            ctm = Matrix::multiply(matrix, ctm);
                        }
                    }
                    "BT" => {
                        text = Matrix::default();
                    }
                    "Tf" => {
                        if let Some(Token::Name(name)) = operands.first() {
                            font = fonts.get(name);
                        }
                        if let Some(value) = operands.get(1).and_then(Token::number) {
                            font_size = value;
                        }
                    }
                    "Td" => {
                        if let Some(values) = numbers(&operands, 2) {
                            text = text.translate(values[0], values[1]);
                        }
                    }
                    "TD" => {
                        if let Some(values) = numbers(&operands, 2) {
                            leading = -values[1];
                            text = text.translate(values[0], values[1]);
                        }
                    }
                    "TL" => {
                        if let Some(value) = operands.first().and_then(Token::number) {
                            leading = value;
                        }
                    }
                    "Tc" => {
                        if let Some(value) = operands.first().and_then(Token::number) {
                            char_spacing = value;
                        }
                    }
                    "Tw" => {
                        if let Some(value) = operands.first().and_then(Token::number) {
                            word_spacing = value;
                        }
                    }
                    "Tm" => {
                        if let Some(values) = numbers(&operands, 6) {
                            text = Matrix([
                                values[0], values[1], values[2], values[3], values[4], values[5],
                            ]);
                        }
                    }
                    "T*" => {
                        text = text.translate(0.0, -leading);
                    }
                    "Tj" | "'" | "\"" => {
                        if operator != "Tj" {
                            text = text.translate(0.0, -leading);
                        }
                        if let Some(Token::Str(bytes)) = operands.last() {
                            show(
                                bytes,
                                &mut text,
                                font,
                                char_spacing,
                                word_spacing,
                                font_size,
                                ctm,
                                runs,
                            );
                        }
                    }
                    "TJ" => {
                        if let Some(Token::Array(items)) = operands.last() {
                            for item in items {
                                match item {
                                    Token::Str(bytes) => show(
                                        bytes,
                                        &mut text,
                                        font,
                                        char_spacing,
                                        word_spacing,
                                        font_size,
                                        ctm,
                                        runs,
                                    ),
                                    Token::Num(value) => {
                                        // TJ 里的数字是字距修正（千分之一 em）。
                                        text = text.translate(-value / 1000.0 * font_size, 0.0);
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }
                    _ => {}
                }
                operands.clear();
            }
            Token::Array(items) => operands.push(Token::Array(items.clone())),
            other => operands.push(other.clone()),
        }
    }
}

impl Token {
    fn number(&self) -> Option<f64> {
        match self {
            Token::Num(value) => Some(*value),
            _ => None,
        }
    }
}

fn numbers(tokens: &[Token], count: usize) -> Option<Vec<f64>> {
    if tokens.len() < count {
        return None;
    }
    let values: Vec<f64> = tokens[tokens.len() - count..]
        .iter()
        .filter_map(Token::number)
        .collect();
    if values.len() != count {
        None
    } else {
        Some(values)
    }
}

#[derive(Clone, Debug)]
enum Token {
    Num(f64),
    Str(Vec<u8>),
    Name(String),
    Array(Vec<Token>),
    Op(String),
}

fn tokenize(data: &[u8]) -> Vec<Token> {
    let mut tokens = Vec::new();
    let mut index = 0usize;
    while index < data.len() {
        let byte = data[index];
        match byte {
            b'%' => {
                while index < data.len() && data[index] != b'\n' {
                    index += 1;
                }
            }
            byte if is_space(byte) || byte < 0x20 => index += 1,
            b'<' => {
                if data.get(index + 1) == Some(&b'<') {
                    // 内联字典（属性列表等）不参与文字识别，整体跳过。
                    index = skip_balanced(data, index + 2, b'<', b'>');
                } else if let Some(end) = find_byte(data, index + 1, b'>') {
                    tokens.push(Token::Str(parse_hex(&data[index + 1..end])));
                    index = end + 1;
                } else {
                    break;
                }
            }
            b'>' => {
                if data.get(index + 1) == Some(&b'>') {
                    index += 2;
                } else {
                    index += 1;
                }
            }
            b'[' => {
                let (items, next) = parse_array(data, index + 1);
                tokens.push(Token::Array(items));
                index = next;
            }
            b']' => index += 1,
            b'(' => {
                let (bytes, next) = parse_literal(data, index + 1);
                tokens.push(Token::Str(bytes));
                index = next;
            }
            b')' => index += 1,
            b'/' => {
                let start = index + 1;
                index = start;
                while index < data.len()
                    && !is_space(data[index])
                    && !matches!(data[index], b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'/' | b'%')
                {
                    index += 1;
                }
                tokens.push(Token::Name(
                    String::from_utf8_lossy(&data[start..index]).into_owned(),
                ));
            }
            b'+' | b'-' | b'.' | b'0'..=b'9' => {
                let start = index;
                if matches!(data[index], b'+' | b'-') {
                    index += 1;
                }
                let mut seen_dot = false;
                while index < data.len() {
                    match data[index] {
                        b'0'..=b'9' => index += 1,
                        b'.' if !seen_dot => {
                            seen_dot = true;
                            index += 1;
                        }
                        _ => break,
                    }
                }
                let text = String::from_utf8_lossy(&data[start..index]);
                let value = text.parse::<f64>().unwrap_or(0.0);
                tokens.push(Token::Num(value));
            }
            _ => {
                let start = index;
                while index < data.len()
                    && !is_space(data[index])
                    && !matches!(data[index], b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'/' | b'%')
                {
                    index += 1;
                }
                let word = String::from_utf8_lossy(&data[start..index]).into_owned();
                if !word.is_empty() {
                    tokens.push(Token::Op(word));
                }
            }
        }
    }
    tokens
}

fn parse_array(data: &[u8], mut index: usize) -> (Vec<Token>, usize) {
    let mut items = Vec::new();
    while index < data.len() {
        let byte = data[index];
        if is_space(byte) || byte < 0x20 {
            index += 1;
            continue;
        }
        match byte {
            b']' => return (items, index + 1),
            b'<' => {
                if data.get(index + 1) == Some(&b'<') {
                    index = skip_balanced(data, index + 2, b'<', b'>');
                    continue;
                }
                if let Some(end) = find_byte(data, index + 1, b'>') {
                    items.push(Token::Str(parse_hex(&data[index + 1..end])));
                    index = end + 1;
                } else {
                    break;
                }
            }
            b'(' => {
                let (bytes, next) = parse_literal(data, index + 1);
                items.push(Token::Str(bytes));
                index = next;
            }
            b'[' => {
                let (nested, next) = parse_array(data, index + 1);
                items.push(Token::Array(nested));
                index = next;
            }
            _ => {
                let start = index;
                while index < data.len()
                    && !is_space(data[index])
                    && !matches!(data[index], b'(' | b')' | b'<' | b'>' | b'[' | b']')
                {
                    index += 1;
                }
                if index == start {
                    index += 1;
                    continue;
                }
                let word = String::from_utf8_lossy(&data[start..index]).into_owned();
                if let Ok(value) = word.parse::<f64>() {
                    items.push(Token::Num(value));
                } else if let Some(name) = word.strip_prefix('/') {
                    items.push(Token::Name(name.to_owned()));
                } else {
                    items.push(Token::Op(word));
                }
            }
        }
    }
    (items, index)
}

fn parse_literal(data: &[u8], mut index: usize) -> (Vec<u8>, usize) {
    let mut bytes = Vec::new();
    let mut depth = 1usize;
    while index < data.len() {
        let byte = data[index];
        match byte {
            b'\\' => {
                index += 1;
                if index >= data.len() {
                    break;
                }
                match data[index] {
                    b'n' => bytes.push(b'\n'),
                    b'r' => bytes.push(b'\r'),
                    b't' => bytes.push(b'\t'),
                    b'b' => bytes.push(8),
                    b'f' => bytes.push(12),
                    b'(' => bytes.push(b'('),
                    b')' => bytes.push(b')'),
                    b'\\' => bytes.push(b'\\'),
                    digit @ b'0'..=b'7' => {
                        let mut value = (digit - b'0') as u16;
                        let mut consumed = 1;
                        while consumed < 3
                            && index + 1 < data.len()
                            && (b'0'..=b'7').contains(&data[index + 1])
                        {
                            index += 1;
                            value = value * 8 + (data[index] - b'0') as u16;
                            consumed += 1;
                        }
                        bytes.push(value as u8);
                    }
                    other => bytes.push(other),
                }
                index += 1;
            }
            b'(' => {
                depth += 1;
                bytes.push(byte);
                index += 1;
            }
            b')' => {
                depth -= 1;
                index += 1;
                if depth == 0 {
                    break;
                }
                bytes.push(byte);
            }
            _ => {
                bytes.push(byte);
                index += 1;
            }
        }
    }
    (bytes, index)
}

fn parse_hex(bytes: &[u8]) -> Vec<u8> {
    let mut digits = Vec::with_capacity(bytes.len());
    for &byte in bytes {
        if let Some(value) = (byte as char).to_digit(16) {
            digits.push(value as u8);
        }
    }
    if digits.len() % 2 == 1 {
        digits.push(0);
    }
    digits
        .chunks(2)
        .map(|pair| pair[0] * 16 + pair[1])
        .collect()
}

fn parse_cmap(data: &[u8]) -> HashMap<u16, String> {
    let text = String::from_utf8_lossy(data);
    let mut map = HashMap::new();
    let tokens: Vec<&str> = text.split_whitespace().collect();
    let mut index = 0usize;
    while index < tokens.len() {
        match tokens[index] {
            "beginbfchar" => {
                index += 1;
                while index + 1 < tokens.len() && tokens[index] != "endbfchar" {
                    if let (Some(code), Some(value)) =
                        (cmap_code(tokens[index]), cmap_string(tokens[index + 1]))
                    {
                        map.insert(code, value);
                    }
                    index += 2;
                }
            }
            "beginbfrange" => {
                index += 1;
                while index < tokens.len() && tokens[index] != "endbfrange" {
                    if index + 2 >= tokens.len() {
                        break;
                    }
                    let low = cmap_code(tokens[index]);
                    let high = cmap_code(tokens[index + 1]);
                    let (Some(low), Some(high)) = (low, high) else {
                        index += 1;
                        continue;
                    };
                    if tokens[index + 2].starts_with('[') {
                        let mut offset = 0u16;
                        let mut cursor = index + 2;
                        while cursor < tokens.len() && !tokens[cursor].ends_with(']') {
                            if let Some(value) = cmap_string(tokens[cursor].trim_start_matches('['))
                            {
                                if low + offset <= high {
                                    map.insert(low + offset, value);
                                }
                                offset = offset.saturating_add(1);
                            }
                            cursor += 1;
                        }
                        if cursor < tokens.len() {
                            if let Some(value) =
                                cmap_string(tokens[cursor].trim_end_matches(']'))
                            {
                                if low + offset <= high {
                                    map.insert(low + offset, value);
                                }
                            }
                        }
                        index = cursor + 1;
                    } else {
                        if let Some(base) = cmap_string(tokens[index + 2]) {
                            for code in low..=high {
                                map.insert(code, increment_last(&base, code - low));
                            }
                        }
                        index += 3;
                    }
                }
            }
            _ => index += 1,
        }
    }
    map
}

/// 读取 CMap 里的源字符码（`<0001>`）；非十六进制写法返回 None。
fn cmap_code(token: &str) -> Option<u16> {
    let token = token.trim_start_matches('[').trim_end_matches(']');
    if !token.starts_with('<') {
        return None;
    }
    let hex = token.trim_start_matches('<').trim_end_matches('>');
    if hex.is_empty() {
        return None;
    }
    let bytes = parse_hex(hex.as_bytes());
    Some(match bytes.len() {
        0 => 0,
        1 => bytes[0] as u16,
        _ => u16::from_be_bytes([bytes[0], bytes[1]]),
    })
}

/// 目标字符串按 UTF-16BE 解码；多字节时保留全部字符。
fn cmap_string(token: &str) -> Option<String> {
    let token = token.trim_start_matches('[').trim_end_matches(']');
    if !token.starts_with('<') || !token.ends_with('>') {
        return None;
    }
    let bytes = parse_hex(token[1..token.len() - 1].as_bytes());
    if bytes.is_empty() {
        return Some(String::new());
    }
    let units: Vec<u16> = bytes
        .chunks(2)
        .map(|pair| {
            if pair.len() == 2 {
                u16::from_be_bytes([pair[0], pair[1]])
            } else {
                pair[0] as u16
            }
        })
        .collect();
    let text: String = units
        .iter()
        .map(|unit| char::from_u32(*unit as u32).unwrap_or('\u{FFFD}'))
        .collect();
    Some(text)
}

/// bfrange 的第三种写法：目标码逐个递增。
fn increment_last(base: &str, offset: u16) -> String {
    let mut chars: Vec<char> = base.chars().collect();
    if let Some(last) = chars.last_mut() {
        *last = char::from_u32(*last as u32 + offset as u32).unwrap_or(*last);
    }
    chars.into_iter().collect()
}

fn is_space(byte: u8) -> bool {
    matches!(byte, b' ' | b'\r' | b'\n' | b'\t' | 0x0c)
}

fn find_byte(data: &[u8], from: usize, needle: u8) -> Option<usize> {
    data.get(from..)
        .and_then(|slice| slice.iter().position(|byte| *byte == needle))
        .map(|position| from + position)
}

/// 查找完整的关键字（前后都是分隔符），避免把 “endobj” 匹配到别的单词里。
fn find_token(data: &[u8], from: usize, token: &[u8]) -> Option<usize> {
    let mut index = from.max(1);
    while index + token.len() <= data.len() {
        let offset = data[index..]
            .iter()
            .position(|byte| *byte == token[0])?;
        let position = index + offset;
        if position > 0
            && data[position..].starts_with(token)
            && (is_space(data[position - 1]) || is_delimiter(data[position - 1]))
            && (position + token.len() >= data.len()
                || is_space(data[position + token.len()])
                || is_delimiter(data[position + token.len()]))
        {
            return Some(position);
        }
        index = position + 1;
    }
    None
}

fn is_delimiter(byte: u8) -> bool {
    matches!(byte, b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'/' | b'%' | b'{' | b'}' | b';')
}


fn skip_balanced(data: &[u8], mut index: usize, open: u8, close: u8) -> usize {
    let mut depth = 1u32;
    while index < data.len() {
        if data[index] == open && data.get(index + 1) == Some(&open) {
            depth += 1;
            index += 2;
        } else if data[index] == close && data.get(index + 1) == Some(&close) {
            depth -= 1;
            index += 2;
            if depth == 0 {
                return index;
            }
        } else {
            index += 1;
        }
    }
    index
}

/// 解析对象流头部的 “对象号 偏移” 对，返回 (对象号, 正文绝对偏移)。
fn object_stream_pairs(data: &[u8], first: usize, count: usize) -> Vec<(u32, usize)> {
    let header = &data[..first.min(data.len())];
    let mut numbers = Vec::new();
    let mut offsets = Vec::new();
    let mut cursor = 0usize;
    let mut tokens = 0usize;
    while tokens < count.saturating_mul(2) && cursor < header.len() {
        while cursor < header.len() && is_space(header[cursor]) {
            cursor += 1;
        }
        let start = cursor;
        while cursor < header.len() && header[cursor].is_ascii_digit() {
            cursor += 1;
        }
        if start == cursor {
            break;
        }
        let Some(value) = parse_u32(&header[start..cursor]) else {
            break;
        };
        if tokens % 2 == 0 {
            numbers.push(value);
        } else {
            offsets.push(first + value as usize);
        }
        tokens += 1;
    }
    numbers.into_iter().zip(offsets).collect()
}

fn parse_u32(bytes: &[u8]) -> Option<u32> {
    if bytes.is_empty() {
        return None;
    }
    let mut value = 0u32;
    for &byte in bytes {
        if !byte.is_ascii_digit() {
            return None;
        }
        value = value.checked_mul(10)?.checked_add((byte - b'0') as u32)?;
    }
    Some(value)
}

fn as_reference(value: &str) -> Option<u32> {
    let mut parts = value.split_whitespace();
    let number = parts.next()?.parse::<u32>().ok()?;
    let generation = parts.next()?.parse::<u32>().ok()?;
    if parts.next()? != "R" {
        return None;
    }
    let _ = generation;
    Some(number)
}

fn references_in(value: &str) -> Vec<u32> {
    let tokens: Vec<&str> = value
        .split(|ch: char| ch.is_whitespace() || matches!(ch, '[' | ']' | '(' | ')'))
        .filter(|token| !token.is_empty())
        .collect();
    let mut out = Vec::new();
    let mut index = 0usize;
    while index + 2 < tokens.len() {
        if let (Ok(number), Ok(generation)) = (
            tokens[index].parse::<u32>(),
            tokens[index + 1].parse::<u32>(),
        ) {
            if tokens[index + 2] == "R" {
                let _ = generation;
                out.push(number);
                index += 3;
                continue;
            }
        }
        index += 1;
    }
    out
}

/// 读取字典中某个键对应的值文本（保持原始写法，可能含引用或嵌套）。
///
/// PDF 允许把键值连写成 “<</Pages 3 0 R/Type/Catalog>>”，因此不能靠
/// 在整串里搜 “/Pages”，必须按记号顺序解析键值对。
fn dict_value<'a>(dict: &'a str, key: &str) -> Option<&'a str> {
    dict_entries(dict)
        .into_iter()
        .find(|(name, _)| name == key)
        .map(|(_, value)| value)
}

/// 提取值文本：平衡 `<< >>` / `[ ]`，字面量字符串原样返回，
/// 数字引用 “N G R” 整体返回。
fn value_text(text: &str) -> Option<&str> {
    let trimmed = text.trim_start();
    let bytes = trimmed.as_bytes();
    let first = *bytes.first()?;
    match first {
        b'<' if bytes.get(1) == Some(&b'<') => {
            let end = skip_balanced(bytes, 2, b'<', b'>');
            Some(trimmed.get(..end).unwrap_or(trimmed))
        }
        b'[' => {
            let mut depth = 0usize;
            let mut index = 0usize;
            while index < bytes.len() {
                match bytes[index] {
                    b'[' => depth += 1,
                    b']' => {
                        depth -= 1;
                        if depth == 0 {
                            return trimmed.get(..=index);
                        }
                    }
                    b'(' => {
                        // 跳过字面量字符串，避免其中的方括号干扰。
                        while index < bytes.len() && bytes[index] != b')' {
                            index += 1;
                        }
                    }
                    _ => {}
                }
                index += 1;
            }
            Some(trimmed)
        }
        b'(' => {
            let mut depth = 0usize;
            let mut index = 0usize;
            while index < bytes.len() {
                match bytes[index] {
                    b'\\' => index += 1,
                    b'(' => depth += 1,
                    b')' => {
                        depth -= 1;
                        if depth == 0 {
                            return trimmed.get(..=index);
                        }
                    }
                    _ => {}
                }
                index += 1;
            }
            Some(trimmed)
        }
        b'/' => {
            let mut end = 1;
            while end < bytes.len() && !is_space(bytes[end]) && !is_delimiter(bytes[end]) {
                end += 1;
            }
            Some(trimmed.get(..end).unwrap_or(trimmed))
        }
        _ => {
            let end = token_end(bytes);
            if first.is_ascii_digit() {
                if let Some(reference_end) = reference_end(trimmed) {
                    return Some(&trimmed[..reference_end]);
                }
            }
            Some(trimmed.get(..end).unwrap_or(trimmed))
        }
    }
}

fn token_end(bytes: &[u8]) -> usize {
    let end = bytes
        .iter()
        .position(|byte| is_space(*byte) || is_delimiter(*byte))
        .unwrap_or(bytes.len());
    if end == 0 {
        1
    } else {
        end
    }
}

/// 若值文本是 “N G R”，返回三个记号的总长度。
fn reference_end(text: &str) -> Option<usize> {
    let mut cursor = 0usize;
    let bytes = text.as_bytes();
    let mut parts = Vec::new();
    while parts.len() < 3 {
        cursor = first_non_space(bytes, cursor);
        let start = cursor;
        while cursor < bytes.len() && !is_space(bytes[cursor]) && !is_delimiter(bytes[cursor]) {
            cursor += 1;
        }
        if start == cursor {
            return None;
        }
        parts.push(&text[start..cursor]);
    }
    if parts[0].parse::<u32>().is_ok() && parts[1].parse::<u32>().is_ok() && parts[2] == "R" {
        Some(cursor)
    } else {
        None
    }
}

fn first_non_space(bytes: &[u8], mut index: usize) -> usize {
    while index < bytes.len() && is_space(bytes[index]) {
        index += 1;
    }
    index
}

/// 列出字典的 (键, 值文本) 对（只处理一层）。
fn dict_entries(dict: &str) -> Vec<(String, &str)> {
    let mut out = Vec::new();
    let mut index = 0usize;
    let bytes = dict.as_bytes();
    while index < bytes.len() {
        if bytes[index] == b'/' {
            let start = index;
            index += 1;
            while index < bytes.len()
                && !is_space(bytes[index])
                && !matches!(bytes[index], b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'/' | b'%')
            {
                index += 1;
            }
            let name = String::from_utf8_lossy(&bytes[start..index]).into_owned();
            let rest = &dict[index..];
            if let Some(value) = value_text(rest) {
                let leading = rest.len() - rest.trim_start().len();
                index += leading + value.len();
                out.push((name, value));
            } else {
                index += 1;
            }
        } else {
            index += 1;
        }
    }
    out
}

/// 从页面字典读取 /MediaBox 的宽高（单位磅）。
fn page_size(dict: &str) -> Option<(f64, f64)> {
    let value = dict_value(dict, "/MediaBox")?;
    let numbers: Vec<f64> = value
        .split(|ch: char| !ch.is_ascii_digit() && ch != '.' && ch != '-')
        .filter_map(|part| part.parse::<f64>().ok())
        .collect();
    if numbers.len() >= 4 {
        let width = (numbers[2] - numbers[0]).abs();
        let height = (numbers[3] - numbers[1]).abs();
        if width > 0.0 && height > 0.0 {
            return Some((width, height));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_hex_strings() {
        assert_eq!(parse_hex(b"0014"), vec![0x00, 0x14]);
        assert_eq!(parse_hex(b"0 0 1 4"), vec![0x00, 0x14]);
    }

    #[test]
    fn cid_widths_use_ranges_and_lists() {
        let widths = parse_cid_widths("/W[1[223]9 10 331 14[343 270 392]17 26 549]");
        assert_eq!(widths.get(&1), Some(&0.223));
        assert_eq!(widths.get(&9), Some(&0.331));
        assert_eq!(widths.get(&10), Some(&0.331));
        assert_eq!(widths.get(&14), Some(&0.343));
        assert_eq!(widths.get(&15), Some(&0.270));
        assert_eq!(widths.get(&20), Some(&0.549));
    }

    #[test]
    fn cmap_reads_bfchar_and_bfrange() {
        let cmap = b"begincmap 1 begincodespacerange <0000> <FFFF> endcodespacerange \
                     2 beginbfchar <0003> <0033> <0004> <002E> endbfchar \
                     1 beginbfrange <0005> <0006> <0035> endbfrange endcmap";
        let map = parse_cmap(cmap);
        assert_eq!(map.get(&3).map(String::as_str), Some("3"));
        assert_eq!(map.get(&4).map(String::as_str), Some("."));
        assert_eq!(map.get(&5).map(String::as_str), Some("5"));
        assert_eq!(map.get(&6).map(String::as_str), Some("6"));
    }

    #[test]
    fn object_stream_pairs_read_offsets() {
        let data = b"2 0 3 10 << /A 1 >><< /B 2 >>";
        let pairs = object_stream_pairs(data, 9, 2);
        assert_eq!(pairs, vec![(2, 9), (3, 19)]);
    }

    #[test]
    fn references_are_extracted_from_arrays() {
        assert_eq!(references_in("[12 0 R 13 0 R]"), vec![12, 13]);
        assert_eq!(as_reference("12 0 R"), Some(12));
    }

    #[test]
    fn dict_value_handles_compact_dicts() {
        let dict = "<< /Metadata 2 0 R/OCProperties<</D<</ON[19 0 R]/Order 20 0 R/RBGroups[]>>/OCGs[19 0 R]>>/Pages 3 0 R/Type/Catalog>>";
        assert_eq!(dict_value(dict, "/Pages"), Some("3 0 R"));
        assert_eq!(dict_value(dict, "/Type"), Some("/Catalog"));
        assert_eq!(
            dict_value(
                "<< /D<</ON[19 0 R]/Order 20 0 R/RBGroups[]>>/OCGs[19 0 R]>>",
                "/OCGs"
            ),
            Some("[19 0 R]")
        );
    }

    #[test]
    fn tokenizer_reads_text_operators() {
        let tokens = tokenize(b"BT /F1 5 Tf 1 0 0 1 10 20 Tm <3135> Tj ET");
        assert!(tokens.iter().any(|token| matches!(token, Token::Name(name) if name == "F1")));
        assert!(tokens.iter().any(|token| matches!(token, Token::Num(value) if *value == 5.0)));
        assert!(tokens.iter().any(|token| matches!(token, Token::Str(bytes) if bytes == &[0x31, 0x35])));
    }

    #[test]
    fn literal_string_handles_escapes() {
        let (bytes, _) = parse_literal(b"a\\(b\\)c", 0);
        assert_eq!(bytes, b"a(b)c");
    }

    #[test]
    fn numeric_runs_in_different_cells_are_split() {
        // 两列数值的中线相距 26 磅；右列数值紧贴着左列，但它们属于
        // 不同单元格，不能被拼成一个数字串（1.18E-031.18E-03）。
        let runs = vec![
            TextRun {
                text: "1.18E-03".to_owned(),
                x: 227.99,
                y: 100.0,
                size: 5.0,
                width: 19.94,
            },
            TextRun {
                text: "1.18E-03".to_owned(),
                x: 252.11,
                y: 100.0,
                size: 5.0,
                width: 19.94,
            },
        ];
        let lines = group_runs(runs, 800.0);
        assert_eq!(lines.len(), 2, "相邻两列的数值必须分开");
        assert_eq!(lines[0].text, "1.18E-03");
        assert_eq!(lines[1].text, "1.18E-03");
    }

    #[test]
    fn adjacent_runs_in_one_cell_are_merged() {
        let runs = vec![
            TextRun {
                text: "1".to_owned(),
                x: 100.0,
                y: 100.0,
                size: 5.0,
                width: 2.8,
            },
            TextRun {
                text: ".5".to_owned(),
                x: 102.7,
                y: 100.0,
                size: 5.0,
                width: 5.6,
            },
        ];
        let lines = group_runs(runs, 800.0);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].text, "1.5");
    }

    /// 诊断：检查文本层能否直接重建表格（用于判断是否可跳过 OCR）。
    #[test]
    fn fixture_text_tables_detect() {
        let Ok(path) = std::env::var("MOTORSIZER_PDF_FIXTURE") else {
            return;
        };
        let (sheets, _) = read_text_sheets(Path::new(&path), 1, 0).expect("解析 PDF 失败");
        let mut tables = 0;
        for sheet in &sheets {
            if let Some(table) = crate::import::parse::detect_table(sheet) {
                tables += 1;
                if tables <= 8 {
                    eprintln!(
                        "page {} rows={} cols={} names={:?} header_bottom={:.1} bottom={:.1}",
                        sheet.page,
                        table.rows.len(),
                        table.columns.len(),
                        table.columns.iter().map(|c| c.name.clone()).collect::<Vec<_>>(),
                        table.header_bottom,
                        table.bottom
                    );
                }
            }
        }
        eprintln!("sheets={} tables={}", sheets.len(), tables);
    }

    /// 用真实样册验证文本层提取（只在设置环境变量时运行，避免测试依赖
    /// 用户桌面的文件）。
    #[test]
    fn extracts_fixture_text_layer() {
        let Ok(path) = std::env::var("MOTORSIZER_PDF_FIXTURE") else {
            return;
        };
        let (sheets, total) = read_text_sheets(Path::new(&path), 1, 0).expect("解析 PDF 失败");
        assert!(total > 0);
        assert!(!sheets.is_empty(), "没有提取到文本层内容");
        let joined: String = sheets
            .iter()
            .flat_map(|sheet| sheet.lines.iter().map(|line| line.text.clone()))
            .collect::<Vec<_>>()
            .join("\n");
        // AXD 样册的专项回归断言；换成目录类文件时只要求提取到文字。
        if joined.contains("AXD") {
            // Windows OCR 读不出这类短小数；文本层必须能提供。
            assert!(joined.contains("18.5"), "文本层中缺少 18.5（页 7 并联电感）");
            assert!(joined.contains("150.3"), "文本层中缺少 150.3（页 6 峰值转矩）");
        }
        if std::env::var_os("MOTORSIZER_PDF_DUMP").is_some() {
            let dir = std::env::temp_dir().join("msdiag2");
            let _ = fs::create_dir_all(&dir);
            let mut text = String::new();
            for sheet in &sheets {
                text.push_str(&format!(
                    "PAGE {} {:.1}x{:.1}\n",
                    sheet.page, sheet.width, sheet.height
                ));
                for line in &sheet.lines {
                    text.push_str(&format!(
                        "  y={:7.1} x={:7.1} {:?}\n",
                        line.center_y(),
                        line.min_x(),
                        line.text
                    ));
                }
            }
            let _ = fs::write(dir.join("pdftext.txt"), text);
        }
    }
}
