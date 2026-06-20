use std::collections::{HashMap, HashSet};

use crate::config::ModifierTarget;
use crate::platform::ConnectionProfile;
use crate::protocol::{InputEvent, KeyState};

const CAPS_LOCK_SCANCODE: u16 = 58;

#[derive(Debug, Clone, Copy)]
pub struct ModifierMapping {
    command: ModifierTarget,
    control: ModifierTarget,
    option: ModifierTarget,
}

impl ModifierMapping {
    pub fn for_profile(profile: ConnectionProfile) -> Self {
        let defaults = match profile {
            ConnectionProfile::MacOSToMacOS => Self {
                command: ModifierTarget::Meta,
                control: ModifierTarget::Control,
                option: ModifierTarget::Alt,
            },
            _ => Self {
                command: ModifierTarget::Control,
                control: ModifierTarget::Control,
                option: ModifierTarget::Alt,
            },
        };
        let mapping = Self {
            command: env_modifier_target("DESKBRIDGE_MAC_COMMAND_MAPPING", defaults.command),
            control: env_modifier_target("DESKBRIDGE_MAC_CONTROL_MAPPING", defaults.control),
            option: env_modifier_target("DESKBRIDGE_MAC_OPTION_MAPPING", defaults.option),
        };
        eprintln!(
            "macOS server modifier mapping for {}: Command->{}, Control->{}, Option->{}",
            profile.as_str(),
            mapping.command.as_str(),
            mapping.control.as_str(),
            mapping.option.as_str()
        );
        mapping
    }

    #[cfg(test)]
    fn new(command: ModifierTarget, control: ModifierTarget, option: ModifierTarget) -> Self {
        Self {
            command,
            control,
            option,
        }
    }

    fn target(self, group: ModifierGroup) -> ModifierTarget {
        match group {
            ModifierGroup::Command => self.command,
            ModifierGroup::Control => self.control,
            ModifierGroup::Option => self.option,
            ModifierGroup::Shift => ModifierTarget::Disabled,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum ModifierGroup {
    Command,
    Control,
    Option,
    Shift,
}

pub struct KeyboardRouter {
    mapping: ModifierMapping,
    pressed_keys: HashMap<u16, usize>,
    active_groups: HashMap<ModifierGroup, u16>,
    regular_keys: HashSet<u16>,
}

impl KeyboardRouter {
    pub fn new(mapping: ModifierMapping) -> Self {
        Self {
            mapping,
            pressed_keys: HashMap::new(),
            active_groups: HashMap::new(),
            regular_keys: HashSet::new(),
        }
    }

    pub fn key_down(&mut self, mac_keycode: u16, _repeat: bool) -> Vec<InputEvent> {
        if modifier_group(mac_keycode).is_some() {
            return Vec::new();
        }
        let Some(scancode) = mac_keycode_to_windows_scancode(mac_keycode) else {
            return Vec::new();
        };
        if self.regular_keys.contains(&mac_keycode) {
            return vec![key_event(scancode, KeyState::Repeat)];
        }
        self.regular_keys.insert(mac_keycode);
        self.press_logical(scancode)
    }

    pub fn key_up(&mut self, mac_keycode: u16) -> Vec<InputEvent> {
        if modifier_group(mac_keycode).is_some() {
            return Vec::new();
        }
        let Some(scancode) = mac_keycode_to_windows_scancode(mac_keycode) else {
            return Vec::new();
        };
        if !self.regular_keys.remove(&mac_keycode) {
            return Vec::new();
        }
        self.release_logical(scancode)
    }

    pub fn flags_changed(&mut self, mac_keycode: u16, flags: u64) -> Vec<InputEvent> {
        if mac_keycode == 57 {
            return vec![
                key_event(CAPS_LOCK_SCANCODE, KeyState::Down),
                key_event(CAPS_LOCK_SCANCODE, KeyState::Up),
            ];
        }
        let Some(group) = modifier_group(mac_keycode) else {
            return Vec::new();
        };
        self.update_group(group, mac_keycode, modifier_flag_down(group, flags))
    }

    pub fn sync_flags(&mut self, flags: u64) -> Vec<InputEvent> {
        let mut events = Vec::new();
        for (group, representative) in [
            (ModifierGroup::Command, 55),
            (ModifierGroup::Control, 59),
            (ModifierGroup::Option, 58),
            (ModifierGroup::Shift, 56),
        ] {
            events.extend(self.update_group(
                group,
                representative,
                modifier_flag_down(group, flags),
            ));
        }
        events
    }

    pub fn release_all(&mut self) -> Vec<InputEvent> {
        let events = self
            .pressed_keys
            .keys()
            .copied()
            .map(|scancode| key_event(scancode, KeyState::Up))
            .collect();
        self.pressed_keys.clear();
        self.active_groups.clear();
        self.regular_keys.clear();
        events
    }

    fn update_group(&mut self, group: ModifierGroup, keycode: u16, down: bool) -> Vec<InputEvent> {
        if down {
            if self.active_groups.contains_key(&group) {
                return Vec::new();
            }
            let Some(scancode) = self.modifier_scancode(group, keycode) else {
                return Vec::new();
            };
            self.active_groups.insert(group, scancode);
            self.press_logical(scancode)
        } else if let Some(scancode) = self.active_groups.remove(&group) {
            self.release_logical(scancode)
        } else {
            Vec::new()
        }
    }

    fn modifier_scancode(&self, group: ModifierGroup, keycode: u16) -> Option<u16> {
        let right = matches!(keycode, 54 | 60 | 61 | 62);
        match group {
            ModifierGroup::Shift => Some(if right { 54 } else { 42 }),
            _ => target_scancode(self.mapping.target(group), right),
        }
    }

    fn press_logical(&mut self, scancode: u16) -> Vec<InputEvent> {
        let count = self.pressed_keys.entry(scancode).or_insert(0);
        *count += 1;
        if *count == 1 {
            vec![key_event(scancode, KeyState::Down)]
        } else {
            Vec::new()
        }
    }

    fn release_logical(&mut self, scancode: u16) -> Vec<InputEvent> {
        let Some(count) = self.pressed_keys.get_mut(&scancode) else {
            return Vec::new();
        };
        *count = count.saturating_sub(1);
        if *count == 0 {
            self.pressed_keys.remove(&scancode);
            vec![key_event(scancode, KeyState::Up)]
        } else {
            Vec::new()
        }
    }
}

fn key_event(scancode: u16, state: KeyState) -> InputEvent {
    InputEvent::Key { scancode, state }
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

fn modifier_group(keycode: u16) -> Option<ModifierGroup> {
    match keycode {
        54 | 55 => Some(ModifierGroup::Command),
        58 | 61 => Some(ModifierGroup::Option),
        59 | 62 => Some(ModifierGroup::Control),
        56 | 60 => Some(ModifierGroup::Shift),
        _ => None,
    }
}

fn modifier_flag_down(group: ModifierGroup, flags: u64) -> bool {
    let mask = match group {
        ModifierGroup::Command => 0x0010_0000,
        ModifierGroup::Control => 0x0004_0000,
        ModifierGroup::Option => 0x0008_0000,
        ModifierGroup::Shift => 0x0002_0000,
    };
    flags & mask != 0
}

fn mac_keycode_to_windows_scancode(keycode: u16) -> Option<u16> {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duplicate_modifier_targets_use_reference_counts() {
        let mapping = ModifierMapping::new(
            ModifierTarget::Control,
            ModifierTarget::Control,
            ModifierTarget::Alt,
        );
        let mut keyboard = KeyboardRouter::new(mapping);
        let command_down = keyboard.flags_changed(55, 0x0010_0000);
        let control_down = keyboard.flags_changed(59, 0x0014_0000);
        let command_up = keyboard.flags_changed(55, 0x0004_0000);
        let control_up = keyboard.flags_changed(59, 0);

        assert_eq!(command_down, vec![key_event(29, KeyState::Down)]);
        assert!(control_down.is_empty());
        assert!(command_up.is_empty());
        assert_eq!(control_up, vec![key_event(29, KeyState::Up)]);
    }

    #[test]
    fn repeated_flags_event_does_not_toggle_modifier_off() {
        let mapping = ModifierMapping::new(
            ModifierTarget::Meta,
            ModifierTarget::Control,
            ModifierTarget::Alt,
        );
        let mut keyboard = KeyboardRouter::new(mapping);
        assert_eq!(
            keyboard.flags_changed(55, 0x0010_0000),
            vec![key_event(347, KeyState::Down)]
        );
        assert!(keyboard.flags_changed(55, 0x0010_0000).is_empty());
        assert_eq!(
            keyboard.flags_changed(55, 0),
            vec![key_event(347, KeyState::Up)]
        );
    }

    #[test]
    fn first_autorepeat_after_crossing_still_has_a_balanced_down_and_up() {
        let mapping = ModifierMapping::new(
            ModifierTarget::Meta,
            ModifierTarget::Control,
            ModifierTarget::Alt,
        );
        let mut keyboard = KeyboardRouter::new(mapping);
        assert_eq!(
            keyboard.key_down(0, true),
            vec![key_event(30, KeyState::Down)]
        );
        assert_eq!(
            keyboard.key_down(0, true),
            vec![key_event(30, KeyState::Repeat)]
        );
        assert_eq!(keyboard.key_up(0), vec![key_event(30, KeyState::Up)]);
    }
}
