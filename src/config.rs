use std::fs;
use std::path::PathBuf;

use crate::server::Edge;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Server,
    Client,
}

#[derive(Debug, Clone)]
pub struct AppConfig {
    pub role: Role,
    pub bind: String,
    pub server: String,
    pub edge: Edge,
    pub scroll_scale: f64,
    pub scroll_response: f64,
    pub scroll_max_step: f64,
    pub scroll_frame_ms: u64,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            role: Role::Client,
            bind: "0.0.0.0:24920".to_string(),
            server: "192.168.1.10:24920".to_string(),
            edge: Edge::Right,
            scroll_scale: 1.35,
            scroll_response: 0.38,
            scroll_max_step: 120.0,
            scroll_frame_ms: 8,
        }
    }
}

impl AppConfig {
    pub fn load() -> Self {
        let mut config = Self::default();
        let Ok(text) = fs::read_to_string(config_path()) else {
            return config;
        };
        for line in text.lines() {
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            match key.trim() {
                "role" => {
                    config.role = if value.trim() == "server" {
                        Role::Server
                    } else {
                        Role::Client
                    }
                }
                "bind" => config.bind = value.trim().to_string(),
                "server" => config.server = value.trim().to_string(),
                "edge" => {
                    if let Ok(edge) = Edge::parse(value.trim()) {
                        config.edge = edge;
                    }
                }
                "scroll_scale" => config.scroll_scale = parse_f64(value, config.scroll_scale),
                "scroll_response" => {
                    config.scroll_response = parse_f64(value, config.scroll_response)
                }
                "scroll_max_step" => {
                    config.scroll_max_step = parse_f64(value, config.scroll_max_step)
                }
                "scroll_frame_ms" => {
                    config.scroll_frame_ms = value.trim().parse().unwrap_or(config.scroll_frame_ms)
                }
                _ => {}
            }
        }
        config
    }

    pub fn save(&self) -> std::io::Result<()> {
        let path = config_path();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(
            path,
            format!(
                "role={}\nbind={}\nserver={}\nedge={}\nscroll_scale={:.3}\nscroll_response={:.3}\nscroll_max_step={:.1}\nscroll_frame_ms={}\n",
                match self.role {
                    Role::Server => "server",
                    Role::Client => "client",
                },
                self.bind,
                self.server,
                match self.edge {
                    Edge::Left => "left",
                    Edge::Right => "right",
                },
                self.scroll_scale,
                self.scroll_response,
                self.scroll_max_step,
                self.scroll_frame_ms,
            ),
        )
    }
}

pub fn config_path() -> PathBuf {
    config_dir().join("config.ini")
}

pub fn config_dir() -> PathBuf {
    std::env::var_os("DESKBRIDGE_CONFIG_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| home_dir().join(".deskbridge"))
}

fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

fn parse_f64(value: &str, default: f64) -> f64 {
    value.trim().parse().unwrap_or(default)
}
