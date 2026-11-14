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

struct CropSpec {
    column: usize,
    top: f64,
    scale: f64,
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

    let render_dir = doc.render_dir.clone();
    let mut jobs: Vec<Job> = Vec::new();
    for (index, sheet) in doc.sheets.iter().enumerate() {
        let Some(table) = parse::detect_table(sheet) else {
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

    // 为每一页生成“整页 + 每列两种缩放 + 去网格整页”的识别任务。
    let batch_file = workspace.join("batch.txt");
    let mut batch = String::new();
    let mut crop_specs: HashMap<String, CropSpec> = HashMap::new();
    let mut clean_ids: HashSet<String> = HashSet::new();
    let mut vertical_scales = Vec::with_capacity(jobs.len());
    for (job_index, job) in jobs.iter().enumerate() {
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

    let batch_doc = ocr::recognize_batch(&batch_file, cancel, workspace, progress)?;
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

    let mut candidates: Vec<Candidate> = Vec::new();
    let mut seen: HashMap<String, usize> = HashMap::new();
    let mut used: HashSet<String> = HashSet::new();
    for (job_index, job) in jobs.iter().enumerate() {
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

        let recognized = parse::collect(&job.table, &passes);
        for (column, values) in job.table.columns.iter().zip(recognized) {
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

    Ok(Preview {
        source: setup.path.clone(),
        pages: doc.page_count,
        engine: doc.engine.clone(),
        candidates,
        existing: existing.to_vec(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

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
