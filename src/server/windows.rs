use std::collections::HashSet;
use std::ffi::OsStr;
use std::mem::{size_of, zeroed};
use std::net::{TcpListener, TcpStream};
use std::os::windows::ffi::OsStrExt;
use std::path::PathBuf;
use std::ptr;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows_sys::Win32::System::DataExchange::GetClipboardSequenceNumber;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::VK_SCROLL;
use windows_sys::Win32::UI::Input::{
    GetRawInputData, HRAWINPUT, MOUSE_MOVE_ABSOLUTE, RAWINPUT, RAWINPUTDEVICE, RAWINPUTHEADER,
    RID_INPUT, RIDEV_INPUTSINK, RIM_TYPEMOUSE, RegisterRawInputDevices,
};
use windows_sys::Win32::UI::Shell::{DragAcceptFiles, DragFinish, DragQueryFileW, HDROP};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, ClipCursor, CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW,
    GetMessageW, GetSystemMetrics, HC_ACTION, HHOOK, KBDLLHOOKSTRUCT, LLKHF_EXTENDED, LLKHF_UP,
    LLMHF_INJECTED, LWA_ALPHA, MSG, MSLLHOOKSTRUCT, RegisterClassW, SM_CXSCREEN, SM_CYSCREEN,
    SW_SHOWNA, SetCursorPos, SetLayeredWindowAttributes, SetProcessDPIAware, SetWindowsHookExW,
    ShowWindow, TranslateMessage, UnhookWindowsHookEx, WH_KEYBOARD_LL, WH_MOUSE_LL, WM_DROPFILES,
    WM_INPUT, WM_KEYDOWN, WM_KEYUP, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MBUTTONDOWN, WM_MBUTTONUP,
    WM_MOUSEHWHEEL, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_RBUTTONDOWN, WM_RBUTTONUP, WM_SYSKEYDOWN,
    WM_SYSKEYUP, WM_XBUTTONDOWN, WM_XBUTTONUP, WNDCLASSW, WS_EX_ACCEPTFILES, WS_EX_LAYERED,
    WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP, XBUTTON1, XBUTTON2,
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
const INPUT_FLUSH_INTERVAL: Duration = Duration::from_millis(4);
const DROP_STRIP_WIDTH: i32 = 18;
const DROP_STRIP_ALPHA: u8 = 48;
const EDGE_TRIGGER_MARGIN: i32 = 6;
const CURSOR_LOCK_RADIUS: i32 = 2;

static CAPTURE_STATE: OnceLock<Arc<Mutex<CaptureState>>> = OnceLock::new();
static TRANSFER_WRITER: OnceLock<SharedWriter> = OnceLock::new();

pub fn run(config: ServerConfig) -> std::io::Result<()> {
    unsafe {
        SetProcessDPIAware();
    }

    let listener = TcpListener::bind(&config.bind)?;
    eprintln!("deskbridge server listening on {}", config.bind);
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
    let _ = TRANSFER_WRITER.set(writer.clone());
    eprintln!(
        "remote screen {}x{}, edge {:?}, client {}",
        remote_size.0, remote_size.1, config.edge, addr
    );

    let last_clipboard = Arc::new(Mutex::new(None));
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

    spawn_inbound_reader(stream, Arc::clone(&last_clipboard));
    spawn_clipboard_watcher(writer, last_clipboard);

    let drop_strip = if drop_strip_enabled() {
        Some(DropStrip::create(config.edge)?)
    } else {
        eprintln!("file drop strip disabled; use the GUI drop zone for file drag transfer");
        None
    };
    let raw_input = RawInputWindow::create()?;
    eprintln!("raw mouse input registered for relative movement");
    let hooks = Hooks::install()?;
    eprintln!("input hooks installed; move the mouse through the configured edge to control macOS");
    if drop_strip.is_some() {
        eprintln!("file drop strip active on the {:?} edge", config.edge);
    }
    message_loop();
    unlock_cursor();
    drop(hooks);
    drop(raw_input);
    drop(drop_strip);
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

struct RawInputWindow {
    hwnd: HWND,
}

impl RawInputWindow {
    fn create() -> std::io::Result<Self> {
        let class_name = wide_null("DeskbridgeRawInput");
        let title = wide_null("Deskbridge Raw Input");
        let module = unsafe { GetModuleHandleW(ptr::null()) };
        let wnd_class = WNDCLASSW {
            style: 0,
            lpfnWndProc: Some(raw_input_proc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: module,
            hIcon: ptr::null_mut(),
            hCursor: ptr::null_mut(),
            hbrBackground: ptr::null_mut(),
            lpszMenuName: ptr::null(),
            lpszClassName: class_name.as_ptr(),
        };
        unsafe {
            RegisterClassW(&wnd_class);
        }

        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                class_name.as_ptr(),
                title.as_ptr(),
                WS_POPUP,
                0,
                0,
                0,
                0,
                ptr::null_mut(),
                ptr::null_mut(),
                module,
                ptr::null(),
            )
        };
        if hwnd.is_null() {
            return Err(last_os_error("CreateWindowExW raw input failed"));
        }

        let device = RAWINPUTDEVICE {
            usUsagePage: 0x01,
            usUsage: 0x02,
            dwFlags: RIDEV_INPUTSINK,
            hwndTarget: hwnd,
        };
        let ok = unsafe {
            RegisterRawInputDevices(&device, 1, size_of::<RAWINPUTDEVICE>().try_into().unwrap())
        };
        if ok == 0 {
            unsafe {
                DestroyWindow(hwnd);
            }
            return Err(last_os_error("RegisterRawInputDevices mouse failed"));
        }
        Ok(Self { hwnd })
    }
}

impl Drop for RawInputWindow {
    fn drop(&mut self) {
        unsafe {
            DestroyWindow(self.hwnd);
        }
    }
}

struct DropStrip {
    hwnd: HWND,
}

impl DropStrip {
    fn create(edge: Edge) -> std::io::Result<Self> {
        let class_name = wide_null("DeskbridgeDropStrip");
        let title = wide_null("Deskbridge File Drop");
        let module = unsafe { GetModuleHandleW(ptr::null()) };
        let wnd_class = WNDCLASSW {
            style: 0,
            lpfnWndProc: Some(drop_strip_proc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: module,
            hIcon: ptr::null_mut(),
            hCursor: ptr::null_mut(),
            hbrBackground: ptr::null_mut(),
            lpszMenuName: ptr::null(),
            lpszClassName: class_name.as_ptr(),
        };
        unsafe {
            RegisterClassW(&wnd_class);
        }

        let screen = screen_size();
        let x = match edge {
            Edge::Right => screen.0.saturating_sub(DROP_STRIP_WIDTH),
            Edge::Left => 0,
        };
        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_TOPMOST
                    | WS_EX_TOOLWINDOW
                    | WS_EX_NOACTIVATE
                    | WS_EX_ACCEPTFILES
                    | WS_EX_LAYERED,
                class_name.as_ptr(),
                title.as_ptr(),
                WS_POPUP,
                x,
                0,
                DROP_STRIP_WIDTH,
                screen.1,
                ptr::null_mut(),
                ptr::null_mut(),
                module,
                ptr::null(),
            )
        };
        if hwnd.is_null() {
            return Err(last_os_error("CreateWindowExW drop strip failed"));
        }
        unsafe {
            SetLayeredWindowAttributes(hwnd, 0, DROP_STRIP_ALPHA, LWA_ALPHA);
            DragAcceptFiles(hwnd, 1);
            ShowWindow(hwnd, SW_SHOWNA);
        }
        Ok(Self { hwnd })
    }
}

impl Drop for DropStrip {
    fn drop(&mut self) {
        unsafe {
            DragAcceptFiles(self.hwnd, 0);
            DestroyWindow(self.hwnd);
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

unsafe extern "system" fn drop_strip_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if message == WM_DROPFILES {
        let hdrop = wparam as HDROP;
        let files = dropped_files_from_hdrop(hdrop);
        unsafe {
            DragFinish(hdrop);
        }
        if !files.is_empty() {
            send_dropped_files(files);
        }
        return 0;
    }
    unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
}

unsafe extern "system" fn raw_input_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if message == WM_INPUT {
        handle_raw_mouse_input(lparam);
        return unsafe { DefWindowProcW(hwnd, message, wparam, lparam) };
    }
    unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
}

fn handle_raw_mouse_input(lparam: LPARAM) {
    let mut size = 0u32;
    let header_size = size_of::<RAWINPUTHEADER>() as u32;
    let query = unsafe {
        GetRawInputData(
            lparam as HRAWINPUT,
            RID_INPUT,
            ptr::null_mut(),
            &mut size,
            header_size,
        )
    };
    if query == u32::MAX || size == 0 {
        return;
    }

    let mut buffer = vec![0u8; size as usize];
    let read = unsafe {
        GetRawInputData(
            lparam as HRAWINPUT,
            RID_INPUT,
            buffer.as_mut_ptr().cast(),
            &mut size,
            header_size,
        )
    };
    if read == u32::MAX || read != size {
        return;
    }

    let raw = unsafe { &*(buffer.as_ptr() as *const RAWINPUT) };
    if raw.header.dwType != RIM_TYPEMOUSE {
        return;
    }
    let mouse = unsafe { raw.data.mouse };
    if mouse.usFlags & MOUSE_MOVE_ABSOLUTE != 0 {
        return;
    }
    if mouse.lLastX == 0 && mouse.lLastY == 0 {
        return;
    }

    if let Some(state) = CAPTURE_STATE.get() {
        state
            .lock()
            .unwrap()
            .handle_raw_mouse_delta(mouse.lLastX, mouse.lLastY);
    }
}

fn dropped_files_from_hdrop(hdrop: HDROP) -> Vec<PathBuf> {
    let count = unsafe { DragQueryFileW(hdrop, u32::MAX, ptr::null_mut(), 0) };
    let mut files = Vec::new();
    for i in 0..count {
        let len = unsafe { DragQueryFileW(hdrop, i, ptr::null_mut(), 0) };
        if len == 0 {
            continue;
        }
        let mut buffer = vec![0u16; len as usize + 1];
        let written = unsafe { DragQueryFileW(hdrop, i, buffer.as_mut_ptr(), buffer.len() as u32) };
        if written != 0 {
            files.push(PathBuf::from(String::from_utf16_lossy(
                &buffer[..written as usize],
            )));
        }
    }
    files
}

fn send_dropped_files(files: Vec<PathBuf>) {
    let Some(writer) = TRANSFER_WRITER.get().cloned() else {
        eprintln!("drop ignored: no connected client");
        return;
    };
    eprintln!("sending {} dropped file(s) to macOS", files.len());
    thread::spawn(move || {
        let result = file_transfer::send_files(&writer, &files)
            .and_then(|_| writer.write(Frame::new(FrameKind::DragEnd, Vec::new())));
        if let Err(e) = result {
            eprintln!("dropped file transfer failed: {e}");
        }
    });
}

struct CaptureState {
    input: InputEmitter,
    edge: Edge,
    win_size: (i32, i32),
    remote_size: (i32, i32),
    active: bool,
    local_left_down: bool,
    pressed_keys: HashSet<u16>,
}

impl CaptureState {
    fn new(input: InputEmitter, edge: Edge, remote_size: (i32, i32)) -> Self {
        Self {
            input,
            edge,
            win_size: screen_size(),
            remote_size,
            active: false,
            local_left_down: false,
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
            match message {
                WM_LBUTTONDOWN => self.local_left_down = true,
                WM_LBUTTONUP => self.local_left_down = false,
                _ => {}
            }
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
            if (hook.flags & LLMHF_INJECTED) != 0 {
                return true;
            }
            self.lock_cursor_to_anchor();
            return true;
        }

        self.win_size = screen_size();
        if self.crossed_edge(hook.pt.x) {
            if self.local_left_down {
                return false;
            }
            self.activate(hook.pt.y);
            return true;
        }
        false
    }

    fn handle_raw_mouse_delta(&mut self, dx: i32, dy: i32) {
        if !self.active {
            return;
        }
        self.send_input(InputEvent::MouseDelta { dx, dy });
    }

    fn activate(&mut self, local_y: i32) {
        self.active = true;
        self.local_left_down = false;
        self.win_size = screen_size();
        let entry_pos = match self.edge {
            Edge::Right => (0, scaled_y(local_y, self.win_size.1, self.remote_size.1)),
            Edge::Left => (
                self.remote_size.0 - 1,
                scaled_y(local_y, self.win_size.1, self.remote_size.1),
            ),
        };
        self.send_input(InputEvent::MouseEnter {
            x: entry_pos.0,
            y: entry_pos.1,
        });
        self.lock_cursor_to_anchor();
        eprintln!(
            "entered macOS control at {},{}; press Scroll Lock to release",
            entry_pos.0, entry_pos.1
        );
    }

    fn deactivate(&mut self) {
        for scancode in self.pressed_keys.drain().collect::<Vec<_>>() {
            self.send_input(InputEvent::Key {
                scancode,
                state: KeyState::Up,
            });
        }
        self.active = false;
        unlock_cursor();
        let x = match self.edge {
            Edge::Right => self.win_size.0.saturating_sub(2),
            Edge::Left => 1,
        };
        unsafe {
            SetCursorPos(x, self.win_size.1 / 2);
        }
        eprintln!("released control back to Windows");
    }

    fn crossed_edge(&self, x: i32) -> bool {
        match self.edge {
            Edge::Right => x >= self.win_size.0.saturating_sub(EDGE_TRIGGER_MARGIN),
            Edge::Left => x <= EDGE_TRIGGER_MARGIN,
        }
    }

    fn anchor(&self) -> (i32, i32) {
        (self.win_size.0 / 2, self.win_size.1 / 2)
    }

    fn lock_cursor_to_anchor(&self) {
        let anchor = self.anchor();
        let rect = RECT {
            left: anchor.0 - CURSOR_LOCK_RADIUS,
            top: anchor.1 - CURSOR_LOCK_RADIUS,
            right: anchor.0 + CURSOR_LOCK_RADIUS + 1,
            bottom: anchor.1 + CURSOR_LOCK_RADIUS + 1,
        };
        unsafe {
            SetCursorPos(anchor.0, anchor.1);
            ClipCursor(&rect);
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
        self.input.send(event);
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
    let mut log = InputSendLog::default();

    loop {
        match receiver.recv_timeout(INPUT_FLUSH_INTERVAL) {
            Ok(InputEvent::MouseDelta { dx, dy }) => {
                pending_delta.0 += dx;
                pending_delta.1 += dy;
                flush_pending_input(&writer, &mut pending_delta, &mut pending_wheel, &mut log);
            }
            Ok(InputEvent::MouseWheel {
                horizontal,
                vertical,
            }) => {
                pending_wheel.0 += horizontal as i32;
                pending_wheel.1 += vertical as i32;
            }
            Ok(event) => {
                flush_pending_input(&writer, &mut pending_delta, &mut pending_wheel, &mut log);
                write_input_event(&writer, event, &mut log);
            }
            Err(RecvTimeoutError::Timeout) => {
                flush_pending_input(&writer, &mut pending_delta, &mut pending_wheel, &mut log);
            }
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
}

fn flush_pending_input(
    writer: &SharedWriter,
    pending_delta: &mut (i32, i32),
    pending_wheel: &mut (i32, i32),
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
                horizontal: clamp(pending_wheel.0, i16::MIN as i32, i16::MAX as i32) as i16,
                vertical: clamp(pending_wheel.1, i16::MIN as i32, i16::MAX as i32) as i16,
            },
            log,
        );
        *pending_wheel = (0, 0);
    }
}

struct InputSendLog {
    count: u64,
    last_print: Instant,
}

impl Default for InputSendLog {
    fn default() -> Self {
        Self {
            count: 0,
            last_print: Instant::now() - Duration::from_secs(2),
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
    if log.count == 1 || log.last_print.elapsed() >= Duration::from_secs(1) {
        eprintln!("sent input event #{}: {:?}", log.count, event);
        log.last_print = Instant::now();
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

fn unlock_cursor() {
    unsafe {
        ClipCursor(ptr::null());
    }
}

fn drop_strip_enabled() -> bool {
    std::env::var("DESKBRIDGE_DROP_STRIP")
        .map(|value| matches!(value.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"))
        .unwrap_or(false)
}

fn wide_null(value: &str) -> Vec<u16> {
    OsStr::new(value).encode_wide().chain(Some(0)).collect()
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
