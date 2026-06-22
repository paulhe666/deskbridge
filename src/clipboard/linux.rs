use std::io::{self, Cursor};
use std::path::{Path, PathBuf};

use crate::clipboard::ClipboardApi;
use crate::linux::{self, DisplayServer};
use crate::protocol::ClipboardPayload;

const TEXT_TARGETS: &[&str] = &[
    "text/plain;charset=utf-8",
    "text/plain",
    "UTF8_STRING",
    "STRING",
];
const IMAGE_TARGETS: &[&str] = &["image/png", "image/jpeg", "image/bmp", "image/tiff"];
const FILE_TARGETS: &[&str] = &[
    "x-special/gnome-copied-files",
    "text/uri-list",
    "text/plain;charset=utf-8",
    "text/plain",
    "UTF8_STRING",
];

pub struct Clipboard {
    backend: ClipboardBackend,
}

#[derive(Debug, Clone, Copy)]
enum ClipboardBackend {
    Wayland,
    X11Xclip,
    X11Xsel,
}

fn select_backend(display: DisplayServer) -> io::Result<ClipboardBackend> {
    let wayland_ready = has_wayland_clipboard();
    let xclip_ready = linux::command_exists("xclip");
    let xsel_ready = linux::command_exists("xsel");

    match display {
        DisplayServer::Wayland if wayland_ready => Ok(ClipboardBackend::Wayland),
        DisplayServer::Wayland if xclip_ready => {
            eprintln!("wl-clipboard unavailable; falling back to xclip");
            Ok(ClipboardBackend::X11Xclip)
        }
        DisplayServer::Wayland if xsel_ready => {
            eprintln!("wl-clipboard unavailable; falling back to xsel for text clipboard");
            Ok(ClipboardBackend::X11Xsel)
        }
        DisplayServer::Wayland => Err(linux::unsupported(
            "Wayland clipboard requires wl-copy/wl-paste, or xclip/xsel fallback",
        )),
        DisplayServer::X11 if xclip_ready => Ok(ClipboardBackend::X11Xclip),
        DisplayServer::X11 if xsel_ready => Ok(ClipboardBackend::X11Xsel),
        DisplayServer::X11 => Err(linux::unsupported(
            "X11 clipboard requires xclip or xsel; xclip is required for images/files",
        )),
        DisplayServer::Unknown if wayland_ready => Ok(ClipboardBackend::Wayland),
        DisplayServer::Unknown if xclip_ready => Ok(ClipboardBackend::X11Xclip),
        DisplayServer::Unknown if xsel_ready => Ok(ClipboardBackend::X11Xsel),
        DisplayServer::Unknown => Err(linux::unsupported(
            "Linux clipboard requires wl-copy/wl-paste or xclip/xsel",
        )),
    }
}

impl Clipboard {
    pub fn new() -> io::Result<Self> {
        let display = DisplayServer::detect();
        let backend = select_backend(display)?;
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
        self.backend.read_text().map(|text| {
            text.filter(|text| !text.is_empty())
                .map(ClipboardPayload::Text)
        })
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
            Self::Wayland => self.read_first_target(TEXT_TARGETS)?.unwrap_or_default(),
            Self::X11Xclip => self.read_first_target(TEXT_TARGETS)?.unwrap_or_default(),
            Self::X11Xsel => linux::run_output("xsel", ["--clipboard", "--output"])?,
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
                [
                    "-selection",
                    "clipboard",
                    "-in",
                    "-target",
                    "text/plain;charset=utf-8",
                ],
                text.as_bytes(),
            )
            .or_else(|_| {
                linux::run_with_stdin("xclip", ["-selection", "clipboard", "-in"], text.as_bytes())
            }),
            Self::X11Xsel => {
                linux::run_with_stdin("xsel", ["--clipboard", "--input"], text.as_bytes())
            }
        }
    }

    fn read_bitmap(self) -> io::Result<Option<Vec<u8>>> {
        if matches!(self, Self::X11Xsel) {
            return Ok(None);
        }
        for target in IMAGE_TARGETS {
            let Some(bytes) = self.read_clipboard_target(target)? else {
                continue;
            };
            if bytes.is_empty() {
                continue;
            }
            if *target == "image/png" || is_png(&bytes) {
                return Ok(Some(bytes));
            }
            match image_to_png(&bytes) {
                Ok(png) => return Ok(Some(png)),
                Err(error) => eprintln!("linux clipboard: failed to normalize {target}: {error}"),
            }
        }
        Ok(None)
    }

    fn write_bitmap(self, bitmap: &[u8]) -> io::Result<()> {
        let png = image_to_png(bitmap).unwrap_or_else(|_| bitmap.to_vec());
        match self {
            Self::Wayland => linux::run_with_stdin("wl-copy", ["--type", "image/png"], &png),
            Self::X11Xclip => linux::run_with_stdin(
                "xclip",
                ["-selection", "clipboard", "-in", "-target", "image/png"],
                &png,
            ),
            Self::X11Xsel => Err(linux::unsupported(
                "image clipboard on X11 requires xclip; xsel only supports text",
            )),
        }
    }

    fn read_files(self) -> io::Result<Option<Vec<PathBuf>>> {
        if matches!(self, Self::X11Xsel) {
            return Ok(None);
        }
        for target in FILE_TARGETS {
            let Some(bytes) = self.read_clipboard_target(target)? else {
                continue;
            };
            let text = String::from_utf8_lossy(&bytes);
            let files = if *target == "x-special/gnome-copied-files" {
                parse_gnome_copied_files(&text)
            } else {
                parse_uri_list(&text)
            };
            if !files.is_empty() {
                return Ok(Some(files));
            }
        }
        Ok(None)
    }

    fn read_first_target(self, targets: &[&str]) -> io::Result<Option<Vec<u8>>> {
        for target in targets {
            if let Some(bytes) = self.read_clipboard_target(target)? {
                if !bytes.is_empty() {
                    return Ok(Some(bytes));
                }
            }
        }
        match self {
            Self::Wayland => linux::run_output("wl-paste", ["--no-newline"]).map(non_empty),
            Self::X11Xclip => {
                linux::run_output("xclip", ["-selection", "clipboard", "-out"]).map(non_empty)
            }
            Self::X11Xsel => linux::run_output("xsel", ["--clipboard", "--output"]).map(non_empty),
        }
    }

    fn read_clipboard_target(self, target: &str) -> io::Result<Option<Vec<u8>>> {
        let bytes = match self {
            Self::Wayland => linux::run_output("wl-paste", ["--type", target]).ok(),
            Self::X11Xclip => linux::run_output(
                "xclip",
                ["-selection", "clipboard", "-out", "-target", target],
            )
            .ok(),
            Self::X11Xsel => None,
        };
        Ok(bytes.filter(|bytes| !bytes.is_empty()))
    }

    fn write_files(self, files: &[PathBuf]) -> io::Result<()> {
        let uri_list = files
            .iter()
            .map(|file| path_to_file_uri(file))
            .collect::<Vec<_>>()
            .join("\r\n");
        let uri_list = format!("{uri_list}\r\n");
        let gnome_files = uri_list_to_gnome_copied_files(&uri_list);
        match self {
            Self::Wayland => write_wayland_files(&gnome_files, &uri_list),
            Self::X11Xclip => write_xclip_files(&gnome_files, &uri_list),
            Self::X11Xsel => Err(linux::unsupported(
                "file clipboard on X11 requires xclip; xsel only supports text",
            )),
        }
    }
}

fn non_empty(bytes: Vec<u8>) -> Option<Vec<u8>> {
    if bytes.is_empty() { None } else { Some(bytes) }
}

fn write_wayland_files(gnome_files: &str, uri_list: &str) -> io::Result<()> {
    let targets = preferred_file_write_targets();
    let mut last_error = None;
    for target in targets {
        let data = if target == "x-special/gnome-copied-files" {
            gnome_files.as_bytes()
        } else {
            uri_list.as_bytes()
        };
        match linux::run_with_stdin("wl-copy", ["--type", target], data) {
            Ok(()) => return Ok(()),
            Err(error) => last_error = Some(error),
        }
    }
    Err(last_error.unwrap_or_else(|| linux::other("wl-copy file clipboard failed")))
}

fn write_xclip_files(gnome_files: &str, uri_list: &str) -> io::Result<()> {
    let targets = preferred_file_write_targets();
    let mut last_error = None;
    for target in targets {
        let data = if target == "x-special/gnome-copied-files" {
            gnome_files.as_bytes()
        } else {
            uri_list.as_bytes()
        };
        match linux::run_with_stdin(
            "xclip",
            ["-selection", "clipboard", "-in", "-target", target],
            data,
        ) {
            Ok(()) => return Ok(()),
            Err(error) => last_error = Some(error),
        }
    }
    Err(last_error.unwrap_or_else(|| linux::other("xclip file clipboard failed")))
}

fn preferred_file_write_targets() -> [&'static str; 2] {
    if let Ok(value) = std::env::var("DESKBRIDGE_LINUX_FILE_CLIPBOARD_TARGET") {
        if value.eq_ignore_ascii_case("uri-list") || value.eq_ignore_ascii_case("text/uri-list") {
            return ["text/uri-list", "x-special/gnome-copied-files"];
        }
        if value.eq_ignore_ascii_case("gnome")
            || value.eq_ignore_ascii_case("x-special/gnome-copied-files")
        {
            return ["x-special/gnome-copied-files", "text/uri-list"];
        }
    }
    let desktop = std::env::var("XDG_CURRENT_DESKTOP")
        .unwrap_or_default()
        .to_ascii_lowercase();
    if desktop.contains("kde") || desktop.contains("plasma") {
        ["text/uri-list", "x-special/gnome-copied-files"]
    } else {
        ["x-special/gnome-copied-files", "text/uri-list"]
    }
}

fn uri_list_to_gnome_copied_files(uri_list: &str) -> String {
    let body = uri_list
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    format!("copy\n{body}\n")
}

fn has_wayland_clipboard() -> bool {
    linux::command_exists("wl-copy") && linux::command_exists("wl-paste")
}

fn parse_gnome_copied_files(value: &str) -> Vec<PathBuf> {
    let mut lines = value.lines().map(str::trim).filter(|line| !line.is_empty());
    let Some(first) = lines.next() else {
        return Vec::new();
    };
    let file_lines: Vec<&str> =
        if first.eq_ignore_ascii_case("copy") || first.eq_ignore_ascii_case("cut") {
            lines.collect()
        } else if first.starts_with("file://") {
            std::iter::once(first).chain(lines).collect()
        } else {
            Vec::new()
        };
    file_lines
        .into_iter()
        .filter_map(file_uri_to_path)
        .collect()
}

fn parse_uri_list(value: &str) -> Vec<PathBuf> {
    value
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .filter_map(file_uri_to_path)
        .collect()
}

fn file_uri_to_path(uri: &str) -> Option<PathBuf> {
    let value = uri
        .strip_prefix("file://localhost")
        .or_else(|| uri.strip_prefix("file://"))?;
    Some(percent_decode_path(value))
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

fn is_png(bytes: &[u8]) -> bool {
    bytes.starts_with(b"\x89PNG\r\n\x1a\n")
}

fn image_to_png(bytes: &[u8]) -> io::Result<Vec<u8>> {
    if is_png(bytes) {
        return Ok(bytes.to_vec());
    }
    let image = image::load_from_memory(bytes).map_err(|error| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("invalid image clipboard data: {error}"),
        )
    })?;
    let mut cursor = Cursor::new(Vec::new());
    image
        .write_to(&mut cursor, image::ImageFormat::Png)
        .map_err(|error| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("failed to encode image/png: {error}"),
            )
        })?;
    Ok(cursor.into_inner())
}
