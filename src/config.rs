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
    Shift,
    Disabled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyTarget {
    Escape,
    Backspace,
    Delete,
    Enter,
    Tab,
    Space,
    CapsLock,
    ArrowLeft,
    ArrowRight,
    ArrowUp,
    ArrowDown,
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
    pub mac_shift_mapping: ModifierTarget,
    pub mac_caps_lock_mapping: KeyTarget,
    pub mac_escape_mapping: KeyTarget,
    pub mac_backspace_mapping: KeyTarget,
    pub mac_delete_mapping: KeyTarget,
    pub mac_arrow_left_mapping: KeyTarget,
    pub mac_arrow_right_mapping: KeyTarget,
    pub mac_arrow_up_mapping: KeyTarget,
    pub mac_arrow_down_mapping: KeyTarget,
    pub auto_update_check: bool,
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
            mac_shift_mapping: ModifierTarget::Shift,
            mac_caps_lock_mapping: KeyTarget::CapsLock,
            mac_escape_mapping: KeyTarget::Escape,
            mac_backspace_mapping: KeyTarget::Backspace,
            mac_delete_mapping: KeyTarget::Delete,
            mac_arrow_left_mapping: KeyTarget::ArrowLeft,
            mac_arrow_right_mapping: KeyTarget::ArrowRight,
            mac_arrow_up_mapping: KeyTarget::ArrowUp,
            mac_arrow_down_mapping: KeyTarget::ArrowDown,
            auto_update_check: true,
        }
    }
}

impl AppConfig {
    pub fn load() -> Self {
        let mut config = Self::default();
        let Ok(text) = fs::read_to_string(config_path()) else {
            return config;
        };
        let mut saved_language_without_user_marker = false;
        let mut language_user_set = false;
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
                    saved_language_without_user_marker = true;
                    config.language = Language::parse(value.trim()).unwrap_or(config.language)
                }
                "language_source" => {
                    language_user_set = value.trim() == "user";
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
                "mac_shift_mapping" => {
                    config.mac_shift_mapping =
                        ModifierTarget::parse(value.trim()).unwrap_or(config.mac_shift_mapping)
                }
                "mac_caps_lock_mapping" => {
                    config.mac_caps_lock_mapping =
                        KeyTarget::parse(value.trim()).unwrap_or(config.mac_caps_lock_mapping)
                }
                "mac_escape_mapping" => {
                    config.mac_escape_mapping =
                        KeyTarget::parse(value.trim()).unwrap_or(config.mac_escape_mapping)
                }
                "mac_backspace_mapping" => {
                    config.mac_backspace_mapping =
                        KeyTarget::parse(value.trim()).unwrap_or(config.mac_backspace_mapping)
                }
                "mac_delete_mapping" => {
                    config.mac_delete_mapping =
                        KeyTarget::parse(value.trim()).unwrap_or(config.mac_delete_mapping)
                }
                "mac_arrow_left_mapping" => {
                    config.mac_arrow_left_mapping =
                        KeyTarget::parse(value.trim()).unwrap_or(config.mac_arrow_left_mapping)
                }
                "mac_arrow_right_mapping" => {
                    config.mac_arrow_right_mapping =
                        KeyTarget::parse(value.trim()).unwrap_or(config.mac_arrow_right_mapping)
                }
                "mac_arrow_up_mapping" => {
                    config.mac_arrow_up_mapping =
                        KeyTarget::parse(value.trim()).unwrap_or(config.mac_arrow_up_mapping)
                }
                "mac_arrow_down_mapping" => {
                    config.mac_arrow_down_mapping =
                        KeyTarget::parse(value.trim()).unwrap_or(config.mac_arrow_down_mapping)
                }
                "auto_update_check" => {
                    config.auto_update_check = parse_bool(value, config.auto_update_check)
                }
                _ => {}
            }
        }
        if saved_language_without_user_marker && !language_user_set {
            config.language = Language::English;
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
                "role={}\nlanguage={}\nlanguage_source=user\nbind={}\nserver={}\nedge={}\nscroll_scale={:.3}\nscroll_response={:.3}\nscroll_max_step={:.1}\nscroll_frame_ms={}\nmac_command_mapping={}\nmac_control_mapping={}\nmac_option_mapping={}\nmac_shift_mapping={}\nmac_caps_lock_mapping={}\nmac_escape_mapping={}\nmac_backspace_mapping={}\nmac_delete_mapping={}\nmac_arrow_left_mapping={}\nmac_arrow_right_mapping={}\nmac_arrow_up_mapping={}\nmac_arrow_down_mapping={}\nauto_update_check={}\n",
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
                self.mac_shift_mapping.as_str(),
                self.mac_caps_lock_mapping.as_str(),
                self.mac_escape_mapping.as_str(),
                self.mac_backspace_mapping.as_str(),
                self.mac_delete_mapping.as_str(),
                self.mac_arrow_left_mapping.as_str(),
                self.mac_arrow_right_mapping.as_str(),
                self.mac_arrow_up_mapping.as_str(),
                self.mac_arrow_down_mapping.as_str(),
                self.auto_update_check,
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
            "shift" => Some(Self::Shift),
            "disabled" | "none" | "off" => Some(Self::Disabled),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Control => "control",
            Self::Meta => "meta",
            Self::Alt => "alt",
            Self::Shift => "shift",
            Self::Disabled => "disabled",
        }
    }
}

impl KeyTarget {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "escape" | "esc" => Some(Self::Escape),
            "backspace" => Some(Self::Backspace),
            "delete" | "forward_delete" => Some(Self::Delete),
            "enter" | "return" => Some(Self::Enter),
            "tab" => Some(Self::Tab),
            "space" => Some(Self::Space),
            "caps_lock" | "capslock" => Some(Self::CapsLock),
            "arrow_left" | "left" => Some(Self::ArrowLeft),
            "arrow_right" | "right" => Some(Self::ArrowRight),
            "arrow_up" | "up" => Some(Self::ArrowUp),
            "arrow_down" | "down" => Some(Self::ArrowDown),
            "disabled" | "none" | "off" => Some(Self::Disabled),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Escape => "escape",
            Self::Backspace => "backspace",
            Self::Delete => "delete",
            Self::Enter => "enter",
            Self::Tab => "tab",
            Self::Space => "space",
            Self::CapsLock => "caps_lock",
            Self::ArrowLeft => "arrow_left",
            Self::ArrowRight => "arrow_right",
            Self::ArrowUp => "arrow_up",
            Self::ArrowDown => "arrow_down",
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

fn parse_bool(value: &str, default: bool) -> bool {
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => true,
        "0" | "false" | "no" | "off" => false,
        _ => default,
    }
}

fn default_language() -> Language {
    Language::English
}
