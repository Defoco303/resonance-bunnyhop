use anyhow::{Result, anyhow};
use windows::Win32::UI::Input::XboxController::{
    XINPUT_GAMEPAD_A, XINPUT_GAMEPAD_B, XINPUT_GAMEPAD_BACK, XINPUT_GAMEPAD_BUTTON_FLAGS,
    XINPUT_GAMEPAD_DPAD_DOWN, XINPUT_GAMEPAD_DPAD_LEFT, XINPUT_GAMEPAD_DPAD_RIGHT,
    XINPUT_GAMEPAD_DPAD_UP, XINPUT_GAMEPAD_LEFT_SHOULDER, XINPUT_GAMEPAD_LEFT_THUMB,
    XINPUT_GAMEPAD_RIGHT_SHOULDER, XINPUT_GAMEPAD_RIGHT_THUMB, XINPUT_GAMEPAD_START,
    XINPUT_GAMEPAD_X, XINPUT_GAMEPAD_Y, XINPUT_STATE, XInputGetState,
};

use crate::{
    assist::{Buttons, ControllerState, DpadDirection, StickState},
    dualsense::InputDeviceInfo,
};

const ERROR_DEVICE_NOT_CONNECTED: u32 = 1_167;
const ERROR_SUCCESS: u32 = 0;
const XINPUT_SLOT_COUNT: u32 = 4;
const XINPUT_TRIGGER_BUTTON_THRESHOLD: u8 = 30;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct XInputSnapshot {
    pub packet_number: u32,
    pub state: ControllerState,
}

pub fn scan_xinput_devices() -> Vec<InputDeviceInfo> {
    let mut devices = Vec::new();

    for slot in 0..XINPUT_SLOT_COUNT {
        if matches!(poll_xinput_slot(slot), Ok(Some(_))) {
            devices.push(InputDeviceInfo::xinput(slot, "XInput Controller"));
        }
    }

    devices
}

pub fn poll_xinput_slot(slot: u32) -> Result<Option<XInputSnapshot>> {
    let mut state = XINPUT_STATE::default();
    let status = unsafe { XInputGetState(slot, &mut state) };

    match status {
        ERROR_SUCCESS => Ok(Some(XInputSnapshot {
            packet_number: state.dwPacketNumber,
            state: map_xinput_state(state),
        })),
        ERROR_DEVICE_NOT_CONNECTED => Ok(None),
        code => Err(anyhow!(
            "XInput slot {slot} polling failed with error code {code}"
        )),
    }
}

fn map_xinput_state(raw: XINPUT_STATE) -> ControllerState {
    let gamepad = raw.Gamepad;
    let buttons = gamepad.wButtons;

    ControllerState {
        left_stick: StickState::new(
            raw_from_xinput_axis(i32::from(gamepad.sThumbLX)),
            raw_from_xinput_axis(-i32::from(gamepad.sThumbLY)),
        ),
        right_stick: StickState::new(
            raw_from_xinput_axis(i32::from(gamepad.sThumbRX)),
            raw_from_xinput_axis(-i32::from(gamepad.sThumbRY)),
        ),
        l2: gamepad.bLeftTrigger,
        r2: gamepad.bRightTrigger,
        dpad: dpad_from_buttons(buttons),
        buttons: Buttons {
            square: button_pressed(buttons, XINPUT_GAMEPAD_X),
            circle: button_pressed(buttons, XINPUT_GAMEPAD_B),
            cross: button_pressed(buttons, XINPUT_GAMEPAD_A),
            triangle: button_pressed(buttons, XINPUT_GAMEPAD_Y),
            l1: button_pressed(buttons, XINPUT_GAMEPAD_LEFT_SHOULDER),
            r1: button_pressed(buttons, XINPUT_GAMEPAD_RIGHT_SHOULDER),
            l2_button: gamepad.bLeftTrigger >= XINPUT_TRIGGER_BUTTON_THRESHOLD,
            r2_button: gamepad.bRightTrigger >= XINPUT_TRIGGER_BUTTON_THRESHOLD,
            create: button_pressed(buttons, XINPUT_GAMEPAD_BACK),
            options: button_pressed(buttons, XINPUT_GAMEPAD_START),
            l3: button_pressed(buttons, XINPUT_GAMEPAD_LEFT_THUMB),
            r3: button_pressed(buttons, XINPUT_GAMEPAD_RIGHT_THUMB),
            ps: false,
            touchpad: false,
            mute: false,
        },
    }
}

fn button_pressed(buttons: XINPUT_GAMEPAD_BUTTON_FLAGS, mask: XINPUT_GAMEPAD_BUTTON_FLAGS) -> bool {
    (buttons & mask) == mask
}

fn dpad_from_buttons(buttons: XINPUT_GAMEPAD_BUTTON_FLAGS) -> DpadDirection {
    match (
        button_pressed(buttons, XINPUT_GAMEPAD_DPAD_UP),
        button_pressed(buttons, XINPUT_GAMEPAD_DPAD_DOWN),
        button_pressed(buttons, XINPUT_GAMEPAD_DPAD_LEFT),
        button_pressed(buttons, XINPUT_GAMEPAD_DPAD_RIGHT),
    ) {
        (true, false, false, false) => DpadDirection::North,
        (true, false, false, true) => DpadDirection::NorthEast,
        (false, false, false, true) => DpadDirection::East,
        (false, true, false, true) => DpadDirection::SouthEast,
        (false, true, false, false) => DpadDirection::South,
        (false, true, true, false) => DpadDirection::SouthWest,
        (false, false, true, false) => DpadDirection::West,
        (true, false, true, false) => DpadDirection::NorthWest,
        _ => DpadDirection::Neutral,
    }
}

fn raw_from_xinput_axis(value: i32) -> u8 {
    let value = value.clamp(i16::MIN as i32, i16::MAX as i32);

    if value >= 0 {
        let scaled = 128 + ((value * 127 + 16_383) / 32_767);
        scaled.clamp(0, 255) as u8
    } else {
        let scaled = 128 + ((value * 128 - 16_384) / 32_768);
        scaled.clamp(0, 255) as u8
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xinput_axis_conversion_matches_controller_raw_convention() {
        assert_eq!(raw_from_xinput_axis(i16::MIN as i32), 0);
        assert_eq!(raw_from_xinput_axis(0), 128);
        assert_eq!(raw_from_xinput_axis(i16::MAX as i32), 255);
    }

    #[test]
    fn xinput_mapping_preserves_face_buttons_and_triggers() {
        let mut raw = XINPUT_STATE::default();
        raw.dwPacketNumber = 42;
        raw.Gamepad.wButtons =
            XINPUT_GAMEPAD_A | XINPUT_GAMEPAD_X | XINPUT_GAMEPAD_START | XINPUT_GAMEPAD_DPAD_LEFT;
        raw.Gamepad.bLeftTrigger = 64;
        raw.Gamepad.bRightTrigger = 200;
        raw.Gamepad.sThumbLX = i16::MAX;
        raw.Gamepad.sThumbLY = i16::MIN;

        let snapshot = XInputSnapshot {
            packet_number: raw.dwPacketNumber,
            state: map_xinput_state(raw),
        };

        assert_eq!(snapshot.packet_number, 42);
        assert!(snapshot.state.buttons.cross);
        assert!(snapshot.state.buttons.square);
        assert!(snapshot.state.buttons.options);
        assert_eq!(snapshot.state.dpad, DpadDirection::West);
        assert_eq!(snapshot.state.l2, 64);
        assert_eq!(snapshot.state.r2, 200);
        assert!(snapshot.state.buttons.l2_button);
        assert!(snapshot.state.buttons.r2_button);
        assert_eq!(snapshot.state.left_stick.raw_x, 255);
        assert_eq!(snapshot.state.left_stick.raw_y, 255);
    }
}
