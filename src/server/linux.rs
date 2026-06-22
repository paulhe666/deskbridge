use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use super::ServerConfig;
use crate::clipboard::{Clipboard, ClipboardApi};
use crate::file_transfer;
use crate::linux::DisplayServer;
use crate::platform::{ConnectionProfile, Platform};
use crate::protocol::{self, ClipboardPayload, Frame, FrameKind};
use crate::transport::SharedWriter;

const DEFAULT_REMOTE_WIDTH: i32 = 1366;
const DEFAULT_REMOTE_HEIGHT: i32 = 768;
const REMOTE_CLIPBOARD_SUPPRESS_WINDOW: Duration = Duration::from_millis(1200);

pub fn run(config: ServerConfig) -> std::io::Result<()> {
    let display = DisplayServer::detect();
    eprintln!(
        "linux server backend active ({display}); global input capture is isolated from Windows/macOS backends",
        display = display.as_str()
    );
    eprintln!(
        "linux server currently provides protocol handshake and clipboard sync; X11/Wayland pointer capture is intentionally not coupled to other platforms"
    );

    let listener = TcpListener::bind(&config.bind)?;
    eprintln!("deskbridge linux server listening on {}", config.bind);

    let (mut stream, addr, writer, remote_size, client_platform) = loop {
        let (mut stream, addr) = listener.accept()?;
        stream.set_nodelay(true)?;
        eprintln!("client connected from {addr}");

        let writer = SharedWriter::new(stream.try_clone()?);
        if let Err(e) = writer.write(Frame::new(FrameKind::Hello, protocol::hello_payload())) {
            eprintln!("failed to send hello to {addr}: {e}; waiting for another client");
            continue;
        }

        match read_client_hello(&mut stream) {
            Ok((remote_size, client_platform)) => {
                break (stream, addr, writer, remote_size, client_platform);
            }
            Err(e) => {
                eprintln!(
                    "client {addr} disconnected during handshake: {e}; waiting for another client"
                );
                continue;
            }
        }
    };

    let remote_profile = ConnectionProfile::local_server(client_platform);
    eprintln!(
        "connection profile: {} -> {} ({})",
        Platform::local().as_str(),
        client_platform.as_str(),
        remote_profile.as_str()
    );
    eprintln!(
        "remote screen {}x{}, edge {:?}, client {}",
        remote_size.0, remote_size.1, config.edge, addr
    );

    let stop_requested = Arc::new(AtomicBool::new(false));
    let stop_stream = stream.try_clone()?;
    let stop_flag = Arc::clone(&stop_requested);
    crate::shutdown::spawn_gui_stop_watcher(move || {
        stop_flag.store(true, Ordering::Release);
        let _ = stop_stream.shutdown(Shutdown::Both);
    });

    let clipboard_state = Arc::new(Mutex::new(ClipboardSyncState::default()));
    spawn_inbound_reader(stream.try_clone()?, Arc::clone(&clipboard_state), Arc::clone(&stop_requested));
    spawn_clipboard_watcher(writer, clipboard_state, Arc::clone(&stop_requested));

    while !stop_requested.load(Ordering::Acquire) {
        thread::sleep(Duration::from_millis(250));
    }
    Ok(())
}

fn read_client_hello(stream: &mut TcpStream) -> std::io::Result<((i32, i32), Platform)> {
    let frame = protocol::read_frame(stream)?;
    if frame.kind != FrameKind::Hello {
        return Ok(((DEFAULT_REMOTE_WIDTH, DEFAULT_REMOTE_HEIGHT), Platform::Unknown));
    }
    let hello = protocol::decode_hello(&frame.payload)?;
    protocol::validate_version(hello)?;
    let width = hello.screen_width.unwrap_or(DEFAULT_REMOTE_WIDTH as u32) as i32;
    let height = hello.screen_height.unwrap_or(DEFAULT_REMOTE_HEIGHT as u32) as i32;
    Ok(((width.max(1), height.max(1)), hello.platform))
}

fn spawn_inbound_reader(
    mut stream: TcpStream,
    clipboard_state: Arc<Mutex<ClipboardSyncState>>,
    stop_requested: Arc<AtomicBool>,
) {
    thread::spawn(move || {
        let mut clipboard = match Clipboard::new() {
            Ok(clipboard) => clipboard,
            Err(e) => {
                eprintln!("clipboard receiver disabled: {e}");
                return;
            }
        };
        let receive_root = std::env::temp_dir().join("deskbridge-received");
        let mut incoming_files = file_transfer::IncomingBundle::new(receive_root);

        loop {
            let frame = match protocol::read_frame(&mut stream) {
                Ok(frame) => frame,
                Err(e) if stop_requested.load(Ordering::Acquire) => {
                    eprintln!("linux server stopped: {e}");
                    return;
                }
                Err(e) => {
                    eprintln!("connection closed: {e}");
                    stop_requested.store(true, Ordering::Release);
                    return;
                }
            };
            match frame.kind {
                FrameKind::Clipboard => match protocol::decode_clipboard(&frame.payload) {
                    Ok(payload) => {
                        eprintln!("received clipboard {}", clipboard_summary(&payload));
                        if let Err(e) = clipboard.write(&payload) {
                            eprintln!("clipboard write failed: {e}");
                        } else {
                            note_remote_clipboard(&clipboard_state, &payload);
                        }
                    }
                    Err(e) => eprintln!("clipboard decode failed: {e}"),
                },
                FrameKind::FileStart => match protocol::decode_file_start(&frame.payload) {
                    Ok((relative, len)) => {
                        if let Err(e) = incoming_files.start_file(&relative, len) {
                            eprintln!("file receive failed: {e}");
                        }
                    }
                    Err(e) => eprintln!("file start decode failed: {e}"),
                },
                FrameKind::FileChunk => {
                    if let Err(e) = incoming_files.write_chunk(&frame.payload) {
                        eprintln!("file chunk failed: {e}");
                    }
                }
                FrameKind::DragEnd => {
                    let payload = ClipboardPayload::Files(incoming_files.finish());
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
    });
}

fn spawn_clipboard_watcher(
    writer: SharedWriter,
    clipboard_state: Arc<Mutex<ClipboardSyncState>>,
    stop_requested: Arc<AtomicBool>,
) {
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
            if stop_requested.load(Ordering::Acquire) {
                return;
            }
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

fn send_clipboard_payload(writer: &SharedWriter, payload: &ClipboardPayload) -> std::io::Result<()> {
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

fn note_remote_clipboard(state: &Arc<Mutex<ClipboardSyncState>>, payload: &ClipboardPayload) {
    state.lock().unwrap().note_remote_write(payload);
}

fn clipboard_summary(payload: &ClipboardPayload) -> String {
    match payload {
        ClipboardPayload::Text(text) => format!("text({} chars)", text.chars().count()),
        ClipboardPayload::Bitmap(bytes) => format!("bitmap({} bytes)", bytes.len()),
        ClipboardPayload::Files(files) => format!("files({})", files.len()),
    }
}
