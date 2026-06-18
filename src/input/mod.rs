#[cfg(target_os = "macos")]
mod macos;
#[cfg(not(any(target_os = "macos", windows)))]
mod unsupported;
#[cfg(windows)]
mod windows;

#[cfg(target_os = "macos")]
pub use macos::InputSink;
#[cfg(target_os = "macos")]
pub use macos::screen_size;
#[cfg(not(any(target_os = "macos", windows)))]
pub use unsupported::InputSink;
#[cfg(not(any(target_os = "macos", windows)))]
pub use unsupported::screen_size;
#[cfg(windows)]
pub use windows::InputSink;
#[cfg(windows)]
pub use windows::screen_size;
