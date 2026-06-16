use std::net::TcpStream;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use crate::clipboard::{Clipboard, ClipboardApi};
use crate::file_transfer;
use crate::input::InputSink;
use crate::protocol::{self, ClipboardPayload, Frame, FrameKind, InputEvent};
use crate::transport::SharedWriter;

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
    let last_clipboard = Arc::new(Mutex::new(None));
    spawn_clipboard_watcher(writer.clone(), Arc::clone(&last_clipboard));
    let mut input_log = InputLog::default();

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
                remember_clipboard(&last_clipboard, &payload);
                clipboard.write(&payload)?;
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
                remember_clipboard(&last_clipboard, &payload);
                clipboard.write(&payload)?;
            }
            _ => {}
        }
    }
}

struct InputLog {
    count: u64,
    last_print: Instant,
}

impl Default for InputLog {
    fn default() -> Self {
        Self {
            count: 0,
            last_print: Instant::now() - Duration::from_secs(2),
        }
    }
}

impl InputLog {
    fn observe(&mut self, event: &InputEvent) {
        self.count += 1;
        if self.count == 1 || self.last_print.elapsed() >= Duration::from_secs(1) {
            eprintln!("received input event #{}: {:?}", self.count, event);
            self.last_print = Instant::now();
        }
    }
}

fn spawn_clipboard_watcher(writer: SharedWriter, last_clipboard: Arc<Mutex<Option<Vec<u8>>>>) {
    thread::spawn(move || {
        let mut clipboard = match Clipboard::new() {
            Ok(clipboard) => clipboard,
            Err(e) => {
                eprintln!("clipboard watcher disabled: {e}");
                return;
            }
        };
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
            {
                let mut last = last_clipboard.lock().unwrap();
                if last.as_ref() == Some(&encoded) {
                    continue;
                }
                *last = Some(encoded);
            }
            if let Err(e) = send_clipboard_payload(&writer, &payload) {
                eprintln!("clipboard send failed: {e}");
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

fn remember_clipboard(last_clipboard: &Arc<Mutex<Option<Vec<u8>>>>, payload: &ClipboardPayload) {
    *last_clipboard.lock().unwrap() = Some(protocol::encode_clipboard(payload));
}
