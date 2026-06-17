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
        if has_file_markers()? {
            eprintln!("file clipboard markers found, but no readable file paths were resolved");
            return Ok(None);
        }
        if let Some(text) = read_text()? {
            return Ok(Some(ClipboardPayload::Text(text)));
        }
        if let Some(bitmap) = read_bitmap()? {
            return Ok(Some(ClipboardPayload::Bitmap(bitmap)));
        }
        Ok(None)
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
    let script = r#"ObjC.import("AppKit");
ObjC.import("Foundation");
function exists(value) { return value !== undefined && value !== null && !(value.isNil && value.isNil()); }
const pasteboard = $.NSPasteboard.generalPasteboard;
for (const type of ["public.utf8-plain-text", "NSStringPboardType", "public.utf16-plain-text"]) {
  const value = pasteboard.stringForType(type);
  if (exists(value)) {
    const text = ObjC.unwrap(value);
    if (text.length > 0) {
      const data = $.NSString.alloc.initWithUTF8String(text).dataUsingEncoding($.NSUTF8StringEncoding);
      $.NSFileHandle.fileHandleWithStandardOutput.writeData(data);
      break;
    }
  }
}
"#;
    let output = Command::new("osascript")
        .args(["-l", "JavaScript", "-e", script])
        .output()?;
    if !output.status.success() || output.stdout.is_empty() {
        return Ok(None);
    }
    let text = String::from_utf8(output.stdout)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    Ok(Some(text.replace("\r\n", "\n").replace('\r', "\n")))
}

fn write_text(text: &str) -> std::io::Result<()> {
    write_filter("pbcopy", &[], text.as_bytes()).map(|_| ())
}

fn read_bitmap() -> std::io::Result<Option<Vec<u8>>> {
    let script = r#"ObjC.import("AppKit");
ObjC.import("Foundation");
function exists(value) { return value !== undefined && value !== null && !(value.isNil && value.isNil()); }
function pngDataFromImage(image) {
  if (!exists(image) || !image.isValid) return null;
  const tiff = image.TIFFRepresentation;
  if (!exists(tiff)) return null;
  const rep = $.NSBitmapImageRep.imageRepWithData(tiff);
  if (!exists(rep)) return null;
  return rep.representationUsingTypeProperties(4, $({}));
}
function pngDataFromPasteboardData(data) {
  if (!exists(data)) return null;
  const image = $.NSImage.alloc.initWithData(data);
  return pngDataFromImage(image);
}
function hasType(types, expected) {
  for (let i = 0; types && !types.isNil() && i < types.count; i++) {
    if (ObjC.unwrap(types.objectAtIndex(i)) === expected) return true;
  }
  return false;
}
const pasteboard = $.NSPasteboard.generalPasteboard;
const availableTypes = pasteboard.types;
const imageTypes = ["public.png", "public.tiff", "public.jpeg", "public.heic", "com.microsoft.bmp", "public.bmp", "com.apple.pict"];
let out = null;
for (const type of imageTypes) {
  if (!hasType(availableTypes, type)) continue;
  out = pngDataFromPasteboardData(pasteboard.dataForType(type));
  if (exists(out)) break;
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
let wrote = pasteboard.setDataForType(data, "com.microsoft.bmp");
const image = $.NSImage.alloc.initWithData(data);
if (image && !image.isNil() && image.isValid) {
  const tiff = image.TIFFRepresentation;
  if (tiff && !tiff.isNil()) {
    wrote = pasteboard.setDataForType(tiff, "public.tiff") || wrote;
    const rep = $.NSBitmapImageRep.imageRepWithData(tiff);
    if (rep && !rep.isNil()) {
      const png = rep.representationUsingTypeProperties(4, $({}));
      if (png && !png.isNil()) wrote = pasteboard.setDataForType(png, "public.png") || wrote;
    }
  }
}
if (!wrote) throw new Error("set image data failed");
"#;
    write_filter("osascript", &["-l", "JavaScript", "-e", script], bmp).map(|_| ())
}

fn read_files() -> std::io::Result<Option<Vec<PathBuf>>> {
    let script = r#"ObjC.import("AppKit");
ObjC.import("Foundation");
const pasteboard = $.NSPasteboard.generalPasteboard;
const seen = {};
function emitPath(path) {
  if (!path || path.isNil && path.isNil()) return;
  const value = ObjC.unwrap(path);
  if (value && !seen[value]) {
    seen[value] = true;
    console.log(value);
  }
}
function emitUrl(url) {
  if (!url || url.isNil && url.isNil()) return;
  const filePathURL = url.filePathURL;
  if (filePathURL && !filePathURL.isNil()) {
    emitPath(filePathURL.path);
    return;
  }
  emitPath(url.path);
}
function emitFileUrl(value) {
  if (!value || value.isNil && value.isNil()) return;
  const string = ObjC.unwrap(value);
  if (!string) return;
  const url = $.NSURL.URLWithString(string);
  emitUrl(url);
}
const urls = pasteboard.readObjectsForClassesOptions($[$.NSURL.class], $({}));
if (urls && !urls.isNil()) {
  for (let i = 0; i < urls.count; i++) {
    emitUrl(urls.objectAtIndex(i));
  }
}
const legacy = pasteboard.propertyListForType("NSFilenamesPboardType");
if (legacy && !legacy.isNil()) {
  for (let i = 0; i < legacy.count; i++) emitPath(legacy.objectAtIndex(i));
}
const items = pasteboard.pasteboardItems;
if (items && !items.isNil()) {
  for (let i = 0; i < items.count; i++) {
    const item = items.objectAtIndex(i);
    for (const type of ["public.file-url", "com.apple.pasteboard.promised-file-url"]) {
      emitFileUrl(item.stringForType(type));
    }
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

fn has_file_markers() -> std::io::Result<bool> {
    let script = r#"ObjC.import("AppKit");
const pasteboard = $.NSPasteboard.generalPasteboard;
const fileTypes = {
  "public.file-url": true,
  "NSFilenamesPboardType": true,
  "com.apple.finder.noderef": true,
  "com.apple.pasteboard.promised-file-url": true,
  "Apple URL pasteboard type": true
};
const types = pasteboard.types;
let found = false;
for (let i = 0; types && !types.isNil() && i < types.count; i++) {
  if (fileTypes[ObjC.unwrap(types.objectAtIndex(i))]) found = true;
}
const items = pasteboard.pasteboardItems;
for (let i = 0; items && !items.isNil() && i < items.count; i++) {
  const itemTypes = items.objectAtIndex(i).types;
  for (let j = 0; itemTypes && !itemTypes.isNil() && j < itemTypes.count; j++) {
    if (fileTypes[ObjC.unwrap(itemTypes.objectAtIndex(j))]) found = true;
  }
}
if (found) console.log("1");
"#;
    let output = Command::new("osascript")
        .args(["-l", "JavaScript", "-e", script])
        .output()?;
    Ok(output.status.success() && !output.stdout.is_empty())
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
let wrote = pasteboard.writeObjects(urls);
if (!wrote) {
  const fallbackItems = $.NSMutableArray.array;
  for (let i = 0; i < urls.count; i++) {
    const url = urls.objectAtIndex(i);
    const item = $.NSPasteboardItem.alloc.init;
    item.setStringForType(url.absoluteString, "public.file-url");
    item.setStringForType(url.absoluteString, "NSURLPboardType");
    fallbackItems.addObject(item);
  }
  wrote = pasteboard.writeObjects(fallbackItems);
}
if (!wrote) throw new Error("write file URLs failed");
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
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(std::io::Error::new(
            std::io::ErrorKind::Other,
            format!("clipboard command failed: {stderr}"),
        ))
    }
}
