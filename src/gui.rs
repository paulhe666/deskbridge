use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::thread;

use serde::{Deserialize, Serialize};

use crate::config::{AppConfig, Language, ModifierTarget, Role};
use crate::control::{ControlBackend, ProcessBackend};
use crate::server::Edge;

const INDEX_HTML: &str = include_str!("../web/dist/index.html");
const APP_JS: &str = include_str!("../web/dist/assets/app.js");
const STYLE_CSS: &str = include_str!("../web/dist/assets/style.css");

pub fn run() -> std::io::Result<()> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let address = listener.local_addr()?;
    let url = format!("http://{address}/");

    let state = Arc::new(AppState {
        config: Mutex::new(AppConfig::load()),
        backend: Mutex::new(ProcessBackend::default()),
    });

    println!("Deskbridge Web UI running at {url}");
    println!("Close this terminal or press Ctrl+C to stop the GUI backend.");
    open_browser(&url);

    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                let state = Arc::clone(&state);
                thread::spawn(move || {
                    if let Err(error) = handle_connection(stream, state) {
                        eprintln!("gui request failed: {error}");
                    }
                });
            }
            Err(error) => eprintln!("gui accept failed: {error}"),
        }
    }

    Ok(())
}

struct AppState {
    config: Mutex<AppConfig>,
    backend: Mutex<ProcessBackend>,
}

#[derive(Debug)]
struct HttpRequest {
    method: String,
    path: String,
    body: Vec<u8>,
}

fn handle_connection(mut stream: TcpStream, state: Arc<AppState>) -> std::io::Result<()> {
    let request = read_request(&mut stream)?;
    let response = route_request(request, state);
    stream.write_all(&response.to_bytes())?;
    stream.flush()
}

fn read_request(stream: &mut TcpStream) -> std::io::Result<HttpRequest> {
    let mut buffer = Vec::new();
    let mut temp = [0u8; 4096];
    let header_end;

    loop {
        let read = stream.read(&mut temp)?;
        if read == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "connection closed before headers",
            ));
        }
        buffer.extend_from_slice(&temp[..read]);
        if let Some(index) = find_header_end(&buffer) {
            header_end = index;
            break;
        }
        if buffer.len() > 64 * 1024 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "request headers too large",
            ));
        }
    }

    let headers = String::from_utf8_lossy(&buffer[..header_end]);
    let mut lines = headers.lines();
    let request_line = lines.next().unwrap_or_default();
    let mut request_parts = request_line.split_whitespace();
    let method = request_parts.next().unwrap_or_default().to_string();
    let path = request_parts.next().unwrap_or("/").to_string();

    let mut header_map = HashMap::new();
    for line in lines {
        if let Some((key, value)) = line.split_once(':') {
            header_map.insert(key.trim().to_ascii_lowercase(), value.trim().to_string());
        }
    }

    let content_length = header_map
        .get("content-length")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(0);

    let body_start = header_end + 4;
    let mut body = buffer.get(body_start..).unwrap_or_default().to_vec();
    while body.len() < content_length {
        let read = stream.read(&mut temp)?;
        if read == 0 {
            break;
        }
        body.extend_from_slice(&temp[..read]);
    }
    body.truncate(content_length);

    Ok(HttpRequest { method, path, body })
}

fn find_header_end(buffer: &[u8]) -> Option<usize> {
    buffer.windows(4).position(|window| window == b"\r\n\r\n")
}

fn route_request(request: HttpRequest, state: Arc<AppState>) -> HttpResponse {
    let path = request
        .path
        .split('?')
        .next()
        .unwrap_or("/")
        .trim_end_matches('/');
    let path = if path.is_empty() { "/" } else { path };

    match (request.method.as_str(), path) {
        ("GET", "/") => HttpResponse::html(INDEX_HTML),
        ("GET", "/assets/app.js") => HttpResponse::javascript(APP_JS),
        ("GET", "/assets/style.css") => HttpResponse::css(STYLE_CSS),
        ("GET", "/api/state") => json_response(build_state(&state)),
        ("POST", "/api/config") => update_config(&request.body, &state),
        ("POST", "/api/start") => start_service(&state),
        ("POST", "/api/stop") => stop_service(&state),
        ("POST", "/api/logs/clear") => clear_logs(&state),
        _ => HttpResponse::not_found(),
    }
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

fn update_config(body: &[u8], state: &AppState) -> HttpResponse {
    let update = match serde_json::from_slice::<ConfigView>(body) {
        Ok(update) => update,
        Err(error) => return json_error(400, format!("invalid config json: {error}")),
    };

    {
        let mut config = state.config.lock().expect("config mutex poisoned");
        update.apply_to(&mut config);
        if let Err(error) = config.save() {
            return json_error(500, format!("failed to save config: {error}"));
        }
    }

    json_response(build_state(state))
}

fn start_service(state: &AppState) -> HttpResponse {
    let config = state.config.lock().expect("config mutex poisoned").clone();
    if let Err(error) = config.save() {
        return json_error(500, format!("failed to save config: {error}"));
    }

    let mut backend = state.backend.lock().expect("backend mutex poisoned");
    if let Err(error) = backend.start(&config) {
        backend.push_log(format!("Failed to start service: {error}"));
        return json_error(500, format!("failed to start service: {error}"));
    }
    drop(backend);

    json_response(build_state(state))
}

fn stop_service(state: &AppState) -> HttpResponse {
    let mut backend = state.backend.lock().expect("backend mutex poisoned");
    if let Err(error) = backend.stop() {
        backend.push_log(format!("Failed to stop service: {error}"));
        return json_error(500, format!("failed to stop service: {error}"));
    }
    drop(backend);

    json_response(build_state(state))
}

fn clear_logs(state: &AppState) -> HttpResponse {
    state
        .backend
        .lock()
        .expect("backend mutex poisoned")
        .clear_logs();
    json_response(build_state(state))
}

#[derive(Debug, Serialize)]
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

#[derive(Debug, Serialize)]
struct ErrorBody {
    error: String,
}

fn json_response<T: Serialize>(body: T) -> HttpResponse {
    match serde_json::to_vec(&body) {
        Ok(bytes) => HttpResponse::new(200, "application/json; charset=utf-8", bytes),
        Err(error) => json_error(500, format!("failed to serialize response: {error}")),
    }
}

fn json_error(status: u16, error: String) -> HttpResponse {
    let body = serde_json::to_vec(&ErrorBody { error }).unwrap_or_else(|_| b"{}".to_vec());
    HttpResponse::new(status, "application/json; charset=utf-8", body)
}

struct HttpResponse {
    status: u16,
    content_type: &'static str,
    body: Vec<u8>,
}

impl HttpResponse {
    fn new(status: u16, content_type: &'static str, body: Vec<u8>) -> Self {
        Self {
            status,
            content_type,
            body,
        }
    }

    fn html(body: &str) -> Self {
        Self::new(200, "text/html; charset=utf-8", body.as_bytes().to_vec())
    }

    fn javascript(body: &str) -> Self {
        Self::new(
            200,
            "application/javascript; charset=utf-8",
            body.as_bytes().to_vec(),
        )
    }

    fn css(body: &str) -> Self {
        Self::new(200, "text/css; charset=utf-8", body.as_bytes().to_vec())
    }

    fn not_found() -> Self {
        Self::new(404, "text/plain; charset=utf-8", b"Not found".to_vec())
    }

    fn to_bytes(&self) -> Vec<u8> {
        let reason = match self.status {
            200 => "OK",
            400 => "Bad Request",
            404 => "Not Found",
            500 => "Internal Server Error",
            _ => "OK",
        };
        let mut response = format!(
            "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
            self.status,
            reason,
            self.content_type,
            self.body.len()
        )
        .into_bytes();
        response.extend_from_slice(&self.body);
        response
    }
}

fn open_browser(url: &str) {
    let result = if cfg!(target_os = "macos") {
        Command::new("open").arg(url).spawn()
    } else if cfg!(windows) {
        Command::new("cmd").args(["/C", "start", "", url]).spawn()
    } else {
        Command::new("xdg-open").arg(url).spawn()
    };

    if let Err(error) = result {
        eprintln!("failed to open browser automatically: {error}");
        eprintln!("open this URL manually: {url}");
    }
}
