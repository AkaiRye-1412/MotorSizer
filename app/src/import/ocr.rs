//! 调用 Windows 本地 OCR（PowerShell + WinRT），不依赖网络或第三方服务。
//!
//! 脚本通过 `include_str!` 内嵌到程序，运行时释放到临时目录；所有参数用
//! `Command` 逐个传入，不经过 shell，避免路径注入。识别结果以 NDJSON 返回，
//! 包含每个词的像素坐标，供 `parse` 模块重建表格。

use std::fs;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use image::{imageops, GrayImage, Luma, RgbaImage};
use serde_json::Value;

/// 内嵌的识别脚本；内容保持纯 ASCII，避免 Windows PowerShell 5.1
/// 把无 BOM 的 UTF-8 当 ANSI 读取而破坏中文注释。
const SCRIPT: &str = include_str!("../../scripts/import_ocr.ps1");

/// 取消标记：脚本看到该文件后尽快退出，工作线程据此返回取消结果。
pub const CANCELLED: &str = "cancelled";

#[derive(Clone, Debug, Default)]
pub struct Progress {
    pub stage: String,
    pub done: u32,
    pub total: u32,
}

#[derive(Debug)]
pub struct OcrDoc {
    pub kind: String,
    pub page_count: u32,
    pub engine: String,
    pub render_dir: Option<PathBuf>,
    pub sheets: Vec<OcrSheet>,
}

#[derive(Debug)]
pub struct OcrSheet {
    /// 文档输入时为页码字符串；批量输入时为调用方提供的标识。
    pub id: String,
    pub page: u32,
    pub width: f64,
    pub height: f64,
    pub lines: Vec<OcrLine>,
}

#[derive(Clone, Debug)]
pub struct OcrLine {
    pub text: String,
    pub words: Vec<OcrWord>,
}

#[derive(Clone, Debug)]
pub struct OcrWord {
    pub text: String,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl OcrLine {
    /// 去掉所有空白后的文字；中文 OCR 常在字之间插入空格。
    pub fn joined(&self) -> String {
        self.text.chars().filter(|c| !c.is_whitespace()).collect()
    }

    pub fn center_x(&self) -> f64 {
        self.words
            .iter()
            .map(|w| w.x + w.w / 2.0)
            .sum::<f64>()
            / self.words.len().max(1) as f64
    }

    pub fn center_y(&self) -> f64 {
        self.words
            .iter()
            .map(|w| w.y + w.h / 2.0)
            .sum::<f64>()
            / self.words.len().max(1) as f64
    }

    pub fn min_x(&self) -> f64 {
        self.words.iter().map(|w| w.x).fold(f64::INFINITY, f64::min)
    }
}

/// 释放内嵌脚本并返回路径。脚本内容变化时覆盖，避免旧版本残留。
fn script_path() -> Result<PathBuf, String> {
    let dir = std::env::temp_dir().join("motionsizer-import");
    fs::create_dir_all(&dir)
        .map_err(|error| format!("无法创建临时目录 {}：{error}", dir.display()))?;
    let path = dir.join("import_ocr.ps1");
    let stale = fs::read_to_string(&path).map(|text| text != SCRIPT).unwrap_or(true);
    if stale {
        fs::write(&path, SCRIPT)
            .map_err(|error| format!("无法写入识别脚本 {}：{error}", path.display()))?;
    }
    Ok(path)
}

pub fn new_workspace() -> Result<PathBuf, String> {
    let unique = format!(
        "job-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0)
    );
    let dir = std::env::temp_dir().join("motionsizer-import").join(unique);
    fs::create_dir_all(&dir)
        .map_err(|error| format!("无法创建导入临时目录 {}：{error}", dir.display()))?;
    Ok(dir)
}

pub fn remove_workspace(dir: &Path) {
    let _ = fs::remove_dir_all(dir);
}

/// 识别整个文档（PDF 或图片）。
pub fn recognize_document(
    path: &Path,
    start_page: u32,
    end_page: u32,
    cancel: &Path,
    workspace: &Path,
    progress: &dyn Fn(Progress),
) -> Result<OcrDoc, String> {
    let out = workspace.join("document.ndjson");
    let script = script_path()?;
    let cwd = std::env::current_dir()
        .map_err(|error| format!("无法获取当前目录：{error}"))?;
    let resolved = resolve_document_path(path, &cwd);
    if !resolved.is_file() {
        return Err(format!("待识别文件不存在：{}", resolved.display()));
    }
    let args = vec![
        "-Path".to_owned(),
        resolved.to_string_lossy().into_owned(),
        "-Out".to_owned(),
        out.display().to_string(),
        "-StartPage".to_owned(),
        start_page.max(1).to_string(),
        "-EndPage".to_owned(),
        end_page.to_string(),
        "-Width".to_owned(),
        "1800".to_owned(),
        "-CancelFile".to_owned(),
        cancel.display().to_string(),
        // [switch] 参数不接受值，传 "-KeepRenders true" 会被当成多余位置参数。
        "-KeepRenders".to_owned(),
    ];
    run_script(&script, &args, cancel, progress, "识别文档")
        .map_err(|error| error_with_sidecar(&out, error))?;
    parse_doc(&out)
}

/// 批量识别裁剪图与预处理页；`batch_file` 每行是 `标识<TAB>路径`。
pub fn recognize_batch(
    batch_file: &Path,
    cancel: &Path,
    workspace: &Path,
    progress: &dyn Fn(Progress),
    stage: &str,
) -> Result<OcrDoc, String> {
    let out = workspace.join("batch.ndjson");
    let script = script_path()?;
    let args = vec![
        "-PathList".to_owned(),
        batch_file.display().to_string(),
        "-Out".to_owned(),
        out.display().to_string(),
        "-CancelFile".to_owned(),
        cancel.display().to_string(),
    ];
    run_script(&script, &args, cancel, progress, stage)
        .map_err(|error| error_with_sidecar(&out, error))?;
    parse_doc(&out)
}

fn run_script(
    script: &Path,
    args: &[String],
    cancel: &Path,
    progress: &dyn Fn(Progress),
    stage: &str,
) -> Result<(), String> {
    let mut command = Command::new("powershell.exe");
    command
        .arg("-NoProfile")
        // Windows 客户端默认 ExecutionPolicy 为 Restricted，直接运行 .ps1 会被拒绝；
        // 命令行参数对非组策略设置的机器生效，避免要求用户改系统设置或要管理员权限。
        .arg("-ExecutionPolicy")
        .arg("Bypass")
        .arg("-File")
        .arg(script)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // 发布版是 GUI 子系统，直接 spawn 控制台程序会弹出一个蓝色 PowerShell 窗口。
    // CREATE_NO_WINDOW 只让子进程不分配控制台，标准输入输出仍走上面的管道。
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = command
        .spawn()
        .map_err(|error| format!("无法启动 Windows OCR：{error}"))?;

    if let Some(stdout) = child.stdout.take() {
        for line in BufReader::new(stdout).lines() {
            let Ok(line) = line else { break };
            if let Some(rest) = line.trim().strip_prefix("progress ") {
                if let Some((done, total)) = rest.split_once('/') {
                    progress(Progress {
                        stage: stage.to_owned(),
                        done: done.trim().parse().unwrap_or(0),
                        total: total.trim().parse().unwrap_or(0),
                    });
                }
            }
        }
    }

    // 按原始字节读取：PowerShell 的错误流可能是 UTF-16LE，
    // 直接 read_to_string 会因无效 UTF-8 失败并留下空字符串。
    let mut stderr_bytes = Vec::new();
    if let Some(mut pipe) = child.stderr.take() {
        let _ = pipe.read_to_end(&mut stderr_bytes);
    }
    let status = child
        .wait()
        .map_err(|error| format!("等待 OCR 进程失败：{error}"))?;

    if std::env::var_os("MOTORSIZER_KEEP_IMPORT").is_some() {
        // 调试用：保留原始 stderr，便于排查 PowerShell 本地化输出。
        let dir = std::env::temp_dir().join("motionsizer-import");
        let _ = fs::write(dir.join("last-ocr-stderr.bin"), &stderr_bytes);
    }

    if !status.success() {
        if cancel.exists() {
            return Err(CANCELLED.to_owned());
        }
        // PowerShell 5.1 在重定向时可能输出 UTF-16LE；先解码再取第一条非空信息。
        // 若 stderr 确实为空（例如子进程被外部终止），至少把退出码带回界面。
        let decoded = decode_stderr(&stderr_bytes);
        let detail = decoded
            .lines()
            .find(|line| !line.trim().is_empty())
            .map(|line| line.trim().to_owned())
            .unwrap_or_else(|| match status.code() {
                Some(code) => format!("退出码 {code}，未返回错误信息"),
                None => "进程被外部终止，未返回错误信息".to_owned(),
            });
        return Err(format!("Windows OCR 执行失败：{detail}"));
    }
    Ok(())
}

/// 解码 PowerShell 的错误输出。优先识别 UTF-16 BOM；没有 BOM 时按
/// “ASCII 字符的 UTF-16 高位为零字节”判断，其余按 UTF-8/lossy 处理。
/// 真正面向用户的错误信息由脚本写出的 UTF-8 sidecar 文件提供。
fn decode_stderr(bytes: &[u8]) -> String {
    let decode_utf16 = |offset: usize, big_endian: bool| {
        let units: Vec<u16> = bytes[offset..]
            .chunks_exact(2)
            .map(|pair| {
                if big_endian {
                    u16::from_be_bytes([pair[0], pair[1]])
                } else {
                    u16::from_le_bytes([pair[0], pair[1]])
                }
            })
            .collect();
        String::from_utf16_lossy(&units)
    };
    if bytes.starts_with(&[0xFF, 0xFE]) {
        return decode_utf16(2, false);
    }
    if bytes.starts_with(&[0xFE, 0xFF]) {
        return decode_utf16(2, true);
    }
    if bytes.is_empty() {
        return String::new();
    }
    let zeros = bytes.iter().skip(1).step_by(2).filter(|byte| **byte == 0).count();
    if bytes.len() % 2 == 0 && zeros * 4 > bytes.len() / 2 {
        return decode_utf16(0, false);
    }
    if let Ok(text) = std::str::from_utf8(bytes) {
        return text.to_owned();
    }
    // Windows PowerShell 5.1 在中文系统上按本地代码页（GBK/936）写错误流，
    // 按 UTF-8 强行解码会把整句中文变成乱码，这里回退到 GBK。
    let (text, _, _) = encoding_rs::GBK.decode(bytes);
    text.into_owned()
}

/// 读取识别脚本写出的 `*.error` sidecar（UTF-8 JSON），把阶段与原因
/// 合并进错误信息；sidecar 不存在时沿用 PowerShell 的 stderr 解码结果。
fn error_with_sidecar(out: &Path, error: String) -> String {
    let sidecar = PathBuf::from(format!("{}.error", out.display()));
    let Ok(content) = fs::read_to_string(&sidecar) else {
        return error;
    };
    let _ = fs::remove_file(&sidecar);
    let Ok(value) = serde_json::from_str::<Value>(&content) else {
        return error;
    };
    let message = value.get("message").and_then(Value::as_str).unwrap_or("");
    if message.is_empty() {
        return error;
    }
    let stage = value.get("stage").and_then(Value::as_str).unwrap_or("未知阶段");
    format!("Windows OCR 执行失败（{stage}）：{message}")
}

/// WinRT 的 `StorageFile.GetFileFromPathAsync` 只接受普通绝对路径：
/// 相对路径会抛错，`canonicalize` 产生的 `\\?\` 前缀同样不被接受，
/// 并且必须使用 Windows 反斜杠分隔符。这里拼接当前目录并统一分隔符。
fn resolve_document_path(path: &Path, cwd: &Path) -> PathBuf {
    let joined = if path.is_absolute() {
        path.to_path_buf()
    } else {
        cwd.join(path)
    };
    PathBuf::from(joined.to_string_lossy().replace('/', "\\"))
}

fn parse_doc(path: &Path) -> Result<OcrDoc, String> {
    let content = fs::read_to_string(path)
        .map_err(|error| format!("无法读取识别结果 {}：{error}", path.display()))?;
    let mut lines = content.lines().filter(|line| !line.trim().is_empty());
    let meta: Value = serde_json::from_str(lines.next().ok_or("识别结果为空")?)
        .map_err(|error| format!("识别结果格式错误：{error}"))?;

    let mut doc = OcrDoc {
        kind: meta["kind"].as_str().unwrap_or("image").to_owned(),
        page_count: meta["page_count"].as_u64().unwrap_or(1) as u32,
        engine: meta["engine"].as_str().unwrap_or("").to_owned(),
        render_dir: meta["render_dir"]
            .as_str()
            .filter(|text| !text.is_empty())
            .map(PathBuf::from),
        sheets: Vec::new(),
    };

    for line in lines {
        let value: Value = serde_json::from_str(line)
            .map_err(|error| format!("识别结果格式错误：{error}"))?;
        let id = match &value["page"] {
            Value::String(text) => text.clone(),
            Value::Number(number) => number.to_string(),
            _ => String::new(),
        };
        let page = value["page"].as_u64().unwrap_or(0) as u32;
        let sheet = OcrSheet {
            id,
            page,
            width: value["width"].as_f64().unwrap_or(0.0),
            height: value["height"].as_f64().unwrap_or(0.0),
            lines: parse_lines(&value["lines"]),
        };
        doc.sheets.push(sheet);
    }
    Ok(doc)
}

fn parse_lines(value: &Value) -> Vec<OcrLine> {
    let mut out = Vec::new();
    let Some(lines) = value.as_array() else {
        return out;
    };
    for line in lines {
        let text = line["text"].as_str().unwrap_or("").to_owned();
        let mut words = Vec::new();
        if let Some(list) = line["words"].as_array() {
            for word in list {
                words.push(OcrWord {
                    text: word["t"].as_str().unwrap_or("").to_owned(),
                    x: word["x"].as_f64().unwrap_or(0.0),
                    y: word["y"].as_f64().unwrap_or(0.0),
                    w: word["w"].as_f64().unwrap_or(0.0),
                    h: word["h"].as_f64().unwrap_or(0.0),
                });
            }
        }
        if !words.is_empty() {
            // 脚本偶尔只给出词级结果（如批量裁剪路径），此时用词文本拼回整行，
            // 避免整行标签为空导致表格检测失败。
            let text = if text.is_empty() {
                words
                    .iter()
                    .map(|word| word.text.as_str())
                    .collect::<Vec<_>>()
                    .join(" ")
            } else {
                text
            };
            out.push(OcrLine { text, words });
        }
    }
    out
}

/// 读取页面图；PDF 使用脚本保留的渲染图，图片直接读取原文件。
pub fn load_page_image(path: &Path) -> Result<RgbaImage, String> {
    image::open(path)
        .map(|img| img.to_rgba8())
        .map_err(|error| format!("无法读取页面图像 {}：{error}", path.display()))
}

/// 从页面图中裁出一列，并返回附带原图纵坐标的裁剪图。
pub fn crop_column(
    image: &RgbaImage,
    center_x: f64,
    half_width: f64,
    top: f64,
    bottom: f64,
) -> RgbaImage {
    let width = image.width() as f64;
    let height = image.height() as f64;
    let x0 = (center_x - half_width).clamp(0.0, width - 1.0);
    let x1 = (center_x + half_width).clamp(x0 + 1.0, width);
    let y0 = top.clamp(0.0, height - 1.0);
    let y1 = bottom.clamp(y0 + 1.0, height);
    let w = (x1 - x0).round().max(1.0) as u32;
    let h = (y1 - y0).round().max(1.0) as u32;
    imageops::crop_imm(image, x0.round() as u32, y0.round() as u32, w, h).to_image()
}

pub fn resize(image: &RgbaImage, scale: f64) -> RgbaImage {
    let w = ((image.width() as f64 * scale).round() as u32).max(1);
    let h = ((image.height() as f64 * scale).round() as u32).max(1);
    imageops::resize(image, w, h, imageops::FilterType::Triangle)
}

/// 在图像四周添加白色留白。Windows OCR 对紧贴裁剪边缘的单个数字经常
/// 直接丢弃（整列裁剪里的 3.3、4.7 就属于这种情况）；留白后识别稳定。
pub fn pad_white(image: &RgbaImage, margin: u32) -> RgbaImage {
    let (w, h) = image.dimensions();
    let mut out = RgbaImage::from_pixel(
        w + margin * 2,
        h + margin * 2,
        image::Rgba([255, 255, 255, 255]),
    );
    imageops::overlay(&mut out, image, margin as i64, margin as i64);
    out
}

pub fn save_png(image: &RgbaImage, path: &Path) -> Result<(), String> {
    image
        .save_with_format(path, image::ImageFormat::Png)
        .map_err(|error| format!("无法写入临时图像 {}：{error}", path.display()))
}

/// 去除表格长直线并二值化。只对“纸面”占比足够高的页面启用，
/// 避免把照片类文档整页涂白。返回 `None` 表示该页不适合清理。
pub fn clean_page(image: &RgbaImage) -> Option<GrayImage> {
    let (width, height) = image.dimensions();
    let mut luma = GrayImage::new(width, height);
    let mut bright = 0usize;
    for (x, y, pixel) in image.enumerate_pixels() {
        let [r, g, b, a] = pixel.0;
        let value = if a < 32 {
            255
        } else {
            ((r as u32 * 299 + g as u32 * 587 + b as u32 * 114) / 1000) as u8
        };
        if value >= 200 {
            bright += 1;
        }
        luma.put_pixel(x, y, Luma([value]));
    }
    let total = (width * height) as usize;
    if bright * 100 < total * 55 {
        return None;
    }

    let mut kill_row = vec![false; height as usize];
    let mut kill_col = vec![false; width as usize];
    for y in 0..height {
        let dark = (0..width)
            .filter(|x| luma.get_pixel(*x, y).0[0] < 230)
            .count();
        kill_row[y as usize] = dark > (width as f64 * 0.45) as usize;
    }
    for x in 0..width {
        let dark = (0..height)
            .filter(|y| luma.get_pixel(x, *y).0[0] < 230)
            .count();
        kill_col[x as usize] = dark > (height as f64 * 0.45) as usize;
    }

    let mut cleaned = GrayImage::new(width, height);
    for y in 0..height {
        for x in 0..width {
            let value = if kill_row[y as usize] || kill_col[x as usize] {
                255
            } else {
                let v = luma.get_pixel(x, y).0[0];
                if v < 175 { 0 } else { 255 }
            };
            cleaned.put_pixel(x, y, Luma([value]));
        }
    }
    Some(cleaned)
}

pub fn save_gray(image: &GrayImage, path: &Path) -> Result<(), String> {
    image
        .save_with_format(path, image::ImageFormat::Png)
        .map_err(|error| format!("无法写入临时图像 {}：{error}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_helpers_ignore_ocr_spaces() {
        let line = OcrLine {
            text: "持 续 推 力".to_owned(),
            words: vec![
                OcrWord { text: "持".into(), x: 10.0, y: 20.0, w: 20.0, h: 20.0 },
                OcrWord { text: "续".into(), x: 32.0, y: 20.0, w: 20.0, h: 20.0 },
            ],
        };
        assert_eq!(line.joined(), "持续推力");
        assert_eq!(line.center_y(), 30.0);
        assert!(line.center_x() > 10.0);
    }

    #[test]
    fn embedded_script_is_ascii_only() {
        // PowerShell 5.1 会把无 BOM 的 UTF-8 当 ANSI；非 ASCII 注释会破坏脚本。
        assert!(SCRIPT.is_ascii(), "OCR 脚本必须保持纯 ASCII");
    }

    #[test]
    fn relative_document_paths_use_backslashes() {
        let cwd = Path::new(r"C:\work\app");
        let resolved = resolve_document_path(Path::new("shots/page-032.png"), cwd);
        assert_eq!(resolved, PathBuf::from(r"C:\work\app\shots\page-032.png"));
    }

    #[test]
    fn absolute_document_paths_keep_their_form() {
        let resolved = resolve_document_path(Path::new(r"C:\docs/page.png"), Path::new(r"C:\work"));
        assert_eq!(resolved, PathBuf::from(r"C:\docs\page.png"));
    }

    #[test]
    fn decodes_gbk_stderr_instead_of_mojibake() {
        // 中文系统上 PowerShell 5.1 的错误流是 GBK（936）编码。
        assert_eq!(decode_stderr(&[0xBD, 0xFB, 0xD6, 0xB9]), "禁止");
    }

    #[test]
    fn decodes_utf16_stderr_with_bom() {
        let mut bytes = vec![0xFF, 0xFE];
        bytes.extend("错误".encode_utf16().flat_map(u16::to_le_bytes));
        assert_eq!(decode_stderr(&bytes), "错误");
        assert_eq!(decode_stderr(b"plain error"), "plain error");
    }

    #[test]
    fn sidecar_error_replaces_powershell_text() {
        let dir = std::env::temp_dir().join(format!("motionsizer-ocr-test-{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        let out = dir.join("batch.ndjson");
        let sidecar = dir.join("batch.ndjson.error");
        fs::write(
            &sidecar,
            r#"{"stage":"recognize page 3","message":"no language pack"}"#,
        )
        .unwrap();
        let error = error_with_sidecar(&out, "Windows OCR 执行失败：mojibake".to_owned());
        assert_eq!(error, "Windows OCR 执行失败（recognize page 3）：no language pack");
        assert!(!sidecar.exists(), "读取后应删除 sidecar");
        let _ = fs::remove_dir_all(&dir);
    }
}
