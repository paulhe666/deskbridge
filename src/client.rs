use std::net::TcpStream;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use crate::clipboard::{Clipboard, ClipboardApi};
use crate::file_transfer;
use crate::input::InputSink;
use crate::protocol::{self, ClipboardPayload, Frame, FrameKind, InputEvent};
use crate::transport::SharedWriter;

const REMOTE_CLIPBOARD_SUPPRESS_WINDOW: Duration = Duration::from_millis(1200);

pub fn run(server: &str) -> std::io::Result<()> {
    eprintln!("initializing macOS input sink");
    let mut input = InputSink::new()?;
    eprintln!("initializing macOS clipboard");
    let mut clipboard = Clipboard::new()?;
    eprintln!("connecting to {server}");
    let mut stream = TcpStream::connect(server)?;
    stream.set_nodelay(true)?;
    let writer = SharedWriter::new(stream.try_clone()?);
    let (width, height) = crate::input::screen_size();
    eprintln!("connected; sending hello with screen {width}x{height}");
    writer.write(crate::protocol::Frame::new(
        FrameKind::Hello,
        protocol::hello_payload_with_screen(width, height),
    ))?;

    eprintln!("client ready");
    let receive_root = std::env::temp_dir().join("deskbridge-received");
    let mut incoming_files = file_transfer::IncomingBundle::new(receive_root);
    let clipboard_state = Arc::new(Mutex::new(ClipboardSyncState::default()));
    spawn_clipboard_watcher(writer.clone(), Arc::clone(&clipboard_state));
    let mut input_log = InputLog::new();

    loop {
        let frame = protocol::read_frame(&mut stream)?;
        match frame.kind {
            FrameKind::Input => {
                let event = protocol::decode_input(&frame.payload)?;
                input_log.observe(&event);
                input.apply(event)?;
            }
            FrameKind::Clipboard => {
                let payload = protocol::decode_clipboard(&frame.payload)?;
                eprintln!("received clipboard {}", clipboard_summary(&payload));
                if let Err(e) = clipboard.write(&payload) {
                    eprintln!("clipboard write failed: {e}");
                } else {
                    note_remote_clipboard(&clipboard_state, &payload);
                }
            }
            FrameKind::FileStart => {
                let (relative, len) = protocol::decode_file_start(&frame.payload)?;
                incoming_files.start_file(&relative, len)?;
            }
            FrameKind::FileChunk => {
                incoming_files.write_chunk(&frame.payload)?;
            }
            FrameKind::DragEnd => {
                let files = incoming_files.finish();
                let payload = ClipboardPayload::Files(files);
                eprintln!("received clipboard {}", clipboard_summary(&payload));
                if let Err(e) = clipboard.write(&payload) {
                    eprintln!("file clipboard write failed: {e}");
                } else {
                    note_remote_clipboard(&clipboard_state, &payload);
                }
            }
            _ => {}
        }
    }
}

struct InputLog {
    enabled: bool,
    count: u64,
    last_print: Instant,
}

impl Default for InputLog {
    fn default() -> Self {
        Self {
            enabled: false,
            count: 0,
            last_print: Instant::now() - Duration::from_secs(2),
        }
    }
}

impl InputLog {
    fn new() -> Self {
        Self {
            enabled: std::env::var_os("DESKBRIDGE_INPUT_LOG").is_some(),
            ..Self::default()
        }
    }

    fn observe(&mut self, event: &InputEvent) {
        if !self.enabled {
            return;
        }
        self.count += 1;
        if self.count == 1 || self.last_print.elapsed() >= Duration::from_secs(1) {
            eprintln!("received input event #{}: {:?}", self.count, event);
            self.last_print = Instant::now();
        }
    }
}

fn spawn_clipboard_watcher(writer: SharedWriter, clipboard_state: Arc<Mutex<ClipboardSyncState>>) {
    thread::spawn(move || {
        let mut clipboard = match Clipboard::new() {
            Ok(clipboard) => clipboard,
            Err(e) => {
                eprintln!("clipboard watcher disabled: {e}");
                return;
            }
        };
        eprintln!("clipboard watcher active");
        loop {
            thread::sleep(Duration::from_millis(450));
            let payload = match clipboard.read() {
                Ok(Some(payload)) => payload,
                Ok(None) => continue,
                Err(e) => {
                    eprintln!("clipboard read failed: {e}");
                    continue;
                }
            };
            let encoded = protocol::encode_clipboard(&payload);
            if !clipboard_state
                .lock()
                .unwrap()
                .accept_local_change(&payload, encoded)
            {
                continue;
            }
            eprintln!("local clipboard changed {}", clipboard_summary(&payload));
            if let Err(e) = send_clipboard_payload(&writer, &payload) {
                eprintln!("clipboard send failed: {e}");
            } else {
                eprintln!("sent clipboard {}", clipboard_summary(&payload));
            }
        }
    });
}

fn send_clipboard_payload(
    writer: &SharedWriter,
    payload: &ClipboardPayload,
) -> std::io::Result<()> {
    match payload {
        ClipboardPayload::Files(files) => {
            file_transfer::send_files(writer, files)?;
            writer.write(Frame::new(FrameKind::DragEnd, Vec::new()))
        }
        _ => writer.write(Frame::new(
            FrameKind::Clipboard,
            protocol::encode_clipboard(payload),
        )),
    }
}

#[derive(Default)]
struct ClipboardSyncState {
    last_observed: Option<Vec<u8>>,
    suppress_next_kind: Option<ClipboardKind>,
    suppress_until: Option<Instant>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ClipboardKind {
    Text,
    Bitmap,
    Files,
}

impl ClipboardSyncState {
    fn accept_local_change(&mut self, payload: &ClipboardPayload, encoded: Vec<u8>) -> bool {
        if self.last_observed.as_ref() == Some(&encoded) {
            return false;
        }

        let kind = ClipboardKind::from_payload(payload);
        if self.should_suppress(kind) {
            eprintln!("suppressed remote clipboard echo");
            self.last_observed = Some(encoded);
            return false;
        }

        self.clear_suppression();
        self.last_observed = Some(encoded);
        true
    }

    fn note_remote_write(&mut self, payload: &ClipboardPayload) {
        self.last_observed = Some(protocol::encode_clipboard(payload));
        self.suppress_next_kind = Some(ClipboardKind::from_payload(payload));
        self.suppress_until = Some(Instant::now() + REMOTE_CLIPBOARD_SUPPRESS_WINDOW);
    }

    fn should_suppress(&mut self, kind: ClipboardKind) -> bool {
        if self.suppress_next_kind != Some(kind) {
            return false;
        }
        if self
            .suppress_until
            .map(|until| Instant::now() <= until)
            .unwrap_or(false)
        {
            self.clear_suppression();
            return true;
        }
        self.clear_suppression();
        false
    }

    fn clear_suppression(&mut self) {
        self.suppress_next_kind = None;
        self.suppress_until = None;
    }
}

impl ClipboardKind {
    fn from_payload(payload: &ClipboardPayload) -> Self {
        match payload {
            ClipboardPayload::Text(_) => Self::Text,
            ClipboardPayload::Bitmap(_) => Self::Bitmap,
            ClipboardPayload::Files(_) => Self::Files,
        }
    }
}

fn note_remote_clipboard(
    clipboard_state: &Arc<Mutex<ClipboardSyncState>>,
    payload: &ClipboardPayload,
) {
    clipboard_state.lock().unwrap().note_remote_write(payload);
}

fn clipboard_summary(payload: &ClipboardPayload) -> String {
    match payload {
        ClipboardPayload::Text(text) => format!("text ({} chars)", text.chars().count()),
        ClipboardPayload::Bitmap(bitmap) => format!("image ({} bytes)", bitmap.len()),
        ClipboardPayload::Files(files) => format!("files ({} item(s))", files.len()),
    }
}
