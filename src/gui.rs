use std::collections::VecDeque;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::thread;

use eframe::egui;

use crate::clipboard::{Clipboard, ClipboardApi};
use crate::config::{AppConfig, Role};
use crate::protocol::ClipboardPayload;
use crate::server::Edge;

const MAX_LOG_LINES: usize = 500;

pub fn run() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Deskbridge")
            .with_inner_size([760.0, 640.0])
            .with_min_inner_size([620.0, 520.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Deskbridge",
        options,
        Box::new(|_cc| Ok(Box::new(DeskbridgeApp::new()))),
    )
}

struct DeskbridgeApp {
    config: AppConfig,
    service: ServiceProcess,
    status: String,
}

impl DeskbridgeApp {
    fn new() -> Self {
        Self {
            config: AppConfig::load(),
            service: ServiceProcess::default(),
            status: "Ready".to_string(),
        }
    }

    fn start_service(&mut self) {
        match self.config.save() {
            Ok(()) => {}
            Err(e) => self.status = format!("Failed to save config: {e}"),
        }
        match self.service.start(&self.config) {
            Ok(()) => self.status = "Service started".to_string(),
            Err(e) => self.status = format!("Failed to start: {e}"),
        }
    }

    fn stop_service(&mut self) {
        match self.service.stop() {
            Ok(()) => self.status = "Service stopped".to_string(),
            Err(e) => self.status = format!("Failed to stop: {e}"),
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
                self.status = format!("Queued {} file(s) for transfer", files.len());
                self.service
                    .push_log(format!("GUI queued {} dropped file(s)", files.len()));
            }
            Err(e) => self.status = format!("File drop failed: {e}"),
        }
    }
}

impl eframe::App for DeskbridgeApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.service.collect_logs();
        self.handle_dropped_files(ctx);

        egui::TopBottomPanel::top("top").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.heading("Deskbridge");
                ui.separator();
                ui.label(if self.service.is_running() {
                    "Running"
                } else {
                    "Stopped"
                });
            });
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.columns(2, |columns| {
                self.draw_settings(&mut columns[0]);
                self.draw_runtime(&mut columns[1]);
            });
            ui.add_space(12.0);
            self.draw_drop_zone(ui);
            ui.add_space(12.0);
            self.draw_logs(ui);
        });

        egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label(&self.status);
                ui.separator();
                ui.monospace(crate::config::config_path().display().to_string());
            });
        });
    }
}

impl DeskbridgeApp {
    fn draw_settings(&mut self, ui: &mut egui::Ui) {
        ui.heading("Connection");
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.config.role, Role::Client, "Client");
            ui.selectable_value(&mut self.config.role, Role::Server, "Server");
        });
        match self.config.role {
            Role::Client => {
                ui.label("Windows server address");
                ui.text_edit_singleline(&mut self.config.server);
            }
            Role::Server => {
                ui.label("Bind address");
                ui.text_edit_singleline(&mut self.config.bind);
                ui.label("macOS screen edge");
                egui::ComboBox::from_id_salt("edge")
                    .selected_text(match self.config.edge {
                        Edge::Left => "left",
                        Edge::Right => "right",
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.config.edge, Edge::Left, "left");
                        ui.selectable_value(&mut self.config.edge, Edge::Right, "right");
                    });
            }
        }

        ui.add_space(16.0);
        ui.heading("macOS Scroll");
        ui.add(
            egui::Slider::new(&mut self.config.scroll_scale, 0.2..=5.0)
                .text("distance")
                .clamping(egui::SliderClamping::Always),
        );
        ui.add(
            egui::Slider::new(&mut self.config.scroll_response, 0.12..=0.75)
                .text("response")
                .clamping(egui::SliderClamping::Always),
        );
        ui.add(
            egui::Slider::new(&mut self.config.scroll_max_step, 12.0..=320.0)
                .text("max step")
                .clamping(egui::SliderClamping::Always),
        );
        ui.add(egui::Slider::new(&mut self.config.scroll_frame_ms, 4..=24).text("frame ms"));
    }

    fn draw_runtime(&mut self, ui: &mut egui::Ui) {
        ui.heading("Runtime");
        ui.horizontal(|ui| {
            if ui
                .add_enabled(!self.service.is_running(), egui::Button::new("Start"))
                .clicked()
            {
                self.start_service();
            }
            if ui
                .add_enabled(self.service.is_running(), egui::Button::new("Stop"))
                .clicked()
            {
                self.stop_service();
            }
            if ui.button("Save").clicked() {
                match self.config.save() {
                    Ok(()) => self.status = "Config saved".to_string(),
                    Err(e) => self.status = format!("Save failed: {e}"),
                }
            }
        });
        ui.add_space(8.0);
        ui.label("Current command");
        ui.monospace(self.service.command_preview(&self.config));
        ui.add_space(8.0);
        ui.label("File transfer");
        ui.label(
            "Drop files anywhere on this window to queue them through the local clipboard watcher.",
        );
        ui.label("On Windows server, an edge strip also accepts Explorer drops.");
    }

    fn draw_drop_zone(&mut self, ui: &mut egui::Ui) {
        let desired_size = egui::vec2(ui.available_width(), 84.0);
        let (rect, response) = ui.allocate_exact_size(desired_size, egui::Sense::hover());
        let visuals = ui.style().interact(&response);
        let painter = ui.painter_at(rect);
        painter.rect(
            rect,
            8.0,
            visuals.bg_fill,
            egui::Stroke::new(1.0, visuals.bg_stroke.color),
            egui::StrokeKind::Inside,
        );
        painter.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            "Drop files here to send them to the other computer",
            egui::TextStyle::Button.resolve(ui.style()),
            visuals.text_color(),
        );
    }

    fn draw_logs(&mut self, ui: &mut egui::Ui) {
        ui.heading("Logs");
        egui::ScrollArea::vertical()
            .stick_to_bottom(true)
            .max_height(250.0)
            .show(ui, |ui| {
                for line in &self.service.logs {
                    ui.monospace(line);
                }
            });
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
