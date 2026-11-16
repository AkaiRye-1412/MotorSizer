//! 导入工作流：后台线程执行“文档识别 → 表格重建 → 多通道识别 → 预览数据”，
//! 通过事件通道把进度送回界面；中间图像全部放在临时目录，结束后删除。

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver};
use std::thread;

use eframe::egui;

use crate::data::motor::Motor;
use crate::import::ocr::{self, OcrSheet, Progress};
use crate::import::parse::{self, Field, PageTable, Pass, PassKind};
use crate::import::pdftext;

pub enum Event {
    Progress(Progress),
    Finished(Result<Preview, String>),
}

pub struct Setup {
    pub path: PathBuf,
    /// 1 基页码；图片输入时忽略。
    pub start_page: u32,
    /// 0 表示到最后一页；图片输入时忽略。
    pub end_page: u32,
}

pub struct Candidate {
    pub page: u32,
    pub motor: Motor,
    pub detected: Vec<Field>,
    pub conflicts: Vec<Field>,
    pub include: bool,
}

pub struct Preview {
    pub source: PathBuf,
    pub pages: u32,
    pub engine: String,
    pub candidates: Vec<Candidate>,
    /// 导入前的数据库快照，用于在用户改名后重新判断“覆盖/新增”。
    pub existing: Vec<Motor>,
}

impl Preview {
    /// 与数据库同名、将被覆盖的候选数量（仅统计勾选项）。
    pub fn overwrite_count(&self) -> usize {
        self.candidates
            .iter()
            .filter(|candidate| {
                candidate.include
                    && parse::match_existing(&candidate.motor.name, &self.existing).is_some()
            })
            .count()
    }
}

/// 判断参数是否足以参与选型计算；预览中不满足的行默认不勾选。
pub fn is_usable(motor: &Motor) -> bool {
    motor.cont > 0.0
        && motor.peak > 0.0
        && motor.kf > 0.0
        && motor.km > 0.0
        && motor.i_cont > 0.0
        && motor.i_peak > 0.0
}

struct Job {
    sheet_index: usize,
    table: PageTable,
    image_path: PathBuf,
}

/// 单行补扫条带的放大倍数：Windows OCR 在 4 倍放大时能稳定读出
/// 整列裁剪漏掉的数字单元格。
const ROW_STRIP_SCALE: f64 = 4.0;

/// 单元格补扫的放大倍数与四周白色留白（像素）。Windows OCR 对紧贴
/// 裁剪边缘的单个数字容易丢字或丢小数点（3.3 读成 33、4.7 整行丢失），
/// 裁出单个单元格、放大并留白后最稳定。
///
/// 不同单元格在 OCR 里“恰好被读对”的放大倍数并不一致（实测页 3 并联
/// Kf 只有 1.5 倍列裁剪再放大到约 7.5 倍才读对，页 7 并联 Peak 需要
/// 12 倍以上，页 5 并联 Resist 需要 18 倍）；因此每个单元格都生成若干
/// 放大倍数，由 `parse::decide` 按读数质量挑出可信的那个。
const CELL_SCALES: [f64; 4] = [5.0, 8.0, 12.0, 18.0];
const CELL_PADDING: u32 = 60;

/// 单元格补扫的几何信息；`job` 指向 `jobs` 下标。
struct CellSpec {
    job: usize,
    column: usize,
    row: usize,
    /// 目标数值行在放大裁剪图中的纵坐标，以及允许的偏差。
    y: f64,
    tolerance: f64,
    weight: u32,
}

struct CropSpec {
    column: usize,
    top: f64,
    scale: f64,
    weight: u32,
}

/// 单行补扫条带的几何信息；`x0`/`top` 是裁剪图左上角在页面中的坐标。
struct RowSpec {
    job: usize,
    x0: f64,
    top: f64,
    scale_x: f64,
    scale_y: f64,
    weight: u32,
}

pub struct Running {
    pub receiver: Receiver<Event>,
    pub cancel: PathBuf,
}

/// 启动后台识别任务。界面在每帧轮询 `Running::receiver`，
/// 取消时向 `Running::cancel` 写入标记文件。
pub fn spawn(
    setup: Setup,
    existing: Vec<Motor>,
    ctx: egui::Context,
) -> Result<Running, String> {
    let workspace = ocr::new_workspace()?;
    let cancel = workspace.join("cancel-requested");
    let (sender, receiver) = channel();
    let thread_cancel = cancel.clone();
    thread::spawn(move || {
        let progress = |update: Progress| {
            let _ = sender.send(Event::Progress(update));
            ctx.request_repaint();
        };
        let result = run(&setup, &existing, &workspace, &thread_cancel, &progress);
        if std::env::var_os("MOTORSIZER_KEEP_IMPORT").is_none() {
            ocr::remove_workspace(&workspace);
        }
        let _ = sender.send(Event::Finished(result));
        ctx.request_repaint();
    });
    Ok(Running { receiver, cancel })
}

fn run(
    setup: &Setup,
    existing: &[Motor],
    workspace: &std::path::Path,
    cancel: &std::path::Path,
    progress: &dyn Fn(Progress),
) -> Result<Preview, String> {
    let doc = ocr::recognize_document(
        &setup.path,
        setup.start_page,
        setup.end_page,
        cancel,
        workspace,
        progress,
    )?;
    let cleanup = doc.render_dir.clone();
    let outcome = analyze(&doc, setup, existing, workspace, cancel, progress);
    if let Some(dir) = cleanup {
        // 渲染页只用于本次裁剪；成功或失败都清理，避免临时目录越积越多。
        let _ = fs::remove_dir_all(dir);
    }
    outcome
}

fn analyze(
    doc: &ocr::OcrDoc,
    setup: &Setup,
    existing: &[Motor],
    workspace: &std::path::Path,
    cancel: &std::path::Path,
    progress: &dyn Fn(Progress),
) -> Result<Preview, String> {
    if cancel.exists() {
        return Err(ocr::CANCELLED.to_owned());
    }

    // 矢量 PDF 优先读取文本层：数值没有识别误差，也不需要生成放大裁剪图。
    // 按页混合：有文本层的页面直接用文本层数值，图片型页面才走 OCR。
    let text_doc: Option<Vec<OcrSheet>> = if doc.kind == "pdf" {
        pdftext::read_text_sheets(&setup.path, setup.start_page, setup.end_page)
            .ok()
            .map(|(sheets, _)| sheets)
            .filter(|sheets| !sheets.is_empty())
    } else {
        None
    };
    let text_map: HashMap<u32, usize> = text_doc
        .as_ref()
        .map(|sheets| {
            sheets
                .iter()
                .enumerate()
                .map(|(index, sheet)| (sheet.page, index))
                .collect()
        })
        .unwrap_or_default();

    let render_dir = doc.render_dir.clone();
    let mut jobs: Vec<Job> = Vec::new();
    for (index, sheet) in doc.sheets.iter().enumerate() {
        // 有文本层的页面优先用文本层建表：文字和坐标都来自 PDF，不受 OCR
        // 误识、空页或坐标漂移影响；OCR 结果只作为兜底。
        let text_index = text_map.get(&sheet.page).copied();
        let text_table = text_doc.as_ref().and_then(|sheets| {
            text_index.and_then(|index| parse::detect_table(&sheets[index]))
        });
        let table = text_table.or_else(|| parse::detect_table(sheet));
        let Some(table) = table else {
            continue;
        };
        let image_path = if doc.kind == "pdf" {
            let dir = render_dir
                .as_ref()
                .ok_or("PDF 页面渲染目录缺失，无法识别参数单元格")?;
            dir.join(format!("p-{:04}.png", sheet.page))
        } else {
            setup.path.clone()
        };
        jobs.push(Job {
            sheet_index: index,
            table,
            image_path,
        });
    }
    if jobs.is_empty() {
        return Err(
            "未在选定页面中识别到电机参数表。请确认页码范围，或换用更清晰的扫描件。"
                .to_owned(),
        );
    }

    progress(Progress {
        stage: "分析表格".to_owned(),
        done: 0,
        total: 0,
    });

    // 全部页面都有文本层时直接返回，避免任何裁剪。
    if !jobs.is_empty() && jobs.iter().all(|job| text_map.contains_key(&job.table.page)) {
        if let Some(sheets) = text_doc.as_ref() {
            return analyze_text(doc, &jobs, sheets, setup, existing, workspace);
        }
    }

    // 为每一页生成“整页 + 每列两种缩放 + 去网格整页”的识别任务。
    let batch_file = workspace.join("batch.txt");
    let mut batch = String::new();
    let mut crop_specs: HashMap<String, CropSpec> = HashMap::new();
    let mut clean_ids: HashSet<String> = HashSet::new();
    let mut vertical_scales = Vec::with_capacity(jobs.len());
    for (job_index, job) in jobs.iter().enumerate() {
        if text_map.contains_key(&job.table.page) {
            // 文本层页面不需要渲染图；占位以保持垂直缩放数组的下标对齐。
            vertical_scales.push(1.0);
            continue;
        }
        let image = ocr::load_page_image(&job.image_path)?;
        let fx = image.width() as f64 / job.table.width.max(1.0);
        let fy = image.height() as f64 / job.table.height.max(1.0);
        vertical_scales.push(fy);

        let min_gap = job
            .table
            .columns
            .windows(2)
            .map(|pair| (pair[1].center_x - pair[0].center_x).abs())
            .fold(f64::INFINITY, f64::min);
        let half = (min_gap * 0.48).clamp(55.0, 95.0);
        let top = (job.table.header_bottom - 8.0).max(0.0);
        let bottom = (job.table.bottom + 12.0).min(job.table.height);

        for (column_index, column) in job.table.columns.iter().enumerate() {
            for (tag, scale) in [("1", 1.0_f64), ("15", 1.5_f64)] {
                let crop = ocr::crop_column(
                    &image,
                    column.center_x * fx,
                    half * fx,
                    top * fy,
                    bottom * fy,
                );
                let output = if (scale - 1.0).abs() < f64::EPSILON {
                    crop
                } else {
                    ocr::resize(&crop, scale)
                };
                let id = format!("j{job_index}-c{column_index}-{tag}");
                let path = workspace.join(format!("{id}.png"));
                ocr::save_png(&output, &path)?;
                batch.push_str(&format!("{id}\t{}\n", path.display()));
                crop_specs.insert(
                    id,
                    CropSpec {
                        column: column_index,
                        top,
                        scale: scale * fy,
                        weight: if tag == "1" { 2 } else { 3 },
                    },
                );
            }
        }

        if let Some(cleaned) = ocr::clean_page(&image) {
            let id = format!("j{job_index}-clean");
            let path = workspace.join(format!("{id}.png"));
            ocr::save_gray(&cleaned, &path)?;
            batch.push_str(&format!("{id}\t{}\n", path.display()));
            clean_ids.insert(id);
        }
    }
    fs::write(&batch_file, batch)
        .map_err(|error| format!("无法写入识别任务清单：{error}"))?;

    let batch_doc = ocr::recognize_batch(&batch_file, cancel, workspace, progress, "识别参数")?;
    if cancel.exists() {
        return Err(ocr::CANCELLED.to_owned());
    }
    let mut sheets_by_id: HashMap<&str, &OcrSheet> = HashMap::new();
    for sheet in &batch_doc.sheets {
        sheets_by_id.insert(sheet.id.as_str(), sheet);
    }

    progress(Progress {
        stage: "整理候选型号".to_owned(),
        done: 0,
        total: 0,
    });

    // 补扫结果需要活到函数结束：`job_passes` 会借用其中的 OcrSheet。
    let row_doc: Option<ocr::OcrDoc>;
    let cell_doc: Option<ocr::OcrDoc>;

    // 第一轮：整页 + 每列两种缩放 + 去网格整页。
    let mut job_passes: Vec<Vec<Pass>> = Vec::with_capacity(jobs.len());
    for (job_index, job) in jobs.iter().enumerate() {
        // 该页有文本层：只用一个整页通道，数值全部精确。
        if let (Some(sheets), Some(&text_index)) =
            (text_doc.as_ref(), text_map.get(&job.table.page))
        {
            job_passes.push(vec![Pass {
                sheet: &sheets[text_index],
                kind: PassKind::Text { weight: 1 },
            }]);
            continue;
        }
        let fy = vertical_scales[job_index];
        let mut passes: Vec<Pass> = vec![Pass {
            sheet: &doc.sheets[job.sheet_index],
            kind: PassKind::Full {
                weight: 1,
                scale: fy,
            },
        }];
        for (column_index, _) in job.table.columns.iter().enumerate() {
            for tag in ["1", "15"] {
                let id = format!("j{job_index}-c{column_index}-{tag}");
                if let (Some(sheet), Some(spec)) = (sheets_by_id.get(id.as_str()), crop_specs.get(&id)) {
                    passes.push(Pass {
                        sheet: *sheet,
                        kind: PassKind::Column {
                            column: spec.column,
                            top: spec.top,
                            scale: spec.scale,
                            weight: spec.weight,
                        },
                    });
                }
            }
        }
        let clean_id = format!("j{job_index}-clean");
        if let Some(sheet) = sheets_by_id.get(clean_id.as_str()) {
            if clean_ids.contains(&clean_id) {
                passes.push(Pass {
                    sheet: *sheet,
                    kind: PassKind::Full {
                        weight: 2,
                        scale: fy,
                    },
                });
            }
        }
        job_passes.push(passes);
    }

    let mut recognized: Vec<Vec<parse::Recognized>> = jobs
        .iter()
        .zip(&job_passes)
        .map(|(job, passes)| parse::collect(&job.table, passes))
        .collect();

    // Windows OCR 偶尔会把整行数字漏掉（窄列裁剪里尤其明显）。对这些
    // 行补做一次横向条带识别：只裁剪各数值列覆盖的区域，4 倍放大，
    // 行数少、上下文宽，通常能恢复被丢弃的单元格。
    let mut row_batch = String::new();
    let mut row_specs: HashMap<String, RowSpec> = HashMap::new();
    for (job_index, job) in jobs.iter().enumerate() {
        // 文本层页面不需要 OCR 补扫。
        if text_map.contains_key(&job.table.page) {
            continue;
        }
        let missing_rows: Vec<usize> = job
            .table
            .rows
            .iter()
            .enumerate()
            .filter(|(_, row)| {
                recognized[job_index]
                    .iter()
                    .any(|column| column.value(row.field).is_none())
            })
            .map(|(index, _)| index)
            .collect();
        if missing_rows.is_empty() {
            continue;
        }

        let image = ocr::load_page_image(&job.image_path)?;
        let fx = image.width() as f64 / job.table.width.max(1.0);
        let fy = image.height() as f64 / job.table.height.max(1.0);
        let first = job.table.columns.first().map(|c| c.center_x).unwrap_or(0.0);
        let last = job.table.columns.last().map(|c| c.center_x).unwrap_or(0.0);
        let min_gap = job
            .table
            .columns
            .windows(2)
            .map(|pair| (pair[1].center_x - pair[0].center_x).abs())
            .fold(f64::INFINITY, f64::min);
        let half = (min_gap * 0.48).clamp(55.0, 95.0);
        let x0 = (first - half).max(0.0);
        let x1 = (last + half).min(job.table.width.max(0.0));
        let row_gap = job
            .table
            .rows
            .windows(2)
            .map(|pair| (pair[1].center_y - pair[0].center_y).abs())
            .filter(|gap| *gap > 1.0)
            .fold(f64::INFINITY, f64::min);
        let row_half = (row_gap * 0.55).clamp(14.0, 24.0);

        for row_index in missing_rows {
            let row = &job.table.rows[row_index];
            let top = (row.center_y - row_half).max(0.0);
            let bottom = (row.center_y + row_half).min(job.table.height.max(0.0));
            let crop = ocr::crop_column(
                &image,
                (x0 + x1) / 2.0 * fx,
                (x1 - x0) / 2.0 * fx,
                top * fy,
                bottom * fy,
            );
            let output = ocr::resize(&crop, ROW_STRIP_SCALE);
            let id = format!("j{job_index}-r{row_index}");
            let path = workspace.join(format!("{id}.png"));
            ocr::save_png(&output, &path)?;
            row_batch.push_str(&format!("{id}\t{}\n", path.display()));
            row_specs.insert(
                id,
                RowSpec {
                    job: job_index,
                    x0,
                    top,
                    scale_x: ROW_STRIP_SCALE * fx,
                    scale_y: ROW_STRIP_SCALE * fy,
                    weight: 3,
                },
            );
        }
    }

    if !row_batch.is_empty() {
        let row_file = workspace.join("rows.txt");
        fs::write(&row_file, row_batch)
            .map_err(|error| format!("无法写入补扫任务清单：{error}"))?;
        row_doc = Some(ocr::recognize_batch(
            &row_file,
            cancel,
            workspace,
            progress,
            "补扫参数行",
        )?);
        if cancel.exists() {
            return Err(ocr::CANCELLED.to_owned());
        }
        if let Some(row_doc) = row_doc.as_ref() {
            let mut row_sheets: HashMap<&str, &OcrSheet> = HashMap::new();
            for sheet in &row_doc.sheets {
                row_sheets.insert(sheet.id.as_str(), sheet);
            }
            for (id, spec) in &row_specs {
                if let Some(sheet) = row_sheets.get(id.as_str()) {
                    job_passes[spec.job].push(Pass {
                        sheet: *sheet,
                        kind: PassKind::Band {
                            x0: spec.x0,
                            top: spec.top,
                            scale_x: spec.scale_x,
                            scale_y: spec.scale_y,
                            weight: spec.weight,
                        },
                    });
                }
            }
            recognized = jobs
                .iter()
                .zip(&job_passes)
                .map(|(job, passes)| parse::collect(&job.table, passes))
                .collect();
        }
    } else {
        row_doc = None;
    }
    // 读取一次，避免“赋值后未使用”的警告；补扫结果本身已借用给
    // `job_passes`，在函数结束时按声明顺序释放。
    let _ = &row_doc;

    // 第三轮：逐单元格高倍放大识别。整列裁剪会把个别数值连同小数点
    // 一起吞掉，导致“33 而不是 3.3”或整格缺失；单元格通道只含一个
    // 数值，行列直接已知，识别结果优先于整页/整列通道。
    //
    // 单元格裁剪数量是“页 × 列 × 行 × 放大倍数”，大型样册会达到上万张，
    // 生成和识别都要很久。因此：小批量（≤400 格）保留全部放大倍数；
    // 更大的文档只补扫仍然缺失的格子、只用两个倍数；预计超过上限时
    // 直接跳过该通道，交给整列/条带通道，避免导入看起来卡死。
    const CELL_SCAN_ALL_LIMIT: usize = 400;
    const CELL_CROP_LIMIT: usize = 1200;
    let total_cells: usize = jobs
        .iter()
        .filter(|job| !text_map.contains_key(&job.table.page))
        .map(|job| job.table.rows.len() * job.table.columns.len())
        .sum();
    let scan_all = total_cells <= CELL_SCAN_ALL_LIMIT;
    let scales: &[f64] = if scan_all { &CELL_SCALES } else { &[8.0, 18.0] };
    let planned: usize = jobs
        .iter()
        .enumerate()
        .map(|(job_index, job)| {
            if text_map.contains_key(&job.table.page) {
                return 0;
            }
            let cells = job.table.rows.len() * job.table.columns.len();
            if scan_all {
                cells * scales.len()
            } else {
                let mut pending = 0usize;
                for (column_index, _) in job.table.columns.iter().enumerate() {
                    for row in &job.table.rows {
                        let missing = recognized
                            .get(job_index)
                            .and_then(|columns| columns.get(column_index))
                            .map(|values| values.value(row.field).is_none())
                            .unwrap_or(true);
                        if missing {
                            pending += scales.len();
                        }
                    }
                }
                pending
            }
        })
        .sum();
    let run_cell_pass = planned > 0 && planned <= CELL_CROP_LIMIT;
    let mut generated = 0usize;

    let mut cell_batch = String::new();
    let mut cell_specs: Vec<(String, CellSpec)> = Vec::new();
    for (job_index, job) in jobs.iter().enumerate() {
        if !run_cell_pass
            || text_map.contains_key(&job.table.page)
            || job.table.rows.is_empty()
            || job.table.columns.len() < 2
        {
            continue;
        }
        let image = ocr::load_page_image(&job.image_path)?;
        let fx = image.width() as f64 / job.table.width.max(1.0);
        let fy = image.height() as f64 / job.table.height.max(1.0);
        let columns = &job.table.columns;
        let rows = &job.table.rows;
        let col_gap = columns
            .windows(2)
            .map(|pair| (pair[1].center_x - pair[0].center_x).abs())
            .filter(|gap| *gap > 1.0)
            .fold(f64::INFINITY, f64::min);
        let row_gap = rows
            .windows(2)
            .map(|pair| (pair[1].center_y - pair[0].center_y).abs())
            .filter(|gap| *gap > 1.0)
            .fold(f64::INFINITY, f64::min);
        if !col_gap.is_finite() || !row_gap.is_finite() {
            continue;
        }

        for (column_index, column) in columns.iter().enumerate() {
            // 单元格左右边界取相邻列中心的中点，并向内缩几个像素，
            // 避免裁进表格竖线。
            let x0 = if column_index > 0 {
                (columns[column_index - 1].center_x + column.center_x) / 2.0 + 4.0
            } else {
                column.center_x - col_gap / 2.0 + 4.0
            };
            let x1 = if column_index + 1 < columns.len() {
                (column.center_x + columns[column_index + 1].center_x) / 2.0 - 4.0
            } else {
                column.center_x + col_gap / 2.0 - 4.0
            };
            for (row_index, row) in rows.iter().enumerate() {
                // 大文档只补扫仍然缺值的格子，避免为已经读到的数值重复
                // 生成裁剪图。
                if !scan_all {
                    let missing = recognized
                        .get(job_index)
                        .and_then(|columns| columns.get(column_index))
                        .map(|values| values.value(row.field).is_none())
                        .unwrap_or(true);
                    if !missing {
                        continue;
                    }
                }
                // 纵向只取该行中心附近的窄带，不留到相邻行的中线：
                // 表格横线贴着行间中线，包含横线时 Windows OCR 会整行
                // 丢失（页 3 并联 Kf 的 1.5 就是因此读不出来）。
                let gap_prev = if row_index > 0 {
                    (row.center_y - rows[row_index - 1].center_y).abs()
                } else {
                    f64::INFINITY
                };
                let gap_next = if row_index + 1 < rows.len() {
                    (rows[row_index + 1].center_y - row.center_y).abs()
                } else {
                    f64::INFINITY
                };
                let local_gap = gap_prev.min(gap_next).min(row_gap);
                let half_y = (local_gap * 0.40).max(10.0);
                let y0 = row.center_y - half_y;
                let y1 = row.center_y + half_y;
                let crop = ocr::crop_column(
                    &image,
                    (x0 + x1) / 2.0 * fx,
                    (x1 - x0) / 2.0 * fx,
                    y0 * fy,
                    y1 * fy,
                );
                for &scale in scales {
                    generated += 1;
                    if generated % 100 == 0 {
                        progress(Progress {
                            stage: "生成单元格裁剪".to_owned(),
                            done: generated as u32,
                            total: planned as u32,
                        });
                        if cancel.exists() {
                            return Err(ocr::CANCELLED.to_owned());
                        }
                    }
                    let output = ocr::pad_white(&ocr::resize(&crop, scale), CELL_PADDING);
                    let id = format!("j{job_index}-cell{column_index}-{row_index}-s{scale:.0}");
                    let path = workspace.join(format!("{id}.png"));
                    ocr::save_png(&output, &path)?;
                    cell_batch.push_str(&format!("{id}\t{}\n", path.display()));
                    // 目标数值行在裁剪图中的位置：裁剪坐标乘缩放后加留白。
                    let expected_y = (row.center_y - y0) * fy * scale + CELL_PADDING as f64;
                    cell_specs.push((
                        id,
                        CellSpec {
                            job: job_index,
                            column: column_index,
                            row: row_index,
                            y: expected_y,
                            tolerance: half_y * fy * scale,
                            weight: 2,
                        },
                    ));
                }
            }
        }
    }

    if !cell_batch.is_empty() {
        let cell_file = workspace.join("cells.txt");
        fs::write(&cell_file, cell_batch)
            .map_err(|error| format!("无法写入单元格识别清单：{error}"))?;
        cell_doc = Some(ocr::recognize_batch(
            &cell_file,
            cancel,
            workspace,
            progress,
            "逐格核对参数",
        )?);
        if cancel.exists() {
            return Err(ocr::CANCELLED.to_owned());
        }
        if let Some(cell_doc) = cell_doc.as_ref() {
            let mut cell_sheets: HashMap<&str, &OcrSheet> = HashMap::new();
            for sheet in &cell_doc.sheets {
                cell_sheets.insert(sheet.id.as_str(), sheet);
            }
            for (id, spec) in &cell_specs {
                if let Some(sheet) = cell_sheets.get(id.as_str()) {
                    job_passes[spec.job].push(Pass {
                        sheet: *sheet,
                        kind: PassKind::Cell {
                            column: spec.column,
                            row: spec.row,
                            y: spec.y,
                            tolerance: spec.tolerance,
                            weight: spec.weight,
                        },
                    });
                }
            }
            recognized = jobs
                .iter()
                .zip(&job_passes)
                .map(|(job, passes)| parse::collect(&job.table, passes))
                .collect();
        }
    } else {
        cell_doc = None;
    }
    let _ = &cell_doc;

    let candidates = build_candidates(&jobs, &recognized, existing);

    if std::env::var_os("MOTORSIZER_DUMP_IMPORT").is_some() {
        dump_candidates(workspace, &jobs, &recognized, &candidates);
    }

    Ok(Preview {
        source: setup.path.clone(),
        pages: doc.page_count,
        engine: doc.engine.clone(),
        candidates,
        existing: existing.to_vec(),
    })
}

/// 调试用：把候选的识别结果写到工作目录，便于核对数值。
fn dump_candidates(
    workspace: &std::path::Path,
    jobs: &[Job],
    recognized: &[Vec<parse::Recognized>],
    candidates: &[Candidate],
) {
    let mut text = String::new();
    for (job, values) in jobs.iter().zip(recognized) {
        text.push_str(&format!(
            "\nPAGE {} columns={:?}\n",
            job.table.page,
            job
                .table
                .columns
                .iter()
                .map(|c| (c.name.clone(), c.center_x))
                .collect::<Vec<_>>()
        ));
        for row in &job.table.rows {
            text.push_str(&format!("  {:?} y={:.1}\n", row.field, row.center_y));
        }
        for (column, values) in job.table.columns.iter().zip(values) {
            text.push_str(&format!(
                "  {}: {:?}\n",
                column.name,
                values
                    .recognized_fields()
                    .into_iter()
                    .map(|field| (field, values.value(field)))
                    .collect::<Vec<_>>()
            ));
        }
    }
    text.push('\n');
    for candidate in candidates {
        let m = &candidate.motor;
        text.push_str(&format!(
            "{} p{} include={} usable={} detected={:?} conflicts={:?}\n  cont={} peak={} kf={} km={} icont={} ipeak={} mass={} inertia={}\n",
            m.name,
            candidate.page,
            candidate.include,
            is_usable(m),
            candidate.detected,
            candidate.conflicts,
            m.cont,
            m.peak,
            m.kf,
            m.km,
            m.i_cont,
            m.i_peak,
            m.mover_mass,
            m.inertia,
        ));
    }
    let _ = fs::write(workspace.join("candidates.txt"), text);
}

/// 把识别结果按列转成候选型号，并合并重复型号。
fn build_candidates(
    jobs: &[Job],
    recognized: &[Vec<parse::Recognized>],
    existing: &[Motor],
) -> Vec<Candidate> {
    let mut candidates: Vec<Candidate> = Vec::new();
    let mut seen: HashMap<String, usize> = HashMap::new();
    let mut used: HashSet<String> = HashSet::new();
    for (job, values_all) in jobs.iter().zip(recognized) {
        for (column, values) in job.table.columns.iter().zip(values_all) {
            let matched = parse::match_existing(&column.name, existing);
            let mut motor = match matched {
                Some(index) => existing[index].clone(),
                None => Motor::blank(&column.name),
            };
            motor.name = column.name.clone();
            values.apply(&mut motor, job.table.rotary);

            let detected = values.recognized_fields();
            let conflicts = detected
                .iter()
                .copied()
                .filter(|field| values.has_conflict(*field))
                .collect();
            let candidate = Candidate {
                page: job.table.page,
                include: is_usable(&motor),
                motor,
                detected,
                conflicts,
            };

            let key = candidate.motor.name.to_ascii_uppercase();
            if used.insert(key.clone()) {
                seen.insert(key, candidates.len());
                candidates.push(candidate);
            } else if let Some(index) = seen.get(&key).copied() {
                // 同一型号出现在多页（索引页/重复表）时保留识别字段更多的一份。
                if candidate.detected.len() > candidates[index].detected.len() {
                    candidates[index] = candidate;
                }
            }
        }
    }
    candidates
}

/// 文本层路径：表格结构仍来自 OCR（行列标签与列标题识别可靠），
/// 但每个数值直接取自 PDF 文本层，因此没有小数点丢失、短数字漏读
/// 之类的 OCR 误差。
fn analyze_text(
    doc: &ocr::OcrDoc,
    jobs: &[Job],
    sheets: &[ocr::OcrSheet],
    setup: &Setup,
    existing: &[Motor],
    workspace: &std::path::Path,
) -> Result<Preview, String> {
    let by_page: HashMap<u32, &ocr::OcrSheet> =
        sheets.iter().map(|sheet| (sheet.page, sheet)).collect();
    let mut passes: Vec<Vec<Pass>> = Vec::with_capacity(jobs.len());
    for job in jobs {
        let sheet = by_page
            .get(&job.table.page)
            .ok_or("PDF 文本层缺少对应的页面")?;
        // 文本层坐标已经与页面像素一一对应（见 pdftext 的 PIXELS_PER_POINT），
        // 使用文本层通道按行内 x 顺序分配列。
        passes.push(vec![Pass {
            sheet,
            kind: PassKind::Text { weight: 1 },
        }]);
    }

    let recognized: Vec<Vec<parse::Recognized>> = jobs
        .iter()
        .zip(&passes)
        .map(|(job, passes)| parse::collect(&job.table, passes))
        .collect();
    let candidates = build_candidates(jobs, &recognized, existing);
    if std::env::var_os("MOTORSIZER_DUMP_IMPORT").is_some() {
        dump_candidates(workspace, jobs, &recognized, &candidates);
    }

    Ok(Preview {
        source: setup.path.clone(),
        pages: doc.page_count,
        engine: format!("{} + 文本层", doc.engine),
        candidates,
        existing: existing.to_vec(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 诊断：对真实样册指定页码跑完整导入，打印候选字段。
    #[test]
    fn diagnostic_real_pdf_page() {
        let Ok(path) = std::env::var("MOTORSIZER_PDF_FIXTURE") else {
            return;
        };
        let page: u32 = std::env::var("MOTORSIZER_PDF_PAGE")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(1);
        let workspace = crate::import::ocr::new_workspace().expect("工作区");
        let cancel = workspace.join("cancel");
        let setup = Setup {
            path: std::path::PathBuf::from(&path),
            start_page: page,
            end_page: page,
        };
        let preview = run(&setup, &[], &workspace, &cancel, &|_| {}).expect("导入失败");
        for candidate in &preview.candidates {
            eprintln!(
                "{} p{} usable={} cont={} peak={} kf={} km={} ic={} ip={} ke={} mass={} len={} page={}",
                candidate.motor.name,
                candidate.page,
                is_usable(&candidate.motor),
                candidate.motor.cont,
                candidate.motor.peak,
                candidate.motor.kf,
                candidate.motor.km,
                candidate.motor.i_cont,
                candidate.motor.i_peak,
                candidate.motor.ke,
                candidate.motor.mover_mass,
                candidate.motor.length,
                candidate.motor.kind == crate::data::motor::MotorKind::Rotary,
            );
        }
        eprintln!("engine={} candidates={}", preview.engine, preview.candidates.len());
        let _ = std::fs::remove_dir_all(&workspace);
    }

    #[test]
    fn usable_requires_core_selection_fields() {
        let mut motor = Motor::blank("AUM1-S-S1");
        assert!(!is_usable(&motor));
        motor.cont = 3.0;
        motor.peak = 11.9;
        motor.kf = 1.75;
        motor.km = 1.67;
        motor.i_cont = 1.7;
        motor.i_peak = 6.8;
        assert!(is_usable(&motor));
    }
}
