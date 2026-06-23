use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use crate::protocol::InputEvent;

pub struct PointerTrace {
    enabled: bool,
    writer: Option<BufWriter<File>>,
    start: Instant,
    role: &'static str,
    has_position: bool,
    x: i64,
    y: i64,
}

impl PointerTrace {
    fn disabled(role: &'static str) -> Self {
        Self {
            enabled: false,
            writer: None,
            start: Instant::now(),
            role,
            has_position: false,
            x: 0,
            y: 0,
        }
    }

    pub fn from_env(role: &'static str) -> Self {
        let Some(path) = trace_path(role) else {
            return Self::disabled(role);
        };

        match open_trace_file(&path) {
            Ok((file, should_write_header)) => {
                let mut writer = BufWriter::new(file);
                if should_write_header {
                    if let Err(e) = writeln!(writer, "t_ms,x,y,dx,dy,event,role,source") {
                        eprintln!(
                            "pointer trace disabled: failed to write {} header: {e}",
                            path.display()
                        );
                        return Self::disabled(role);
                    }
                }
                eprintln!("pointer trace enabled: {}", path.display());
                Self {
                    enabled: true,
                    writer: Some(writer),
                    start: Instant::now(),
                    role,
                    has_position: false,
                    x: 0,
                    y: 0,
                }
            }
            Err(e) => {
                eprintln!(
                    "pointer trace disabled: failed to open {}: {e}",
                    path.display()
                );
                Self::disabled(role)
            }
        }
    }

    pub fn observe(&mut self, event: &InputEvent, source: &'static str) {
        if !self.enabled {
            return;
        }

        let (event_name, dx, dy) = match *event {
            InputEvent::MouseEnter { x, y } => {
                self.x = x as i64;
                self.y = y as i64;
                self.has_position = true;
                ("enter", 0, 0)
            }
            InputEvent::MouseDelta { dx, dy } => {
                if !self.has_position {
                    self.has_position = true;
                }
                self.x = self.x.saturating_add(dx as i64);
                self.y = self.y.saturating_add(dy as i64);
                ("delta", dx, dy)
            }
            _ => return,
        };

        let t_ms = self.start.elapsed().as_secs_f64() * 1000.0;
        let Some(writer) = self.writer.as_mut() else {
            return;
        };
        if let Err(e) = writeln!(
            writer,
            "{t_ms:.3},{},{},{},{},{},{},{}",
            self.x, self.y, dx, dy, event_name, self.role, source
        ) {
            eprintln!("pointer trace disabled: write failed: {e}");
            self.enabled = false;
            self.writer.take();
        }
    }
}

impl Drop for PointerTrace {
    fn drop(&mut self) {
        if let Some(writer) = self.writer.as_mut() {
            let _ = writer.flush();
        }
    }
}

fn trace_path(role: &str) -> Option<PathBuf> {
    let role_key = format!("DESKBRIDGE_POINTER_TRACE_{}", role.to_ascii_uppercase());
    std::env::var_os(role_key)
        .or_else(|| std::env::var_os("DESKBRIDGE_POINTER_TRACE"))
        .map(PathBuf::from)
        .map(|target| run_trace_path(&target, role))
}

fn run_trace_path(target: &Path, role: &str) -> PathBuf {
    let stamp_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0);
    let pid = std::process::id();

    if target.extension().and_then(|value| value.to_str()) == Some("csv") {
        let parent = target.parent().unwrap_or_else(|| Path::new("."));
        let stem = target
            .file_stem()
            .and_then(|value| value.to_str())
            .filter(|value| !value.is_empty())
            .unwrap_or("deskbridge-pointer");
        return parent.join(format!("{stem}-{role}-{stamp_ms}-{pid}.csv"));
    }

    target.join(format!("deskbridge-pointer-{role}-{stamp_ms}-{pid}.csv"))
}

fn open_trace_file(path: &PathBuf) -> std::io::Result<(File, bool)> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    let file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(path)?;
    Ok((file, true))
}
