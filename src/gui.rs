use std::collections::VecDeque;
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver};
use std::thread;

use eframe::egui;
use egui::{Color32, RichText, Stroke};

use crate::clipboard::{Clipboard, ClipboardApi};
use crate::config::{AppConfig, Language, Role};
use crate::protocol::ClipboardPayload;
use crate::server::Edge;

const MAX_LOG_LINES: usize = 500;
const ICON_PNG: &[u8] = include_bytes!("../assets/deskbridge.png");

pub fn run() -> eframe::Result {
    let mut viewport = egui::ViewportBuilder::default()
        .with_title("Deskbridge")
        .with_inner_size([940.0, 700.0])
        .with_min_inner_size([760.0, 620.0]);
    if let Some(icon) = app_icon() {
        viewport = viewport.with_icon(icon);
    }
    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };
    eframe::run_native(
        "Deskbridge",
        options,
        Box::new(|cc| Ok(Box::new(DeskbridgeApp::new(cc)))),
    )
}

struct DeskbridgeApp {
    config: AppConfig,
    service: ServiceProcess,
    status: String,
    icon: Option<egui::TextureHandle>,
}

impl DeskbridgeApp {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        apply_visual_style(&cc.egui_ctx);
        let config = AppConfig::load();
        let icon = load_icon_texture(&cc.egui_ctx);
        Self {
            status: tr(config.language, "ready").to_string(),
            config,
            service: ServiceProcess::default(),
            icon,
        }
    }

    fn start_service(&mut self) {
        if let Err(e) = self.config.save() {
            self.status = format!("{}: {e}", tr(self.config.language, "save_failed"));
        }
        match self.service.start(&self.config) {
            Ok(()) => self.status = tr(self.config.language, "service_started").to_string(),
            Err(e) => self.status = format!("{}: {e}", tr(self.config.language, "start_failed")),
        }
    }

    fn stop_service(&mut self) {
        match self.service.stop() {
            Ok(()) => self.status = tr(self.config.language, "service_stopped").to_string(),
            Err(e) => self.status = format!("{}: {e}", tr(self.config.language, "stop_failed")),
        }
    }

    fn save_config(&mut self) {
        match self.config.save() {
            Ok(()) => self.status = tr(self.config.language, "config_saved").to_string(),
            Err(e) => self.status = format!("{}: {e}", tr(self.config.language, "save_failed")),
        }
    }

    fn handle_dropped_files(&mut self, ctx: &egui::Context) {
        let files = ctx.input(|input| {
            input
                .raw
                .dropped_files
                .iter()
                .filter_map(|file| file.path.clone())
                .collect::<Vec<_>>()
        });
        if files.is_empty() {
            return;
        }
        match publish_files_to_clipboard(&files) {
            Ok(()) => {
                self.status = format!(
                    "{} {}",
                    tr(self.config.language, "queued_files"),
                    files.len()
                );
                self.service
                    .push_log(format!("GUI queued {} dropped file(s)", files.len()));
            }
            Err(e) => self.status = format!("{}: {e}", tr(self.config.language, "drop_failed")),
        }
    }
}

impl eframe::App for DeskbridgeApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.service.collect_logs();
        self.handle_dropped_files(ctx);
        let running = self.service.is_running();

        egui::TopBottomPanel::top("top")
            .frame(panel_frame(Color32::from_rgb(246, 248, 251)))
            .show(ctx, |ui| self.draw_top_bar(ui, running));

        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(Color32::from_rgb(246, 248, 251)))
            .show(ctx, |ui| {
                ui.add_space(4.0);
                self.draw_summary(ui, running);
                ui.add_space(12.0);
                if ui.available_width() > 820.0 {
                    ui.horizontal_top(|ui| {
                        ui.set_width(ui.available_width());
                        ui.vertical(|ui| {
                            ui.set_width(360.0);
                            self.draw_connection(ui);
                            ui.add_space(12.0);
                            self.draw_runtime(ui, running);
                        });
                        ui.add_space(12.0);
                        ui.vertical(|ui| {
                            self.draw_scroll(ui);
                            ui.add_space(12.0);
                            self.draw_drop_zone(ui);
                        });
                    });
                } else {
                    self.draw_connection(ui);
                    ui.add_space(12.0);
                    self.draw_runtime(ui, running);
                    ui.add_space(12.0);
                    self.draw_scroll(ui);
                    ui.add_space(12.0);
                    self.draw_drop_zone(ui);
                }
                ui.add_space(12.0);
                self.draw_logs(ui);
            });

        egui::TopBottomPanel::bottom("status")
            .frame(panel_frame(Color32::from_rgb(241, 244, 248)))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(&self.status).color(Color32::from_rgb(46, 56, 70)));
                    ui.separator();
                    ui.label(
                        RichText::new(crate::config::config_path().display().to_string())
                            .small()
                            .color(Color32::from_rgb(100, 111, 128)),
                    );
                });
            });
    }
}

impl DeskbridgeApp {
    fn draw_top_bar(&mut self, ui: &mut egui::Ui, running: bool) {
        ui.set_height(64.0);
        ui.horizontal_centered(|ui| {
            if let Some(icon) = &self.icon {
                ui.image((icon.id(), egui::vec2(36.0, 36.0)));
            }
            ui.vertical(|ui| {
                ui.label(RichText::new("Deskbridge").size(22.0).strong());
                ui.label(
                    RichText::new(tr(self.config.language, "subtitle"))
                        .size(12.0)
                        .color(Color32::from_rgb(97, 109, 126)),
                );
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                language_switch(ui, &mut self.config.language);
                ui.add_space(10.0);
                status_pill(ui, self.config.language, running);
            });
        });
    }

    fn draw_summary(&mut self, ui: &mut egui::Ui, running: bool) {
        section_frame().show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                metric(
                    ui,
                    tr(self.config.language, "role"),
                    role_label(self.config.language, self.config.role),
                );
                metric(
                    ui,
                    tr(self.config.language, "endpoint"),
                    &self.endpoint_text(),
                );
                metric(
                    ui,
                    tr(self.config.language, "edge"),
                    edge_label(self.config.language, self.config.edge),
                );
                metric(
                    ui,
                    tr(self.config.language, "state"),
                    if running {
                        tr(self.config.language, "running")
                    } else {
                        tr(self.config.language, "stopped")
                    },
                );
            });
        });
    }

    fn draw_connection(&mut self, ui: &mut egui::Ui) {
        section_frame().show(ui, |ui| {
            section_title(ui, tr(self.config.language, "connection"));
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.selectable_value(
                    &mut self.config.role,
                    Role::Client,
                    tr(self.config.language, "client"),
                );
                ui.selectable_value(
                    &mut self.config.role,
                    Role::Server,
                    tr(self.config.language, "server"),
                );
            });
            ui.add_space(12.0);
            match self.config.role {
                Role::Client => {
                    field_label(ui, tr(self.config.language, "server_address"));
                    ui.add_sized(
                        [ui.available_width(), 28.0],
                        egui::TextEdit::singleline(&mut self.config.server),
                    );
                }
                Role::Server => {
                    field_label(ui, tr(self.config.language, "bind_address"));
                    ui.add_sized(
                        [ui.available_width(), 28.0],
                        egui::TextEdit::singleline(&mut self.config.bind),
                    );
                    ui.add_space(10.0);
                    field_label(ui, tr(self.config.language, "mac_edge"));
                    egui::ComboBox::from_id_salt("edge")
                        .width(ui.available_width())
                        .selected_text(edge_label(self.config.language, self.config.edge))
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut self.config.edge,
                                Edge::Left,
                                edge_label(self.config.language, Edge::Left),
                            );
                            ui.selectable_value(
                                &mut self.config.edge,
                                Edge::Right,
                                edge_label(self.config.language, Edge::Right),
                            );
                        });
                }
            }
        });
    }

    fn draw_runtime(&mut self, ui: &mut egui::Ui, running: bool) {
        section_frame().show(ui, |ui| {
            section_title(ui, tr(self.config.language, "runtime"));
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(
                        !running,
                        egui::Button::new(format!("▶ {}", tr(self.config.language, "start"))),
                    )
                    .clicked()
                {
                    self.start_service();
                }
                if ui
                    .add_enabled(
                        running,
                        egui::Button::new(format!("■ {}", tr(self.config.language, "stop"))),
                    )
                    .clicked()
                {
                    self.stop_service();
                }
                if ui
                    .button(format!("✓ {}", tr(self.config.language, "save")))
                    .clicked()
                {
                    self.save_config();
                }
            });
            ui.add_space(12.0);
            field_label(ui, tr(self.config.language, "command"));
            command_box(ui, &self.service.command_preview(&self.config));
        });
    }

    fn draw_scroll(&mut self, ui: &mut egui::Ui) {
        section_frame().show(ui, |ui| {
            section_title(ui, tr(self.config.language, "scroll"));
            ui.add_space(8.0);
            slider_row(
                ui,
                tr(self.config.language, "distance"),
                &mut self.config.scroll_scale,
                0.2..=5.0,
            );
            slider_row(
                ui,
                tr(self.config.language, "response"),
                &mut self.config.scroll_response,
                0.12..=0.75,
            );
            slider_row(
                ui,
                tr(self.config.language, "max_step"),
                &mut self.config.scroll_max_step,
                12.0..=320.0,
            );
            ui.horizontal(|ui| {
                ui.label(tr(self.config.language, "frame_ms"));
                ui.add(
                    egui::Slider::new(&mut self.config.scroll_frame_ms, 4..=24)
                        .show_value(true)
                        .clamping(egui::SliderClamping::Always),
                );
            });
        });
    }

    fn draw_drop_zone(&mut self, ui: &mut egui::Ui) {
        section_frame().show(ui, |ui| {
            section_title(ui, tr(self.config.language, "file_transfer"));
            ui.add_space(8.0);
            let desired_size = egui::vec2(ui.available_width(), 92.0);
            let (rect, response) = ui.allocate_exact_size(desired_size, egui::Sense::hover());
            let fill = if response.hovered() {
                Color32::from_rgb(229, 248, 247)
            } else {
                Color32::from_rgb(238, 244, 250)
            };
            ui.painter().rect(
                rect,
                8.0,
                fill,
                Stroke::new(1.0, Color32::from_rgb(190, 207, 224)),
                egui::StrokeKind::Inside,
            );
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                tr(self.config.language, "drop_hint"),
                egui::TextStyle::Button.resolve(ui.style()),
                Color32::from_rgb(49, 65, 86),
            );
        });
    }

    fn draw_logs(&mut self, ui: &mut egui::Ui) {
        section_frame().show(ui, |ui| {
            ui.horizontal(|ui| {
                section_title(ui, tr(self.config.language, "logs"));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button(tr(self.config.language, "clear")).clicked() {
                        self.service.clear_logs();
                    }
                });
            });
            ui.add_space(8.0);
            egui::ScrollArea::vertical()
                .stick_to_bottom(true)
                .max_height(180.0)
                .show(ui, |ui| {
                    if self.service.logs.is_empty() {
                        ui.label(
                            RichText::new(tr(self.config.language, "no_logs"))
                                .color(Color32::from_rgb(112, 123, 140)),
                        );
                    } else {
                        for line in &self.service.logs {
                            ui.monospace(line);
                        }
                    }
                });
        });
    }

    fn endpoint_text(&self) -> String {
        match self.config.role {
            Role::Client => self.config.server.clone(),
            Role::Server => self.config.bind.clone(),
        }
    }
}

#[derive(Default)]
struct ServiceProcess {
    child: Option<Child>,
    receiver: Option<Receiver<String>>,
    logs: VecDeque<String>,
}

impl ServiceProcess {
    fn is_running(&mut self) -> bool {
        if let Some(child) = self.child.as_mut() {
            match child.try_wait() {
                Ok(Some(status)) => {
                    self.push_log(format!("Service exited: {status}"));
                    self.child = None;
                    false
                }
                Ok(None) => true,
                Err(e) => {
                    self.push_log(format!("Service status failed: {e}"));
                    false
                }
            }
        } else {
            false
        }
    }

    fn start(&mut self, config: &AppConfig) -> std::io::Result<()> {
        if self.is_running() {
            return Ok(());
        }
        let exe = std::env::current_exe()?;
        let mut command = Command::new(exe);
        match config.role {
            Role::Server => {
                command
                    .arg("server")
                    .arg("--bind")
                    .arg(&config.bind)
                    .arg("--edge")
                    .arg(match config.edge {
                        Edge::Left => "left",
                        Edge::Right => "right",
                    });
            }
            Role::Client => {
                command.arg("client").arg("--server").arg(&config.server);
                command
                    .env(
                        "DESKBRIDGE_SCROLL_SCALE",
                        format!("{:.3}", config.scroll_scale),
                    )
                    .env(
                        "DESKBRIDGE_SCROLL_RESPONSE",
                        format!("{:.3}", config.scroll_response),
                    )
                    .env(
                        "DESKBRIDGE_SCROLL_MAX_STEP",
                        format!("{:.1}", config.scroll_max_step),
                    )
                    .env(
                        "DESKBRIDGE_SCROLL_FRAME_MS",
                        config.scroll_frame_ms.to_string(),
                    );
            }
        }
        command.stdout(Stdio::piped()).stderr(Stdio::piped());
        let mut child = command.spawn()?;
        let (sender, receiver) = mpsc::channel();
        if let Some(stdout) = child.stdout.take() {
            spawn_log_reader(stdout, sender.clone());
        }
        if let Some(stderr) = child.stderr.take() {
            spawn_log_reader(stderr, sender);
        }
        self.receiver = Some(receiver);
        self.child = Some(child);
        Ok(())
    }

    fn stop(&mut self) -> std::io::Result<()> {
        if let Some(mut child) = self.child.take() {
            child.kill()?;
            let _ = child.wait();
        }
        Ok(())
    }

    fn command_preview(&self, config: &AppConfig) -> String {
        match config.role {
            Role::Server => format!(
                "deskbridge server --bind {} --edge {}",
                config.bind,
                match config.edge {
                    Edge::Left => "left",
                    Edge::Right => "right",
                }
            ),
            Role::Client => format!("deskbridge client --server {}", config.server),
        }
    }

    fn collect_logs(&mut self) {
        if let Some(receiver) = &self.receiver {
            for line in receiver.try_iter().collect::<Vec<_>>() {
                self.push_log(line);
            }
        }
    }

    fn push_log(&mut self, line: String) {
        if self.logs.len() >= MAX_LOG_LINES {
            self.logs.pop_front();
        }
        self.logs.push_back(line);
    }

    fn clear_logs(&mut self) {
        self.logs.clear();
    }
}

impl Drop for ServiceProcess {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

fn spawn_log_reader<R>(reader: R, sender: mpsc::Sender<String>)
where
    R: std::io::Read + Send + 'static,
{
    thread::spawn(move || {
        let reader = BufReader::new(reader);
        for line in reader.lines().map_while(Result::ok) {
            let _ = sender.send(line);
        }
    });
}

fn publish_files_to_clipboard(files: &[PathBuf]) -> std::io::Result<()> {
    let mut clipboard = Clipboard::new()?;
    clipboard.write(&ClipboardPayload::Files(files.to_vec()))
}

fn app_icon() -> Option<egui::IconData> {
    eframe::icon_data::from_png_bytes(ICON_PNG).ok()
}

fn load_icon_texture(ctx: &egui::Context) -> Option<egui::TextureHandle> {
    app_icon().map(|icon| {
        let image = egui::ColorImage::from_rgba_unmultiplied(
            [icon.width as usize, icon.height as usize],
            &icon.rgba,
        );
        ctx.load_texture("deskbridge-icon", image, egui::TextureOptions::LINEAR)
    })
}

fn apply_visual_style(ctx: &egui::Context) {
    install_system_cjk_font(ctx);

    let mut visuals = egui::Visuals::light();
    visuals.panel_fill = Color32::from_rgb(246, 248, 251);
    visuals.window_fill = Color32::from_rgb(255, 255, 255);
    visuals.widgets.noninteractive.bg_fill = Color32::from_rgb(255, 255, 255);
    visuals.widgets.inactive.bg_fill = Color32::from_rgb(238, 243, 249);
    visuals.widgets.hovered.bg_fill = Color32::from_rgb(225, 241, 249);
    visuals.widgets.active.bg_fill = Color32::from_rgb(210, 232, 245);
    visuals.selection.bg_fill = Color32::from_rgb(21, 163, 184);
    visuals.selection.stroke = Stroke::new(1.0, Color32::from_rgb(16, 122, 151));
    ctx.set_visuals(visuals);

    let mut style = (*ctx.style()).clone();
    style.spacing.item_spacing = egui::vec2(8.0, 8.0);
    style.spacing.button_padding = egui::vec2(12.0, 7.0);
    ctx.set_style(style);
}

fn install_system_cjk_font(ctx: &egui::Context) {
    let Some(bytes) = read_first_existing(CJK_FONT_CANDIDATES) else {
        eprintln!("warning: no CJK UI font found; Chinese text may render as tofu boxes");
        return;
    };

    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert(
        "deskbridge-cjk".to_string(),
        Arc::new(egui::FontData::from_owned(bytes)),
    );
    for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
        fonts
            .families
            .entry(family)
            .or_default()
            .insert(0, "deskbridge-cjk".to_string());
    }
    ctx.set_fonts(fonts);
}

fn read_first_existing(paths: &[&str]) -> Option<Vec<u8>> {
    paths.iter().find_map(|path| fs::read(path).ok())
}

const CJK_FONT_CANDIDATES: &[&str] = &[
    "/System/Library/Fonts/Hiragino Sans GB.ttc",
    "/System/Library/Fonts/STHeiti Medium.ttc",
    "/System/Library/Fonts/STHeiti Light.ttc",
    "/System/Library/Fonts/Supplemental/Songti.ttc",
    "C:\\Windows\\Fonts\\msyh.ttc",
    "C:\\Windows\\Fonts\\msyh.ttf",
    "C:\\Windows\\Fonts\\msyhbd.ttc",
    "C:\\Windows\\Fonts\\simhei.ttf",
    "C:\\Windows\\Fonts\\simsun.ttc",
    "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
    "/usr/share/fonts/truetype/noto/NotoSansCJK-Regular.ttc",
];

fn panel_frame(fill: Color32) -> egui::Frame {
    egui::Frame::new()
        .fill(fill)
        .inner_margin(egui::Margin::symmetric(18, 10))
}

fn section_frame() -> egui::Frame {
    egui::Frame::new()
        .fill(Color32::from_rgb(255, 255, 255))
        .stroke(Stroke::new(1.0, Color32::from_rgb(222, 229, 237)))
        .corner_radius(8)
        .inner_margin(egui::Margin::same(16))
}

fn section_title(ui: &mut egui::Ui, text: &str) {
    ui.label(
        RichText::new(text)
            .size(16.0)
            .strong()
            .color(Color32::from_rgb(28, 38, 52)),
    );
}

fn field_label(ui: &mut egui::Ui, text: &str) {
    ui.label(
        RichText::new(text)
            .size(12.0)
            .color(Color32::from_rgb(91, 103, 120)),
    );
}

fn command_box(ui: &mut egui::Ui, command: &str) {
    egui::Frame::new()
        .fill(Color32::from_rgb(246, 248, 251))
        .stroke(Stroke::new(1.0, Color32::from_rgb(226, 232, 240)))
        .corner_radius(6)
        .inner_margin(egui::Margin::same(8))
        .show(ui, |ui| {
            ui.monospace(command);
        });
}

fn slider_row(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut f64,
    range: std::ops::RangeInclusive<f64>,
) {
    ui.horizontal(|ui| {
        ui.label(label);
        ui.add(
            egui::Slider::new(value, range)
                .show_value(true)
                .clamping(egui::SliderClamping::Always),
        );
    });
}

fn metric(ui: &mut egui::Ui, label: &str, value: &str) {
    ui.vertical(|ui| {
        ui.label(
            RichText::new(label)
                .small()
                .color(Color32::from_rgb(105, 116, 132)),
        );
        ui.label(
            RichText::new(value)
                .strong()
                .color(Color32::from_rgb(32, 42, 57)),
        );
    });
    ui.add_space(24.0);
}

fn status_pill(ui: &mut egui::Ui, language: Language, running: bool) {
    let (text, fill, text_color) = if running {
        (
            tr(language, "running"),
            Color32::from_rgb(221, 245, 235),
            Color32::from_rgb(19, 121, 86),
        )
    } else {
        (
            tr(language, "stopped"),
            Color32::from_rgb(239, 242, 246),
            Color32::from_rgb(88, 99, 116),
        )
    };
    egui::Frame::new()
        .fill(fill)
        .corner_radius(8)
        .inner_margin(egui::Margin::symmetric(10, 5))
        .show(ui, |ui| {
            ui.label(RichText::new(text).strong().color(text_color));
        });
}

fn language_switch(ui: &mut egui::Ui, language: &mut Language) {
    ui.horizontal(|ui| {
        ui.selectable_value(language, Language::Chinese, "中文");
        ui.selectable_value(language, Language::English, "EN");
    });
}

fn role_label(language: Language, role: Role) -> &'static str {
    match role {
        Role::Client => tr(language, "client"),
        Role::Server => tr(language, "server"),
    }
}

fn edge_label(language: Language, edge: Edge) -> &'static str {
    match edge {
        Edge::Left => tr(language, "left"),
        Edge::Right => tr(language, "right"),
    }
}

fn tr(language: Language, key: &str) -> &'static str {
    match language {
        Language::Chinese => match key {
            "ready" => "就绪",
            "subtitle" => "跨设备键鼠、剪贴板与文件投递",
            "running" => "运行中",
            "stopped" => "已停止",
            "role" => "角色",
            "endpoint" => "地址",
            "edge" => "屏幕边缘",
            "state" => "状态",
            "connection" => "连接",
            "client" => "客户端",
            "server" => "服务端",
            "server_address" => "Windows 服务端地址",
            "bind_address" => "监听地址",
            "mac_edge" => "macOS 位于 Windows 的哪一侧",
            "left" => "左侧",
            "right" => "右侧",
            "runtime" => "运行",
            "start" => "启动",
            "stop" => "停止",
            "save" => "保存",
            "command" => "当前命令",
            "scroll" => "macOS 滚轮",
            "distance" => "距离",
            "response" => "响应",
            "max_step" => "最大步长",
            "frame_ms" => "帧间隔",
            "file_transfer" => "文件投递",
            "drop_hint" => "把文件拖到这里，发送到另一台电脑",
            "logs" => "日志",
            "clear" => "清空",
            "no_logs" => "暂无日志",
            "service_started" => "服务已启动",
            "service_stopped" => "服务已停止",
            "config_saved" => "配置已保存",
            "start_failed" => "启动失败",
            "stop_failed" => "停止失败",
            "save_failed" => "保存失败",
            "drop_failed" => "文件投递失败",
            "queued_files" => "已加入投递队列",
            _ => "",
        },
        Language::English => match key {
            "ready" => "Ready",
            "subtitle" => "Keyboard, clipboard, and file bridge",
            "running" => "Running",
            "stopped" => "Stopped",
            "role" => "Role",
            "endpoint" => "Endpoint",
            "edge" => "Screen edge",
            "state" => "State",
            "connection" => "Connection",
            "client" => "Client",
            "server" => "Server",
            "server_address" => "Windows server address",
            "bind_address" => "Bind address",
            "mac_edge" => "macOS screen edge",
            "left" => "Left",
            "right" => "Right",
            "runtime" => "Runtime",
            "start" => "Start",
            "stop" => "Stop",
            "save" => "Save",
            "command" => "Current command",
            "scroll" => "macOS Scroll",
            "distance" => "Distance",
            "response" => "Response",
            "max_step" => "Max step",
            "frame_ms" => "Frame ms",
            "file_transfer" => "File transfer",
            "drop_hint" => "Drop files here to send them to the other computer",
            "logs" => "Logs",
            "clear" => "Clear",
            "no_logs" => "No logs yet",
            "service_started" => "Service started",
            "service_stopped" => "Service stopped",
            "config_saved" => "Config saved",
            "start_failed" => "Failed to start",
            "stop_failed" => "Failed to stop",
            "save_failed" => "Failed to save",
            "drop_failed" => "File drop failed",
            "queued_files" => "Queued file count:",
            _ => "",
        },
    }
}
