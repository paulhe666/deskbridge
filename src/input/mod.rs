#[cfg(target_os = "macos")]
mod macos;
#[cfg(not(target_os = "macos"))]
mod unsupported;

#[cfg(target_os = "macos")]
pub use macos::InputSink;
#[cfg(target_os = "macos")]
pub use macos::screen_size;
#[cfg(not(target_os = "macos"))]
pub use unsupported::InputSink;
#[cfg(not(target_os = "macos"))]
pub use unsupported::screen_size;
