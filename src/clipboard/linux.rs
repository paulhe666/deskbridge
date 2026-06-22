use std::io;
use std::path::{Path, PathBuf};

use crate::clipboard::ClipboardApi;
use crate::linux::{self, DisplayServer};
use crate::protocol::ClipboardPayload;

pub struct Clipboard {
    backend: ClipboardBackend,
}

#[derive(Debug, Clone, Copy)]
enum ClipboardBackend {
    Wayland,
    X11Xclip,
    X11Xsel,
}

impl Clipboard {
    pub fn new() -> io::Result<Self> {
        let display = DisplayServer::detect();
        let backend = match display {
            DisplayServer::Wayland if has_wayland_clipboard() => ClipboardBackend::Wayland,
            DisplayServer::Wayland => {
                return Err(linux::unsupported(
                    "Wayland clipboard requires wl-copy and wl-paste from wl-clipboard",
                ));
            }
            DisplayServer::X11 if linux::command_exists("xclip") => ClipboardBackend::X11Xclip,
            DisplayServer::X11 if linux::command_exists("xsel") => ClipboardBackend::X11Xsel,
            DisplayServer::X11 => {
                return Err(linux::unsupported(
                    "X11 clipboard requires xclip or xsel; xclip is required for images/files",
                ));
            }
            DisplayServer::Unknown => {
                return Err(linux::unsupported(
                    "Linux clipboard requires DISPLAY or WAYLAND_DISPLAY",
                ));
            }
        };
        eprintln!(
            "linux clipboard backend: {} ({display})",
            backend.as_str(),
            display = display.as_str()
        );
        Ok(Self { backend })
    }
}

impl ClipboardApi for Clipboard {
    fn read(&mut self) -> io::Result<Option<ClipboardPayload>> {
        if let Ok(Some(bitmap)) = self.backend.read_bitmap() {
            if !bitmap.is_empty() {
                return Ok(Some(ClipboardPayload::Bitmap(bitmap)));
            }
        }
        if let Ok(Some(files)) = self.backend.read_files() {
            if !files.is_empty() {
                return Ok(Some(ClipboardPayload::Files(files)));
            }
        }
        self.backend
            .read_text()
            .map(|text| text.filter(|text| !text.is_empty()).map(ClipboardPayload::Text))
    }

    fn write(&mut self, payload: &ClipboardPayload) -> io::Result<()> {
        match payload {
            ClipboardPayload::Text(text) => self.backend.write_text(text),
            ClipboardPayload::Bitmap(bitmap) => self.backend.write_bitmap(bitmap),
            ClipboardPayload::Files(files) => self.backend.write_files(files),
        }
    }
}

impl ClipboardBackend {
    fn as_str(self) -> &'static str {
        match self {
            Self::Wayland => "wl-clipboard",
            Self::X11Xclip => "xclip",
            Self::X11Xsel => "xsel",
        }
    }

    fn read_text(self) -> io::Result<Option<String>> {
        let bytes = match self {
            Self::Wayland => linux::run_output(
                "wl-paste",
                ["--no-newline", "--type", "text/plain;charset=utf-8"],
            )?,
            Self::X11Xclip => linux::run_output(
                "xclip",
                ["-selection", "clipboard", "-out", "-target", "text/plain;charset=utf-8"],
            )?,
            Self::X11Xsel => linux::run_output("xsel", ["--clipboard", "--output"] )?,
        };
        if bytes.is_empty() {
            return Ok(None);
        }
        Ok(Some(String::from_utf8_lossy(&bytes).to_string()))
    }

    fn write_text(self, text: &str) -> io::Result<()> {
        match self {
            Self::Wayland => linux::run_with_stdin(
                "wl-copy",
                ["--type", "text/plain;charset=utf-8"],
                text.as_bytes(),
            ),
            Self::X11Xclip => linux::run_with_stdin(
                "xclip",
                ["-selection", "clipboard", "-in", "-target", "text/plain;charset=utf-8"],
                text.as_bytes(),
            ),
            Self::X11Xsel => linux::run_with_stdin("xsel", ["--clipboard", "--input"], text.as_bytes()),
        }
    }

    fn read_bitmap(self) -> io::Result<Option<Vec<u8>>> {
        let bytes = match self {
            Self::Wayland => linux::run_output("wl-paste", ["--type", "image/png"] )?,
            Self::X11Xclip => linux::run_output(
                "xclip",
                ["-selection", "clipboard", "-out", "-target", "image/png"],
            )?,
            Self::X11Xsel => return Ok(None),
        };
        if bytes.is_empty() {
            Ok(None)
        } else {
            Ok(Some(bytes))
        }
    }

    fn write_bitmap(self, bitmap: &[u8]) -> io::Result<()> {
        match self {
            Self::Wayland => linux::run_with_stdin("wl-copy", ["--type", "image/png"], bitmap),
            Self::X11Xclip => linux::run_with_stdin(
                "xclip",
                ["-selection", "clipboard", "-in", "-target", "image/png"],
                bitmap,
            ),
            Self::X11Xsel => Err(linux::unsupported(
                "image clipboard on X11 requires xclip; xsel only supports text",
            )),
        }
    }

    fn read_files(self) -> io::Result<Option<Vec<PathBuf>>> {
        let bytes = match self {
            Self::Wayland => linux::run_output("wl-paste", ["--type", "text/uri-list"] )?,
            Self::X11Xclip => linux::run_output(
                "xclip",
                ["-selection", "clipboard", "-out", "-target", "text/uri-list"],
            )?,
            Self::X11Xsel => return Ok(None),
        };
        let files = parse_uri_list(&String::from_utf8_lossy(&bytes));
        if files.is_empty() {
            Ok(None)
        } else {
            Ok(Some(files))
        }
    }

    fn write_files(self, files: &[PathBuf]) -> io::Result<()> {
        let uri_list = files
            .iter()
            .map(|file| path_to_file_uri(file))
            .collect::<Vec<_>>()
            .join("\r\n");
        let uri_list = format!("{uri_list}\r\n");
        match self {
            Self::Wayland => linux::run_with_stdin(
                "wl-copy",
                ["--type", "text/uri-list"],
                uri_list.as_bytes(),
            ),
            Self::X11Xclip => linux::run_with_stdin(
                "xclip",
                ["-selection", "clipboard", "-in", "-target", "text/uri-list"],
                uri_list.as_bytes(),
            ),
            Self::X11Xsel => Err(linux::unsupported(
                "file clipboard on X11 requires xclip; xsel only supports text",
            )),
        }
    }
}

fn has_wayland_clipboard() -> bool {
    linux::command_exists("wl-copy") && linux::command_exists("wl-paste")
}

fn parse_uri_list(value: &str) -> Vec<PathBuf> {
    value
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .filter_map(|line| line.strip_prefix("file://"))
        .map(percent_decode_path)
        .collect()
}

fn path_to_file_uri(path: &Path) -> String {
    format!("file://{}", percent_encode_path(&path.to_string_lossy()))
}

fn percent_encode_path(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    for byte in path.as_bytes() {
        let ch = *byte as char;
        if ch.is_ascii_alphanumeric() || matches!(ch, '/' | '-' | '_' | '.' | '~') {
            out.push(ch);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

fn percent_decode_path(path: &str) -> PathBuf {
    let bytes = path.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            if let Ok(hex) = u8::from_str_radix(&path[index + 1..index + 3], 16) {
                out.push(hex);
                index += 3;
                continue;
            }
        }
        out.push(bytes[index]);
        index += 1;
    }
    PathBuf::from(String::from_utf8_lossy(&out).to_string())
}
