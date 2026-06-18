use std::collections::{HashMap, HashSet};
use std::ffi::c_void;
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use super::{Edge, ServerConfig};
use crate::clipboard::{Clipboard, ClipboardApi};
use crate::config::ModifierTarget;
use crate::file_transfer;
use crate::protocol::{
    self, ClipboardPayload, Frame, FrameKind, InputEvent, KeyState, MouseButton,
};
use crate::transport::SharedWriter;

const DEFAULT_REMOTE_WIDTH: i32 = 1366;
const DEFAULT_REMOTE_HEIGHT: i32 = 768;
const REMOTE_CLIPBOARD_SUPPRESS_WINDOW: Duration = Duration::from_millis(1200);
const DEFAULT_INPUT_FLUSH_MS: u64 = 2;
const INPUT_BATCH_LIMIT: usize = 64;
const EDGE_TRIGGER_MARGIN: i32 = 6;
const RETURN_EDGE_MARGIN: i32 = 4;
const RETURN_PUSH_THRESHOLD: i32 = 48;
const CAPS_LOCK_SCANCODE: u16 = 58;

const TAP_MOUSE_MOVED: u32 = 1;
const TAP_MOUSE_LEFT_DOWN: u32 = 2;
const TAP_MOUSE_LEFT_UP: u32 = 3;
const TAP_MOUSE_RIGHT_DOWN: u32 = 4;
const TAP_MOUSE_RIGHT_UP: u32 = 5;
const TAP_MOUSE_OTHER_DOWN: u32 = 6;
const TAP_MOUSE_OTHER_UP: u32 = 7;
const TAP_SCROLL: u32 = 20;
const TAP_KEY_DOWN: u32 = 21;
const TAP_KEY_UP: u32 = 22;
const TAP_FLAGS_CHANGED: u32 = 23;

static CAPTURE_STATE: OnceLock<Arc<Mutex<CaptureState>>> = OnceLock::new();

unsafe extern "C" {
    fn deskbridge_event_tap_run(
        context: *mut c_void,
        callback: extern "C" fn(*mut c_void, u32, i64, i64, i64, i64, f64, f64) -> bool,
    ) -> i32;
    fn deskbridge_macos_set_cursor_position(x: f64, y: f64) -> i32;
}

pub fn run(config: ServerConfig) -> std::io::Result<()> {
    let listener = TcpListener::bind(&config.bind)?;
    eprintln!("deskbridge macOS server listening on {}", config.bind);
    let (stream, addr, writer, remote_size) = loop {
        let (mut stream, addr) = listener.accept()?;
        stream.set_nodelay(true)?;
        eprintln!("client connected from {addr}");

        let writer = SharedWriter::new(stream.try_clone()?);
        if let Err(e) = writer.write(Frame::new(FrameKind::Hello, protocol::hello_payload())) {
            eprintln!("failed to send hello to {addr}: {e}; waiting for another client");
            continue;
        }

        match read_client_hello(&mut stream) {
            Ok(remote_size) => break (stream, addr, writer, remote_size),
            Err(e) => {
                eprintln!(
                    "client {addr} disconnected during handshake: {e}; waiting for another client"
                );
                continue;
            }
        }
    };
    eprintln!(
        "remote screen {}x{}, edge {:?}, client {}",
        remote_size.0, remote_size.1, config.edge, addr
    );

    let clipboard_state = Arc::new(Mutex::new(ClipboardSyncState::default()));
    let input = InputEmitter::spawn(writer.clone());
    let state = Arc::new(Mutex::new(CaptureState::new(
        input,
        config.edge,
        remote_size,
    )));
    if CAPTURE_STATE.set(state).is_err() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            "capture state already initialized",
        ));
    }

    spawn_inbound_reader(stream, Arc::clone(&clipboard_state));
    spawn_clipboard_watcher(writer, clipboard_state);

    eprintln!("macOS event tap starting; move through the configured edge to control Windows");
    let status = unsafe { deskbridge_event_tap_run(std::ptr::null_mut(), event_tap_callback) };
    if status == 0 {
        Ok(())
    } else {
        Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            format!(
                "failed to start macOS event tap (native status {status}); grant Accessibility/Input Monitoring permission"
            ),
        ))
    }
}

fn read_client_hello(stream: &mut TcpStream) -> std::io::Result<(i32, i32)> {
    let frame = protocol::read_frame(stream)?;
    if frame.kind != FrameKind::Hello {
        return Ok((DEFAULT_REMOTE_WIDTH, DEFAULT_REMOTE_HEIGHT));
    }
    let hello = protocol::decode_hello(&frame.payload)?;
    let width = hello.screen_width.unwrap_or(DEFAULT_REMOTE_WIDTH as u32) as i32;
    let height = hello.screen_height.unwrap_or(DEFAULT_REMOTE_HEIGHT as u32) as i32;
    Ok((width.max(1), height.max(1)))
}

fn spawn_inbound_reader(mut stream: TcpStream, clipboard_state: Arc<Mutex<ClipboardSyncState>>) {
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
                Err(e) => {
                    eprintln!("connection closed: {e}");
                    std::process::exit(1);
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

#[derive(Clone)]
struct InputEmitter {
    sender: Sender<InputEvent>,
}

impl InputEmitter {
    fn spawn(writer: SharedWriter) -> Self {
        let (sender, receiver) = mpsc::channel();
        thread::spawn(move || input_writer_loop(writer, receiver));
        Self { sender }
    }

    fn send(&self, event: InputEvent) {
        if let Err(e) = self.sender.send(event) {
            eprintln!("input queue closed: {e}");
        }
    }
}

fn input_writer_loop(writer: SharedWriter, receiver: Receiver<InputEvent>) {
    let mut pending_delta = (0i32, 0i32);
    let mut pending_wheel = (0i32, 0i32);
    let mut pending_since = None;
    let mut log = InputSendLog::new();

    loop {
        let event = match recv_input_event(&receiver, pending_since) {
            Ok(Some(event)) => event,
            Ok(None) => {
                flush_pending_input(
                    &writer,
                    &mut pending_delta,
                    &mut pending_wheel,
                    &mut pending_since,
                    &mut log,
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
                    &writer,
                    &receiver,
                    &mut pending_delta,
                    &mut pending_wheel,
                    &mut pending_since,
                    &mut log,
                );
            }
            event => {
                flush_pending_input(
                    &writer,
                    &mut pending_delta,
                    &mut pending_wheel,
                    &mut pending_since,
                    &mut log,
                );
                write_input_event(&writer, event, &mut log);
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
    writer: &SharedWriter,
    receiver: &Receiver<InputEvent>,
    pending_delta: &mut (i32, i32),
    pending_wheel: &mut (i32, i32),
    pending_since: &mut Option<Instant>,
    log: &mut InputSendLog,
) {
    for _ in 0..INPUT_BATCH_LIMIT {
        match receiver.try_recv() {
            Ok(event @ (InputEvent::MouseDelta { .. } | InputEvent::MouseWheel { .. })) => {
                queue_pending_input(event, pending_delta, pending_wheel, pending_since);
            }
            Ok(event) => {
                flush_pending_input(writer, pending_delta, pending_wheel, pending_since, log);
                write_input_event(writer, event, log);
                return;
            }
            Err(TryRecvError::Empty) => break,
            Err(TryRecvError::Disconnected) => return,
        }
    }

    if pending_ready(*pending_since) {
        flush_pending_input(writer, pending_delta, pending_wheel, pending_since, log);
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
    writer: &SharedWriter,
    pending_delta: &mut (i32, i32),
    pending_wheel: &mut (i32, i32),
    pending_since: &mut Option<Instant>,
    log: &mut InputSendLog,
) {
    if pending_delta.0 != 0 || pending_delta.1 != 0 {
        write_input_event(
            writer,
            InputEvent::MouseDelta {
                dx: pending_delta.0,
                dy: pending_delta.1,
            },
            log,
        );
        *pending_delta = (0, 0);
    }
    if pending_wheel.0 != 0 || pending_wheel.1 != 0 {
        write_input_event(
            writer,
            InputEvent::MouseWheel {
                horizontal: clamp_i16(pending_wheel.0),
                vertical: clamp_i16(pending_wheel.1),
            },
            log,
        );
        *pending_wheel = (0, 0);
    }
    *pending_since = None;
}

struct InputSendLog {
    enabled: bool,
    count: u64,
    last_print: Instant,
}

impl Default for InputSendLog {
    fn default() -> Self {
        Self {
            enabled: false,
            count: 0,
            last_print: Instant::now() - Duration::from_secs(2),
        }
    }
}

impl InputSendLog {
    fn new() -> Self {
        Self {
            enabled: std::env::var_os("DESKBRIDGE_INPUT_LOG").is_some(),
            ..Self::default()
        }
    }
}

fn write_input_event(writer: &SharedWriter, event: InputEvent, log: &mut InputSendLog) {
    let encoded = protocol::encode_input(&event);
    if let Err(e) = writer.write(Frame::new(FrameKind::Input, encoded)) {
        eprintln!("input send failed after {} sent event(s): {e}", log.count);
        return;
    }
    log.count += 1;
    if !log.enabled {
        return;
    }
    if log.count == 1 || log.last_print.elapsed() >= Duration::from_secs(1) {
        eprintln!("sent input event #{}: {:?}", log.count, event);
        log.last_print = Instant::now();
    }
}

#[derive(Clone, Copy)]
struct TapEvent {
    kind: u32,
    a: i64,
    b: i64,
    c: i64,
    x: f64,
    y: f64,
}

#[derive(Clone, Copy)]
struct ModifierMapping {
    command: ModifierTarget,
    control: ModifierTarget,
    option: ModifierTarget,
}

impl ModifierMapping {
    fn from_env() -> Self {
        let mapping = Self {
            command: env_modifier_target("DESKBRIDGE_MAC_COMMAND_MAPPING", ModifierTarget::Control),
            control: env_modifier_target("DESKBRIDGE_MAC_CONTROL_MAPPING", ModifierTarget::Control),
            option: env_modifier_target("DESKBRIDGE_MAC_OPTION_MAPPING", ModifierTarget::Alt),
        };
        eprintln!(
            "macOS server modifier mapping: Command->{}, Control->{}, Option->{}",
            mapping.command.as_str(),
            mapping.control.as_str(),
            mapping.option.as_str()
        );
        mapping
    }

    fn scancode_for_mac_modifier(self, keycode: u16) -> Option<u16> {
        let (target, right) = match keycode {
            54 => (self.command, true),
            55 => (self.command, false),
            58 => (self.option, false),
            59 => (self.control, false),
            61 => (self.option, true),
            62 => (self.control, true),
            _ => return None,
        };
        target_scancode(target, right)
    }
}

fn env_modifier_target(name: &str, default: ModifierTarget) -> ModifierTarget {
    std::env::var(name)
        .ok()
        .and_then(|value| ModifierTarget::parse(value.trim()))
        .unwrap_or(default)
}

fn target_scancode(target: ModifierTarget, right: bool) -> Option<u16> {
    match (target, right) {
        (ModifierTarget::Control, false) => Some(29),
        (ModifierTarget::Control, true) => Some(285),
        (ModifierTarget::Meta, false) => Some(347),
        (ModifierTarget::Meta, true) => Some(348),
        (ModifierTarget::Alt, false) => Some(56),
        (ModifierTarget::Alt, true) => Some(312),
        (ModifierTarget::Disabled, _) => None,
    }
}

extern "C" fn event_tap_callback(
    _context: *mut c_void,
    kind: u32,
    a: i64,
    b: i64,
    c: i64,
    _d: i64,
    x: f64,
    y: f64,
) -> bool {
    let event = TapEvent {
        kind,
        a,
        b,
        c,
        x,
        y,
    };
    CAPTURE_STATE
        .get()
        .map(|state| state.lock().unwrap().handle_tap(event))
        .unwrap_or(false)
}

struct CaptureState {
    input: InputEmitter,
    edge: Edge,
    local_size: (i32, i32),
    remote_size: (i32, i32),
    remote_pos: (i32, i32),
    return_push: i32,
    active: bool,
    local_left_down: bool,
    pressed_keys: HashMap<u16, usize>,
    pressed_mac_modifiers: HashSet<u16>,
    modifier_mapping: ModifierMapping,
}

impl CaptureState {
    fn new(input: InputEmitter, edge: Edge, remote_size: (i32, i32)) -> Self {
        Self {
            input,
            edge,
            local_size: screen_size_i32(),
            remote_size,
            remote_pos: (0, 0),
            return_push: 0,
            active: false,
            local_left_down: false,
            pressed_keys: HashMap::new(),
            pressed_mac_modifiers: HashSet::new(),
            modifier_mapping: ModifierMapping::from_env(),
        }
    }

    fn handle_tap(&mut self, event: TapEvent) -> bool {
        match event.kind {
            TAP_MOUSE_MOVED => self.handle_mouse_move(event),
            TAP_MOUSE_LEFT_DOWN | TAP_MOUSE_LEFT_UP | TAP_MOUSE_RIGHT_DOWN | TAP_MOUSE_RIGHT_UP
            | TAP_MOUSE_OTHER_DOWN | TAP_MOUSE_OTHER_UP => self.handle_mouse_button(event),
            TAP_SCROLL => self.handle_scroll(event),
            TAP_KEY_DOWN | TAP_KEY_UP | TAP_FLAGS_CHANGED => self.handle_keyboard(event),
            _ => false,
        }
    }

    fn handle_mouse_move(&mut self, event: TapEvent) -> bool {
        if self.active {
            let dx = event.b as i32;
            let dy = event.c as i32;
            if dx == 0 && dy == 0 {
                return true;
            }
            if self.should_release_to_local(dx) {
                self.deactivate();
                return true;
            }
            self.remote_pos.0 = clamp(self.remote_pos.0 + dx, 0, self.remote_size.0 - 1);
            self.remote_pos.1 = clamp(self.remote_pos.1 + dy, 0, self.remote_size.1 - 1);
            self.send_input(InputEvent::MouseDelta { dx, dy });
            return true;
        }

        self.local_size = screen_size_i32();
        if self.crossed_edge(event.x.round() as i32) {
            if self.local_left_down {
                return false;
            }
            self.activate(event.y.round() as i32);
            return true;
        }
        false
    }

    fn handle_mouse_button(&mut self, event: TapEvent) -> bool {
        if !self.active {
            match event.kind {
                TAP_MOUSE_LEFT_DOWN => self.local_left_down = true,
                TAP_MOUSE_LEFT_UP => self.local_left_down = false,
                _ => {}
            }
            return false;
        }

        let Some((button, down)) = mouse_button_from_tap(event) else {
            return true;
        };
        self.send_input(InputEvent::MouseButton { button, down });
        true
    }

    fn handle_scroll(&mut self, event: TapEvent) -> bool {
        if !self.active {
            return false;
        }
        self.send_input(InputEvent::MouseWheel {
            horizontal: clamp_i16(event.a as i32),
            vertical: clamp_i16(event.b as i32),
        });
        true
    }

    fn handle_keyboard(&mut self, event: TapEvent) -> bool {
        if !self.active {
            return false;
        }

        let mac_keycode = event.a as u16;
        match event.kind {
            TAP_KEY_DOWN => {
                let repeat = event.b != 0;
                self.key_down(mac_keycode, repeat);
            }
            TAP_KEY_UP => self.key_up(mac_keycode),
            TAP_FLAGS_CHANGED => self.flags_changed(mac_keycode, event.c as u64),
            _ => {}
        }
        true
    }

    fn key_down(&mut self, mac_keycode: u16, repeat: bool) {
        let Some(scancode) = mac_keycode_to_windows_scancode(mac_keycode, self.modifier_mapping)
        else {
            return;
        };
        let state = if repeat || self.pressed_keys.contains_key(&scancode) {
            KeyState::Repeat
        } else {
            self.pressed_keys.insert(scancode, 1);
            KeyState::Down
        };
        self.send_input(InputEvent::Key { scancode, state });
    }

    fn key_up(&mut self, mac_keycode: u16) {
        let Some(scancode) = mac_keycode_to_windows_scancode(mac_keycode, self.modifier_mapping)
        else {
            return;
        };
        self.pressed_keys.remove(&scancode);
        self.send_input(InputEvent::Key {
            scancode,
            state: KeyState::Up,
        });
    }

    fn flags_changed(&mut self, mac_keycode: u16, flags: u64) {
        let Some(scancode) = mac_keycode_to_windows_scancode(mac_keycode, self.modifier_mapping)
        else {
            return;
        };
        if scancode == CAPS_LOCK_SCANCODE {
            self.send_input(InputEvent::Key {
                scancode,
                state: KeyState::Down,
            });
            self.send_input(InputEvent::Key {
                scancode,
                state: KeyState::Up,
            });
            return;
        }

        let tracked = self.pressed_mac_modifiers.contains(&mac_keycode);
        let down = tracked || mac_modifier_flag_down(mac_keycode, flags);
        if down && !tracked {
            self.pressed_mac_modifiers.insert(mac_keycode);
            let should_send_down = {
                let count = self.pressed_keys.entry(scancode).or_insert(0);
                let should_send_down = *count == 0;
                *count += 1;
                should_send_down
            };
            if should_send_down {
                self.send_input(InputEvent::Key {
                    scancode,
                    state: KeyState::Down,
                });
            }
        } else if tracked {
            self.pressed_mac_modifiers.remove(&mac_keycode);
            let should_send_up = if let Some(count) = self.pressed_keys.get_mut(&scancode) {
                *count = count.saturating_sub(1);
                *count == 0
            } else {
                false
            };
            if should_send_up {
                self.pressed_keys.remove(&scancode);
                self.send_input(InputEvent::Key {
                    scancode,
                    state: KeyState::Up,
                });
            }
        }
    }

    fn activate(&mut self, local_y: i32) {
        self.active = true;
        self.local_left_down = false;
        self.local_size = screen_size_i32();
        self.remote_pos = match self.edge {
            Edge::Right => (0, scaled_y(local_y, self.local_size.1, self.remote_size.1)),
            Edge::Left => (
                self.remote_size.0 - 1,
                scaled_y(local_y, self.local_size.1, self.remote_size.1),
            ),
        };
        self.return_push = 0;
        self.send_input(InputEvent::MouseEnter {
            x: self.remote_pos.0,
            y: self.remote_pos.1,
        });
        eprintln!(
            "entered Windows control at {},{}; move back through the edge to release",
            self.remote_pos.0, self.remote_pos.1
        );
    }

    fn deactivate(&mut self) {
        for scancode in self.pressed_keys.keys().copied().collect::<Vec<_>>() {
            self.send_input(InputEvent::Key {
                scancode,
                state: KeyState::Up,
            });
        }
        self.pressed_keys.clear();
        self.pressed_mac_modifiers.clear();
        self.active = false;
        self.return_push = 0;
        let x = match self.edge {
            Edge::Right => self.local_size.0.saturating_sub(2),
            Edge::Left => 1,
        };
        let y = self.local_size.1 / 2;
        let _ = unsafe { deskbridge_macos_set_cursor_position(x as f64, y as f64) };
        eprintln!("released control back to macOS");
    }

    fn crossed_edge(&self, x: i32) -> bool {
        match self.edge {
            Edge::Right => x >= self.local_size.0.saturating_sub(EDGE_TRIGGER_MARGIN),
            Edge::Left => x <= EDGE_TRIGGER_MARGIN,
        }
    }

    fn should_release_to_local(&mut self, dx: i32) -> bool {
        let pushing_out = match self.edge {
            Edge::Right => dx < 0,
            Edge::Left => dx > 0,
        };
        let at_return_edge = match self.edge {
            Edge::Right => self.remote_pos.0 <= RETURN_EDGE_MARGIN,
            Edge::Left => {
                self.remote_pos.0 >= self.remote_size.0.saturating_sub(1 + RETURN_EDGE_MARGIN)
            }
        };
        if !pushing_out || !at_return_edge {
            self.return_push = 0;
            return false;
        }

        self.return_push += dx.abs();
        self.return_push >= RETURN_PUSH_THRESHOLD
    }

    fn send_input(&self, event: InputEvent) {
        self.input.send(event);
    }
}

fn mouse_button_from_tap(event: TapEvent) -> Option<(MouseButton, bool)> {
    match event.kind {
        TAP_MOUSE_LEFT_DOWN => Some((MouseButton::Left, true)),
        TAP_MOUSE_LEFT_UP => Some((MouseButton::Left, false)),
        TAP_MOUSE_RIGHT_DOWN => Some((MouseButton::Right, true)),
        TAP_MOUSE_RIGHT_UP => Some((MouseButton::Right, false)),
        TAP_MOUSE_OTHER_DOWN => Some((mac_other_button(event.a), true)),
        TAP_MOUSE_OTHER_UP => Some((mac_other_button(event.a), false)),
        _ => None,
    }
}

fn mac_other_button(button: i64) -> MouseButton {
    match button {
        2 => MouseButton::Middle,
        3 => MouseButton::Extra(4),
        4 => MouseButton::Extra(5),
        value => MouseButton::Extra(value.clamp(4, u8::MAX as i64) as u8),
    }
}

fn mac_modifier_flag_down(keycode: u16, flags: u64) -> bool {
    const ALPHA_SHIFT: u64 = 0x0001_0000;
    const SHIFT: u64 = 0x0002_0000;
    const CONTROL: u64 = 0x0004_0000;
    const ALTERNATE: u64 = 0x0008_0000;
    const COMMAND: u64 = 0x0010_0000;

    let mask = match keycode {
        54 | 55 => COMMAND,
        56 | 60 => SHIFT,
        57 => ALPHA_SHIFT,
        58 | 61 => ALTERNATE,
        59 | 62 => CONTROL,
        _ => 0,
    };
    mask != 0 && flags & mask != 0
}

fn is_mapped_mac_modifier(keycode: u16) -> bool {
    matches!(keycode, 54 | 55 | 58 | 59 | 61 | 62)
}

fn mac_keycode_to_windows_scancode(keycode: u16, modifier_mapping: ModifierMapping) -> Option<u16> {
    if let Some(scancode) = modifier_mapping.scancode_for_mac_modifier(keycode) {
        return Some(scancode);
    }
    if is_mapped_mac_modifier(keycode) {
        return None;
    }

    Some(match keycode {
        0 => 30,
        1 => 31,
        2 => 32,
        3 => 33,
        4 => 35,
        5 => 34,
        6 => 44,
        7 => 45,
        8 => 46,
        9 => 47,
        11 => 48,
        12 => 16,
        13 => 17,
        14 => 18,
        15 => 19,
        16 => 21,
        17 => 20,
        18 => 2,
        19 => 3,
        20 => 4,
        21 => 5,
        22 => 7,
        23 => 6,
        24 => 13,
        25 => 10,
        26 => 8,
        27 => 12,
        28 => 9,
        29 => 11,
        30 => 27,
        31 => 24,
        32 => 22,
        33 => 26,
        34 => 23,
        35 => 25,
        36 => 28,
        37 => 38,
        38 => 36,
        39 => 40,
        40 => 37,
        41 => 39,
        42 => 43,
        43 => 51,
        44 => 53,
        45 => 49,
        46 => 50,
        47 => 52,
        48 => 15,
        49 => 57,
        50 => 41,
        51 => 14,
        53 => 1,
        56 => 42,
        57 => CAPS_LOCK_SCANCODE,
        60 => 54,
        65 => 83,
        67 => 55,
        69 => 78,
        71 => 69,
        75 => 309,
        76 => 284,
        78 => 74,
        82 => 82,
        83 => 79,
        84 => 80,
        85 => 81,
        86 => 75,
        87 => 76,
        88 => 77,
        89 => 71,
        91 => 72,
        92 => 73,
        96 => 63,
        97 => 64,
        98 => 65,
        99 => 61,
        100 => 66,
        101 => 67,
        103 => 87,
        109 => 68,
        111 => 88,
        114 => 338,
        115 => 327,
        116 => 329,
        117 => 339,
        118 => 62,
        119 => 335,
        120 => 60,
        121 => 337,
        122 => 59,
        123 => 331,
        124 => 333,
        125 => 336,
        126 => 328,
        _ => return None,
    })
}

fn screen_size_i32() -> (i32, i32) {
    let (width, height) = crate::input::screen_size();
    (width.max(1) as i32, height.max(1) as i32)
}

fn scaled_y(y: i32, from_height: i32, to_height: i32) -> i32 {
    clamp(
        (y as i64 * to_height.max(1) as i64 / from_height.max(1) as i64) as i32,
        0,
        to_height.saturating_sub(1),
    )
}

fn clamp(value: i32, min: i32, max: i32) -> i32 {
    value.max(min).min(max)
}

fn clamp_i16(value: i32) -> i16 {
    value.clamp(i16::MIN as i32, i16::MAX as i32) as i16
}
