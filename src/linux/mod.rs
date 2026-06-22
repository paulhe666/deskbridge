use std::env;
use std::ffi::OsStr;
use std::io::{self, ErrorKind};
use std::path::Path;
use std::process::{Command, Stdio};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisplayServer {
    Wayland,
    X11,
    Unknown,
}

pub fn prepare_gui_environment() {
    set_env_if_missing("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
    set_env_if_missing("WEBKIT_DISABLE_COMPOSITING_MODE", "1");

    if env_flag_enabled("DESKBRIDGE_LINUX_SOFTWARE_RENDERING") {
        set_env_if_missing("LIBGL_ALWAYS_SOFTWARE", "1");
    }

    match env::var("DESKBRIDGE_LINUX_GUI_BACKEND") {
        Ok(value) if value.eq_ignore_ascii_case("x11") => set_env("GDK_BACKEND", "x11"),
        Ok(value) if value.eq_ignore_ascii_case("wayland") => set_env("GDK_BACKEND", "wayland"),
        _ if env_flag_enabled("DESKBRIDGE_LINUX_PREFER_X11") => {
            set_env_if_missing("GDK_BACKEND", "x11")
        }
        _ => {}
    }
}

impl DisplayServer {
    pub fn detect() -> Self {
        if matches_env("DESKBRIDGE_LINUX_SESSION", "wayland") {
            return Self::Wayland;
        }
        if matches_env("DESKBRIDGE_LINUX_SESSION", "x11") {
            return Self::X11;
        }
        if env::var_os("WAYLAND_DISPLAY").is_some() {
            return Self::Wayland;
        }
        if env::var_os("DISPLAY").is_some() {
            return Self::X11;
        }
        Self::Unknown
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Wayland => "wayland",
            Self::X11 => "x11",
            Self::Unknown => "unknown",
        }
    }
}

pub fn command_exists(command: &str) -> bool {
    env::var_os("PATH")
        .map(|paths| {
            env::split_paths(&paths).any(|path| {
                let candidate = path.join(command);
                candidate.is_file() && is_executable(&candidate)
            })
        })
        .unwrap_or(false)
}

pub fn first_available<'a>(commands: &'a [&'a str]) -> Option<&'a str> {
    commands
        .iter()
        .copied()
        .find(|command| command_exists(command))
}

pub fn run_output<I, S>(command: &str, args: I) -> io::Result<Vec<u8>>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let output = Command::new(command).args(args).output()?;
    if output.status.success() {
        Ok(output.stdout)
    } else {
        Err(io::Error::new(
            ErrorKind::Other,
            format!(
                "{command} failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ),
        ))
    }
}

pub fn run_status<I, S>(command: &str, args: I) -> io::Result<()>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let status = Command::new(command).args(args).status()?;
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::new(
            ErrorKind::Other,
            format!("{command} exited with status {status}"),
        ))
    }
}

pub fn run_with_stdin<I, S>(command: &str, args: I, stdin: &[u8]) -> io::Result<()>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let mut child = Command::new(command)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()?;
    if let Some(mut child_stdin) = child.stdin.take() {
        use std::io::Write;
        child_stdin.write_all(stdin)?;
    }
    let output = child.wait_with_output()?;
    if output.status.success() {
        Ok(())
    } else {
        Err(io::Error::new(
            ErrorKind::Other,
            format!(
                "{command} failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ),
        ))
    }
}

pub fn unsupported(message: impl Into<String>) -> io::Error {
    io::Error::new(ErrorKind::Unsupported, message.into())
}

pub fn other(message: impl Into<String>) -> io::Error {
    io::Error::new(ErrorKind::Other, message.into())
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.metadata()
        .map(|metadata| metadata.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    path.is_file()
}

fn matches_env(key: &str, expected: &str) -> bool {
    env::var(key)
        .map(|value| value.eq_ignore_ascii_case(expected))
        .unwrap_or(false)
}

fn env_flag_enabled(key: &str) -> bool {
    env::var(key)
        .map(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false)
}

fn set_env_if_missing(key: &str, value: &str) {
    if env::var_os(key).is_none() {
        set_env(key, value);
    }
}

fn set_env(key: &str, value: &str) {
    unsafe {
        env::set_var(key, value);
    }
}
