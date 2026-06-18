use std::collections::VecDeque;
#[cfg(target_os = "macos")]
use std::ffi::CStr;
use std::fs;
use std::io::{BufRead, BufReader};
#[cfg(target_os = "macos")]
use std::os::raw::c_char;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;

use eframe::egui;
use egui::{Color32, RichText, Stroke};

use crate::clipboard::{Clipboard, ClipboardApi};
use crate::config::{AppConfig, Language, ModifierTarget, Role};
use crate::protocol::ClipboardPayload;
use crate::server::Edge;

const MAX_LOG_LINES: usize = 500;
const ICON_PNG: &[u8] = include_bytes!("../assets/deskbridge.png");
const STATUS_ICON_PNG: &[u8] = include_bytes!("../assets/deskbridge-status.png");
const BG: Color32 = Color32::from_rgb(246, 248, 251);
const SIDEBAR_BG: Color32 = Color32::from_rgb(232, 238, 243);
const CARD: Color32 = Color32::from_rgb(255, 255, 255);
const BORDER: Color32 = Color32::from_rgb(221, 230, 240);
const TEXT: Color32 = Color32::from_rgb(23, 32, 51);
const MUTED: Color32 = Color32::from_rgb(100, 116, 139);
const PRIMARY: Color32 = Color32::from_rgb(6, 182, 212);
const PRIMARY_DARK: Color32 = Color32::from_rgb(8, 145, 178);
const SUCCESS: Color32 = Color32::from_rgb(34, 197, 94);
const STOPPED: Color32 = Color32::from_rgb(148, 163, 184);
const WARNING: Color32 = Color32::from_rgb(245, 158, 11);
const ERROR: Color32 = Color32::from_rgb(239, 68, 68);

#[cfg(target_os = "macos")]
unsafe extern "C" {
    fn deskbridge_install_status_item(png_bytes: *const u8, png_len: usize);
}

pub fn run() -> eframe::Result {
    let mut viewport = egui::ViewportBuilder::default()
        .with_title("Deskbridge")
        .with_inner_size([1180.0, 720.0])
        .with_min_inner_size([960.0, 640.0]);
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
    status_commands: Receiver<StatusMenuCommand>,
    status: String,
    icon: Option<egui::TextureHandle>,
    show_settings: bool,
}

#[derive(Clone, Copy)]
enum StatusMenuCommand {
    ToggleRun,
    Quit,
}

static STATUS_MENU_SENDER: OnceLock<Mutex<Option<Sender<StatusMenuCommand>>>> = OnceLock::new();

fn register_status_menu_sender(sender: Sender<StatusMenuCommand>) {
    let slot = STATUS_MENU_SENDER.get_or_init(|| Mutex::new(None));
    if let Ok(mut guard) = slot.lock() {
        *guard = Some(sender);
    }
}

#[cfg(target_os = "macos")]
#[unsafe(no_mangle)]
pub extern "C" fn deskbridge_handle_status_menu_command(command: *const c_char) {
    if command.is_null() {
        return;
    }
    let Ok(command) = (unsafe { CStr::from_ptr(command) }).to_str() else {
        return;
    };
    let Some(event) = (match command {
        "toggle-run" => Some(StatusMenuCommand::ToggleRun),
        "quit" => Some(StatusMenuCommand::Quit),
        _ => None,
    }) else {
        return;
    };
    if let Some(slot) = STATUS_MENU_SENDER.get() {
        if let Ok(guard) = slot.lock() {
            if let Some(sender) = guard.as_ref() {
                let _ = sender.send(event);
            }
        }
    }
}

impl DeskbridgeApp {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        apply_visual_style(&cc.egui_ctx);
        let (status_sender, status_commands) = mpsc::channel();
        register_status_menu_sender(status_sender);
        install_macos_status_item();
        let config = AppConfig::load();
        let icon = load_icon_texture(&cc.egui_ctx);
        Self {
            status: tr(config.language, "ready").to_string(),
            config,
            service: ServiceProcess::default(),
            status_commands,
            icon,
            show_settings: false,
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

    fn handle_status_menu_commands(&mut self, ctx: &egui::Context, mut running: bool) -> bool {
        while let Ok(command) = self.status_commands.try_recv() {
            match command {
                StatusMenuCommand::ToggleRun => {
                    if running {
                        self.stop_service();
                        running = false;
                    } else {
                        self.start_service();
                        running = self.service.is_running();
                    }
                }
                StatusMenuCommand::Quit => {
                    let _ = self.service.stop();
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
        }
        running
    }
}

impl eframe::App for DeskbridgeApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.service.collect_logs();
        self.handle_dropped_files(ctx);
        let running = self.service.is_running();
        let running = self.handle_status_menu_commands(ctx, running);

        egui::TopBottomPanel::top("top")
            .frame(panel_frame(CARD))
            .show(ctx, |ui| self.draw_top_bar(ui, running));

        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(CARD))
            .show(ctx, |ui| {
                if ui.available_width() > 980.0 {
                    ui.horizontal_top(|ui| {
                        let height = ui.available_height();
                        ui.vertical(|ui| {
                            ui.set_width(304.0);
                            ui.set_min_height(height);
                            egui::ScrollArea::vertical()
                                .id_salt("config_pane_scroll")
                                .auto_shrink([false, false])
                                .show(ui, |ui| {
                                    self.draw_config_pane(ui);
                                });
                        });
                        vertical_rule(ui, height);
                        ui.vertical(|ui| {
                            ui.set_width((ui.available_width() - 334.0).max(360.0));
                            ui.set_min_height(height);
                            egui::ScrollArea::vertical()
                                .id_salt("status_pane_scroll")
                                .auto_shrink([false, false])
                                .show(ui, |ui| {
                                    self.draw_status_pane(ui, running);
                                });
                        });
                        vertical_rule(ui, height);
                        ui.vertical(|ui| {
                            ui.set_width(320.0);
                            ui.set_min_height(height);
                            self.draw_log_pane(ui, height);
                        });
                    });
                } else {
                    egui::ScrollArea::vertical()
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            self.draw_config_pane(ui);
                            ui.add_space(12.0);
                            self.draw_status_pane(ui, running);
                            ui.add_space(12.0);
                            self.draw_log_pane(ui, 360.0);
                        });
                }
            });
        self.draw_settings_window(ctx, running);
    }
}

impl DeskbridgeApp {
    fn draw_top_bar(&mut self, ui: &mut egui::Ui, running: bool) {
        ui.set_height(84.0);
        ui.horizontal_centered(|ui| {
            if let Some(icon) = &self.icon {
                ui.image((icon.id(), egui::vec2(42.0, 42.0)));
            }
            ui.add_space(16.0);
            if run_switch(ui, running).clicked() {
                if running {
                    self.stop_service();
                } else {
                    self.start_service();
                }
            }
            ui.add_space(16.0);
            ui.vertical(|ui| {
                ui.label(RichText::new("Deskbridge").size(24.0).strong().color(TEXT));
                ui.horizontal(|ui| {
                    status_dot(ui, running);
                    ui.label(
                        RichText::new(if running {
                            tr(self.config.language, "connected")
                        } else {
                            tr(self.config.language, "disconnected")
                        })
                        .size(14.0)
                        .color(MUTED),
                    );
                });
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                language_switch(ui, &mut self.config.language);
                ui.add_space(12.0);
                if secondary_button(ui, tr(self.config.language, "save_config"), true).clicked() {
                    self.save_config();
                }
                ui.add_space(12.0);
                status_pill(ui, self.config.language, running);
            });
        });
    }

    fn draw_config_pane(&mut self, ui: &mut egui::Ui) {
        pane_frame(SIDEBAR_BG).show(ui, |ui| {
            pane_title(ui, tr(self.config.language, "parameters"));
            ui.add_space(14.0);
            selected_sidebar_row(ui, "⌘", tr(self.config.language, "connection"));
            ui.add_space(14.0);

            subsection(ui, tr(self.config.language, "role"));
            segmented_role(ui, self.config.language, &mut self.config.role);
            ui.add_space(14.0);

            match self.config.role {
                Role::Client => {
                    field_label(ui, tr(self.config.language, "server_address"));
                    ui.add_sized(
                        [ui.available_width(), 30.0],
                        egui::TextEdit::singleline(&mut self.config.server),
                    );
                }
                Role::Server => {
                    field_label(ui, tr(self.config.language, "bind_address"));
                    ui.add_sized(
                        [ui.available_width(), 30.0],
                        egui::TextEdit::singleline(&mut self.config.bind),
                    );
                    ui.add_space(12.0);
                    field_label(ui, tr(self.config.language, "mac_edge"));
                    egui::ComboBox::from_id_salt("edge-pane")
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

            ui.add_space(20.0);
            subsection(ui, tr(self.config.language, "scroll"));
            compact_slider(
                ui,
                tr(self.config.language, "distance"),
                &mut self.config.scroll_scale,
                0.2..=5.0,
            );
            compact_slider(
                ui,
                tr(self.config.language, "response"),
                &mut self.config.scroll_response,
                0.12..=0.75,
            );
            compact_slider(
                ui,
                tr(self.config.language, "max_step"),
                &mut self.config.scroll_max_step,
                12.0..=320.0,
            );
            ui.horizontal(|ui| {
                ui.label(RichText::new(tr(self.config.language, "frame_ms")).color(MUTED));
                ui.add(
                    egui::Slider::new(&mut self.config.scroll_frame_ms, 4..=24)
                        .show_value(true)
                        .clamping(egui::SliderClamping::Always),
                );
            });

            ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
                ui.label(
                    RichText::new(crate::config::config_path().display().to_string())
                        .small()
                        .color(MUTED),
                );
            });
        });
    }

    fn draw_status_pane(&mut self, ui: &mut egui::Ui, running: bool) {
        pane_frame(CARD).show(ui, |ui| {
            ui.horizontal(|ui| {
                pane_title(ui, tr(self.config.language, "diagnostics"));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if running {
                        if secondary_button(ui, tr(self.config.language, "stop_connection"), true)
                            .clicked()
                        {
                            self.stop_service();
                        }
                    } else if primary_button(ui, tr(self.config.language, "start_connection"), true)
                        .clicked()
                    {
                        self.start_service();
                    }
                    ui.add_space(8.0);
                    if secondary_button(ui, tr(self.config.language, "settings"), true).clicked() {
                        self.show_settings = true;
                    }
                });
            });
            ui.add_space(14.0);

            status_row(
                ui,
                tr(self.config.language, "service_state"),
                if running {
                    tr(self.config.language, "connected")
                } else {
                    tr(self.config.language, "disconnected")
                },
                running,
            );
            status_row(
                ui,
                tr(self.config.language, "active_role"),
                role_label(self.config.language, self.config.role),
                true,
            );
            status_row(
                ui,
                tr(self.config.language, "endpoint"),
                &self.endpoint_text(),
                true,
            );
            status_row(
                ui,
                tr(self.config.language, "input_bridge"),
                tr(self.config.language, "ready"),
                true,
            );
            status_row(
                ui,
                tr(self.config.language, "clipboard_bridge"),
                tr(self.config.language, "ready"),
                true,
            );

            ui.add_space(18.0);
            subsection(ui, tr(self.config.language, "command"));
            command_box(ui, &self.service.command_preview(&self.config));
            ui.add_space(16.0);

            subsection(ui, tr(self.config.language, "file_transfer"));
            ui.label(RichText::new(tr(self.config.language, "drop_intro")).color(MUTED));
            ui.add_space(8.0);
            self.draw_drop_zone_inline(ui);
            ui.add_space(12.0);
            ui.label(RichText::new(&self.status).color(TEXT));
        });
    }

    fn draw_settings_window(&mut self, ctx: &egui::Context, running: bool) {
        if !self.show_settings {
            return;
        }

        let mut open = self.show_settings;
        let mut save_clicked = false;
        let mut close_clicked = false;
        egui::Window::new(tr(self.config.language, "settings"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(430.0)
            .show(ctx, |ui| {
                ui.label(RichText::new(tr(self.config.language, "modifier_mapping")).color(TEXT));
                ui.add_space(4.0);
                ui.label(
                    RichText::new(tr(self.config.language, "modifier_mapping_hint")).color(MUTED),
                );
                ui.add_space(14.0);

                modifier_mapping_row(
                    ui,
                    self.config.language,
                    tr(self.config.language, "mac_command_key"),
                    &mut self.config.mac_command_mapping,
                );
                modifier_mapping_row(
                    ui,
                    self.config.language,
                    tr(self.config.language, "mac_control_key"),
                    &mut self.config.mac_control_mapping,
                );
                modifier_mapping_row(
                    ui,
                    self.config.language,
                    tr(self.config.language, "mac_option_key"),
                    &mut self.config.mac_option_mapping,
                );

                if running {
                    ui.add_space(10.0);
                    ui.label(
                        RichText::new(tr(self.config.language, "restart_required")).color(WARNING),
                    );
                }

                ui.add_space(16.0);
                ui.horizontal(|ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if primary_button(ui, tr(self.config.language, "save_settings"), true)
                            .clicked()
                        {
                            save_clicked = true;
                        }
                        ui.add_space(8.0);
                        if secondary_button(ui, tr(self.config.language, "close"), true).clicked() {
                            close_clicked = true;
                        }
                    });
                });
            });

        if save_clicked {
            match self.config.save() {
                Ok(()) => {
                    self.status = if running {
                        tr(self.config.language, "config_saved_restart").to_string()
                    } else {
                        tr(self.config.language, "config_saved").to_string()
                    };
                    open = false;
                }
                Err(e) => {
                    self.status = format!("{}: {e}", tr(self.config.language, "save_failed"));
                }
            }
        }
        if close_clicked {
            open = false;
        }
        self.show_settings = open;
    }

    fn draw_drop_zone_inline(&mut self, ui: &mut egui::Ui) {
        let desired_size = egui::vec2(ui.available_width(), 150.0);
        let (rect, response) = ui.allocate_exact_size(desired_size, egui::Sense::hover());
        let hovered_files = ui.ctx().input(|input| !input.raw.hovered_files.is_empty());
        let fill = if response.hovered() || hovered_files {
            Color32::from_rgb(224, 252, 255)
        } else {
            Color32::from_rgb(234, 246, 255)
        };
        ui.painter()
            .rect_filled(rect, egui::CornerRadius::same(14), fill);
        paint_dashed_rect(ui, rect.shrink(1.0), 14.0, Stroke::new(1.5, PRIMARY));
        ui.painter().text(
            rect.center() - egui::vec2(0.0, 16.0),
            egui::Align2::CENTER_CENTER,
            tr(self.config.language, "drop_zone_title"),
            egui::TextStyle::Heading.resolve(ui.style()),
            PRIMARY_DARK,
        );
        ui.painter().text(
            rect.center() + egui::vec2(0.0, 18.0),
            egui::Align2::CENTER_CENTER,
            tr(self.config.language, "drop_hint"),
            egui::TextStyle::Button.resolve(ui.style()),
            MUTED,
        );
    }

    fn draw_log_pane(&mut self, ui: &mut egui::Ui, height: f32) {
        pane_frame(Color32::from_rgb(250, 252, 254)).show(ui, |ui| {
            ui.horizontal(|ui| {
                pane_title(ui, tr(self.config.language, "logs"));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if secondary_button(ui, tr(self.config.language, "clear"), true).clicked() {
                        self.service.clear_logs();
                    }
                });
            });
            ui.add_space(12.0);
            egui::Frame::new()
                .fill(Color32::from_rgb(248, 250, 252))
                .stroke(Stroke::new(1.0, BORDER))
                .corner_radius(12)
                .inner_margin(egui::Margin::same(12))
                .show(ui, |ui| {
                    egui::ScrollArea::vertical()
                        .stick_to_bottom(true)
                        .max_height((height - 116.0).max(260.0))
                        .show(ui, |ui| {
                            if self.service.logs.is_empty() {
                                ui.label(
                                    RichText::new(tr(self.config.language, "no_logs")).color(MUTED),
                                );
                            } else {
                                for line in &self.service.logs {
                                    ui.monospace(
                                        RichText::new(line).color(Color32::from_rgb(51, 65, 85)),
                                    );
                                }
                            }
                        });
                });
        });
    }

    fn endpoint_text(&self) -> String {
        match self.config.role {
            Role::Client => self.config.server.clone(),
            Role::Server => self.config.bind.clone(),
        }
    }

    fn draw_connection(&mut self, ui: &mut egui::Ui) {
        card_frame().show(ui, |ui| {
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
        card_frame().show(ui, |ui| {
            section_title(ui, tr(self.config.language, "runtime"));
            ui.add_space(8.0);
            ui.label(RichText::new(runtime_hint(self.config.language, running)).color(MUTED));
            ui.add_space(12.0);
            ui.horizontal_wrapped(|ui| {
                if secondary_button(ui, tr(self.config.language, "save_config"), true).clicked() {
                    self.save_config();
                }
                ui.add_space(6.0);
                status_pill(ui, self.config.language, running);
            });
            ui.add_space(12.0);
            field_label(ui, tr(self.config.language, "command"));
            command_box(ui, &self.service.command_preview(&self.config));
        });
    }

    fn draw_scroll(&mut self, ui: &mut egui::Ui) {
        card_frame().show(ui, |ui| {
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
        card_frame().show(ui, |ui| {
            section_title(ui, tr(self.config.language, "file_transfer"));
            ui.add_space(8.0);
            ui.label(RichText::new(tr(self.config.language, "drop_intro")).color(MUTED));
            ui.add_space(8.0);
            ui.label(RichText::new(tr(self.config.language, "drop_after")).color(MUTED));
            ui.add_space(14.0);
            let desired_size = egui::vec2(ui.available_width(), 220.0);
            let (rect, response) = ui.allocate_exact_size(desired_size, egui::Sense::hover());
            let hovered_files = ui.ctx().input(|input| !input.raw.hovered_files.is_empty());
            let fill = if response.hovered() || hovered_files {
                Color32::from_rgb(224, 247, 255)
            } else {
                Color32::from_rgb(239, 249, 255)
            };
            ui.painter()
                .rect_filled(rect, egui::CornerRadius::same(14), fill);
            paint_dashed_rect(
                ui,
                rect.shrink(1.0),
                14.0,
                Stroke::new(1.5, Color32::from_rgb(14, 165, 233)),
            );
            ui.painter().text(
                rect.center() - egui::vec2(0.0, 18.0),
                egui::Align2::CENTER_CENTER,
                tr(self.config.language, "drop_zone_title"),
                egui::TextStyle::Heading.resolve(ui.style()),
                PRIMARY_DARK,
            );
            ui.painter().text(
                rect.center() + egui::vec2(0.0, 18.0),
                egui::Align2::CENTER_CENTER,
                tr(self.config.language, "drop_hint"),
                egui::TextStyle::Button.resolve(ui.style()),
                MUTED,
            );
        });
    }

    fn draw_logs(&mut self, ui: &mut egui::Ui, max_height: f32) {
        card_frame().show(ui, |ui| {
            ui.horizontal(|ui| {
                section_title(ui, tr(self.config.language, "logs"));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if secondary_button(ui, tr(self.config.language, "clear"), true).clicked() {
                        self.service.clear_logs();
                    }
                });
            });
            ui.add_space(8.0);
            egui::Frame::new()
                .fill(Color32::from_rgb(248, 250, 252))
                .stroke(Stroke::new(1.0, BORDER))
                .corner_radius(10)
                .inner_margin(egui::Margin::same(12))
                .show(ui, |ui| {
                    egui::ScrollArea::vertical()
                        .stick_to_bottom(true)
                        .max_height(max_height)
                        .show(ui, |ui| {
                            if self.service.logs.is_empty() {
                                ui.label(
                                    RichText::new(tr(self.config.language, "no_logs")).color(MUTED),
                                );
                            } else {
                                for line in &self.service.logs {
                                    ui.monospace(
                                        RichText::new(line).color(Color32::from_rgb(51, 65, 85)),
                                    );
                                }
                            }
                        });
                });
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
                command
                    .env(
                        "DESKBRIDGE_MAC_COMMAND_MAPPING",
                        config.mac_command_mapping.as_str(),
                    )
                    .env(
                        "DESKBRIDGE_MAC_CONTROL_MAPPING",
                        config.mac_control_mapping.as_str(),
                    )
                    .env(
                        "DESKBRIDGE_MAC_OPTION_MAPPING",
                        config.mac_option_mapping.as_str(),
                    );
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

#[cfg(target_os = "macos")]
fn install_macos_status_item() {
    unsafe {
        deskbridge_install_status_item(STATUS_ICON_PNG.as_ptr(), STATUS_ICON_PNG.len());
    }
}

#[cfg(not(target_os = "macos"))]
fn install_macos_status_item() {}

fn apply_visual_style(ctx: &egui::Context) {
    install_system_cjk_font(ctx);

    let mut visuals = egui::Visuals::light();
    visuals.panel_fill = BG;
    visuals.window_fill = CARD;
    visuals.widgets.noninteractive.bg_fill = CARD;
    visuals.widgets.inactive.bg_fill = Color32::from_rgb(238, 243, 249);
    visuals.widgets.hovered.bg_fill = Color32::from_rgb(225, 241, 249);
    visuals.widgets.active.bg_fill = Color32::from_rgb(210, 232, 245);
    visuals.selection.bg_fill = PRIMARY;
    visuals.selection.stroke = Stroke::new(1.0, PRIMARY_DARK);
    ctx.set_visuals(visuals);

    let mut style = (*ctx.style()).clone();
    style.spacing.item_spacing = egui::vec2(8.0, 7.0);
    style.spacing.button_padding = egui::vec2(13.0, 7.0);
    style.spacing.interact_size = egui::vec2(40.0, 30.0);
    style.visuals.window_corner_radius = 12.into();
    style.visuals.widgets.inactive.corner_radius = 8.into();
    style.visuals.widgets.hovered.corner_radius = 8.into();
    style.visuals.widgets.active.corner_radius = 8.into();
    style.text_styles.insert(
        egui::TextStyle::Heading,
        egui::FontId::new(24.0, egui::FontFamily::Proportional),
    );
    style.text_styles.insert(
        egui::TextStyle::Body,
        egui::FontId::new(14.0, egui::FontFamily::Proportional),
    );
    style.text_styles.insert(
        egui::TextStyle::Button,
        egui::FontId::new(14.0, egui::FontFamily::Proportional),
    );
    style.text_styles.insert(
        egui::TextStyle::Small,
        egui::FontId::new(12.0, egui::FontFamily::Proportional),
    );
    style.text_styles.insert(
        egui::TextStyle::Monospace,
        egui::FontId::new(12.0, egui::FontFamily::Monospace),
    );
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
    "/System/Library/Fonts/PingFang.ttc",
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

fn pane_frame(fill: Color32) -> egui::Frame {
    egui::Frame::new()
        .fill(fill)
        .inner_margin(egui::Margin::same(18))
}

fn vertical_rule(ui: &mut egui::Ui, height: f32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(1.0, height), egui::Sense::hover());
    ui.painter().line_segment(
        [rect.center_top(), rect.center_bottom()],
        Stroke::new(1.0, BORDER),
    );
}

fn card_shadow() -> egui::epaint::Shadow {
    egui::epaint::Shadow {
        offset: [0, 8],
        blur: 24,
        spread: 0,
        color: Color32::from_black_alpha(18),
    }
}

fn card_frame() -> egui::Frame {
    egui::Frame::new()
        .fill(CARD)
        .stroke(Stroke::new(1.0, BORDER))
        .corner_radius(14)
        .shadow(card_shadow())
        .inner_margin(egui::Margin::same(18))
}

fn section_title(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text).size(16.0).strong().color(TEXT));
}

fn pane_title(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text).size(23.0).strong().color(TEXT));
}

fn subsection(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text).size(12.0).strong().color(MUTED));
}

fn selected_sidebar_row(ui: &mut egui::Ui, glyph: &str, text: &str) {
    egui::Frame::new()
        .fill(Color32::from_rgb(210, 218, 226))
        .corner_radius(8)
        .inner_margin(egui::Margin::symmetric(12, 9))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(glyph).size(17.0).color(PRIMARY_DARK));
                ui.label(RichText::new(text).size(16.0).strong().color(TEXT));
            });
        });
}

fn field_label(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text).size(12.0).color(MUTED));
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

fn brand_accent(ui: &mut egui::Ui, width: f32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, 4.0), egui::Sense::hover());
    let colors = [
        Color32::from_rgb(34, 197, 94),
        Color32::from_rgb(6, 182, 212),
        PRIMARY_DARK,
        Color32::from_rgb(124, 58, 237),
    ];
    let segment_width = rect.width() / colors.len() as f32;
    for (index, color) in colors.iter().enumerate() {
        let x0 = rect.left() + segment_width * index as f32;
        let x1 = if index == colors.len() - 1 {
            rect.right()
        } else {
            x0 + segment_width + 0.5
        };
        let segment =
            egui::Rect::from_min_max(egui::pos2(x0, rect.top()), egui::pos2(x1, rect.bottom()));
        ui.painter()
            .rect_filled(segment, egui::CornerRadius::same(2), *color);
    }
}

fn primary_button(ui: &mut egui::Ui, label: &str, enabled: bool) -> egui::Response {
    ui.add_enabled(
        enabled,
        egui::Button::new(RichText::new(label).strong().color(Color32::WHITE))
            .fill(PRIMARY)
            .stroke(Stroke::new(1.0, PRIMARY_DARK))
            .corner_radius(9)
            .min_size(egui::vec2(116.0, 34.0)),
    )
}

fn secondary_button(ui: &mut egui::Ui, label: &str, enabled: bool) -> egui::Response {
    ui.add_enabled(
        enabled,
        egui::Button::new(RichText::new(label).color(TEXT))
            .fill(Color32::from_rgb(248, 250, 252))
            .stroke(Stroke::new(1.0, BORDER))
            .corner_radius(9)
            .min_size(egui::vec2(88.0, 32.0)),
    )
}

fn run_switch(ui: &mut egui::Ui, running: bool) -> egui::Response {
    let size = egui::vec2(72.0, 36.0);
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
    let fill = if running {
        PRIMARY
    } else {
        Color32::from_rgb(203, 213, 225)
    };
    let stroke = if response.hovered() {
        Stroke::new(1.0, PRIMARY_DARK)
    } else {
        Stroke::NONE
    };
    ui.painter()
        .rect(rect, 18.0, fill, stroke, egui::StrokeKind::Inside);
    let knob_radius = 14.0;
    let knob_x = if running {
        rect.right() - 18.0
    } else {
        rect.left() + 18.0
    };
    ui.painter().circle_filled(
        egui::pos2(knob_x, rect.center().y),
        knob_radius,
        Color32::WHITE,
    );
    response
}

fn status_dot(ui: &mut egui::Ui, ok: bool) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(12.0, 12.0), egui::Sense::hover());
    ui.painter()
        .circle_filled(rect.center(), 5.0, if ok { SUCCESS } else { STOPPED });
}

fn segmented_role(ui: &mut egui::Ui, language: Language, role: &mut Role) {
    ui.horizontal(|ui| {
        for candidate in [Role::Client, Role::Server] {
            let selected = *role == candidate;
            let text = role_label(language, candidate);
            let response = ui.add(
                egui::Button::new(RichText::new(text).color(if selected {
                    Color32::WHITE
                } else {
                    TEXT
                }))
                .fill(if selected {
                    PRIMARY
                } else {
                    Color32::from_rgb(246, 248, 251)
                })
                .stroke(Stroke::new(
                    1.0,
                    if selected { PRIMARY_DARK } else { BORDER },
                ))
                .corner_radius(8)
                .min_size(egui::vec2(96.0, 32.0)),
            );
            if response.clicked() {
                *role = candidate;
            }
        }
    });
}

fn compact_slider(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut f64,
    range: std::ops::RangeInclusive<f64>,
) {
    ui.vertical(|ui| {
        ui.horizontal(|ui| {
            ui.label(RichText::new(label).size(13.0).color(MUTED));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(
                    RichText::new(format!("{value:.2}"))
                        .monospace()
                        .size(12.0)
                        .color(TEXT),
                );
            });
        });
        ui.add(
            egui::Slider::new(value, range)
                .show_value(false)
                .clamping(egui::SliderClamping::Always),
        );
    });
}

fn modifier_mapping_row(
    ui: &mut egui::Ui,
    language: Language,
    label: &str,
    target: &mut ModifierTarget,
) {
    ui.horizontal(|ui| {
        ui.set_min_height(34.0);
        ui.label(RichText::new(label).color(TEXT));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            egui::ComboBox::from_id_salt(label)
                .width(150.0)
                .selected_text(modifier_target_label(language, *target))
                .show_ui(ui, |ui| {
                    for candidate in [
                        ModifierTarget::Control,
                        ModifierTarget::Meta,
                        ModifierTarget::Alt,
                        ModifierTarget::Disabled,
                    ] {
                        ui.selectable_value(
                            target,
                            candidate,
                            modifier_target_label(language, candidate),
                        );
                    }
                });
        });
    });
    ui.add_space(6.0);
}

fn modifier_target_label(language: Language, target: ModifierTarget) -> &'static str {
    match target {
        ModifierTarget::Control => tr(language, "map_to_control"),
        ModifierTarget::Meta => tr(language, "map_to_win"),
        ModifierTarget::Alt => tr(language, "map_to_alt"),
        ModifierTarget::Disabled => tr(language, "map_disabled"),
    }
}

fn status_row(ui: &mut egui::Ui, label: &str, value: &str, ok: bool) {
    egui::Frame::new()
        .fill(Color32::from_rgb(248, 250, 252))
        .corner_radius(9)
        .inner_margin(egui::Margin::symmetric(12, 8))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                status_dot(ui, ok);
                ui.vertical(|ui| {
                    ui.label(RichText::new(label).size(12.0).color(MUTED));
                    ui.label(RichText::new(value).size(15.0).strong().color(TEXT));
                });
            });
        });
    ui.add_space(6.0);
}

fn runtime_hint(language: Language, running: bool) -> &'static str {
    match (language, running) {
        (Language::Chinese, true) => "后台服务正在运行，可以直接控制另一台设备。",
        (Language::Chinese, false) => "确认连接信息后启动服务；保存会写入本机配置。",
        (Language::English, true) => "The background service is running and ready for control.",
        (Language::English, false) => "Confirm the connection details, then start the bridge.",
    }
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

fn status_pill(ui: &mut egui::Ui, language: Language, running: bool) {
    let (text, fill, text_color) = if running {
        (
            tr(language, "running"),
            Color32::from_rgb(221, 245, 235),
            SUCCESS,
        )
    } else {
        (
            tr(language, "stopped"),
            Color32::from_rgb(239, 242, 246),
            STOPPED,
        )
    };
    egui::Frame::new()
        .fill(fill)
        .stroke(Stroke::new(1.0, Color32::from_white_alpha(120)))
        .corner_radius(18)
        .inner_margin(egui::Margin::symmetric(12, 6))
        .show(ui, |ui| {
            ui.label(RichText::new(text).strong().color(text_color));
        });
}

fn paint_dashed_rect(ui: &egui::Ui, rect: egui::Rect, radius: f32, stroke: Stroke) {
    let inset = radius.min(rect.width() * 0.25).min(rect.height() * 0.25);
    let left_top = egui::pos2(rect.left() + inset, rect.top());
    let right_top = egui::pos2(rect.right() - inset, rect.top());
    let right_left = egui::pos2(rect.right(), rect.top() + inset);
    let right_bottom = egui::pos2(rect.right(), rect.bottom() - inset);
    let bottom_right = egui::pos2(rect.right() - inset, rect.bottom());
    let bottom_left = egui::pos2(rect.left() + inset, rect.bottom());
    let left_bottom = egui::pos2(rect.left(), rect.bottom() - inset);
    let left_left = egui::pos2(rect.left(), rect.top() + inset);

    paint_dashed_line(ui, left_top, right_top, stroke);
    paint_dashed_line(ui, right_left, right_bottom, stroke);
    paint_dashed_line(ui, bottom_right, bottom_left, stroke);
    paint_dashed_line(ui, left_bottom, left_left, stroke);
}

fn paint_dashed_line(ui: &egui::Ui, start: egui::Pos2, end: egui::Pos2, stroke: Stroke) {
    let delta = end - start;
    let length = delta.length();
    if length <= 0.0 {
        return;
    }
    let direction = delta / length;
    let dash = 9.0;
    let gap = 6.0;
    let mut cursor = 0.0;
    while cursor < length {
        let next = (cursor + dash).min(length);
        ui.painter().line_segment(
            [start + direction * cursor, start + direction * next],
            stroke,
        );
        cursor += dash + gap;
    }
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
            "connected" => "已连接",
            "disconnected" => "未连接",
            "parameters" => "参数",
            "diagnostics" => "状态",
            "service_state" => "服务状态",
            "active_role" => "当前角色",
            "input_bridge" => "键鼠桥接",
            "clipboard_bridge" => "剪贴板桥接",
            "role" => "角色",
            "endpoint" => "地址",
            "edge" => "屏幕边缘",
            "state" => "状态",
            "connection" => "连接",
            "client" => "客户端",
            "server" => "服务端",
            "server_address" => "服务端地址",
            "bind_address" => "监听地址",
            "mac_edge" => "客户端位于本机哪一侧",
            "left" => "左侧",
            "right" => "右侧",
            "runtime" => "运行",
            "start_connection" => "启动连接",
            "stop_connection" => "停止连接",
            "settings" => "设置",
            "close" => "关闭",
            "save_config" => "保存配置",
            "save_settings" => "保存设置",
            "command" => "当前命令",
            "scroll" => "滚轮",
            "modifier_mapping" => "macOS 服务端修饰键映射",
            "modifier_mapping_hint" => {
                "当 macOS 作为服务端控制 Windows 时，选择这些物理按键发送到 Windows 的行为。"
            }
            "mac_command_key" => "Command 键",
            "mac_control_key" => "Control 键",
            "mac_option_key" => "Option 键",
            "map_to_control" => "Ctrl",
            "map_to_win" => "Win",
            "map_to_alt" => "Alt",
            "map_disabled" => "禁用",
            "restart_required" => "当前服务正在运行，保存后需重启服务才会生效。",
            "distance" => "距离",
            "response" => "响应",
            "max_step" => "最大步长",
            "frame_ms" => "帧间隔",
            "file_transfer" => "文件投递",
            "drop_intro" => "正常情况下，文件可以直接通过复制粘贴在两端同步；这里是备用投递方案。",
            "drop_after" => "把文件拖到这里后，到另一端使用粘贴即可接收文件。",
            "drop_zone_title" => "拖放文件到这里",
            "drop_hint" => "投递完成后，在另一端按 Ctrl+V / Cmd+V 粘贴",
            "logs" => "日志",
            "clear" => "清空",
            "no_logs" => "暂无日志",
            "service_started" => "服务已启动",
            "service_stopped" => "服务已停止",
            "config_saved" => "配置已保存",
            "config_saved_restart" => "配置已保存，重启服务后生效",
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
            "connected" => "Connected",
            "disconnected" => "Disconnected",
            "parameters" => "Parameters",
            "diagnostics" => "Status",
            "service_state" => "Service state",
            "active_role" => "Active role",
            "input_bridge" => "Input bridge",
            "clipboard_bridge" => "Clipboard bridge",
            "role" => "Role",
            "endpoint" => "Endpoint",
            "edge" => "Screen edge",
            "state" => "State",
            "connection" => "Connection",
            "client" => "Client",
            "server" => "Server",
            "server_address" => "Server address",
            "bind_address" => "Bind address",
            "mac_edge" => "Client screen side",
            "left" => "Left",
            "right" => "Right",
            "runtime" => "Runtime",
            "start_connection" => "Start connection",
            "stop_connection" => "Stop connection",
            "settings" => "Settings",
            "close" => "Close",
            "save_config" => "Save config",
            "save_settings" => "Save settings",
            "command" => "Current command",
            "scroll" => "Scroll",
            "modifier_mapping" => "macOS server modifier mapping",
            "modifier_mapping_hint" => {
                "Choose what these physical keys send to Windows when this Mac runs as the server."
            }
            "mac_command_key" => "Command key",
            "mac_control_key" => "Control key",
            "mac_option_key" => "Option key",
            "map_to_control" => "Ctrl",
            "map_to_win" => "Win",
            "map_to_alt" => "Alt",
            "map_disabled" => "Disabled",
            "restart_required" => {
                "The service is running. Restart it for saved settings to take effect."
            }
            "distance" => "Distance",
            "response" => "Response",
            "max_step" => "Max step",
            "frame_ms" => "Frame ms",
            "file_transfer" => "File transfer",
            "drop_intro" => {
                "Normally, use copy and paste to move files between devices. This drop zone is a fallback path."
            }
            "drop_after" => {
                "After dropping files here, paste on the other computer to receive them."
            }
            "drop_zone_title" => "Drop files here",
            "drop_hint" => "Then paste on the other side with Ctrl+V / Cmd+V",
            "logs" => "Logs",
            "clear" => "Clear",
            "no_logs" => "No logs yet",
            "service_started" => "Service started",
            "service_stopped" => "Service stopped",
            "config_saved" => "Config saved",
            "config_saved_restart" => "Config saved; restart the service to apply",
            "start_failed" => "Failed to start",
            "stop_failed" => "Failed to stop",
            "save_failed" => "Failed to save",
            "drop_failed" => "File drop failed",
            "queued_files" => "Queued file count:",
            _ => "",
        },
    }
}
