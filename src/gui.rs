use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::config::{AppConfig, Language, ModifierTarget, Role};
use crate::control::{ControlBackend, ProcessBackend};
use crate::server::Edge;

pub fn run() -> std::io::Result<()> {
    tauri::Builder::default()
        .manage(AppState {
            config: Mutex::new(AppConfig::load()),
            backend: Mutex::new(ProcessBackend::default()),
        })
        .invoke_handler(tauri::generate_handler![
            get_state,
            save_config,
            start_service,
            stop_service,
            clear_logs
        ])
        .run(tauri::generate_context!())
        .map_err(|error| std::io::Error::other(error.to_string()))
}

struct AppState {
    config: Mutex<AppConfig>,
    backend: Mutex<ProcessBackend>,
}

#[tauri::command]
fn get_state(state: tauri::State<'_, AppState>) -> Result<ApiState, String> {
    Ok(build_state(state.inner()))
}

#[tauri::command]
fn save_config(
    config: ConfigView,
    state: tauri::State<'_, AppState>,
) -> Result<ApiState, String> {
    {
        let mut current = state
            .config
            .lock()
            .map_err(|_| "config mutex poisoned".to_string())?;
        config.apply_to(&mut current);
        current
            .save()
            .map_err(|error| format!("failed to save config: {error}"))?;
    }

    Ok(build_state(state.inner()))
}

#[tauri::command]
fn start_service(state: tauri::State<'_, AppState>) -> Result<ApiState, String> {
    let config = {
        let config = state
            .config
            .lock()
            .map_err(|_| "config mutex poisoned".to_string())?
            .clone();
        config
            .save()
            .map_err(|error| format!("failed to save config: {error}"))?;
        config
    };

    {
        let mut backend = state
            .backend
            .lock()
            .map_err(|_| "backend mutex poisoned".to_string())?;
        if let Err(error) = backend.start(&config) {
            backend.push_log(format!("Failed to start service: {error}"));
            return Err(format!("failed to start service: {error}"));
        }
    }

    Ok(build_state(state.inner()))
}

#[tauri::command]
fn stop_service(state: tauri::State<'_, AppState>) -> Result<ApiState, String> {
    {
        let mut backend = state
            .backend
            .lock()
            .map_err(|_| "backend mutex poisoned".to_string())?;
        if let Err(error) = backend.stop() {
            backend.push_log(format!("Failed to stop service: {error}"));
            return Err(format!("failed to stop service: {error}"));
        }
    }

    Ok(build_state(state.inner()))
}

#[tauri::command]
fn clear_logs(state: tauri::State<'_, AppState>) -> Result<ApiState, String> {
    state
        .backend
        .lock()
        .map_err(|_| "backend mutex poisoned".to_string())?
        .clear_logs();
    Ok(build_state(state.inner()))
}

fn build_state(state: &AppState) -> ApiState {
    let config = state.config.lock().expect("config mutex poisoned").clone();
    let mut backend = state.backend.lock().expect("backend mutex poisoned");
    backend.collect_logs();
    let running = backend.is_running();
    let command_preview = backend.command_preview(&config);
    let logs = backend.logs().iter().cloned().collect();

    ApiState {
        config: ConfigView::from_config(&config),
        running,
        command_preview,
        logs,
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ApiState {
    config: ConfigView,
    running: bool,
    command_preview: String,
    logs: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ConfigView {
    role: String,
    language: String,
    bind: String,
    server: String,
    edge: String,
    scroll_scale: f64,
    scroll_response: f64,
    scroll_max_step: f64,
    scroll_frame_ms: u64,
    mac_command_mapping: String,
    mac_control_mapping: String,
    mac_option_mapping: String,
}

impl ConfigView {
    fn from_config(config: &AppConfig) -> Self {
        Self {
            role: match config.role {
                Role::Server => "server",
                Role::Client => "client",
            }
            .to_string(),
            language: config.language.as_str().to_string(),
            bind: config.bind.clone(),
            server: config.server.clone(),
            edge: match config.edge {
                Edge::Left => "left",
                Edge::Right => "right",
            }
            .to_string(),
            scroll_scale: config.scroll_scale,
            scroll_response: config.scroll_response,
            scroll_max_step: config.scroll_max_step,
            scroll_frame_ms: config.scroll_frame_ms,
            mac_command_mapping: config.mac_command_mapping.as_str().to_string(),
            mac_control_mapping: config.mac_control_mapping.as_str().to_string(),
            mac_option_mapping: config.mac_option_mapping.as_str().to_string(),
        }
    }

    fn apply_to(self, config: &mut AppConfig) {
        config.role = if self.role == "server" {
            Role::Server
        } else {
            Role::Client
        };
        config.language = Language::parse(&self.language).unwrap_or(config.language);
        config.bind = self.bind;
        config.server = self.server;
        config.edge = Edge::parse(&self.edge).unwrap_or(config.edge);
        config.scroll_scale = self.scroll_scale;
        config.scroll_response = self.scroll_response;
        config.scroll_max_step = self.scroll_max_step;
        config.scroll_frame_ms = self.scroll_frame_ms;
        config.mac_command_mapping =
            ModifierTarget::parse(&self.mac_command_mapping).unwrap_or(config.mac_command_mapping);
        config.mac_control_mapping =
            ModifierTarget::parse(&self.mac_control_mapping).unwrap_or(config.mac_control_mapping);
        config.mac_option_mapping =
            ModifierTarget::parse(&self.mac_option_mapping).unwrap_or(config.mac_option_mapping);
    }
}
