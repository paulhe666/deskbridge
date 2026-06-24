use std::collections::HashSet;
use std::mem::size_of;

use windows_sys::Win32::Foundation::POINT;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBDINPUT, KEYEVENTF_EXTENDEDKEY,
    KEYEVENTF_KEYUP, KEYEVENTF_SCANCODE, MOUSEEVENTF_HWHEEL, MOUSEEVENTF_LEFTDOWN,
    MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP, MOUSEEVENTF_MOVE,
    MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP, MOUSEEVENTF_WHEEL, MOUSEEVENTF_XDOWN,
    MOUSEEVENTF_XUP, MOUSEINPUT, SendInput,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetCursorPos, GetSystemMetrics, SM_CXSCREEN, SM_CYSCREEN, SetCursorPos, SetProcessDPIAware,
};

use crate::platform::ConnectionProfile;
use crate::protocol::{InputEvent, KeyState, MouseButton};

const XBUTTON1_DATA: u32 = 1;
const XBUTTON2_DATA: u32 = 2;

pub struct InputSink {
    pressed_keys: HashSet<u16>,
    pressed_buttons: HashSet<MouseButton>,
    mouse_position: (i32, i32),
    screen_size: (i32, i32),
    remote_active: bool,
}

impl InputSink {
    pub fn new(profile: ConnectionProfile) -> std::io::Result<Self> {
        eprintln!("Windows input profile: {}", profile.as_str());
        unsafe {
            SetProcessDPIAware();
        }
        let screen_size = screen_size_i32();
        let mut point = POINT { x: 0, y: 0 };
        let mouse_position = if unsafe { GetCursorPos(&mut point) } != 0 {
            (
                point.x.clamp(0, screen_size.0 - 1),
                point.y.clamp(0, screen_size.1 - 1),
            )
        } else {
            (0, 0)
        };
        Ok(Self {
            pressed_keys: HashSet::new(),
            pressed_buttons: HashSet::new(),
            mouse_position,
            screen_size,
            remote_active: false,
        })
    }

    pub fn apply(&mut self, event: InputEvent) -> std::io::Result<()> {
        match event {
            InputEvent::MouseEnter { x, y } => self.mouse_enter(x, y),
            InputEvent::MouseLeave => {
                self.mouse_leave();
                Ok(())
            }
            _ if !self.remote_active => Ok(()),
            InputEvent::Key { scancode, state } => self.key(scancode, state),
            InputEvent::MouseDelta { dx, dy } => self.mouse_delta(dx, dy),
            InputEvent::MouseButton { button, down } => self.mouse_button(button, down),
            InputEvent::MouseWheel {
                horizontal,
                vertical,
            } => self.mouse_wheel(horizontal, vertical),
        }
    }

    pub fn screen_size(&self) -> (u32, u32) {
        (self.screen_size.0 as u32, self.screen_size.1 as u32)
    }

    fn key(&mut self, scancode: u16, state: KeyState) -> std::io::Result<()> {
        match state {
            KeyState::Down => {
                if self.pressed_keys.insert(scancode) {
                    send_key(scancode, false)?;
                }
            }
            KeyState::Up => {
                self.pressed_keys.remove(&scancode);
                send_key(scancode, true)?;
            }
            KeyState::Repeat => {
                send_key(scancode, false)?;
            }
        }
        Ok(())
    }

    fn mouse_enter(&mut self, x: i32, y: i32) -> std::io::Result<()> {
        if self.remote_active {
            self.release_remote_input();
        }
        self.remote_active = true;
        self.screen_size = screen_size_i32();
        self.mouse_position = (
            x.clamp(0, self.screen_size.0 - 1),
            y.clamp(0, self.screen_size.1 - 1),
        );
        if unsafe { SetCursorPos(self.mouse_position.0, self.mouse_position.1) } == 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(())
    }

    fn mouse_delta(&mut self, dx: i32, dy: i32) -> std::io::Result<()> {
        let old = self.mouse_position;
        let next = (
            old.0.saturating_add(dx).clamp(0, self.screen_size.0 - 1),
            old.1.saturating_add(dy).clamp(0, self.screen_size.1 - 1),
        );
        self.mouse_position = next;

        let allowed_dx = next.0.saturating_sub(old.0);
        let allowed_dy = next.1.saturating_sub(old.1);
        if allowed_dx == 0 && allowed_dy == 0 {
            return Ok(());
        }
        send_mouse(MOUSEEVENTF_MOVE, allowed_dx, allowed_dy, 0)
    }

    fn mouse_button(&mut self, button: MouseButton, down: bool) -> std::io::Result<()> {
        let (flags, data) = match button {
            MouseButton::Left => (
                if down {
                    MOUSEEVENTF_LEFTDOWN
                } else {
                    MOUSEEVENTF_LEFTUP
                },
                0,
            ),
            MouseButton::Middle => (
                if down {
                    MOUSEEVENTF_MIDDLEDOWN
                } else {
                    MOUSEEVENTF_MIDDLEUP
                },
                0,
            ),
            MouseButton::Right => (
                if down {
                    MOUSEEVENTF_RIGHTDOWN
                } else {
                    MOUSEEVENTF_RIGHTUP
                },
                0,
            ),
            MouseButton::Extra(id) => (
                if down {
                    MOUSEEVENTF_XDOWN
                } else {
                    MOUSEEVENTF_XUP
                },
                if id == 5 {
                    XBUTTON2_DATA
                } else {
                    XBUTTON1_DATA
                },
            ),
        };
        send_mouse(flags, 0, 0, data)?;
        if down {
            self.pressed_buttons.insert(button);
        } else {
            self.pressed_buttons.remove(&button);
        }
        Ok(())
    }

    fn mouse_wheel(&self, horizontal: i16, vertical: i16) -> std::io::Result<()> {
        if vertical != 0 {
            send_mouse(MOUSEEVENTF_WHEEL, 0, 0, vertical as i32 as u32)?;
        }
        if horizontal != 0 {
            send_mouse(MOUSEEVENTF_HWHEEL, 0, 0, horizontal as i32 as u32)?;
        }
        Ok(())
    }

    fn mouse_leave(&mut self) {
        self.release_remote_input();
        self.remote_active = false;
    }

    fn release_remote_input(&mut self) {
        for scancode in self.pressed_keys.drain().collect::<Vec<_>>() {
            let _ = send_key(scancode, true);
        }
        for button in self.pressed_buttons.drain().collect::<Vec<_>>() {
            let _ = release_mouse_button(button);
        }
    }
}

impl Drop for InputSink {
    fn drop(&mut self) {
        self.release_remote_input();
    }
}

fn release_mouse_button(button: MouseButton) -> std::io::Result<()> {
    let (flags, data) = match button {
        MouseButton::Left => (MOUSEEVENTF_LEFTUP, 0),
        MouseButton::Middle => (MOUSEEVENTF_MIDDLEUP, 0),
        MouseButton::Right => (MOUSEEVENTF_RIGHTUP, 0),
        MouseButton::Extra(id) => (
            MOUSEEVENTF_XUP,
            if id == 5 {
                XBUTTON2_DATA
            } else {
                XBUTTON1_DATA
            },
        ),
    };
    send_mouse(flags, 0, 0, data)
}

fn send_key(scancode: u16, key_up: bool) -> std::io::Result<()> {
    if let Some(vk) = windows_modifier_vk(scancode) {
        return send_virtual_key(vk, true, key_up);
    }
    if let Some(vk) = printable_vk_from_scancode(scancode) {
        return send_virtual_key(vk, false, key_up);
    }

    let (scan, extended) = decode_scancode(scancode);
    let mut flags = KEYEVENTF_SCANCODE;
    if extended {
        flags |= KEYEVENTF_EXTENDEDKEY;
    }
    if key_up {
        flags |= KEYEVENTF_KEYUP;
    }

    send_input(INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: 0,
                wScan: scan,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    })
}

fn windows_modifier_vk(scancode: u16) -> Option<u16> {
    match scancode {
        347 => Some(0x5B), // VK_LWIN
        348 => Some(0x5C), // VK_RWIN
        _ => None,
    }
}

fn printable_vk_from_scancode(scancode: u16) -> Option<u16> {
    match scancode {
        2 => Some(0x31),  // 1
        3 => Some(0x32),  // 2
        4 => Some(0x33),  // 3
        5 => Some(0x34),  // 4
        6 => Some(0x35),  // 5
        7 => Some(0x36),  // 6
        8 => Some(0x37),  // 7
        9 => Some(0x38),  // 8
        10 => Some(0x39), // 9
        11 => Some(0x30), // 0
        12 => Some(0xBD), // VK_OEM_MINUS
        13 => Some(0xBB), // VK_OEM_PLUS
        16 => Some(0x51), // Q
        17 => Some(0x57), // W
        18 => Some(0x45), // E
        19 => Some(0x52), // R
        20 => Some(0x54), // T
        21 => Some(0x59), // Y
        22 => Some(0x55), // U
        23 => Some(0x49), // I
        24 => Some(0x4F), // O
        25 => Some(0x50), // P
        26 => Some(0xDB), // VK_OEM_4 ([)
        27 => Some(0xDD), // VK_OEM_6 (])
        30 => Some(0x41), // A
        31 => Some(0x53), // S
        32 => Some(0x44), // D
        33 => Some(0x46), // F
        34 => Some(0x47), // G
        35 => Some(0x48), // H
        36 => Some(0x4A), // J
        37 => Some(0x4B), // K
        38 => Some(0x4C), // L
        39 => Some(0xBA), // VK_OEM_1 (;)
        40 => Some(0xDE), // VK_OEM_7 (')
        41 => Some(0xC0), // VK_OEM_3 (`)
        43 => Some(0xDC), // VK_OEM_5 (\\)
        44 => Some(0x5A), // Z
        45 => Some(0x58), // X
        46 => Some(0x43), // C
        47 => Some(0x56), // V
        48 => Some(0x42), // B
        49 => Some(0x4E), // N
        50 => Some(0x4D), // M
        51 => Some(0xBC), // VK_OEM_COMMA
        52 => Some(0xBE), // VK_OEM_PERIOD
        53 => Some(0xBF), // VK_OEM_2 (/)
        57 => Some(0x20), // Space
        71 => Some(0x67), // Numpad 7
        72 => Some(0x68), // Numpad 8
        73 => Some(0x69), // Numpad 9
        74 => Some(0x6D), // Numpad subtract
        75 => Some(0x64), // Numpad 4
        76 => Some(0x65), // Numpad 5
        77 => Some(0x66), // Numpad 6
        78 => Some(0x6B), // Numpad add
        79 => Some(0x61), // Numpad 1
        80 => Some(0x62), // Numpad 2
        81 => Some(0x63), // Numpad 3
        82 => Some(0x60), // Numpad 0
        83 => Some(0x6E), // Numpad decimal
        _ => None,
    }
}

fn send_virtual_key(vk: u16, extended: bool, key_up: bool) -> std::io::Result<()> {
    let mut flags = 0;
    if extended {
        flags |= KEYEVENTF_EXTENDEDKEY;
    }
    if key_up {
        flags |= KEYEVENTF_KEYUP;
    }

    send_input(INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: vk,
                wScan: 0,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    })
}

fn decode_scancode(scancode: u16) -> (u16, bool) {
    if scancode >= 256 {
        (scancode - 256, true)
    } else {
        (scancode, false)
    }
}

fn send_mouse(flags: u32, dx: i32, dy: i32, data: u32) -> std::io::Result<()> {
    send_input(INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dx,
                dy,
                mouseData: data,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    })
}

fn send_input(input: INPUT) -> std::io::Result<()> {
    let sent = unsafe { SendInput(1, &input, size_of::<INPUT>() as i32) };
    if sent == 1 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

fn screen_size() -> (u32, u32) {
    let width = unsafe { GetSystemMetrics(SM_CXSCREEN) }.max(1) as u32;
    let height = unsafe { GetSystemMetrics(SM_CYSCREEN) }.max(1) as u32;
    (width, height)
}

fn screen_size_i32() -> (i32, i32) {
    let (width, height) = screen_size();
    (width as i32, height as i32)
}
