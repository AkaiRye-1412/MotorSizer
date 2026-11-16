pub mod report;
pub mod state;

use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver};
use std::sync::Arc;

use eframe::egui;

use crate::ui;
pub use state::{construction_label, kind_label, AppState, Mode, MotionMode, Tab};
use state::ImportPhase;

pub struct App {
    screenshot: Option<PathBuf>,
    frames: u32,
    shot_rx: Option<Receiver<Arc<egui::ColorImage>>>,
    shot_asked: bool,
    window_shown: bool,
    /// 自检入口：首帧后自动开始识别（`--import-run`）。
    import_run: bool,
    /// 截图需等识别进入预览/失败状态再取帧（`--import-run`）。
    wait_import: bool,
    /// 数据加载成功时才有状态；失败时显示错误页而不是崩溃。
    pub state: Option<AppState>,
    pub startup_error: Option<String>,
}

impl App {
    pub fn new(
        screenshot: Option<PathBuf>,
        tab: Tab,
        import_path: Option<PathBuf>,
        import_run: bool,
    ) -> Self {
        let (mut state, startup_error) = match AppState::new() {
            Ok(mut state) => {
                // 截图自检时可直接切到旋转页面；正常启动保持默认直线页面。
                state.set_tab(tab);
                (Some(state), None)
            }
            Err(error) => (None, Some(error)),
        };
        if let (Some(path), Some(state)) = (import_path, state.as_mut()) {
            // 自检入口直接进入导入“准备”页，跳过原生文件对话框。
            state.select_import_document(path);
        }
        Self {
            screenshot,
            frames: 0,
            shot_rx: None,
            shot_asked: false,
            window_shown: false,
            import_run,
            wait_import: import_run,
            state,
            startup_error,
        }
    }

    /// 导入自检截图要等识别结束（预览或失败），避免截到进度页；
    /// 超时后仍取当前画面，防止 OCR 卡住让自检无法退出。
    fn import_ready_for_shot(&self) -> bool {
        if !self.wait_import || self.frames >= 1800 {
            return true;
        }
        match &self.state {
            Some(state) => !matches!(
                &state.import.phase,
                ImportPhase::Ready { .. } | ImportPhase::Running(_)
            ),
            None => true,
        }
    }

    /// 某些启动方式（例如从后台脚本/终端启动）会让窗口以最小化状态创建，
    /// 用户在桌面上看不到任何反应。首帧显式恢复并显示窗口；截图模式不做聚焦，
    /// 避免影响后台截图。
    fn ensure_window_visible(&mut self, ctx: &egui::Context) {
        if self.window_shown || self.screenshot.is_some() {
            return;
        }
        self.window_shown = true;
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
        ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
    }

    fn update_screenshot(&mut self, ctx: &egui::Context) {
        if let Some(rx) = &self.shot_rx {
            if let Ok(image) = rx.try_recv() {
                if let Some(path) = &self.screenshot {
                    if let Err(error) = save_png(path, &image) {
                        eprintln!("截图保存失败：{error}");
                        std::process::exit(2);
                    }
                }
                std::process::exit(0);
            }
        }

        // 等界面完成首轮布局与字体加载后再取帧。
        if self.screenshot.is_some()
            && !self.shot_asked
            && self.frames >= 120
            && self.import_ready_for_shot()
        {
            let (sender, receiver) = channel();
            self.shot_rx = Some(receiver);
            self.shot_asked = true;
            ctx.request_screenshot(move |image| {
                let _ = sender.send(image);
            });
        }
        if self.screenshot.is_some() {
            ctx.request_repaint();
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.frames += 1;
        let ctx = ui.ctx().clone();
        self.ensure_window_visible(&ctx);
        self.update_screenshot(&ctx);
        if let Some(error) = &self.startup_error {
            ui::layout::error_page(ui, error);
            return;
        }
        if let Some(state) = self.state.as_mut() {
            if self.import_run && matches!(&state.import.phase, ImportPhase::Ready { .. }) {
                self.import_run = false;
                state.begin_import(&ctx);
            }
            state.poll_import(&ctx);
            state.recompute_if_dirty();
            ui::layout::show(state, ui);
        }
    }
}

fn save_png(path: &Path, image: &egui::ColorImage) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("无法创建目录 {parent:?}: {error}"))?;
    }
    let [width, height] = image.size;
    let rgba = image::RgbaImage::from_raw(
        width as u32,
        height as u32,
        image.as_raw().to_vec(),
    )
    .ok_or_else(|| "截图尺寸与像素缓冲不一致".to_owned())?;
    rgba.save_with_format(path, image::ImageFormat::Png)
        .map_err(|error| format!("无法写入 {path:?}: {error}"))
}
