use std::fs;
use std::path::PathBuf;

use crate::server::Edge;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Server,
    Client,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Language {
    Chinese,
    English,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModifierTarget {
    Control,
    Meta,
    Alt,
    Disabled,
}

#[derive(Debug, Clone)]
pub struct AppConfig {
    pub role: Role,
    pub language: Language,
    pub bind: String,
    pub server: String,
    pub edge: Edge,
    pub scroll_scale: f64,
    pub scroll_response: f64,
    pub scroll_max_step: f64,
    pub scroll_frame_ms: u64,
    pub mac_command_mapping: ModifierTarget,
    pub mac_control_mapping: ModifierTarget,
    pub mac_option_mapping: ModifierTarget,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            role: Role::Client,
            language: default_language(),
            bind: "0.0.0.0:24920".to_string(),
            server: "192.168.1.10:24920".to_string(),
            edge: Edge::Right,
            scroll_scale: 1.35,
            scroll_response: 0.38,
            scroll_max_step: 120.0,
            scroll_frame_ms: 8,
            mac_command_mapping: ModifierTarget::Control,
            mac_control_mapping: ModifierTarget::Control,
            mac_option_mapping: ModifierTarget::Alt,
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
                "language" => {
                    config.language = Language::parse(value.trim()).unwrap_or(config.language)
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
                "mac_command_mapping" => {
                    config.mac_command_mapping =
                        ModifierTarget::parse(value.trim()).unwrap_or(config.mac_command_mapping)
                }
                "mac_control_mapping" => {
                    config.mac_control_mapping =
                        ModifierTarget::parse(value.trim()).unwrap_or(config.mac_control_mapping)
                }
                "mac_option_mapping" => {
                    config.mac_option_mapping =
                        ModifierTarget::parse(value.trim()).unwrap_or(config.mac_option_mapping)
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
                "role={}\nlanguage={}\nbind={}\nserver={}\nedge={}\nscroll_scale={:.3}\nscroll_response={:.3}\nscroll_max_step={:.1}\nscroll_frame_ms={}\nmac_command_mapping={}\nmac_control_mapping={}\nmac_option_mapping={}\n",
                match self.role {
                    Role::Server => "server",
                    Role::Client => "client",
                },
                self.language.as_str(),
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
                self.mac_command_mapping.as_str(),
                self.mac_control_mapping.as_str(),
                self.mac_option_mapping.as_str(),
            ),
        )
    }
}

impl Language {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "zh" | "chinese" => Some(Self::Chinese),
            "en" | "english" => Some(Self::English),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Chinese => "zh",
            Self::English => "en",
        }
    }
}

impl ModifierTarget {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "control" | "ctrl" => Some(Self::Control),
            "meta" | "win" | "windows" | "command" => Some(Self::Meta),
            "alt" | "option" => Some(Self::Alt),
            "disabled" | "none" | "off" => Some(Self::Disabled),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Control => "control",
            Self::Meta => "meta",
            Self::Alt => "alt",
            Self::Disabled => "disabled",
        }
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

fn default_language() -> Language {
    Language::English
}
