// 发布版使用 GUI 子系统：双击 exe 时不再弹出黑色控制台窗口。
// 调试版保留控制台，便于查看 panic 和运行日志。
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod data;
mod domain;
mod import;
mod ui;

use std::io::Write;
use std::path::PathBuf;

use eframe::egui;

fn main() -> eframe::Result<()> {
    let mut screenshot = None;
    let mut report_dir = None;
    let mut tab = app::Tab::Linear;
    // 截图自检用的导入入口：预先选中文件，可选立即开始识别。
    let mut import_path = None;
    let mut import_run = false;
    let mut args = std::env::args().skip(1);
    while let Some(argument) = args.next() {
        if argument == "--screenshot" {
            screenshot = args.next().map(PathBuf::from);
        } else if argument == "--report" {
            report_dir = args.next().map(PathBuf::from);
        } else if argument == "--import" {
            import_path = args.next().map(PathBuf::from);
        } else if argument == "--import-run" {
            import_run = true;
        } else if argument == "--tab" {
            // 便于截图自检直接指定直线/旋转页面。
            tab = match args.next().as_deref() {
                Some("rotary") => app::Tab::Rotary,
                Some("linear") | None => app::Tab::Linear,
                Some(other) => {
                    // GUI 子系统下可能没有控制台，写日志失败不应导致崩溃。
                    let _ = writeln!(std::io::stderr(), "未知页面：{other}（可选 linear / rotary）");
                    std::process::exit(2);
                }
            };
        }
    }

    // --report 无界面导出：用于脚本和自动化检查，不启动窗口。
    if let Some(dir) = report_dir {
        let mut state = match app::AppState::new() {
            Ok(state) => state,
            Err(error) => {
                let _ = writeln!(std::io::stderr(), "无法载入数据：{error}");
                std::process::exit(2);
            }
        };
        state.set_tab(tab);
        state.recompute_if_dirty();
        match app::report::export_to(&state, &dir) {
            Ok(path) => {
                let _ = writeln!(std::io::stdout(), "{}", path.display());
            }
            Err(error) => {
                let _ = writeln!(std::io::stderr(), "{error}");
                std::process::exit(3);
            }
        }
        return Ok(());
    }

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("MotionSizer Studio")
            .with_inner_size(egui::vec2(1424.0, 834.0))
            .with_min_inner_size(egui::vec2(1200.0, 760.0))
            .with_active(screenshot.is_none())
            .with_window_level(if screenshot.is_some() {
                egui::WindowLevel::AlwaysOnBottom
            } else {
                egui::WindowLevel::Normal
            }),
        renderer: eframe::Renderer::Glow,
        ..Default::default()
    };

    eframe::run_native(
        "MotionSizer Studio",
        options,
        Box::new(move |cc| {
            ui::theme::configure(&cc.egui_ctx);
            Ok(Box::new(app::App::new(
                screenshot,
                tab,
                import_path,
                import_run,
            )))
        }),
    )
}
