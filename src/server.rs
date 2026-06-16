#[cfg(not(windows))]
mod unsupported;
#[cfg(windows)]
mod windows;

#[cfg(not(windows))]
pub use unsupported::run;
#[cfg(windows)]
pub use windows::run;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edge {
    Left,
    Right,
}

impl Edge {
    pub fn parse(value: &str) -> Result<Self, String> {
        match value {
            "left" => Ok(Self::Left),
            "right" => Ok(Self::Right),
            _ => Err("edge must be left or right".to_string()),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub bind: String,
    pub edge: Edge,
}
