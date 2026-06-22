use crate::protocol::ClipboardPayload;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
mod unsupported;
#[cfg(windows)]
mod windows;

#[cfg(target_os = "linux")]
pub use linux::Clipboard;
#[cfg(target_os = "macos")]
pub use macos::Clipboard;
#[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
pub use unsupported::Clipboard;
#[cfg(windows)]
pub use windows::Clipboard;

pub trait ClipboardApi {
    fn read(&mut self) -> std::io::Result<Option<ClipboardPayload>>;
    fn write(&mut self, payload: &ClipboardPayload) -> std::io::Result<()>;
}
