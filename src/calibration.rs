use serde::{Deserialize, Serialize};

use crate::{
    assist::{Buttons, ControllerState},
    dualsense::InputDeviceInfo,
};

const DIGITAL_TRIGGER_ACTIVE_THRESHOLD: u8 = 30;
const TRIGGER_CAPTURE_DELTA_THRESHOLD: u8 = 36;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ButtonSource {
    Square,
    Circle,
    Cross,
    Triangle,
    L1,
    R1,
    L2Button,
    R2Button,
    Create,
    Options,
    L3,
    R3,
    Ps,
    Touchpad,
}

impl ButtonSource {
    pub const ALL: [Self; 14] = [
        Self::Square,
        Self::Circle,
        Self::Cross,
        Self::Triangle,
        Self::L1,
        Self::R1,
        Self::L2Button,
        Self::R2Button,
        Self::Create,
        Self::Options,
        Self::L3,
        Self::R3,
        Self::Ps,
        Self::Touchpad,
    ];
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ButtonTarget {
    Square,
    Circle,
    Cross,
    Triangle,
    L1,
    R1,
    Create,
    Options,
    L3,
    R3,
}

impl ButtonTarget {}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TriggerSide {
    Left,
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TriggerSource {
    LeftTrigger,
    RightTrigger,
    Button(ButtonSource),
    None,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CalibrationStep {
    Button(ButtonTarget),
    Trigger(TriggerSide),
}

impl CalibrationStep {
    pub const ALL: [Self; 12] = [
        Self::Button(ButtonTarget::Cross),
        Self::Button(ButtonTarget::Circle),
        Self::Button(ButtonTarget::Square),
        Self::Button(ButtonTarget::Triangle),
        Self::Button(ButtonTarget::L1),
        Self::Button(ButtonTarget::R1),
        Self::Trigger(TriggerSide::Left),
        Self::Trigger(TriggerSide::Right),
        Self::Button(ButtonTarget::Create),
        Self::Button(ButtonTarget::Options),
        Self::Button(ButtonTarget::L3),
        Self::Button(ButtonTarget::R3),
    ];
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ButtonCalibrationMap {
    pub square: ButtonSource,
    pub circle: ButtonSource,
    pub cross: ButtonSource,
    pub triangle: ButtonSource,
    pub l1: ButtonSource,
    pub r1: ButtonSource,
    pub create: ButtonSource,
    pub options: ButtonSource,
    pub l3: ButtonSource,
    pub r3: ButtonSource,
}

impl Default for ButtonCalibrationMap {
    fn default() -> Self {
        Self {
            square: ButtonSource::Square,
            circle: ButtonSource::Circle,
            cross: ButtonSource::Cross,
            triangle: ButtonSource::Triangle,
            l1: ButtonSource::L1,
            r1: ButtonSource::R1,
            create: ButtonSource::Create,
            options: ButtonSource::Options,
            l3: ButtonSource::L3,
            r3: ButtonSource::R3,
        }
    }
}

impl ButtonCalibrationMap {
    pub fn set_source(&mut self, target: ButtonTarget, source: ButtonSource) {
        match target {
            ButtonTarget::Square => self.square = source,
            ButtonTarget::Circle => self.circle = source,
            ButtonTarget::Cross => self.cross = source,
            ButtonTarget::Triangle => self.triangle = source,
            ButtonTarget::L1 => self.l1 = source,
            ButtonTarget::R1 => self.r1 = source,
            ButtonTarget::Create => self.create = source,
            ButtonTarget::Options => self.options = source,
            ButtonTarget::L3 => self.l3 = source,
            ButtonTarget::R3 => self.r3 = source,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceCalibrationProfile {
    pub device_key: String,
    pub device_name: String,
    #[serde(default)]
    pub buttons: ButtonCalibrationMap,
    #[serde(default = "default_left_trigger_source")]
    pub left_trigger: TriggerSource,
    #[serde(default = "default_right_trigger_source")]
    pub right_trigger: TriggerSource,
}

fn default_left_trigger_source() -> TriggerSource {
    TriggerSource::LeftTrigger
}

fn default_right_trigger_source() -> TriggerSource {
    TriggerSource::RightTrigger
}

impl DeviceCalibrationProfile {
    pub fn new(info: &InputDeviceInfo) -> Self {
        Self {
            device_key: device_calibration_key(info),
            device_name: info.product_label(),
            buttons: ButtonCalibrationMap::default(),
            left_trigger: TriggerSource::LeftTrigger,
            right_trigger: TriggerSource::RightTrigger,
        }
    }

    pub fn set_button_source(&mut self, target: ButtonTarget, source: ButtonSource) {
        self.buttons.set_source(target, source);
    }

    pub fn set_trigger_source(&mut self, side: TriggerSide, source: TriggerSource) {
        match side {
            TriggerSide::Left => self.left_trigger = source,
            TriggerSide::Right => self.right_trigger = source,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CalibrationStore {
    #[serde(default)]
    pub devices: Vec<DeviceCalibrationProfile>,
}

impl CalibrationStore {
    pub fn profile_for(&self, info: &InputDeviceInfo) -> Option<&DeviceCalibrationProfile> {
        let key = device_calibration_key(info);
        self.devices
            .iter()
            .find(|profile| profile.device_key == key)
    }

    pub fn upsert(&mut self, profile: DeviceCalibrationProfile) {
        if let Some(existing) = self
            .devices
            .iter_mut()
            .find(|existing| existing.device_key == profile.device_key)
        {
            *existing = profile;
        } else {
            self.devices.push(profile);
        }
    }

    pub fn reset_for(&mut self, info: &InputDeviceInfo) {
        let key = device_calibration_key(info);
        self.devices.retain(|profile| profile.device_key != key);
    }

    pub fn profile_or_default(&self, info: &InputDeviceInfo) -> DeviceCalibrationProfile {
        self.profile_for(info)
            .cloned()
            .unwrap_or_else(|| DeviceCalibrationProfile::new(info))
    }
}

pub fn device_calibration_key(info: &InputDeviceInfo) -> String {
    let serial = info.serial_number.as_deref().unwrap_or("-");
    format!(
        "{:?}:{:04x}:{:04x}:{}:{}:{}:{}",
        info.source_kind,
        info.vendor_id,
        info.product_id,
        info.interface_number,
        serial,
        info.manufacturer,
        info.product,
    )
}

pub fn supports_manual_calibration(info: &InputDeviceInfo) -> bool {
    info.is_supported()
}

pub fn apply_device_calibration(
    info: &InputDeviceInfo,
    raw_state: ControllerState,
    store: &CalibrationStore,
) -> ControllerState {
    let Some(profile) = store.profile_for(info) else {
        return raw_state;
    };

    let mut calibrated = raw_state;
    calibrated.buttons.square = button_source_value(raw_state.buttons, profile.buttons.square);
    calibrated.buttons.circle = button_source_value(raw_state.buttons, profile.buttons.circle);
    calibrated.buttons.cross = button_source_value(raw_state.buttons, profile.buttons.cross);
    calibrated.buttons.triangle = button_source_value(raw_state.buttons, profile.buttons.triangle);
    calibrated.buttons.l1 = button_source_value(raw_state.buttons, profile.buttons.l1);
    calibrated.buttons.r1 = button_source_value(raw_state.buttons, profile.buttons.r1);
    calibrated.buttons.create = button_source_value(raw_state.buttons, profile.buttons.create);
    calibrated.buttons.options = button_source_value(raw_state.buttons, profile.buttons.options);
    calibrated.buttons.l3 = button_source_value(raw_state.buttons, profile.buttons.l3);
    calibrated.buttons.r3 = button_source_value(raw_state.buttons, profile.buttons.r3);
    calibrated.l2 = trigger_source_value(raw_state, profile.left_trigger);
    calibrated.r2 = trigger_source_value(raw_state, profile.right_trigger);
    calibrated.buttons.l2_button = calibrated.l2 >= DIGITAL_TRIGGER_ACTIVE_THRESHOLD;
    calibrated.buttons.r2_button = calibrated.r2 >= DIGITAL_TRIGGER_ACTIVE_THRESHOLD;
    calibrated
}

pub fn is_capture_idle(state: ControllerState) -> bool {
    ButtonSource::ALL
        .into_iter()
        .all(|source| !button_source_value(state.buttons, source))
        && state.l2 < DIGITAL_TRIGGER_ACTIVE_THRESHOLD
        && state.r2 < DIGITAL_TRIGGER_ACTIVE_THRESHOLD
}

pub fn detect_button_source(
    baseline: ControllerState,
    current: ControllerState,
) -> Option<ButtonSource> {
    ButtonSource::ALL.into_iter().find(|source| {
        !button_source_value(baseline.buttons, *source)
            && button_source_value(current.buttons, *source)
    })
}

pub fn detect_trigger_source(
    baseline: ControllerState,
    current: ControllerState,
) -> Option<TriggerSource> {
    let left_delta = current.l2.abs_diff(baseline.l2);
    let right_delta = current.r2.abs_diff(baseline.r2);

    if left_delta >= TRIGGER_CAPTURE_DELTA_THRESHOLD
        || right_delta >= TRIGGER_CAPTURE_DELTA_THRESHOLD
    {
        if left_delta >= right_delta {
            return Some(TriggerSource::LeftTrigger);
        }
        return Some(TriggerSource::RightTrigger);
    }

    detect_button_source(baseline, current).map(TriggerSource::Button)
}

fn button_source_value(buttons: Buttons, source: ButtonSource) -> bool {
    match source {
        ButtonSource::Square => buttons.square,
        ButtonSource::Circle => buttons.circle,
        ButtonSource::Cross => buttons.cross,
        ButtonSource::Triangle => buttons.triangle,
        ButtonSource::L1 => buttons.l1,
        ButtonSource::R1 => buttons.r1,
        ButtonSource::L2Button => buttons.l2_button,
        ButtonSource::R2Button => buttons.r2_button,
        ButtonSource::Create => buttons.create,
        ButtonSource::Options => buttons.options,
        ButtonSource::L3 => buttons.l3,
        ButtonSource::R3 => buttons.r3,
        ButtonSource::Ps => buttons.ps,
        ButtonSource::Touchpad => buttons.touchpad,
    }
}

fn trigger_source_value(state: ControllerState, source: TriggerSource) -> u8 {
    match source {
        TriggerSource::LeftTrigger => state.l2,
        TriggerSource::RightTrigger => state.r2,
        TriggerSource::Button(button) => {
            if button_source_value(state.buttons, button) {
                255
            } else {
                0
            }
        }
        TriggerSource::None => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dualsense::{ConnectionMode, InputBackend, InputSourceKind};

    fn sample_device() -> InputDeviceInfo {
        InputDeviceInfo {
            path: "generic-device".to_owned(),
            source_kind: InputSourceKind::Hid,
            vendor_id: 0x046d,
            product_id: 0xc216,
            usage_page: 0x01,
            usage: 0x05,
            interface_number: 0,
            serial_number: None,
            manufacturer: "Logitech".to_owned(),
            product: "F310".to_owned(),
            backend: Some(InputBackend::GenericGamepad),
            connection_mode: ConnectionMode::Usb,
        }
    }

    #[test]
    fn calibration_can_swap_face_buttons() {
        let device = sample_device();
        let mut store = CalibrationStore::default();
        let mut profile = DeviceCalibrationProfile::new(&device);
        profile.set_button_source(ButtonTarget::Cross, ButtonSource::Circle);
        profile.set_button_source(ButtonTarget::Circle, ButtonSource::Cross);
        store.upsert(profile);

        let raw_state = ControllerState {
            buttons: Buttons {
                circle: true,
                ..Buttons::default()
            },
            ..ControllerState::default()
        };

        let calibrated = apply_device_calibration(&device, raw_state, &store);
        assert!(calibrated.buttons.cross);
        assert!(!calibrated.buttons.circle);
    }

    #[test]
    fn trigger_calibration_can_use_digital_source() {
        let device = sample_device();
        let mut store = CalibrationStore::default();
        let mut profile = DeviceCalibrationProfile::new(&device);
        profile.set_trigger_source(
            TriggerSide::Left,
            TriggerSource::Button(ButtonSource::Circle),
        );
        store.upsert(profile);

        let raw_state = ControllerState {
            buttons: Buttons {
                circle: true,
                ..Buttons::default()
            },
            ..ControllerState::default()
        };

        let calibrated = apply_device_calibration(&device, raw_state, &store);
        assert_eq!(calibrated.l2, 255);
        assert!(calibrated.buttons.l2_button);
    }
}
