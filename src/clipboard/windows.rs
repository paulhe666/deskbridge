use std::mem::{size_of, zeroed};
use std::os::windows::ffi::OsStrExt;
use std::path::PathBuf;
use std::{ptr, slice};

use windows_sys::Win32::Foundation::{GlobalFree, HANDLE, HGLOBAL, POINT};
use windows_sys::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, GetClipboardData, IsClipboardFormatAvailable, OpenClipboard,
    SetClipboardData,
};
use windows_sys::Win32::System::Memory::{
    GMEM_MOVEABLE, GMEM_ZEROINIT, GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock,
};
use windows_sys::Win32::System::Ole::{CF_DIB, CF_HDROP, CF_UNICODETEXT};
use windows_sys::Win32::UI::Shell::{DROPFILES, DragQueryFileW, HDROP};

use crate::clipboard::ClipboardApi;
use crate::protocol::ClipboardPayload;

const BMP_FILE_HEADER_LEN: usize = 14;
const BI_RGB: u32 = 0;

pub struct Clipboard;

impl Clipboard {
    pub fn new() -> std::io::Result<Self> {
        Ok(Self)
    }
}

impl ClipboardApi for Clipboard {
    fn read(&mut self) -> std::io::Result<Option<ClipboardPayload>> {
        let _guard = ClipboardGuard::open()?;
        if unsafe { IsClipboardFormatAvailable(CF_HDROP as u32) } != 0 {
            if let Some(files) = read_files()? {
                return Ok(Some(ClipboardPayload::Files(files)));
            }
        }
        if unsafe { IsClipboardFormatAvailable(CF_DIB as u32) } != 0 {
            if let Some(bitmap) = read_bitmap()? {
                return Ok(Some(ClipboardPayload::Bitmap(bitmap)));
            }
        }
        if unsafe { IsClipboardFormatAvailable(CF_UNICODETEXT as u32) } != 0 {
            if let Some(text) = read_text()? {
                return Ok(Some(ClipboardPayload::Text(text)));
            }
        }
        Ok(None)
    }

    fn write(&mut self, payload: &ClipboardPayload) -> std::io::Result<()> {
        let _guard = ClipboardGuard::open()?;
        if unsafe { EmptyClipboard() } == 0 {
            return Err(last_os_error("EmptyClipboard failed"));
        }
        match payload {
            ClipboardPayload::Text(text) => write_text(text),
            ClipboardPayload::Bitmap(bitmap) => write_bitmap(bitmap),
            ClipboardPayload::Files(files) => write_files(files),
        }
    }
}

struct ClipboardGuard;

impl ClipboardGuard {
    fn open() -> std::io::Result<Self> {
        if unsafe { OpenClipboard(ptr::null_mut()) } == 0 {
            return Err(last_os_error("OpenClipboard failed"));
        }
        Ok(Self)
    }
}

impl Drop for ClipboardGuard {
    fn drop(&mut self) {
        unsafe {
            CloseClipboard();
        }
    }
}

fn read_text() -> std::io::Result<Option<String>> {
    let handle = unsafe { GetClipboardData(CF_UNICODETEXT as u32) };
    if handle.is_null() {
        return Ok(None);
    }
    let bytes = copy_global(handle as HGLOBAL)?;
    let words = bytes
        .chunks_exact(2)
        .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]]))
        .take_while(|ch| *ch != 0)
        .collect::<Vec<_>>();
    if words.is_empty() {
        Ok(None)
    } else {
        Ok(Some(String::from_utf16_lossy(&words)))
    }
}

fn write_text(text: &str) -> std::io::Result<()> {
    let mut wide = text.encode_utf16().collect::<Vec<_>>();
    wide.push(0);
    let bytes = wide
        .iter()
        .flat_map(|ch| ch.to_le_bytes())
        .collect::<Vec<_>>();
    set_clipboard_bytes(CF_UNICODETEXT as u32, &bytes)
}

fn read_bitmap() -> std::io::Result<Option<Vec<u8>>> {
    let handle = unsafe { GetClipboardData(CF_DIB as u32) };
    if handle.is_null() {
        return Ok(None);
    }
    let dib = copy_global(handle as HGLOBAL)?;
    if dib.is_empty() {
        return Ok(None);
    }
    dib_to_bmp(&dib).map(Some)
}

fn write_bitmap(bitmap: &[u8]) -> std::io::Result<()> {
    let dib = bmp_to_dib(bitmap)?;
    set_clipboard_bytes(CF_DIB as u32, &dib)
}

fn read_files() -> std::io::Result<Option<Vec<PathBuf>>> {
    let handle = unsafe { GetClipboardData(CF_HDROP as u32) };
    if handle.is_null() {
        return Ok(None);
    }
    let hdrop = handle as HDROP;
    let count = unsafe { DragQueryFileW(hdrop, u32::MAX, ptr::null_mut(), 0) };
    if count == 0 {
        return Ok(None);
    }

    let mut files = Vec::new();
    for i in 0..count {
        let len = unsafe { DragQueryFileW(hdrop, i, ptr::null_mut(), 0) };
        if len == 0 {
            continue;
        }
        let mut buf = vec![0u16; len as usize + 1];
        let written = unsafe { DragQueryFileW(hdrop, i, buf.as_mut_ptr(), buf.len() as u32) };
        if written != 0 {
            files.push(PathBuf::from(String::from_utf16_lossy(
                &buf[..written as usize],
            )));
        }
    }
    if files.is_empty() {
        Ok(None)
    } else {
        Ok(Some(files))
    }
}

fn write_files(files: &[PathBuf]) -> std::io::Result<()> {
    let mut list = Vec::<u16>::new();
    for file in files {
        let path = if file.is_absolute() {
            file.clone()
        } else {
            std::env::current_dir()?.join(file)
        };
        list.extend(path.as_os_str().encode_wide());
        list.push(0);
    }
    list.push(0);

    let header_len = size_of::<DROPFILES>();
    let bytes_len = header_len + list.len() * size_of::<u16>();
    let handle = unsafe { GlobalAlloc(GMEM_MOVEABLE | GMEM_ZEROINIT, bytes_len) };
    if handle.is_null() {
        return Err(last_os_error("GlobalAlloc failed"));
    }

    let memory = unsafe { GlobalLock(handle) as *mut u8 };
    if memory.is_null() {
        unsafe {
            GlobalFree(handle);
        }
        return Err(last_os_error("GlobalLock failed"));
    }

    unsafe {
        let mut dropfiles: DROPFILES = zeroed();
        dropfiles.pFiles = header_len as u32;
        dropfiles.pt = POINT { x: 0, y: 0 };
        dropfiles.fNC = 0;
        dropfiles.fWide = 1;
        ptr::copy_nonoverlapping(
            &dropfiles as *const DROPFILES as *const u8,
            memory,
            header_len,
        );
        ptr::copy_nonoverlapping(
            list.as_ptr() as *const u8,
            memory.add(header_len),
            list.len() * size_of::<u16>(),
        );
        GlobalUnlock(handle);
    }

    let result = unsafe { SetClipboardData(CF_HDROP as u32, handle as HANDLE) };
    if result.is_null() {
        unsafe {
            GlobalFree(handle);
        }
        Err(last_os_error("SetClipboardData failed"))
    } else {
        Ok(())
    }
}

fn copy_global(handle: HGLOBAL) -> std::io::Result<Vec<u8>> {
    let len = unsafe { GlobalSize(handle) };
    if len == 0 {
        return Ok(Vec::new());
    }
    let ptr = unsafe { GlobalLock(handle) as *const u8 };
    if ptr.is_null() {
        return Err(last_os_error("GlobalLock failed"));
    }
    let bytes = unsafe { slice::from_raw_parts(ptr, len).to_vec() };
    unsafe {
        GlobalUnlock(handle);
    }
    Ok(bytes)
}

fn set_clipboard_bytes(format: u32, bytes: &[u8]) -> std::io::Result<()> {
    let handle = unsafe { GlobalAlloc(GMEM_MOVEABLE, bytes.len()) };
    if handle.is_null() {
        return Err(last_os_error("GlobalAlloc failed"));
    }
    let memory = unsafe { GlobalLock(handle) as *mut u8 };
    if memory.is_null() {
        unsafe {
            GlobalFree(handle);
        }
        return Err(last_os_error("GlobalLock failed"));
    }
    unsafe {
        ptr::copy_nonoverlapping(bytes.as_ptr(), memory, bytes.len());
        GlobalUnlock(handle);
    }
    let result = unsafe { SetClipboardData(format, handle as HANDLE) };
    if result.is_null() {
        unsafe {
            GlobalFree(handle);
        }
        Err(last_os_error("SetClipboardData failed"))
    } else {
        Ok(())
    }
}

fn dib_to_bmp(dib: &[u8]) -> std::io::Result<Vec<u8>> {
    let pixel_offset = dib_pixel_offset(dib)? + BMP_FILE_HEADER_LEN;
    let file_size = BMP_FILE_HEADER_LEN + dib.len();
    let mut bmp = Vec::with_capacity(file_size);
    bmp.extend_from_slice(b"BM");
    bmp.extend_from_slice(&(file_size as u32).to_le_bytes());
    bmp.extend_from_slice(&[0, 0, 0, 0]);
    bmp.extend_from_slice(&(pixel_offset as u32).to_le_bytes());
    bmp.extend_from_slice(dib);
    Ok(bmp)
}

fn bmp_to_dib(bitmap: &[u8]) -> std::io::Result<Vec<u8>> {
    if bitmap.len() >= BMP_FILE_HEADER_LEN && &bitmap[..2] == b"BM" {
        Ok(bitmap[BMP_FILE_HEADER_LEN..].to_vec())
    } else {
        dib_pixel_offset(bitmap)?;
        Ok(bitmap.to_vec())
    }
}

fn dib_pixel_offset(dib: &[u8]) -> std::io::Result<usize> {
    if dib.len() < 4 {
        return Err(invalid_data("short DIB header"));
    }
    let header_len = u32::from_le_bytes(dib[0..4].try_into().unwrap()) as usize;
    if dib.len() < header_len || header_len < 12 {
        return Err(invalid_data("invalid DIB header length"));
    }
    if header_len == 12 {
        return Ok(header_len);
    }
    if dib.len() < 36 {
        return Err(invalid_data("short BITMAPINFOHEADER"));
    }

    let bit_count = u16::from_le_bytes(dib[14..16].try_into().unwrap());
    let compression = u32::from_le_bytes(dib[16..20].try_into().unwrap());
    let clr_used = u32::from_le_bytes(dib[32..36].try_into().unwrap()) as usize;
    let palette_entries = if clr_used != 0 {
        clr_used
    } else if bit_count <= 8 {
        1usize << bit_count
    } else {
        0
    };
    let masks_len = if compression == BI_RGB {
        0
    } else if matches!(compression, 3 | 6) && header_len == 40 {
        12
    } else {
        0
    };
    Ok(header_len + masks_len + palette_entries * 4)
}

fn invalid_data(message: &str) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, message)
}

fn last_os_error(context: &str) -> std::io::Error {
    let error = std::io::Error::last_os_error();
    std::io::Error::new(error.kind(), format!("{context}: {error}"))
}
