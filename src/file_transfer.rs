use std::collections::BTreeSet;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};

use crate::protocol::{self, Frame, FrameKind};
use crate::transport::SharedWriter;

const MAX_BUNDLE_SIZE: u64 = 512 * 1024 * 1024;

pub fn send_files(writer: &SharedWriter, files: &[PathBuf]) -> std::io::Result<()> {
    let entries = collect_entries(files)?;
    let total = entries.iter().map(|entry| entry.len).sum::<u64>();
    if total > MAX_BUNDLE_SIZE {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "file bundle exceeds size limit",
        ));
    }

    for entry in entries {
        if entry.is_dir {
            continue;
        }
        writer.write(Frame::new(
            FrameKind::FileStart,
            protocol::encode_file_start(&entry.relative, entry.len),
        ))?;
        let mut file = File::open(&entry.source)?;
        let mut buffer = vec![0u8; protocol::CHUNK_SIZE];
        loop {
            let n = file.read(&mut buffer)?;
            if n == 0 {
                break;
            }
            writer.write(Frame::new(FrameKind::FileChunk, buffer[..n].to_vec()))?;
        }
        writer.write(Frame::new(FrameKind::FileEnd, Vec::new()))?;
    }
    Ok(())
}

pub struct ReceiveFile {
    file: File,
    remaining: u64,
}

pub struct IncomingBundle {
    root: PathBuf,
    current: Option<ReceiveFile>,
    received_roots: BTreeSet<PathBuf>,
    active: bool,
}

impl IncomingBundle {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            current: None,
            received_roots: BTreeSet::new(),
            active: false,
        }
    }

    pub fn start_file(&mut self, relative: &str, len: u64) -> std::io::Result<()> {
        if !self.active {
            self.reset_root()?;
            self.active = true;
        }
        if let Some(top) = top_level_path(relative)? {
            self.received_roots.insert(self.root.join(top));
        }
        self.current = Some(start_receive(&self.root, relative, len)?);
        Ok(())
    }

    pub fn write_chunk(&mut self, chunk: &[u8]) -> std::io::Result<()> {
        if let Some(file) = self.current.as_mut() {
            if file.write_chunk(chunk)? {
                self.current = None;
            }
        }
        Ok(())
    }

    pub fn finish(&mut self) -> Vec<PathBuf> {
        self.current = None;
        self.active = false;
        if self.received_roots.is_empty() {
            vec![self.root.clone()]
        } else {
            self.received_roots.iter().cloned().collect()
        }
    }

    fn reset_root(&mut self) -> std::io::Result<()> {
        self.current = None;
        self.received_roots.clear();
        if self.root.exists() {
            fs::remove_dir_all(&self.root)?;
        }
        fs::create_dir_all(&self.root)
    }
}

pub fn start_receive(root: &Path, relative: &str, len: u64) -> std::io::Result<ReceiveFile> {
    let path = root.join(safe_relative_path(relative)?);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    Ok(ReceiveFile {
        file: File::create(path)?,
        remaining: len,
    })
}

impl ReceiveFile {
    pub fn write_chunk(&mut self, chunk: &[u8]) -> std::io::Result<bool> {
        if chunk.len() as u64 > self.remaining {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "file chunk exceeds announced length",
            ));
        }
        self.file.write_all(chunk)?;
        self.remaining -= chunk.len() as u64;
        Ok(self.remaining == 0)
    }
}

struct Entry {
    source: PathBuf,
    relative: String,
    len: u64,
    is_dir: bool,
}

fn collect_entries(files: &[PathBuf]) -> std::io::Result<Vec<Entry>> {
    let mut entries = Vec::new();
    for file in files {
        let Some(name) = file.file_name() else {
            continue;
        };
        collect(file, Path::new(name), &mut entries)?;
    }
    Ok(entries)
}

fn collect(path: &Path, relative: &Path, entries: &mut Vec<Entry>) -> std::io::Result<()> {
    let metadata = fs::metadata(path)?;
    if metadata.is_dir() {
        entries.push(Entry {
            source: path.to_path_buf(),
            relative: relative_to_string(relative)?,
            len: 0,
            is_dir: true,
        });
        for child in fs::read_dir(path)? {
            let child = child?;
            collect(&child.path(), &relative.join(child.file_name()), entries)?;
        }
    } else if metadata.is_file() {
        entries.push(Entry {
            source: path.to_path_buf(),
            relative: relative_to_string(relative)?,
            len: metadata.len(),
            is_dir: false,
        });
    }
    Ok(())
}

fn safe_relative_path(value: &str) -> std::io::Result<PathBuf> {
    let path = Path::new(value);
    if path.is_absolute()
        || path
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
    {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "unsafe relative path",
        ));
    }
    Ok(path.to_path_buf())
}

fn top_level_path(value: &str) -> std::io::Result<Option<PathBuf>> {
    let path = safe_relative_path(value)?;
    Ok(path.components().next().and_then(|component| {
        if let Component::Normal(part) = component {
            Some(PathBuf::from(part))
        } else {
            None
        }
    }))
}

fn relative_to_string(path: &Path) -> std::io::Result<String> {
    let mut parts = Vec::new();
    for component in path.components() {
        let Component::Normal(part) = component else {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "unsafe relative path",
            ));
        };
        parts.push(part.to_string_lossy().into_owned());
    }
    Ok(parts.join("/"))
}
