use std::collections::HashSet;
use std::mem::zeroed;
use std::net::{TcpListener, TcpStream};
use std::ptr;
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::Duration;

use windows_sys::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::System::DataExchange::GetClipboardSequenceNumber;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::VK_SCROLL;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, DispatchMessageW, GetMessageW, GetSystemMetrics, HC_ACTION, HHOOK,
    KBDLLHOOKSTRUCT, LLKHF_EXTENDED, LLKHF_UP, LLMHF_INJECTED, MSG, MSLLHOOKSTRUCT, SM_CXSCREEN,
    SM_CYSCREEN, SetCursorPos, SetProcessDPIAware, SetWindowsHookExW, TranslateMessage,
    UnhookWindowsHookEx, WH_KEYBOARD_LL, WH_MOUSE_LL, WM_KEYDOWN, WM_KEYUP, WM_LBUTTONDOWN,
    WM_LBUTTONUP, WM_MBUTTONDOWN, WM_MBUTTONUP, WM_MOUSEHWHEEL, WM_MOUSEMOVE, WM_MOUSEWHEEL,
    WM_RBUTTONDOWN, WM_RBUTTONUP, WM_SYSKEYDOWN, WM_SYSKEYUP, WM_XBUTTONDOWN, WM_XBUTTONUP,
    XBUTTON1, XBUTTON2,
};

use super::{Edge, ServerConfig};
use crate::clipboard::{Clipboard, ClipboardApi};
use crate::file_transfer;
use crate::protocol::{
    self, ClipboardPayload, Frame, FrameKind, InputEvent, KeyState, MouseButton,
};
use crate::transport::SharedWriter;

const DEFAULT_REMOTE_WIDTH: i32 = 1366;
const DEFAULT_REMOTE_HEIGHT: i32 = 768;
const WHEEL_PIXELS_PER_DETENT: i32 = 240;

static CAPTURE_STATE: OnceLock<Arc<Mutex<CaptureState>>> = OnceLock::new();

pub fn run(config: ServerConfig) -> std::io::Result<()> {
    unsafe {
        SetProcessDPIAware();
    }

    let listener = TcpListener::bind(&config.bind)?;
    eprintln!("deskbridge server listening on {}", config.bind);
    let (mut stream, addr) = listener.accept()?;
    stream.set_nodelay(true)?;
    eprintln!("client connected from {addr}");

    let writer = SharedWriter::new(stream.try_clone()?);
    writer.write(Frame::new(FrameKind::Hello, protocol::hello_payload()))?;

    let remote_size = read_client_hello(&mut stream)?;
    eprintln!(
        "remote screen {}x{}, edge {:?}",
        remote_size.0, remote_size.1, config.edge
    );

    let last_clipboard = Arc::new(Mutex::new(None));
    let state = Arc::new(Mutex::new(CaptureState::new(
        writer.clone(),
        config.edge,
        remote_size,
    )));
    if CAPTURE_STATE.set(state).is_err() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            "capture state already initialized",
        ));
    }

    spawn_inbound_reader(stream, Arc::clone(&last_clipboard));
    spawn_clipboard_watcher(writer, last_clipboard);

    let hooks = Hooks::install()?;
    eprintln!("input hooks installed; move the mouse through the configured edge to control macOS");
    message_loop();
    drop(hooks);
    Ok(())
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

fn spawn_inbound_reader(mut stream: TcpStream, last_clipboard: Arc<Mutex<Option<Vec<u8>>>>) {
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
                        remember_clipboard(&last_clipboard, &payload);
                        if let Err(e) = clipboard.write(&payload) {
                            eprintln!("clipboard write failed: {e}");
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
                    remember_clipboard(&last_clipboard, &payload);
                    if let Err(e) = clipboard.write(&payload) {
                        eprintln!("file clipboard write failed: {e}");
                    }
                }
                _ => {}
            }
        }
    });
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
        let mut last_sequence = unsafe { GetClipboardSequenceNumber() };
        loop {
            thread::sleep(Duration::from_millis(350));
            let sequence = unsafe { GetClipboardSequenceNumber() };
            if sequence == last_sequence {
                continue;
            }
            last_sequence = sequence;

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

struct Hooks {
    keyboard: HHOOK,
    mouse: HHOOK,
}

impl Hooks {
    fn install() -> std::io::Result<Self> {
        let module = unsafe { GetModuleHandleW(ptr::null()) };
        let keyboard = unsafe { SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_proc), module, 0) };
        if keyboard.is_null() {
            return Err(last_os_error("SetWindowsHookExW keyboard failed"));
        }
        let mouse = unsafe { SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_proc), module, 0) };
        if mouse.is_null() {
            unsafe {
                UnhookWindowsHookEx(keyboard);
            }
            return Err(last_os_error("SetWindowsHookExW mouse failed"));
        }
        Ok(Self { keyboard, mouse })
    }
}

impl Drop for Hooks {
    fn drop(&mut self) {
        unsafe {
            UnhookWindowsHookEx(self.keyboard);
            UnhookWindowsHookEx(self.mouse);
        }
    }
}

fn message_loop() {
    let mut msg: MSG = unsafe { zeroed() };
    while unsafe { GetMessageW(&mut msg, ptr::null_mut(), 0, 0) } > 0 {
        unsafe {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

struct CaptureState {
    writer: SharedWriter,
    edge: Edge,
    win_size: (i32, i32),
    remote_size: (i32, i32),
    remote_pos: (i32, i32),
    active: bool,
    warping: bool,
    pressed_keys: HashSet<u16>,
}

impl CaptureState {
    fn new(writer: SharedWriter, edge: Edge, remote_size: (i32, i32)) -> Self {
        Self {
            writer,
            edge,
            win_size: screen_size(),
            remote_size,
            remote_pos: (0, 0),
            active: false,
            warping: false,
            pressed_keys: HashSet::new(),
        }
    }

    fn handle_keyboard(&mut self, message: u32, hook: &KBDLLHOOKSTRUCT) -> bool {
        if !self.active {
            return false;
        }
        let down = matches!(message, WM_KEYDOWN | WM_SYSKEYDOWN);
        let up = matches!(message, WM_KEYUP | WM_SYSKEYUP) || (hook.flags & LLKHF_UP) != 0;
        if !down && !up {
            return true;
        }
        if hook.vkCode == VK_SCROLL as u32 && down {
            self.deactivate();
            return true;
        }

        let scancode = normalized_scancode(hook);
        let state = if down {
            if self.pressed_keys.insert(scancode) {
                KeyState::Down
            } else {
                KeyState::Repeat
            }
        } else {
            self.pressed_keys.remove(&scancode);
            KeyState::Up
        };
        self.send_input(InputEvent::Key { scancode, state });
        true
    }

    fn handle_mouse(&mut self, message: u32, hook: &MSLLHOOKSTRUCT) -> bool {
        if message == WM_MOUSEMOVE {
            return self.handle_mouse_move(hook);
        }
        if !self.active {
            return false;
        }

        match message {
            WM_LBUTTONDOWN => self.send_button(MouseButton::Left, true),
            WM_LBUTTONUP => self.send_button(MouseButton::Left, false),
            WM_RBUTTONDOWN => self.send_button(MouseButton::Right, true),
            WM_RBUTTONUP => self.send_button(MouseButton::Right, false),
            WM_MBUTTONDOWN => self.send_button(MouseButton::Middle, true),
            WM_MBUTTONUP => self.send_button(MouseButton::Middle, false),
            WM_XBUTTONDOWN => self.send_button(xbutton(hook.mouseData), true),
            WM_XBUTTONUP => self.send_button(xbutton(hook.mouseData), false),
            WM_MOUSEWHEEL => self.send_wheel(0, wheel_pixels(hook.mouseData)),
            WM_MOUSEHWHEEL => self.send_wheel(wheel_pixels(hook.mouseData), 0),
            _ => {}
        }
        true
    }

    fn handle_mouse_move(&mut self, hook: &MSLLHOOKSTRUCT) -> bool {
        if self.active {
            if self.warping || (hook.flags & LLMHF_INJECTED) != 0 {
                self.warping = false;
                return true;
            }
            let anchor = self.anchor();
            let dx = hook.pt.x - anchor.0;
            let dy = hook.pt.y - anchor.1;
            if dx == 0 && dy == 0 {
                return true;
            }

            if self.should_leave_remote(dx) {
                self.deactivate();
                return true;
            }

            self.remote_pos.0 = clamp(self.remote_pos.0 + dx, 0, self.remote_size.0 - 1);
            self.remote_pos.1 = clamp(self.remote_pos.1 + dy, 0, self.remote_size.1 - 1);
            self.send_input(InputEvent::MouseMove {
                x: self.remote_pos.0,
                y: self.remote_pos.1,
            });
            self.warp_to_anchor();
            return true;
        }

        self.win_size = screen_size();
        if self.crossed_edge(hook.pt.x) {
            self.activate(hook.pt.y);
            return true;
        }
        false
    }

    fn activate(&mut self, local_y: i32) {
        self.active = true;
        self.win_size = screen_size();
        self.remote_pos = match self.edge {
            Edge::Right => (0, scaled_y(local_y, self.win_size.1, self.remote_size.1)),
            Edge::Left => (
                self.remote_size.0 - 1,
                scaled_y(local_y, self.win_size.1, self.remote_size.1),
            ),
        };
        self.send_input(InputEvent::MouseMove {
            x: self.remote_pos.0,
            y: self.remote_pos.1,
        });
        self.warp_to_anchor();
    }

    fn deactivate(&mut self) {
        let y = scaled_y(self.remote_pos.1, self.remote_size.1, self.win_size.1);
        for scancode in self.pressed_keys.drain().collect::<Vec<_>>() {
            self.send_input(InputEvent::Key {
                scancode,
                state: KeyState::Up,
            });
        }
        self.active = false;
        self.warping = false;
        let x = match self.edge {
            Edge::Right => self.win_size.0.saturating_sub(2),
            Edge::Left => 1,
        };
        unsafe {
            SetCursorPos(x, clamp(y, 0, self.win_size.1.saturating_sub(1)));
        }
        eprintln!("released control back to Windows");
    }

    fn crossed_edge(&self, x: i32) -> bool {
        match self.edge {
            Edge::Right => x >= self.win_size.0.saturating_sub(1),
            Edge::Left => x <= 0,
        }
    }

    fn should_leave_remote(&self, dx: i32) -> bool {
        match self.edge {
            Edge::Right => self.remote_pos.0 <= 0 && dx < 0,
            Edge::Left => self.remote_pos.0 >= self.remote_size.0 - 1 && dx > 0,
        }
    }

    fn anchor(&self) -> (i32, i32) {
        (self.win_size.0 / 2, self.win_size.1 / 2)
    }

    fn warp_to_anchor(&mut self) {
        let anchor = self.anchor();
        self.warping = true;
        unsafe {
            SetCursorPos(anchor.0, anchor.1);
        }
    }

    fn send_button(&self, button: MouseButton, down: bool) {
        self.send_input(InputEvent::MouseButton { button, down });
    }

    fn send_wheel(&self, horizontal: i16, vertical: i16) {
        self.send_input(InputEvent::MouseWheel {
            horizontal,
            vertical,
        });
    }

    fn send_input(&self, event: InputEvent) {
        if let Err(e) = self
            .writer
            .write(Frame::new(FrameKind::Input, protocol::encode_input(&event)))
        {
            eprintln!("input send failed: {e}");
        }
    }
}

unsafe extern "system" fn keyboard_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        if let Some(state) = CAPTURE_STATE.get() {
            let hook = unsafe { &*(lparam as *const KBDLLHOOKSTRUCT) };
            if state.lock().unwrap().handle_keyboard(wparam as u32, hook) {
                return 1;
            }
        }
    }
    unsafe { CallNextHookEx(ptr::null_mut(), code, wparam, lparam) }
}

unsafe extern "system" fn mouse_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        if let Some(state) = CAPTURE_STATE.get() {
            let hook = unsafe { &*(lparam as *const MSLLHOOKSTRUCT) };
            if state.lock().unwrap().handle_mouse(wparam as u32, hook) {
                return 1;
            }
        }
    }
    unsafe { CallNextHookEx(ptr::null_mut(), code, wparam, lparam) }
}

fn normalized_scancode(hook: &KBDLLHOOKSTRUCT) -> u16 {
    let base = hook.scanCode as u16;
    if (hook.flags & LLKHF_EXTENDED) != 0 {
        base + 256
    } else {
        base
    }
}

fn xbutton(mouse_data: u32) -> MouseButton {
    match ((mouse_data >> 16) & 0xffff) as u16 {
        XBUTTON1 => MouseButton::Extra(4),
        XBUTTON2 => MouseButton::Extra(5),
        value => MouseButton::Extra(value as u8),
    }
}

fn wheel_pixels(mouse_data: u32) -> i16 {
    let delta = ((mouse_data >> 16) & 0xffff) as u16 as i16 as i32;
    clamp(
        delta * WHEEL_PIXELS_PER_DETENT / 120,
        i16::MIN as i32,
        i16::MAX as i32,
    ) as i16
}

fn screen_size() -> (i32, i32) {
    let width = unsafe { GetSystemMetrics(SM_CXSCREEN) }.max(1);
    let height = unsafe { GetSystemMetrics(SM_CYSCREEN) }.max(1);
    (width, height)
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

fn last_os_error(context: &str) -> std::io::Error {
    let error = std::io::Error::last_os_error();
    std::io::Error::new(error.kind(), format!("{context}: {error}"))
}
