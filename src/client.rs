use std::net::{Shutdown, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use crate::clipboard::{Clipboard, ClipboardApi};
use crate::file_transfer;
use crate::input::InputSink;
use crate::protocol::{self, ClipboardPayload, Frame, FrameKind, InputEvent};
use crate::transport::SharedWriter;

const REMOTE_CLIPBOARD_SUPPRESS_WINDOW: Duration = Duration::from_millis(1200);
const DEFAULT_INPUT_FLUSH_MS: u64 = 2;
const INPUT_BATCH_LIMIT: usize = 64;

pub fn run(server: &str) -> std::io::Result<()> {
    eprintln!("initializing local input sink");
    let input = InputApplier::spawn()?;
    eprintln!("initializing local clipboard");
    let mut clipboard = Clipboard::new()?;
    eprintln!("connecting to {server}");
    let mut stream = TcpStream::connect(server)?;
    stream.set_nodelay(true)?;
    let writer = SharedWriter::new(stream.try_clone()?);
    let stop_requested = Arc::new(AtomicBool::new(false));
    let stop_stream = stream.try_clone()?;
    let stop_flag = Arc::clone(&stop_requested);
    crate::shutdown::spawn_gui_stop_watcher(move || {
        stop_flag.store(true, Ordering::Release);
        let _ = stop_stream.shutdown(Shutdown::Both);
    });
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
        let frame = match protocol::read_frame(&mut stream) {
            Ok(frame) => frame,
            Err(_) if stop_requested.load(Ordering::Acquire) => return Ok(()),
            Err(e) => return Err(e),
        };
        match frame.kind {
            FrameKind::Input => {
                let event = protocol::decode_input(&frame.payload)?;
                input_log.observe(&event);
                input.send(event);
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

struct InputApplier {
    sender: Option<Sender<InputEvent>>,
    worker: Option<thread::JoinHandle<()>>,
}

impl InputApplier {
    fn spawn() -> std::io::Result<Self> {
        let (sender, receiver) = mpsc::channel();
        let (ready_sender, ready_receiver) = mpsc::sync_channel(1);
        let worker = thread::spawn(move || match InputSink::new() {
            Ok(mut input) => {
                let _ = ready_sender.send(Ok(()));
                input_worker_loop(&mut input, receiver);
            }
            Err(e) => {
                let _ = ready_sender.send(Err(e.to_string()));
            }
        });
        match ready_receiver.recv() {
            Ok(Ok(())) => {}
            Ok(Err(e)) => return Err(std::io::Error::new(std::io::ErrorKind::Other, e)),
            Err(e) => return Err(std::io::Error::new(std::io::ErrorKind::BrokenPipe, e)),
        }
        Ok(Self {
            sender: Some(sender),
            worker: Some(worker),
        })
    }

    fn send(&self, event: InputEvent) {
        let Some(sender) = self.sender.as_ref() else {
            return;
        };
        if let Err(e) = sender.send(event) {
            eprintln!("input apply queue closed: {e}");
        }
    }
}

impl Drop for InputApplier {
    fn drop(&mut self) {
        self.sender.take();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn input_worker_loop(input: &mut InputSink, receiver: Receiver<InputEvent>) {
    let mut pending_delta = (0i32, 0i32);
    let mut pending_wheel = (0i32, 0i32);
    let mut pending_since = None;

    loop {
        let event = match recv_input_event(&receiver, pending_since) {
            Ok(Some(event)) => event,
            Ok(None) => {
                flush_pending_input(
                    input,
                    &mut pending_delta,
                    &mut pending_wheel,
                    &mut pending_since,
                );
                continue;
            }
            Err(()) => break,
        };

        match event {
            event @ (InputEvent::MouseDelta { .. } | InputEvent::MouseWheel { .. }) => {
                queue_pending_input(
                    event,
                    &mut pending_delta,
                    &mut pending_wheel,
                    &mut pending_since,
                );
                drain_queued_input(
                    input,
                    &receiver,
                    &mut pending_delta,
                    &mut pending_wheel,
                    &mut pending_since,
                );
            }
            event => {
                flush_pending_input(
                    input,
                    &mut pending_delta,
                    &mut pending_wheel,
                    &mut pending_since,
                );
                apply_input_event(input, event);
            }
        }
    }
}

fn recv_input_event(
    receiver: &Receiver<InputEvent>,
    pending_since: Option<Instant>,
) -> Result<Option<InputEvent>, ()> {
    match pending_since {
        Some(_) => match receiver.recv_timeout(input_flush_timeout(pending_since)) {
            Ok(event) => Ok(Some(event)),
            Err(RecvTimeoutError::Timeout) => Ok(None),
            Err(RecvTimeoutError::Disconnected) => Err(()),
        },
        None => receiver.recv().map(Some).map_err(|_| ()),
    }
}

fn queue_pending_input(
    event: InputEvent,
    pending_delta: &mut (i32, i32),
    pending_wheel: &mut (i32, i32),
    pending_since: &mut Option<Instant>,
) {
    if pending_since.is_none() {
        *pending_since = Some(Instant::now());
    }
    match event {
        InputEvent::MouseDelta { dx, dy } => {
            pending_delta.0 = pending_delta.0.saturating_add(dx);
            pending_delta.1 = pending_delta.1.saturating_add(dy);
        }
        InputEvent::MouseWheel {
            horizontal,
            vertical,
        } => {
            pending_wheel.0 = pending_wheel.0.saturating_add(horizontal as i32);
            pending_wheel.1 = pending_wheel.1.saturating_add(vertical as i32);
        }
        _ => {}
    }
}

fn drain_queued_input(
    input: &mut InputSink,
    receiver: &Receiver<InputEvent>,
    pending_delta: &mut (i32, i32),
    pending_wheel: &mut (i32, i32),
    pending_since: &mut Option<Instant>,
) {
    for _ in 0..INPUT_BATCH_LIMIT {
        match receiver.try_recv() {
            Ok(event @ (InputEvent::MouseDelta { .. } | InputEvent::MouseWheel { .. })) => {
                queue_pending_input(event, pending_delta, pending_wheel, pending_since);
            }
            Ok(event) => {
                flush_pending_input(input, pending_delta, pending_wheel, pending_since);
                apply_input_event(input, event);
                return;
            }
            Err(TryRecvError::Empty) => break,
            Err(TryRecvError::Disconnected) => return,
        }
    }

    if pending_ready(*pending_since) {
        flush_pending_input(input, pending_delta, pending_wheel, pending_since);
    }
}

fn input_flush_timeout(pending_since: Option<Instant>) -> Duration {
    pending_since
        .map(|since| {
            input_flush_interval()
                .checked_sub(since.elapsed())
                .unwrap_or(Duration::ZERO)
        })
        .unwrap_or_else(input_flush_interval)
}

fn pending_ready(pending_since: Option<Instant>) -> bool {
    pending_since
        .map(|since| since.elapsed() >= input_flush_interval())
        .unwrap_or(false)
}

fn input_flush_interval() -> Duration {
    static INTERVAL: OnceLock<Duration> = OnceLock::new();
    *INTERVAL.get_or_init(|| {
        let ms = std::env::var("DESKBRIDGE_INPUT_FLUSH_MS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(DEFAULT_INPUT_FLUSH_MS)
            .clamp(1, 16);
        Duration::from_millis(ms)
    })
}

fn flush_pending_input(
    input: &mut InputSink,
    pending_delta: &mut (i32, i32),
    pending_wheel: &mut (i32, i32),
    pending_since: &mut Option<Instant>,
) {
    if pending_delta.0 != 0 || pending_delta.1 != 0 {
        apply_input_event(
            input,
            InputEvent::MouseDelta {
                dx: pending_delta.0,
                dy: pending_delta.1,
            },
        );
        *pending_delta = (0, 0);
    }
    if pending_wheel.0 != 0 || pending_wheel.1 != 0 {
        apply_input_event(
            input,
            InputEvent::MouseWheel {
                horizontal: clamp_i16(pending_wheel.0),
                vertical: clamp_i16(pending_wheel.1),
            },
        );
        *pending_wheel = (0, 0);
    }
    *pending_since = None;
}

fn apply_input_event(input: &mut InputSink, event: InputEvent) {
    if let Err(e) = input.apply(event) {
        eprintln!("input apply failed: {e}");
    }
}

fn clamp_i16(value: i32) -> i16 {
    value.clamp(i16::MIN as i32, i16::MAX as i32) as i16
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
