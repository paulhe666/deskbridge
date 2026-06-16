#[cfg(target_os = "macos")]
mod macos;
#[cfg(not(target_os = "macos"))]
mod unsupported;

#[cfg(target_os = "macos")]
pub use macos::InputSink;
#[cfg(not(target_os = "macos"))]
pub use unsupported::InputSink;
