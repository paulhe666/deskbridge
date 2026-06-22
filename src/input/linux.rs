use std::io;

use crate::linux::{self, DisplayServer};
use crate::platform::ConnectionProfile;
use crate::protocol::{InputEvent, KeyState, MouseButton};

pub struct InputSink {
    backend: InputBackend,
    screen_size: (u32, u32),
}

#[derive(Debug, Clone, Copy)]
enum InputBackend {
    WaylandYdotool,
    X11Xdotool,
}

impl InputSink {
    pub fn new(_profile: ConnectionProfile) -> io::Result<Self> {
        let display = DisplayServer::detect();
        let backend = match display {
            DisplayServer::Wayland if linux::command_exists("ydotool") => InputBackend::WaylandYdotool,
            DisplayServer::Wayland => {
                return Err(linux::unsupported(
                    "Wayland input injection requires ydotool and an active ydotoold daemon",
                ));
            }
            DisplayServer::X11 if linux::command_exists("xdotool") => InputBackend::X11Xdotool,
            DisplayServer::X11 => {
                return Err(linux::unsupported("X11 input injection requires xdotool"));
            }
            DisplayServer::Unknown => {
                return Err(linux::unsupported(
                    "Linux input injection requires DISPLAY or WAYLAND_DISPLAY",
                ));
            }
        };
        let screen_size = backend.screen_size();
        eprintln!(
            "linux input backend: {} ({display}), screen {}x{}",
            backend.as_str(),
            screen_size.0,
            screen_size.1,
            display = display.as_str()
        );
        Ok(Self {
            backend,
            screen_size,
        })
    }

    pub fn apply(&mut self, event: InputEvent) -> io::Result<()> {
        self.backend.apply(event)
    }

    pub fn screen_size(&self) -> (u32, u32) {
        self.screen_size
    }
}

pub fn screen_size() -> (u32, u32) {
    match DisplayServer::detect() {
        DisplayServer::Wayland if linux::command_exists("ydotool") => {
            InputBackend::WaylandYdotool.screen_size()
        }
        DisplayServer::X11 if linux::command_exists("xdotool") => InputBackend::X11Xdotool.screen_size(),
        _ => fallback_screen_size(),
    }
}

impl InputBackend {
    fn as_str(self) -> &'static str {
        match self {
            Self::WaylandYdotool => "ydotool",
            Self::X11Xdotool => "xdotool",
        }
    }

    fn apply(self, event: InputEvent) -> io::Result<()> {
        match self {
            Self::X11Xdotool => self.apply_xdotool(event),
            Self::WaylandYdotool => self.apply_ydotool(event),
        }
    }

    fn apply_xdotool(self, event: InputEvent) -> io::Result<()> {
        match event {
            InputEvent::MouseEnter { x, y } => linux::run_status(
                "xdotool",
                ["mousemove".to_string(), x.to_string(), y.to_string()],
            ),
            InputEvent::MouseLeave => Ok(()),
            InputEvent::MouseDelta { dx, dy } => linux::run_status(
                "xdotool",
                ["mousemove_relative".to_string(), "--".to_string(), dx.to_string(), dy.to_string()],
            ),
            InputEvent::MouseButton { button, down } => {
                let button = xdotool_button(button).to_string();
                let action = if down { "mousedown" } else { "mouseup" }.to_string();
                linux::run_status("xdotool", [action, button])
            }
            InputEvent::MouseWheel {
                horizontal,
                vertical,
            } => {
                click_repeated_xdotool(if vertical > 0 { 4 } else { 5 }, vertical.unsigned_abs())?;
                click_repeated_xdotool(if horizontal > 0 { 6 } else { 7 }, horizontal.unsigned_abs())
            }
            InputEvent::Key { scancode, state } => {
                let Some(key) = scancode_to_xdotool_key(scancode) else {
                    return Ok(());
                };
                let action = match state {
                    KeyState::Down => "keydown",
                    KeyState::Up => "keyup",
                    KeyState::Repeat => "key",
                }
                .to_string();
                linux::run_status("xdotool", [action, key.to_string()])
            }
        }
    }

    fn apply_ydotool(self, event: InputEvent) -> io::Result<()> {
        match event {
            InputEvent::MouseEnter { x, y } => linux::run_status(
                "ydotool",
                ["mousemove".to_string(), "-a".to_string(), x.to_string(), y.to_string()],
            ),
            InputEvent::MouseLeave => Ok(()),
            InputEvent::MouseDelta { dx, dy } => linux::run_status(
                "ydotool",
                ["mousemove".to_string(), dx.to_string(), dy.to_string()],
            ),
            InputEvent::MouseButton { button, down } => {
                let code = ydotool_button(button, down).to_string();
                linux::run_status("ydotool", ["click".to_string(), code])
            }
            InputEvent::MouseWheel {
                horizontal,
                vertical,
            } => {
                click_repeated_ydotool(if vertical > 0 { "0xC4" } else { "0xC5" }, vertical.unsigned_abs())?;
                click_repeated_ydotool(if horizontal > 0 { "0xC6" } else { "0xC7" }, horizontal.unsigned_abs())
            }
            InputEvent::Key { scancode, state } => {
                let state_suffix = match state {
                    KeyState::Down => 1,
                    KeyState::Up => 0,
                    KeyState::Repeat => 1,
                };
                linux::run_status(
                    "ydotool",
                    ["key".to_string(), format!("{scancode}:{state_suffix}")],
                )?;
                if state == KeyState::Repeat {
                    linux::run_status("ydotool", ["key".to_string(), format!("{scancode}:0")])?;
                }
                Ok(())
            }
        }
    }

    fn screen_size(self) -> (u32, u32) {
        match self {
            Self::X11Xdotool => x11_screen_size().unwrap_or_else(fallback_screen_size),
            Self::WaylandYdotool => wayland_screen_size().unwrap_or_else(fallback_screen_size),
        }
    }
}

fn click_repeated_xdotool(button: u8, count: u16) -> io::Result<()> {
    for _ in 0..count.min(16) {
        linux::run_status("xdotool", ["click".to_string(), button.to_string()])?;
    }
    Ok(())
}

fn click_repeated_ydotool(button: &str, count: u16) -> io::Result<()> {
    for _ in 0..count.min(16) {
        linux::run_status("ydotool", ["click".to_string(), button.to_string()])?;
    }
    Ok(())
}

fn xdotool_button(button: MouseButton) -> u8 {
    match button {
        MouseButton::Left => 1,
        MouseButton::Middle => 2,
        MouseButton::Right => 3,
        MouseButton::Extra(id) => id,
    }
}

fn ydotool_button(button: MouseButton, down: bool) -> &'static str {
    match (button, down) {
        (MouseButton::Left, true) => "0x40",
        (MouseButton::Left, false) => "0x80",
        (MouseButton::Middle, true) => "0x41",
        (MouseButton::Middle, false) => "0x81",
        (MouseButton::Right, true) => "0x42",
        (MouseButton::Right, false) => "0x82",
        (MouseButton::Extra(_), true) => "0x43",
        (MouseButton::Extra(_), false) => "0x83",
    }
}

fn x11_screen_size() -> Option<(u32, u32)> {
    let output = linux::run_output("xdotool", ["getdisplaygeometry"]).ok()?;
    let text = String::from_utf8_lossy(&output);
    let mut parts = text.split_whitespace().filter_map(|value| value.parse::<u32>().ok());
    Some((parts.next()?.max(1), parts.next()?.max(1)))
}

fn wayland_screen_size() -> Option<(u32, u32)> {
    if linux::command_exists("wlr-randr") {
        let output = linux::run_output("wlr-randr", std::iter::empty::<&str>()).ok()?;
        let text = String::from_utf8_lossy(&output);
        for token in text.split_whitespace() {
            if let Some((width, height)) = token.split_once('x') {
                if let (Ok(width), Ok(height)) = (width.parse::<u32>(), height.parse::<u32>()) {
                    return Some((width.max(1), height.max(1)));
                }
            }
        }
    }
    None
}

fn fallback_screen_size() -> (u32, u32) {
    (1920, 1080)
}

fn scancode_to_xdotool_key(scancode: u16) -> Option<&'static str> {
    Some(match scancode {
        1 => "Escape",
        2 => "1",
        3 => "2",
        4 => "3",
        5 => "4",
        6 => "5",
        7 => "6",
        8 => "7",
        9 => "8",
        10 => "9",
        11 => "0",
        12 => "minus",
        13 => "equal",
        14 => "BackSpace",
        15 => "Tab",
        16 => "q",
        17 => "w",
        18 => "e",
        19 => "r",
        20 => "t",
        21 => "y",
        22 => "u",
        23 => "i",
        24 => "o",
        25 => "p",
        26 => "bracketleft",
        27 => "bracketright",
        28 => "Return",
        29 => "Control_L",
        30 => "a",
        31 => "s",
        32 => "d",
        33 => "f",
        34 => "g",
        35 => "h",
        36 => "j",
        37 => "k",
        38 => "l",
        39 => "semicolon",
        40 => "apostrophe",
        41 => "grave",
        42 => "Shift_L",
        43 => "backslash",
        44 => "z",
        45 => "x",
        46 => "c",
        47 => "v",
        48 => "b",
        49 => "n",
        50 => "m",
        51 => "comma",
        52 => "period",
        53 => "slash",
        54 => "Shift_R",
        56 => "Alt_L",
        57 => "space",
        58 => "Caps_Lock",
        59 => "F1",
        60 => "F2",
        61 => "F3",
        62 => "F4",
        63 => "F5",
        64 => "F6",
        65 => "F7",
        66 => "F8",
        67 => "F9",
        68 => "F10",
        87 => "F11",
        88 => "F12",
        96 => "KP_Enter",
        97 => "Control_R",
        100 => "Alt_R",
        103 => "Up",
        105 => "Left",
        106 => "Right",
        108 => "Down",
        110 => "Insert",
        111 => "Delete",
        119 => "Pause",
        125 => "Super_L",
        126 => "Super_R",
        _ => return None,
    })
}
