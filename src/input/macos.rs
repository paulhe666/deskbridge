use std::collections::HashSet;
use std::env;
use std::io::ErrorKind;
use std::os::raw::c_uchar;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::thread;
use std::time::{Duration, Instant};

use core_foundation::base::{Boolean, TCFType};
use core_foundation::boolean::CFBoolean;
use core_foundation::dictionary::{CFDictionary, CFDictionaryRef};
use core_foundation::string::{CFString, CFStringRef};
use core_graphics::display::CGDisplay;
use core_graphics::event::{
    CGEvent, CGEventFlags, CGEventTapLocation, CGEventType, CGMouseButton, EventField,
    ScrollEventUnit,
};
use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};
use core_graphics::geometry::CGPoint;

use crate::protocol::{InputEvent, KeyState, MouseButton};

const COMMAND_KEYCODE: u16 = 55;
const RIGHT_COMMAND_KEYCODE: u16 = 54;
const CONTROL_KEYCODE: u16 = 59;
const OPTION_KEYCODE: u16 = 58;
const RIGHT_OPTION_KEYCODE: u16 = 61;
const LEFT_SHIFT_KEYCODE: u16 = 56;
const RIGHT_SHIFT_KEYCODE: u16 = 60;
const SPACE_KEYCODE: u16 = 49;
const NUMBER_4_KEYCODE: u16 = 21;
const PRINT_SCREEN_SCANCODE: u16 = 311;
const LEFT_SHIFT_SCANCODE: u16 = 42;
const RIGHT_SHIFT_SCANCODE: u16 = 54;
const DEFAULT_SCROLL_FRAME_MS: u64 = 8;
const DEFAULT_SCROLL_SCALE: f64 = 1.25;
const DEFAULT_SCROLL_RESPONSE: f64 = 0.34;
const DEFAULT_SCROLL_MAX_STEP: f64 = 96.0;
const SCROLL_ACCEL_WINDOW: Duration = Duration::from_millis(85);

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn AXIsProcessTrusted() -> c_uchar;
    fn AXIsProcessTrustedWithOptions(options: CFDictionaryRef) -> Boolean;
    static kAXTrustedCheckOptionPrompt: CFStringRef;
}

pub struct InputSink {
    source: CGEventSource,
    pressed_keys: HashSet<u16>,
    mouse_buttons: MouseButtons,
    screen_size: (i32, i32),
    mouse_position: CGPoint,
    scroll: SmoothScroller,
    shift_tap_candidate: Option<u16>,
}

impl InputSink {
    pub fn new() -> std::io::Result<Self> {
        let source = CGEventSource::new(CGEventSourceStateID::HIDSystemState)
            .map_err(|_| event_err("failed to create event source"))?;
        if !accessibility_trusted() {
            eprintln!(
                "warning: macOS Accessibility permission is not granted for this process; keyboard and mouse buttons may be ignored"
            );
            request_accessibility_permission();
        }
        Ok(Self {
            source,
            pressed_keys: HashSet::new(),
            mouse_buttons: MouseButtons::default(),
            screen_size: screen_size_i32(),
            mouse_position: CGPoint::new(0.0, 0.0),
            scroll: SmoothScroller::spawn(),
            shift_tap_candidate: None,
        })
    }

    pub fn apply(&mut self, event: InputEvent) -> std::io::Result<()> {
        match event {
            InputEvent::Key { scancode, state } => self.key(scancode, state),
            InputEvent::MouseEnter { x, y } => self.mouse_enter(x, y),
            InputEvent::MouseDelta { dx, dy } => self.mouse_delta(dx, dy),
            InputEvent::MouseButton { button, down } => self.mouse_button(button, down),
            InputEvent::MouseWheel {
                horizontal,
                vertical,
            } => self.mouse_wheel(horizontal, vertical),
        }
    }

    fn key(&mut self, scancode: u16, state: KeyState) -> std::io::Result<()> {
        if scancode == PRINT_SCREEN_SCANCODE {
            if state == KeyState::Down {
                self.screenshot_hotkey()?;
            }
            return Ok(());
        }

        self.update_shift_tap_state(scancode, state);
        let Some(keycode) = scancode_to_macos_key(scancode) else {
            return Ok(());
        };
        match state {
            KeyState::Down => {
                if self.pressed_keys.insert(keycode) {
                    self.post_key(keycode, true)?;
                }
            }
            KeyState::Up => {
                self.pressed_keys.remove(&keycode);
                self.post_key(keycode, false)?;
                if is_shift_scancode(scancode) && self.shift_tap_candidate == Some(scancode) {
                    self.shift_tap_candidate = None;
                    self.toggle_input_source()?;
                }
            }
            KeyState::Repeat => {
                if self.pressed_keys.contains(&keycode) {
                    self.post_key_repeat(keycode)?;
                } else {
                    self.pressed_keys.insert(keycode);
                    self.post_key(keycode, true)?;
                }
            }
        }
        Ok(())
    }

    fn update_shift_tap_state(&mut self, scancode: u16, state: KeyState) {
        if is_shift_scancode(scancode) {
            if state == KeyState::Down {
                self.shift_tap_candidate = Some(scancode);
            }
            return;
        }
        if matches!(state, KeyState::Down | KeyState::Repeat) {
            self.shift_tap_candidate = None;
        }
    }

    fn screenshot_hotkey(&self) -> std::io::Result<()> {
        let command = CGEventFlags::CGEventFlagCommand;
        let command_control = CGEventFlags::CGEventFlagCommand | CGEventFlags::CGEventFlagControl;
        let full_flags = command_control | CGEventFlags::CGEventFlagShift;
        self.post_key_with_flags(COMMAND_KEYCODE, true, command)?;
        self.post_key_with_flags(CONTROL_KEYCODE, true, command_control)?;
        self.post_key_with_flags(LEFT_SHIFT_KEYCODE, true, full_flags)?;
        self.post_key_with_flags(NUMBER_4_KEYCODE, true, full_flags)?;
        self.post_key_with_flags(NUMBER_4_KEYCODE, false, full_flags)?;
        self.post_key_with_flags(LEFT_SHIFT_KEYCODE, false, command_control)?;
        self.post_key_with_flags(CONTROL_KEYCODE, false, command)?;
        self.post_key_with_flags(COMMAND_KEYCODE, false, CGEventFlags::empty())?;
        Ok(())
    }

    fn toggle_input_source(&self) -> std::io::Result<()> {
        self.post_key_with_flags(CONTROL_KEYCODE, true, CGEventFlags::CGEventFlagControl)?;
        self.post_key_with_flags(SPACE_KEYCODE, true, CGEventFlags::CGEventFlagControl)?;
        self.post_key_with_flags(SPACE_KEYCODE, false, CGEventFlags::CGEventFlagControl)?;
        self.post_key_with_flags(CONTROL_KEYCODE, false, CGEventFlags::empty())
    }

    fn post_key(&self, keycode: u16, down: bool) -> std::io::Result<()> {
        self.post_key_with_flags_and_repeat(
            keycode,
            down,
            flags_for_pressed_keys(&self.pressed_keys),
            false,
        )
    }

    fn post_key_repeat(&self, keycode: u16) -> std::io::Result<()> {
        self.post_key_with_flags_and_repeat(
            keycode,
            true,
            flags_for_pressed_keys(&self.pressed_keys),
            true,
        )
    }

    fn post_key_with_flags(
        &self,
        keycode: u16,
        down: bool,
        flags: CGEventFlags,
    ) -> std::io::Result<()> {
        self.post_key_with_flags_and_repeat(keycode, down, flags, false)
    }

    fn post_key_with_flags_and_repeat(
        &self,
        keycode: u16,
        down: bool,
        flags: CGEventFlags,
        autorepeat: bool,
    ) -> std::io::Result<()> {
        let event = CGEvent::new_keyboard_event(self.source.clone(), keycode, down)
            .map_err(|_| event_err("failed to create key event"))?;
        event.set_flags(flags);
        event.set_integer_value_field(EventField::KEYBOARD_EVENT_AUTOREPEAT, i64::from(autorepeat));
        event.post(CGEventTapLocation::HID);
        Ok(())
    }

    fn mouse_enter(&mut self, x: i32, y: i32) -> std::io::Result<()> {
        self.screen_size = screen_size_i32();
        self.set_mouse_position(x, y);
        self.post_pointer_motion()
    }

    fn mouse_delta(&mut self, dx: i32, dy: i32) -> std::io::Result<()> {
        let x = self.mouse_position.x as i32 + dx;
        let y = self.mouse_position.y as i32 + dy;
        self.set_mouse_position(x, y);
        self.post_pointer_motion()
    }

    fn set_mouse_position(&mut self, x: i32, y: i32) {
        let x = x.clamp(0, self.screen_size.0.saturating_sub(1));
        let y = y.clamp(0, self.screen_size.1.saturating_sub(1));
        self.mouse_position = CGPoint::new(x as f64, y as f64);
    }

    fn post_pointer_motion(&self) -> std::io::Result<()> {
        if let Some((drag_ty, button)) = self.mouse_buttons.drag_event() {
            let _ = CGDisplay::warp_mouse_cursor_position(self.mouse_position);
            self.post_mouse_event(drag_ty, button)?;
            return Ok(());
        }

        if let Err(code) = CGDisplay::warp_mouse_cursor_position(self.mouse_position) {
            eprintln!("cursor warp failed ({code}); falling back to mouse move event");
            self.post_mouse_event(CGEventType::MouseMoved, CGMouseButton::Left)?;
        }
        Ok(())
    }

    fn post_mouse_event(
        &self,
        event_type: CGEventType,
        button: CGMouseButton,
    ) -> std::io::Result<()> {
        let event =
            CGEvent::new_mouse_event(self.source.clone(), event_type, self.mouse_position, button)
                .map_err(|_| event_err("failed to create mouse event"))?;
        event.post(CGEventTapLocation::HID);
        Ok(())
    }

    fn mouse_button(&mut self, button: MouseButton, down: bool) -> std::io::Result<()> {
        let (button, down_ty, up_ty) = match button {
            MouseButton::Left => (
                CGMouseButton::Left,
                CGEventType::LeftMouseDown,
                CGEventType::LeftMouseUp,
            ),
            MouseButton::Right => (
                CGMouseButton::Right,
                CGEventType::RightMouseDown,
                CGEventType::RightMouseUp,
            ),
            _ => (
                CGMouseButton::Center,
                CGEventType::OtherMouseDown,
                CGEventType::OtherMouseUp,
            ),
        };
        self.mouse_buttons.set(button, down);
        self.post_mouse_event(if down { down_ty } else { up_ty }, button)?;
        Ok(())
    }

    fn mouse_wheel(&self, horizontal: i16, vertical: i16) -> std::io::Result<()> {
        self.scroll.push(horizontal as i32, vertical as i32)
    }
}

fn accessibility_trusted() -> bool {
    unsafe { AXIsProcessTrusted() != 0 }
}

fn request_accessibility_permission() {
    let key = unsafe { CFString::wrap_under_get_rule(kAXTrustedCheckOptionPrompt) };
    let prompt = CFBoolean::true_value();
    let options = CFDictionary::from_CFType_pairs(&[(key.as_CFType(), prompt.as_CFType())]);
    let trusted = unsafe { AXIsProcessTrustedWithOptions(options.as_concrete_TypeRef()) != 0 };
    if !trusted {
        eprintln!(
            "open System Settings > Privacy & Security > Accessibility and enable Deskbridge"
        );
    }
}

#[derive(Default)]
struct MouseButtons {
    left: bool,
    right: bool,
    other: bool,
}

impl MouseButtons {
    fn set(&mut self, button: CGMouseButton, down: bool) {
        match button {
            CGMouseButton::Left => self.left = down,
            CGMouseButton::Right => self.right = down,
            _ => self.other = down,
        }
    }

    fn drag_event(&self) -> Option<(CGEventType, CGMouseButton)> {
        if self.left {
            Some((CGEventType::LeftMouseDragged, CGMouseButton::Left))
        } else if self.right {
            Some((CGEventType::RightMouseDragged, CGMouseButton::Right))
        } else if self.other {
            Some((CGEventType::OtherMouseDragged, CGMouseButton::Center))
        } else {
            None
        }
    }
}

#[derive(Clone)]
struct SmoothScroller {
    sender: Sender<ScrollDelta>,
}

#[derive(Debug, Clone, Copy)]
struct ScrollDelta {
    horizontal: i32,
    vertical: i32,
    at: Instant,
}

#[derive(Debug, Clone, Copy)]
struct ScrollConfig {
    frame_interval: Duration,
    scale: f64,
    response: f64,
    max_step: f64,
}

impl SmoothScroller {
    fn spawn() -> Self {
        let (sender, receiver) = mpsc::channel();
        thread::spawn(move || scroll_worker_loop(receiver, ScrollConfig::from_env()));
        Self { sender }
    }

    fn push(&self, horizontal: i32, vertical: i32) -> std::io::Result<()> {
        self.sender
            .send(ScrollDelta {
                horizontal,
                vertical,
                at: Instant::now(),
            })
            .map_err(|e| std::io::Error::new(ErrorKind::BrokenPipe, e))
    }
}

impl ScrollConfig {
    fn from_env() -> Self {
        let frame_ms = env_u64("DESKBRIDGE_SCROLL_FRAME_MS", DEFAULT_SCROLL_FRAME_MS).max(4);
        Self {
            frame_interval: Duration::from_millis(frame_ms),
            scale: env_f64("DESKBRIDGE_SCROLL_SCALE", DEFAULT_SCROLL_SCALE).clamp(0.2, 5.0),
            response: env_f64("DESKBRIDGE_SCROLL_RESPONSE", DEFAULT_SCROLL_RESPONSE)
                .clamp(0.12, 0.75),
            max_step: env_f64("DESKBRIDGE_SCROLL_MAX_STEP", DEFAULT_SCROLL_MAX_STEP)
                .clamp(12.0, 320.0),
        }
    }
}

fn scroll_worker_loop(receiver: Receiver<ScrollDelta>, config: ScrollConfig) {
    let source = match CGEventSource::new(CGEventSourceStateID::HIDSystemState) {
        Ok(source) => source,
        Err(_) => {
            eprintln!("smooth scroll disabled: failed to create event source");
            return;
        }
    };
    let mut pending_h = 0.0;
    let mut pending_v = 0.0;
    let mut last_input = None;
    let mut last_emit = Instant::now();

    loop {
        let timeout = config
            .frame_interval
            .checked_sub(last_emit.elapsed())
            .unwrap_or(Duration::ZERO);
        match receiver.recv_timeout(timeout) {
            Ok(delta) => {
                add_scroll_delta(
                    delta,
                    &config,
                    &mut pending_h,
                    &mut pending_v,
                    &mut last_input,
                );
                loop {
                    match receiver.try_recv() {
                        Ok(delta) => add_scroll_delta(
                            delta,
                            &config,
                            &mut pending_h,
                            &mut pending_v,
                            &mut last_input,
                        ),
                        Err(TryRecvError::Empty) => break,
                        Err(TryRecvError::Disconnected) => return,
                    }
                }
            }
            Err(RecvTimeoutError::Timeout) => {
                emit_scroll_frame(&source, &config, &mut pending_h, &mut pending_v);
                last_emit = Instant::now();
            }
            Err(RecvTimeoutError::Disconnected) => return,
        }

        if last_emit.elapsed() >= config.frame_interval {
            emit_scroll_frame(&source, &config, &mut pending_h, &mut pending_v);
            last_emit = Instant::now();
        }
    }
}

fn add_scroll_delta(
    delta: ScrollDelta,
    config: &ScrollConfig,
    pending_h: &mut f64,
    pending_v: &mut f64,
    last_input: &mut Option<Instant>,
) {
    let boost = last_input
        .map(|last| scroll_boost(delta.at.saturating_duration_since(last)))
        .unwrap_or(1.0);
    *pending_h += delta.horizontal as f64 * config.scale * boost;
    *pending_v += delta.vertical as f64 * config.scale * boost;
    *last_input = Some(delta.at);
}

fn scroll_boost(delta: Duration) -> f64 {
    if delta >= SCROLL_ACCEL_WINDOW {
        1.0
    } else {
        let closeness = 1.0 - delta.as_secs_f64() / SCROLL_ACCEL_WINDOW.as_secs_f64();
        1.0 + closeness * 0.45
    }
}

fn emit_scroll_frame(
    source: &CGEventSource,
    config: &ScrollConfig,
    pending_h: &mut f64,
    pending_v: &mut f64,
) {
    let horizontal = take_scroll_step(pending_h, config);
    let vertical = take_scroll_step(pending_v, config);
    if horizontal == 0 && vertical == 0 {
        return;
    }
    if let Err(e) = post_scroll(source, horizontal, vertical) {
        eprintln!("scroll post failed: {e}");
    }
}

fn take_scroll_step(value: &mut f64, config: &ScrollConfig) -> i32 {
    if value.abs() < 0.75 {
        *value = 0.0;
        return 0;
    }
    let sign = value.signum();
    let mut step = (*value * config.response).clamp(-config.max_step, config.max_step);
    if step.abs() < 1.0 {
        step = sign;
    }
    let mut rounded = step.round() as i32;
    if rounded == 0 {
        rounded = sign as i32;
    }
    *value -= rounded as f64;
    rounded
}

fn post_scroll(source: &CGEventSource, horizontal: i32, vertical: i32) -> std::io::Result<()> {
    let event = CGEvent::new_scroll_event(
        source.clone(),
        ScrollEventUnit::PIXEL,
        2,
        vertical,
        horizontal,
        0,
    )
    .map_err(|_| event_err("failed to create scroll event"))?;
    event.set_integer_value_field(EventField::SCROLL_WHEEL_EVENT_IS_CONTINUOUS, 1);
    event.set_integer_value_field(
        EventField::SCROLL_WHEEL_EVENT_POINT_DELTA_AXIS_1,
        vertical as i64,
    );
    event.set_integer_value_field(
        EventField::SCROLL_WHEEL_EVENT_POINT_DELTA_AXIS_2,
        horizontal as i64,
    );
    event.post(CGEventTapLocation::HID);
    Ok(())
}

fn env_f64(name: &str, default: f64) -> f64 {
    env::var(name)
        .ok()
        .and_then(|value| value.parse::<f64>().ok())
        .unwrap_or(default)
}

fn env_u64(name: &str, default: u64) -> u64 {
    env::var(name)
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(default)
}

fn is_shift_scancode(scancode: u16) -> bool {
    matches!(scancode, LEFT_SHIFT_SCANCODE | RIGHT_SHIFT_SCANCODE)
}

fn scancode_to_macos_key(scancode: u16) -> Option<u16> {
    Some(match scancode {
        1 => 53,
        2 => 18,
        3 => 19,
        4 => 20,
        5 => NUMBER_4_KEYCODE,
        6 => 23,
        7 => 22,
        8 => 26,
        9 => 28,
        10 => 25,
        11 => 29,
        12 => 27,
        13 => 24,
        14 => 51,
        15 => 48,
        16 => 12,
        17 => 13,
        18 => 14,
        19 => 15,
        20 => 17,
        21 => 16,
        22 => 32,
        23 => 34,
        24 => 31,
        25 => 35,
        26 => 33,
        27 => 30,
        28 => 36,
        29 => COMMAND_KEYCODE,
        30 => 0,
        31 => 1,
        32 => 2,
        33 => 3,
        34 => 5,
        35 => 4,
        36 => 38,
        37 => 40,
        38 => 37,
        39 => 41,
        40 => 39,
        41 => 50,
        42 => LEFT_SHIFT_KEYCODE,
        43 => 42,
        44 => 6,
        45 => 7,
        46 => 8,
        47 => 9,
        48 => 11,
        49 => 45,
        50 => 46,
        51 => 43,
        52 => 47,
        53 => 44,
        54 => RIGHT_SHIFT_KEYCODE,
        56 => OPTION_KEYCODE,
        57 => SPACE_KEYCODE,
        58 => 57,
        59 => 122,
        60 => 120,
        61 => 99,
        62 => 118,
        63 => 96,
        64 => 97,
        65 => 98,
        66 => 100,
        67 => 101,
        68 => 109,
        87 => 103,
        88 => 111,
        284 => 76,
        285 => RIGHT_COMMAND_KEYCODE,
        309 => 75,
        312 => RIGHT_OPTION_KEYCODE,
        327 => 115,
        328 => 126,
        329 => 116,
        331 => 123,
        333 => 124,
        335 => 119,
        336 => 125,
        337 => 121,
        338 => 114,
        339 => 117,
        347 => COMMAND_KEYCODE,
        348 => RIGHT_COMMAND_KEYCODE,
        _ => return None,
    })
}

fn flags_for_pressed_keys(keys: &HashSet<u16>) -> CGEventFlags {
    let mut flags = CGEventFlags::empty();
    if keys.contains(&COMMAND_KEYCODE) || keys.contains(&RIGHT_COMMAND_KEYCODE) {
        flags |= CGEventFlags::CGEventFlagCommand;
    }
    if keys.contains(&CONTROL_KEYCODE) {
        flags |= CGEventFlags::CGEventFlagControl;
    }
    if keys.contains(&OPTION_KEYCODE) || keys.contains(&RIGHT_OPTION_KEYCODE) {
        flags |= CGEventFlags::CGEventFlagAlternate;
    }
    if keys.contains(&LEFT_SHIFT_KEYCODE) || keys.contains(&RIGHT_SHIFT_KEYCODE) {
        flags |= CGEventFlags::CGEventFlagShift;
    }
    flags
}

fn event_err(message: &str) -> std::io::Error {
    std::io::Error::new(ErrorKind::Other, message)
}

pub fn screen_size() -> (u32, u32) {
    let display = CGDisplay::main();
    (display.pixels_wide() as u32, display.pixels_high() as u32)
}

fn screen_size_i32() -> (i32, i32) {
    let (width, height) = screen_size();
    (width.max(1) as i32, height.max(1) as i32)
}
