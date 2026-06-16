use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use crate::clipboard::ClipboardApi;
use crate::protocol::ClipboardPayload;

pub struct Clipboard;

impl Clipboard {
    pub fn new() -> std::io::Result<Self> {
        Ok(Self)
    }
}

impl ClipboardApi for Clipboard {
    fn read(&mut self) -> std::io::Result<Option<ClipboardPayload>> {
        if let Some(files) = read_files()? {
            return Ok(Some(ClipboardPayload::Files(files)));
        }
        if let Some(bitmap) = read_bitmap()? {
            return Ok(Some(ClipboardPayload::Bitmap(bitmap)));
        }
        read_text().map(|text| text.map(ClipboardPayload::Text))
    }

    fn write(&mut self, payload: &ClipboardPayload) -> std::io::Result<()> {
        match payload {
            ClipboardPayload::Text(text) => write_text(text),
            ClipboardPayload::Bitmap(bitmap) => write_bitmap(bitmap),
            ClipboardPayload::Files(files) => write_files(files),
        }
    }
}

fn read_text() -> std::io::Result<Option<String>> {
    let output = Command::new("pbpaste").args(["-Prefer", "txt"]).output()?;
    if !output.status.success() || output.stdout.is_empty() {
        return Ok(None);
    }
    let Ok(text) = String::from_utf8(output.stdout) else {
        return Ok(None);
    };
    Ok(Some(text.replace("\r\n", "\n").replace('\r', "\n")))
}

fn write_text(text: &str) -> std::io::Result<()> {
    write_filter("pbcopy", &[], text.as_bytes()).map(|_| ())
}

fn read_bitmap() -> std::io::Result<Option<Vec<u8>>> {
    let script = r#"ObjC.import("AppKit");
ObjC.import("Foundation");
function exists(value) { return value !== undefined && value !== null && !(value.isNil && value.isNil()); }
function bmpDataFromImageData(data) {
  if (!exists(data)) return null;
  const image = $.NSImage.alloc.initWithData(data);
  if (!exists(image) || !image.isValid) return null;
  const tiff = image.TIFFRepresentation;
  if (!exists(tiff)) return null;
  const rep = $.NSBitmapImageRep.imageRepWithData(tiff);
  if (!exists(rep)) return null;
  return rep.representationUsingTypeProperties(1, $({}));
}
const pasteboard = $.NSPasteboard.generalPasteboard;
let out = null;
for (const type of ["com.microsoft.bmp", "public.bmp", "com.apple.pict"]) {
  const data = pasteboard.dataForType(type);
  if (exists(data)) {
    out = data;
    break;
  }
}
if (!exists(out)) {
  for (const type of ["public.png", "public.tiff", "public.jpeg", "public.heic"]) {
    out = bmpDataFromImageData(pasteboard.dataForType(type));
    if (exists(out)) break;
  }
}
if (!exists(out)) {
  const image = $.NSImage.alloc.initWithPasteboard(pasteboard);
  if (exists(image) && image.isValid) {
    const rep = $.NSBitmapImageRep.imageRepWithData(image.TIFFRepresentation);
    if (exists(rep)) out = rep.representationUsingTypeProperties(1, $({}));
  }
}
if (exists(out)) $.NSFileHandle.fileHandleWithStandardOutput.writeData(out);
"#;
    let output = Command::new("osascript")
        .args(["-l", "JavaScript", "-e", script])
        .output()?;
    if output.status.success() && !output.stdout.is_empty() {
        Ok(Some(output.stdout))
    } else {
        Ok(None)
    }
}

fn write_bitmap(bmp: &[u8]) -> std::io::Result<()> {
    let script = r#"ObjC.import("AppKit");
ObjC.import("Foundation");
const data = $.NSFileHandle.fileHandleWithStandardInput.readDataToEndOfFile;
const pasteboard = $.NSPasteboard.generalPasteboard;
pasteboard.clearContents;
if (!pasteboard.setDataForType(data, "com.microsoft.bmp")) throw new Error("setDataForType failed");
"#;
    write_filter("osascript", &["-l", "JavaScript", "-e", script], bmp).map(|_| ())
}

fn read_files() -> std::io::Result<Option<Vec<PathBuf>>> {
    let script = r#"ObjC.import("AppKit");
ObjC.import("Foundation");
const pasteboard = $.NSPasteboard.generalPasteboard;
const urls = pasteboard.readObjectsForClassesOptions($[$.NSURL.class], $({}));
if (urls) {
  for (let i = 0; i < urls.count; i++) {
    const path = urls.objectAtIndex(i).path;
    if (path) console.log(ObjC.unwrap(path));
  }
}
"#;
    let output = Command::new("osascript")
        .args(["-l", "JavaScript", "-e", script])
        .output()?;
    if !output.status.success() || output.stdout.is_empty() {
        return Ok(None);
    }
    let files = String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(PathBuf::from)
        .filter(|path| path.exists())
        .collect::<Vec<_>>();
    if files.is_empty() {
        Ok(None)
    } else {
        Ok(Some(files))
    }
}

fn write_files(files: &[PathBuf]) -> std::io::Result<()> {
    let joined = files
        .iter()
        .map(|path| path.to_string_lossy())
        .collect::<Vec<_>>()
        .join("\n");
    let script = r#"ObjC.import("AppKit");
ObjC.import("Foundation");
const input = $.NSString.alloc.initWithDataEncoding(
  $.NSFileHandle.fileHandleWithStandardInput.readDataToEndOfFile,
  $.NSUTF8StringEncoding
);
const paths = ObjC.unwrap(input).split("\n").filter(Boolean);
const urls = $.NSMutableArray.array;
for (const path of paths) urls.addObject($.NSURL.fileURLWithPath(path));
const pasteboard = $.NSPasteboard.generalPasteboard;
pasteboard.clearContents;
pasteboard.writeObjects(urls);
"#;
    write_filter(
        "osascript",
        &["-l", "JavaScript", "-e", script],
        joined.as_bytes(),
    )
    .map(|_| ())
}

fn write_filter(program: &str, args: &[&str], data: &[u8]) -> std::io::Result<Vec<u8>> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()?;
    let Some(mut stdin) = child.stdin.take() else {
        return Err(std::io::Error::new(
            std::io::ErrorKind::BrokenPipe,
            "stdin unavailable",
        ));
    };
    stdin.write_all(data)?;
    drop(stdin);
    let output = child.wait_with_output()?;
    if output.status.success() {
        Ok(output.stdout)
    } else {
        Err(std::io::Error::new(
            std::io::ErrorKind::Other,
            "clipboard command failed",
        ))
    }
}
