use std::{thread, time::Duration};

use anyhow::{Context, Result, anyhow, bail};
use hidapi::{DeviceInfo, HidApi, HidDevice, MAX_REPORT_DESCRIPTOR_SIZE};
use hidparser::{ReportField, VariableField, parse_report_descriptor};

use crate::assist::{Buttons, ControllerState, DpadDirection, StickState};

pub const SONY_VENDOR_ID: u16 = 0x054c;
pub const DUALSENSE_PRODUCT_ID: u16 = 0x0ce6;
pub const DUALSENSE_EDGE_PRODUCT_ID: u16 = 0x0df2;
pub const DUALSHOCK4_PRODUCT_ID: u16 = 0x05c4;
pub const DUALSHOCK4_V2_PRODUCT_ID: u16 = 0x09cc;
pub const NINTENDO_VENDOR_ID: u16 = 0x057e;
pub const SWITCH_PRO_PRODUCT_ID: u16 = 0x2009;
pub const MICROSOFT_VENDOR_ID: u16 = 0x045e;
pub const XBOX360_WINDOWS_PRODUCT_ID: u16 = 0x028e;
const SWITCH_USB_REPORT_ID: u8 = 0x80;
const SWITCH_USB_SUBCOMMAND_WRAPPER: u8 = 0x92;
const SWITCH_USB_SUBCOMMAND_HEADER: u8 = 0x31;
const SWITCH_USB_HANDSHAKE: u8 = 0x02;
const SWITCH_USB_BAUD_RATE: u8 = 0x03;
const SWITCH_USB_DISABLE_TIMEOUT: u8 = 0x04;
const SWITCH_SUBCOMMAND_REPORT_ID: u8 = 0x01;
const SWITCH_SUBCOMMAND_SET_INPUT_REPORT_MODE: u8 = 0x03;
const SWITCH_SUBCOMMAND_ACK_NOP: u8 = 0x33;
const SWITCH_INPUT_REPORT_MODE_FULL: u8 = 0x30;
const SWITCH_USB_OUTPUT_REPORT_LEN: usize = 64;
const SWITCH_USB_INIT_SETTLE_MS: u64 = 16;
const SWITCH_USB_INIT_READ_TIMEOUT_MS: i32 = 24;
const SWITCH_NEUTRAL_RUMBLE: [u8; 8] = [0x00, 0x01, 0x40, 0x40, 0x00, 0x01, 0x40, 0x40];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputBackend {
    DualSense,
    DualSenseEdge,
    DualShock4,
    DualShock4V2,
    NintendoSwitchPro,
    GenericGamepad,
    GenericJoystick,
}

impl InputBackend {
    pub fn label(self) -> &'static str {
        match self {
            Self::DualSense => "DualSense",
            Self::DualSenseEdge => "DualSense Edge",
            Self::DualShock4 | Self::DualShock4V2 => "DualShock 4",
            Self::NintendoSwitchPro => "Switch Pro Controller",
            Self::GenericGamepad => "Generic Gamepad",
            Self::GenericJoystick => "Generic Joystick",
        }
    }

    fn priority(self) -> u8 {
        match self {
            Self::DualSenseEdge => 6,
            Self::DualSense => 5,
            Self::DualShock4V2 => 4,
            Self::DualShock4 => 3,
            Self::NintendoSwitchPro => 3,
            Self::GenericGamepad => 2,
            Self::GenericJoystick => 1,
        }
    }

    fn is_generic(self) -> bool {
        matches!(self, Self::GenericGamepad | Self::GenericJoystick)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConnectionMode {
    Usb,
    Bluetooth,
    Unknown,
}

impl ConnectionMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::Usb => "USB",
            Self::Bluetooth => "Bluetooth",
            Self::Unknown => "Unknown transport",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InputDeviceInfo {
    pub path: String,
    pub vendor_id: u16,
    pub product_id: u16,
    pub usage_page: u16,
    pub usage: u16,
    pub interface_number: i32,
    pub serial_number: Option<String>,
    pub manufacturer: String,
    pub product: String,
    pub backend: Option<InputBackend>,
    pub connection_mode: ConnectionMode,
}

impl InputDeviceInfo {
    fn from_hid(info: &DeviceInfo) -> Self {
        Self {
            path: info.path().to_string_lossy().into_owned(),
            vendor_id: info.vendor_id(),
            product_id: info.product_id(),
            usage_page: info.usage_page(),
            usage: info.usage(),
            interface_number: info.interface_number(),
            serial_number: info.serial_number().map(ToOwned::to_owned),
            manufacturer: info
                .manufacturer_string()
                .unwrap_or("Unknown manufacturer")
                .to_owned(),
            product: info.product_string().unwrap_or("Unknown product").to_owned(),
            backend: backend_for(info),
            connection_mode: connection_mode_for(info),
        }
    }

    pub fn is_supported(&self) -> bool {
        self.backend.is_some()
    }

    pub fn backend_label(&self) -> String {
        match self.backend {
            Some(backend) => format!("{} {}", backend.label(), self.connection_mode.label()),
            None => "Unsupported HID".to_owned(),
        }
    }

    pub fn product_label(&self) -> String {
        match self.backend {
            Some(backend)
                if !backend.is_generic() && self.product.contains(backend.label()) =>
            {
                self.product.clone()
            }
            Some(backend) if !backend.is_generic() => backend.label().to_owned(),
            _ if self.manufacturer == "Unknown manufacturer" => self.product.clone(),
            _ => format!("{} ({})", self.product, self.manufacturer),
        }
    }

    pub fn is_primary_input_interface(&self) -> bool {
        self.usage_page == 0x01 && matches!(self.usage, 0x04 | 0x05)
    }
}

#[derive(Clone, Debug)]
pub enum InputParser {
    SonyDualSense { connection_mode: ConnectionMode },
    NintendoSwitch {
        connection_mode: ConnectionMode,
        generic: GenericHidParser,
    },
    Generic(GenericHidParser),
}

impl InputParser {
    pub fn parse_report(&self, report: &[u8]) -> Option<ControllerState> {
        match self {
            Self::SonyDualSense { connection_mode } => {
                parse_dualsense_input_report(*connection_mode, report)
            }
            Self::NintendoSwitch {
                connection_mode,
                generic,
            } => parse_nintendo_switch_input_report(*connection_mode, generic, report),
            Self::Generic(parser) => parser.parse(report),
        }
    }
}

fn backend_for(info: &DeviceInfo) -> Option<InputBackend> {
    if should_ignore_hid_device(info) {
        return None;
    }

    if info.vendor_id() == SONY_VENDOR_ID {
        match info.product_id() {
            DUALSENSE_PRODUCT_ID => return Some(InputBackend::DualSense),
            DUALSENSE_EDGE_PRODUCT_ID => return Some(InputBackend::DualSenseEdge),
            DUALSHOCK4_PRODUCT_ID => return Some(InputBackend::DualShock4),
            DUALSHOCK4_V2_PRODUCT_ID => return Some(InputBackend::DualShock4V2),
            _ => {}
        }
    }

    if info.vendor_id() == NINTENDO_VENDOR_ID {
        match info.product_id() {
            SWITCH_PRO_PRODUCT_ID => return Some(InputBackend::NintendoSwitchPro),
            _ => {}
        }
    }

    if info.usage_page() == 0x01 {
        return match info.usage() {
            0x05 => Some(InputBackend::GenericGamepad),
            0x04 => Some(InputBackend::GenericJoystick),
            _ => None,
        };
    }

    None
}

fn should_ignore_hid_device(info: &DeviceInfo) -> bool {
    if should_ignore_device_identity(
        info.vendor_id(),
        info.product_id(),
        info.manufacturer_string(),
        info.product_string(),
    ) {
        return true;
    }

    let path = info.path().to_string_lossy().to_ascii_uppercase();
    let manufacturer = info
        .manufacturer_string()
        .unwrap_or_default()
        .to_ascii_uppercase();
    let product = info.product_string().unwrap_or_default().to_ascii_uppercase();

    path.contains("IG_")
        && (manufacturer.contains("MICROSOFT")
            || product.contains("XBOX 360")
            || product.contains("XINPUT"))
}

fn should_ignore_device_identity(
    vendor_id: u16,
    product_id: u16,
    manufacturer: Option<&str>,
    product: Option<&str>,
) -> bool {
    if vendor_id == MICROSOFT_VENDOR_ID && product_id == XBOX360_WINDOWS_PRODUCT_ID {
        return true;
    }

    let manufacturer = manufacturer.unwrap_or_default().to_ascii_uppercase();
    let product = product.unwrap_or_default().to_ascii_uppercase();

    manufacturer.contains("MICROSOFT")
        && (product.contains("XBOX 360 FOR WINDOWS")
            || product.contains("XBOX 360 CONTROLLER")
            || product.contains("XINPUT"))
}

fn connection_mode_for(info: &DeviceInfo) -> ConnectionMode {
    infer_connection_mode(
        &info.path().to_string_lossy(),
        info.vendor_id(),
        info.product_id(),
    )
}

fn infer_connection_mode(path: &str, vendor_id: u16, product_id: u16) -> ConnectionMode {
    let path = path.to_ascii_uppercase();

    if path.contains("BTH")
        || path.contains("BLUETOOTH")
        || path.contains("{00001124-0000-1000-8000-00805F9B34FB}")
        || path.contains("{00001812-0000-1000-8000-00805F9B34FB}")
    {
        ConnectionMode::Bluetooth
    } else if vendor_id == NINTENDO_VENDOR_ID && product_id == SWITCH_PRO_PRODUCT_ID {
        ConnectionMode::Usb
    } else if path.contains("USB") || path.contains("MI_") {
        ConnectionMode::Usb
    } else {
        ConnectionMode::Unknown
    }
}

fn should_list(info: &DeviceInfo) -> bool {
    if should_ignore_hid_device(info) {
        return false;
    }

    info.vendor_id() == SONY_VENDOR_ID || (info.usage_page() == 0x01 && matches!(info.usage(), 0x04 | 0x05))
}

fn input_priority(info: &InputDeviceInfo) -> (bool, u8, bool, bool, bool) {
    (
        info.is_supported(),
        info.backend.map(InputBackend::priority).unwrap_or_default(),
        info.is_primary_input_interface(),
        info.usage == 0x05,
        info.usage_page == 0x01,
    )
}

fn sticky_match_score(candidate: &InputDeviceInfo, reference: &InputDeviceInfo) -> Option<u32> {
    if candidate.vendor_id != reference.vendor_id || candidate.product_id != reference.product_id {
        return None;
    }

    if let (Some(candidate_serial), Some(reference_serial)) =
        (&candidate.serial_number, &reference.serial_number)
    {
        if candidate_serial != reference_serial {
            return None;
        }
    }

    let mut score = 160_u32;

    if candidate.backend == reference.backend {
        score += 40;
    }
    if candidate.usage_page == reference.usage_page {
        score += 20;
    }
    if candidate.usage == reference.usage {
        score += 20;
    }
    if candidate.interface_number == reference.interface_number {
        score += 14;
    }
    if candidate.connection_mode == reference.connection_mode {
        score += 10;
    }
    if candidate.product == reference.product {
        score += 8;
    }
    if candidate.manufacturer == reference.manufacturer {
        score += 8;
    }
    if candidate.serial_number.is_some() && candidate.serial_number == reference.serial_number {
        score += 100;
    }

    Some(score)
}

pub fn scan_input_devices(api: &mut HidApi) -> Result<Vec<InputDeviceInfo>> {
    api.refresh_devices()?;

    let mut devices = api
        .device_list()
        .filter(|info| should_list(info))
        .map(InputDeviceInfo::from_hid)
        .collect::<Vec<_>>();

    devices.sort_by(|a, b| {
        input_priority(a)
            .cmp(&input_priority(b))
            .reverse()
            .then_with(|| a.product_label().cmp(&b.product_label()))
            .then_with(|| a.path.cmp(&b.path))
    });

    Ok(devices)
}

pub fn open_preferred_input_device(
    api: &mut HidApi,
    preferred_path: Option<&str>,
    preferred_hint: Option<&InputDeviceInfo>,
) -> Result<Option<(HidDevice, InputDeviceInfo, InputParser)>> {
    api.refresh_devices()?;

    let mut supported = api
        .device_list()
        .filter(|info| backend_for(info).is_some())
        .map(InputDeviceInfo::from_hid)
        .collect::<Vec<_>>();

    supported.sort_by(|a, b| {
        input_priority(a)
            .cmp(&input_priority(b))
            .reverse()
            .then_with(|| a.path.cmp(&b.path))
    });

    if supported.is_empty() {
        return Ok(None);
    }

    if let Some(path) = preferred_path {
        if let Some(info) = supported.iter().find(|info| info.path == path).cloned() {
            return open_input_device_candidate(api, info)
                .map(Some)
                .with_context(|| format!("selected input source could not be opened: {path}"));
        }

        if let Some(reference) = preferred_hint {
            if let Some(candidate) = supported
                .iter()
                .filter_map(|candidate| {
                    sticky_match_score(candidate, reference).map(|score| (candidate, score))
                })
                .max_by_key(|(_, score)| *score)
                .map(|(candidate, _)| candidate.clone())
            {
                return open_input_device_candidate(api, candidate)
                    .map(Some)
                    .with_context(|| "selected input source could not be reopened");
            }
        }

        return Err(anyhow!("selected input source is no longer available"));
    }

    let mut ordered = Vec::with_capacity(supported.len());
    if let Some(reference) = preferred_hint {
        if let Some((index, _)) = supported
            .iter()
            .enumerate()
            .filter_map(|(index, candidate)| {
                sticky_match_score(candidate, reference).map(|score| (index, score))
            })
            .max_by_key(|(_, score)| *score)
        {
            ordered.push(supported.remove(index));
        }
    }
    ordered.extend(supported);

    for info in ordered {
        if let Some(opened) = open_input_device_candidate(api, info).ok() {
            return Ok(Some(opened));
        }
    }

    Ok(None)
}

fn open_input_device_candidate(
    api: &HidApi,
    info: InputDeviceInfo,
) -> Result<(HidDevice, InputDeviceInfo, InputParser)> {
    let raw_info = api
        .device_list()
        .find(|candidate| candidate.path().to_string_lossy().as_ref() == info.path)
        .context("input device path is no longer present")?;
    let device = raw_info
        .open_device(api)
        .context("failed to open HID device")?;
    let parser = build_input_parser(&device, &info).context("failed to create input parser")?;
    initialize_input_device(&device, &info).context("failed to initialize input device")?;
    Ok((device, info, parser))
}

fn build_input_parser(device: &HidDevice, info: &InputDeviceInfo) -> Result<InputParser> {
    match info.backend.context("input backend metadata missing")? {
        InputBackend::DualSense | InputBackend::DualSenseEdge => Ok(InputParser::SonyDualSense {
            connection_mode: info.connection_mode,
        }),
        InputBackend::NintendoSwitchPro => Ok(InputParser::NintendoSwitch {
            connection_mode: info.connection_mode,
            generic: GenericHidParser::from_device(device)?,
        }),
        InputBackend::DualShock4
        | InputBackend::DualShock4V2
        | InputBackend::GenericGamepad
        | InputBackend::GenericJoystick => {
            Ok(InputParser::Generic(GenericHidParser::from_device(device)?))
        }
    }
}

fn initialize_input_device(device: &HidDevice, info: &InputDeviceInfo) -> Result<()> {
    if matches!(info.backend, Some(InputBackend::NintendoSwitchPro))
        && matches!(info.connection_mode, ConnectionMode::Usb | ConnectionMode::Unknown)
    {
        initialize_switch_pro_usb(device)
            .context("failed to initialize Nintendo Switch Pro Controller over USB")?;
    }

    Ok(())
}

fn initialize_switch_pro_usb(device: &HidDevice) -> Result<()> {
    write_switch_report(device, &[SWITCH_USB_REPORT_ID, SWITCH_USB_DISABLE_TIMEOUT])?;
    settle_switch_pro_usb(device)?;

    write_switch_report(device, &[SWITCH_USB_REPORT_ID, SWITCH_USB_HANDSHAKE])?;
    settle_switch_pro_usb(device)?;

    write_switch_report(device, &[SWITCH_USB_REPORT_ID, SWITCH_USB_BAUD_RATE])?;
    settle_switch_pro_usb(device)?;

    write_switch_report(device, &[SWITCH_USB_REPORT_ID, SWITCH_USB_HANDSHAKE])?;
    settle_switch_pro_usb(device)?;

    write_switch_report(device, &[SWITCH_USB_REPORT_ID, SWITCH_USB_DISABLE_TIMEOUT])?;
    settle_switch_pro_usb(device)?;

    let mut counter = 0_u8;
    write_switch_report(
        device,
        &build_switch_usb_subcommand_report(counter, SWITCH_SUBCOMMAND_ACK_NOP, &[]),
    )?;
    counter = counter.wrapping_add(1);
    settle_switch_pro_usb(device)?;

    write_switch_report(
        device,
        &build_switch_usb_subcommand_report(
            counter,
            SWITCH_SUBCOMMAND_SET_INPUT_REPORT_MODE,
            &[SWITCH_INPUT_REPORT_MODE_FULL],
        ),
    )?;
    settle_switch_pro_usb(device)?;

    Ok(())
}

fn build_switch_usb_subcommand_report(counter: u8, subcommand: u8, payload: &[u8]) -> Vec<u8> {
    let mut report = vec![
        SWITCH_USB_REPORT_ID,
        SWITCH_USB_SUBCOMMAND_WRAPPER,
        0x00,
        SWITCH_USB_SUBCOMMAND_HEADER,
        0x00,
        0x00,
        0x00,
        0x00,
        SWITCH_SUBCOMMAND_REPORT_ID,
        counter & 0x0F,
    ];
    report.extend_from_slice(&SWITCH_NEUTRAL_RUMBLE);
    report.push(subcommand);
    report.extend_from_slice(payload);
    report
}

fn write_switch_report(device: &HidDevice, report: &[u8]) -> Result<()> {
    let padded = pad_switch_usb_report(report);
    let written = device
        .write(&padded)
        .context("failed to write Switch Pro report")?;
    if written < report.len() {
        bail!(
            "short write while initializing Switch Pro USB: wrote {written} of {} bytes",
            report.len()
        );
    }

    Ok(())
}

fn pad_switch_usb_report(report: &[u8]) -> Vec<u8> {
    let mut padded = report.to_vec();
    if padded.len() < SWITCH_USB_OUTPUT_REPORT_LEN {
        padded.resize(SWITCH_USB_OUTPUT_REPORT_LEN, 0);
    }
    padded
}

fn settle_switch_pro_usb(device: &HidDevice) -> Result<()> {
    thread::sleep(Duration::from_millis(SWITCH_USB_INIT_SETTLE_MS));

    let mut report = [0_u8; 128];
    let mut timeout_ms = SWITCH_USB_INIT_READ_TIMEOUT_MS;
    loop {
        let bytes = device
            .read_timeout(&mut report, timeout_ms)
            .context("failed to read Switch Pro USB reply")?;
        if bytes == 0 {
            break;
        }

        timeout_ms = 1;
    }

    Ok(())
}

fn parse_dualsense_input_report(
    connection_mode: ConnectionMode,
    report: &[u8],
) -> Option<ControllerState> {
    if report.is_empty() {
        return None;
    }

    match report[0] {
        0x31 => parse_dualsense_bluetooth_input_report(report),
        0x01 => match connection_mode {
            ConnectionMode::Bluetooth => parse_dualsense_compact_bluetooth_input_report(report)
                .or_else(|| parse_dualsense_usb_input_report(report)),
            ConnectionMode::Usb => parse_dualsense_usb_input_report(report)
                .or_else(|| parse_dualsense_compact_bluetooth_input_report(report)),
            ConnectionMode::Unknown => {
                if report.len() >= 64 {
                    parse_dualsense_usb_input_report(report)
                        .or_else(|| parse_dualsense_compact_bluetooth_input_report(report))
                } else {
                    parse_dualsense_compact_bluetooth_input_report(report)
                        .or_else(|| parse_dualsense_usb_input_report(report))
                }
            }
        },
        _ => None,
    }
}

fn parse_nintendo_switch_input_report(
    _connection_mode: ConnectionMode,
    generic: &GenericHidParser,
    report: &[u8],
) -> Option<ControllerState> {
    if let Some(state) = parse_nintendo_switch_full_input_report(report) {
        return Some(state);
    }

    generic.parse(report)
}

fn parse_nintendo_switch_full_input_report(report: &[u8]) -> Option<ControllerState> {
    if report.len() < 12 || !matches!(report[0], 0x30 | 0x21) {
        return None;
    }

    let buttons_1 = report[3];
    let buttons_2 = report[4];
    let buttons_3 = report[5];
    let left_x = decode_switch_axis_12(report[6], report[7], report[8], false)?;
    let left_y = decode_switch_axis_12(report[6], report[7], report[8], true)?;
    let right_x = decode_switch_axis_12(report[9], report[10], report[11], false)?;
    let right_y = decode_switch_axis_12(report[9], report[10], report[11], true)?;

    Some(ControllerState {
        left_stick: StickState::new(
            scale_axis_12_to_byte(left_x),
            invert_switch_axis_byte(scale_axis_12_to_byte(left_y)),
        ),
        right_stick: StickState::new(
            scale_axis_12_to_byte(right_x),
            invert_switch_axis_byte(scale_axis_12_to_byte(right_y)),
        ),
        l2: if buttons_3 & 0x80 != 0 { 255 } else { 0 },
        r2: if buttons_1 & 0x80 != 0 { 255 } else { 0 },
        dpad: compose_dpad(
            buttons_3 & 0x02 != 0,
            buttons_3 & 0x01 != 0,
            buttons_3 & 0x08 != 0,
            buttons_3 & 0x04 != 0,
        ),
        buttons: Buttons {
            square: buttons_1 & 0x01 != 0,
            circle: buttons_1 & 0x08 != 0,
            cross: buttons_1 & 0x04 != 0,
            triangle: buttons_1 & 0x02 != 0,
            l1: buttons_3 & 0x40 != 0,
            r1: buttons_1 & 0x40 != 0,
            l2_button: buttons_3 & 0x80 != 0,
            r2_button: buttons_1 & 0x80 != 0,
            create: buttons_2 & 0x01 != 0,
            options: buttons_2 & 0x02 != 0,
            l3: buttons_2 & 0x08 != 0,
            r3: buttons_2 & 0x04 != 0,
            ps: buttons_2 & 0x10 != 0,
            touchpad: buttons_2 & 0x20 != 0,
            mute: false,
        },
    })
}

fn decode_switch_axis_12(b0: u8, b1: u8, b2: u8, y_axis: bool) -> Option<u16> {
    let raw = if y_axis {
        ((b1 as u16) >> 4) | ((b2 as u16) << 4)
    } else {
        (b0 as u16) | (((b1 as u16) & 0x0F) << 8)
    };

    (raw <= 4095).then_some(raw)
}

fn scale_axis_12_to_byte(raw: u16) -> u8 {
    ((u32::from(raw) * 255 + 2047) / 4095) as u8
}

fn invert_switch_axis_byte(value: u8) -> u8 {
    255_u8.saturating_sub(value)
}

fn parse_dualsense_usb_input_report(report: &[u8]) -> Option<ControllerState> {
    if report.len() < 11 || report[0] != 0x01 {
        return None;
    }

    Some(parse_common_layout(
        report, 1, 2, 3, 4, 5, 6, 8, 9, 10,
    ))
}

fn parse_dualsense_bluetooth_input_report(report: &[u8]) -> Option<ControllerState> {
    if report.len() < 12 || report[0] != 0x31 {
        return None;
    }

    Some(parse_common_layout(
        report, 2, 3, 4, 5, 6, 7, 9, 10, 11,
    ))
}

fn parse_dualsense_compact_bluetooth_input_report(report: &[u8]) -> Option<ControllerState> {
    if report.len() < 10 || report[0] != 0x01 {
        return None;
    }

    Some(parse_common_layout(
        report, 1, 2, 3, 4, 8, 9, 5, 6, 7,
    ))
}

fn parse_common_layout(
    report: &[u8],
    left_x: usize,
    left_y: usize,
    right_x: usize,
    right_y: usize,
    l2: usize,
    r2: usize,
    buttons_1: usize,
    buttons_2: usize,
    buttons_3: usize,
) -> ControllerState {
    let buttons_1 = report[buttons_1];
    let buttons_2 = report[buttons_2];
    let buttons_3 = report[buttons_3];

    ControllerState {
        left_stick: StickState::new(report[left_x], report[left_y]),
        right_stick: StickState::new(report[right_x], report[right_y]),
        l2: report[l2],
        r2: report[r2],
        dpad: DpadDirection::from_hat(buttons_1 & 0x0f),
        buttons: Buttons {
            square: buttons_1 & 0x10 != 0,
            circle: buttons_1 & 0x40 != 0,
            cross: buttons_1 & 0x20 != 0,
            triangle: buttons_1 & 0x80 != 0,
            l1: buttons_2 & 0x01 != 0,
            r1: buttons_2 & 0x02 != 0,
            l2_button: buttons_2 & 0x04 != 0,
            r2_button: buttons_2 & 0x08 != 0,
            create: buttons_2 & 0x10 != 0,
            options: buttons_2 & 0x20 != 0,
            l3: buttons_2 & 0x40 != 0,
            r3: buttons_2 & 0x80 != 0,
            ps: buttons_3 & 0x01 != 0,
            touchpad: buttons_3 & 0x02 != 0,
            mute: buttons_3 & 0x04 != 0,
        },
    }
}

#[derive(Clone, Debug)]
pub struct GenericHidParser {
    layouts: Vec<GenericReportLayout>,
}

impl GenericHidParser {
    fn from_device(device: &HidDevice) -> Result<Self> {
        let mut descriptor = [0_u8; MAX_REPORT_DESCRIPTOR_SIZE];
        let descriptor_len = device
            .get_report_descriptor(&mut descriptor)
            .context("failed to read HID report descriptor")?;
        if descriptor_len == 0 {
            bail!("HID report descriptor is empty");
        }

        let parsed = parse_report_descriptor(&descriptor[..descriptor_len])
            .map_err(|error| anyhow!("failed to parse HID report descriptor: {error:?}"))?;

        let mut layouts = parsed
            .input_reports
            .into_iter()
            .filter_map(GenericReportLayout::from_hid_report)
            .collect::<Vec<_>>();

        layouts.sort_by(|a, b| b.score.cmp(&a.score));

        if layouts.is_empty() {
            bail!("device does not expose a gamepad-like input report");
        }

        Ok(Self { layouts })
    }

    fn parse(&self, report: &[u8]) -> Option<ControllerState> {
        self.layouts.iter().find_map(|layout| layout.parse(report))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GenericButtonKind {
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
    Mute,
}

#[derive(Clone, Debug)]
struct ButtonBinding {
    kind: GenericButtonKind,
    field: VariableField,
}

#[derive(Clone, Debug, Default)]
struct DpadButtonFields {
    up: Option<VariableField>,
    down: Option<VariableField>,
    left: Option<VariableField>,
    right: Option<VariableField>,
}

#[derive(Clone, Debug)]
struct GenericReportLayout {
    report_id: Option<u8>,
    report_size_bytes: usize,
    score: u32,
    left_x: VariableField,
    left_y: VariableField,
    right_x: Option<VariableField>,
    right_y: Option<VariableField>,
    l2: Option<VariableField>,
    r2: Option<VariableField>,
    hat: Option<VariableField>,
    dpad: DpadButtonFields,
    buttons: Vec<ButtonBinding>,
}

impl GenericReportLayout {
    fn from_hid_report(report: hidparser::Report) -> Option<Self> {
        let report_id = report
            .report_id
            .map(u32::from)
            .and_then(|value| u8::try_from(value).ok());
        let report_size_bytes = report.size_in_bits.div_ceil(8);

        let mut x = None;
        let mut y = None;
        let mut z = None;
        let mut rx = None;
        let mut ry = None;
        let mut rz = None;
        let mut slider = Vec::new();
        let mut dial = Vec::new();
        let mut accelerator = None;
        let mut brake = None;
        let mut hat = None;
        let mut dpad = DpadButtonFields::default();
        let mut buttons = Vec::new();

        for field in report.fields {
            let ReportField::Variable(field) = field else {
                continue;
            };

            match (field.usage.page(), field.usage.id()) {
                (0x01, 0x30) if x.is_none() => x = Some(field),
                (0x01, 0x31) if y.is_none() => y = Some(field),
                (0x01, 0x32) if z.is_none() => z = Some(field),
                (0x01, 0x33) if rx.is_none() => rx = Some(field),
                (0x01, 0x34) if ry.is_none() => ry = Some(field),
                (0x01, 0x35) if rz.is_none() => rz = Some(field),
                (0x01, 0x36) => slider.push(field),
                (0x01, 0x37) => dial.push(field),
                (0x01, 0x39) if hat.is_none() => hat = Some(field),
                (0x01, 0x90) if dpad.up.is_none() => dpad.up = Some(field),
                (0x01, 0x91) if dpad.down.is_none() => dpad.down = Some(field),
                (0x01, 0x92) if dpad.right.is_none() => dpad.right = Some(field),
                (0x01, 0x93) if dpad.left.is_none() => dpad.left = Some(field),
                (0x02, 0xc4) if accelerator.is_none() => accelerator = Some(field),
                (0x02, 0xc5) if brake.is_none() => brake = Some(field),
                (0x09, usage_id) => {
                    if let Some(kind) = generic_button_kind_for_usage(usage_id) {
                        buttons.push(ButtonBinding { kind, field });
                    }
                }
                _ => {}
            }
        }

        let left_x = x?;
        let left_y = y?;
        if buttons.is_empty() {
            return None;
        }

        let (right_x, right_y) = pick_right_stick(&rx, &ry, &z, &rz, &slider, &dial);
        let (l2, r2) = pick_triggers(
            &accelerator,
            &brake,
            &z,
            &rz,
            &slider,
            &dial,
            right_x.as_ref(),
            right_y.as_ref(),
        );

        let mut score = 40;
        if right_x.is_some() && right_y.is_some() {
            score += 16;
        }
        if l2.is_some() && r2.is_some() {
            score += 8;
        }
        if hat.is_some()
            || dpad.up.is_some()
            || dpad.down.is_some()
            || dpad.left.is_some()
            || dpad.right.is_some()
        {
            score += 6;
        }
        score += buttons.len().min(12) as u32;

        Some(Self {
            report_id,
            report_size_bytes,
            score,
            left_x,
            left_y,
            right_x,
            right_y,
            l2,
            r2,
            hat,
            dpad,
            buttons,
        })
    }

    fn parse(&self, report: &[u8]) -> Option<ControllerState> {
        let payload = self.payload(report)?;
        let mut state = ControllerState {
            left_stick: StickState::new(
                read_axis_field(&self.left_x, payload)?,
                read_axis_field(&self.left_y, payload)?,
            ),
            right_stick: StickState::neutral(),
            l2: self
                .l2
                .as_ref()
                .and_then(|field| read_axis_field(field, payload))
                .unwrap_or(0),
            r2: self
                .r2
                .as_ref()
                .and_then(|field| read_axis_field(field, payload))
                .unwrap_or(0),
            dpad: DpadDirection::Neutral,
            buttons: Buttons::default(),
        };

        if let (Some(right_x), Some(right_y)) = (&self.right_x, &self.right_y) {
            state.right_stick = StickState::new(
                read_axis_field(right_x, payload)?,
                read_axis_field(right_y, payload)?,
            );
        }

        let digital_dpad = compose_dpad(
            self.dpad
                .up
                .as_ref()
                .is_some_and(|field| field_pressed(field, payload)),
            self.dpad
                .down
                .as_ref()
                .is_some_and(|field| field_pressed(field, payload)),
            self.dpad
                .left
                .as_ref()
                .is_some_and(|field| field_pressed(field, payload)),
            self.dpad
                .right
                .as_ref()
                .is_some_and(|field| field_pressed(field, payload)),
        );

        state.dpad = if digital_dpad != DpadDirection::Neutral {
            digital_dpad
        } else {
            self.hat
                .as_ref()
                .and_then(|field| field.field_value(payload))
                .map(dpad_from_hat_value)
                .unwrap_or(DpadDirection::Neutral)
        };

        for binding in &self.buttons {
            let pressed = field_pressed(&binding.field, payload);
            apply_button_binding(&mut state.buttons, binding.kind, pressed);
        }

        Some(state)
    }

    fn payload<'a>(&self, report: &'a [u8]) -> Option<&'a [u8]> {
        match self.report_id {
            Some(report_id) if report.first().copied() == Some(report_id) => {
                let payload = &report[1..];
                (payload.len() >= self.report_size_bytes).then_some(payload)
            }
            None if report.len() >= self.report_size_bytes => Some(report),
            None if report.first().copied() == Some(0) && report.len() > self.report_size_bytes => {
                let payload = &report[1..];
                (payload.len() >= self.report_size_bytes).then_some(payload)
            }
            _ => None,
        }
    }
}

fn generic_button_kind_for_usage(usage_id: u16) -> Option<GenericButtonKind> {
    match usage_id {
        1 => Some(GenericButtonKind::Cross),
        2 => Some(GenericButtonKind::Circle),
        3 => Some(GenericButtonKind::Square),
        4 => Some(GenericButtonKind::Triangle),
        5 => Some(GenericButtonKind::L1),
        6 => Some(GenericButtonKind::R1),
        7 => Some(GenericButtonKind::L2Button),
        8 => Some(GenericButtonKind::R2Button),
        9 => Some(GenericButtonKind::Create),
        10 => Some(GenericButtonKind::Options),
        11 => Some(GenericButtonKind::L3),
        12 => Some(GenericButtonKind::R3),
        13 => Some(GenericButtonKind::Ps),
        14 => Some(GenericButtonKind::Touchpad),
        15 => Some(GenericButtonKind::Mute),
        _ => None,
    }
}

fn pick_right_stick(
    rx: &Option<VariableField>,
    ry: &Option<VariableField>,
    z: &Option<VariableField>,
    rz: &Option<VariableField>,
    slider: &[VariableField],
    dial: &[VariableField],
) -> (Option<VariableField>, Option<VariableField>) {
    if let (Some(right_x), Some(right_y)) = (rx.clone(), ry.clone()) {
        return (Some(right_x), Some(right_y));
    }

    if let (Some(right_x), Some(right_y)) = (z.clone(), rz.clone()) {
        return (Some(right_x), Some(right_y));
    }

    if let (Some(right_x), Some(right_y)) = (rx.clone(), rz.clone()) {
        return (Some(right_x), Some(right_y));
    }

    if let (Some(right_x), Some(right_y)) = (z.clone(), ry.clone()) {
        return (Some(right_x), Some(right_y));
    }

    if let (Some(right_x), Some(right_y)) = (slider.first().cloned(), dial.first().cloned()) {
        return (Some(right_x), Some(right_y));
    }

    (None, None)
}

fn pick_triggers(
    accelerator: &Option<VariableField>,
    brake: &Option<VariableField>,
    z: &Option<VariableField>,
    rz: &Option<VariableField>,
    slider: &[VariableField],
    dial: &[VariableField],
    right_x: Option<&VariableField>,
    right_y: Option<&VariableField>,
) -> (Option<VariableField>, Option<VariableField>) {
    if trigger_axis_supported(accelerator.as_ref()) && trigger_axis_supported(brake.as_ref()) {
        return (accelerator.clone(), brake.clone());
    }

    if trigger_axis_supported(z.as_ref())
        && trigger_axis_supported(rz.as_ref())
        && !same_field_option(z.as_ref(), right_x)
        && !same_field_option(z.as_ref(), right_y)
        && !same_field_option(rz.as_ref(), right_x)
        && !same_field_option(rz.as_ref(), right_y)
    {
        return (z.clone(), rz.clone());
    }

    let mut extras = slider
        .iter()
        .chain(dial.iter())
        .filter(|field| {
            trigger_axis_supported(Some(field))
                && !same_field_option(Some(field), right_x)
                && !same_field_option(Some(field), right_y)
        })
        .cloned()
        .collect::<Vec<_>>();

    if extras.len() >= 2 {
        return (Some(extras.remove(0)), Some(extras.remove(0)));
    }

    (None, None)
}

fn trigger_axis_supported(field: Option<&VariableField>) -> bool {
    let Some(field) = field else {
        return false;
    };

    let min = i32::from(field.logical_minimum) as i64;
    let max = if min < 0 {
        i32::from(field.logical_maximum) as i64
    } else {
        u32::from(field.logical_maximum) as i64
    };

    min >= 0 && max > min
}

fn same_field_option(a: Option<&VariableField>, b: Option<&VariableField>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => same_field(a, b),
        _ => false,
    }
}

fn same_field(a: &VariableField, b: &VariableField) -> bool {
    a.usage == b.usage && a.bits == b.bits
}

fn read_axis_field(field: &VariableField, payload: &[u8]) -> Option<u8> {
    let value = field.field_value(payload)?;
    scale_field_value_to_byte(field, value)
}

fn scale_field_value_to_byte(field: &VariableField, value: i64) -> Option<u8> {
    let min = i32::from(field.logical_minimum) as i64;
    let max = if min < 0 {
        i32::from(field.logical_maximum) as i64
    } else {
        u32::from(field.logical_maximum) as i64
    };

    if max <= min {
        return None;
    }

    let clamped = value.clamp(min, max);
    let scaled = ((clamped - min) * 255 + ((max - min) / 2)) / (max - min);
    Some(scaled.clamp(0, 255) as u8)
}

fn field_pressed(field: &VariableField, payload: &[u8]) -> bool {
    field.field_value(payload).is_some_and(|value| value != 0)
}

fn dpad_from_hat_value(value: i64) -> DpadDirection {
    match value {
        0 => DpadDirection::North,
        1 => DpadDirection::NorthEast,
        2 => DpadDirection::East,
        3 => DpadDirection::SouthEast,
        4 => DpadDirection::South,
        5 => DpadDirection::SouthWest,
        6 => DpadDirection::West,
        7 => DpadDirection::NorthWest,
        _ => DpadDirection::Neutral,
    }
}

fn compose_dpad(up: bool, down: bool, left: bool, right: bool) -> DpadDirection {
    match (up, down, left, right) {
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

fn apply_button_binding(buttons: &mut Buttons, kind: GenericButtonKind, pressed: bool) {
    match kind {
        GenericButtonKind::Square => buttons.square = pressed,
        GenericButtonKind::Circle => buttons.circle = pressed,
        GenericButtonKind::Cross => buttons.cross = pressed,
        GenericButtonKind::Triangle => buttons.triangle = pressed,
        GenericButtonKind::L1 => buttons.l1 = pressed,
        GenericButtonKind::R1 => buttons.r1 = pressed,
        GenericButtonKind::L2Button => buttons.l2_button = pressed,
        GenericButtonKind::R2Button => buttons.r2_button = pressed,
        GenericButtonKind::Create => buttons.create = pressed,
        GenericButtonKind::Options => buttons.options = pressed,
        GenericButtonKind::L3 => buttons.l3 = pressed,
        GenericButtonKind::R3 => buttons.r3 = pressed,
        GenericButtonKind::Ps => buttons.ps = pressed,
        GenericButtonKind::Touchpad => buttons.touchpad = pressed,
        GenericButtonKind::Mute => buttons.mute = pressed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn usb_device(product_id: u16) -> InputDeviceInfo {
        InputDeviceInfo {
            path: "usb".to_owned(),
            vendor_id: SONY_VENDOR_ID,
            product_id,
            usage_page: 0x01,
            usage: 0x05,
            interface_number: 0,
            serial_number: Some("sony-pad-1".to_owned()),
            manufacturer: "Sony".to_owned(),
            product: "Wireless Controller".to_owned(),
            backend: match product_id {
                DUALSENSE_EDGE_PRODUCT_ID => Some(InputBackend::DualSenseEdge),
                DUALSHOCK4_PRODUCT_ID => Some(InputBackend::DualShock4),
                DUALSHOCK4_V2_PRODUCT_ID => Some(InputBackend::DualShock4V2),
                _ => Some(InputBackend::DualSense),
            },
            connection_mode: ConnectionMode::Usb,
        }
    }

    fn bluetooth_device(product_id: u16) -> InputDeviceInfo {
        InputDeviceInfo {
            path: "bth".to_owned(),
            connection_mode: ConnectionMode::Bluetooth,
            ..usb_device(product_id)
        }
    }

    #[test]
    fn primary_input_interface_is_preferred() {
        let mut primary = usb_device(DUALSENSE_PRODUCT_ID);
        primary.path = "primary".to_owned();
        primary.usage_page = 0x01;
        primary.usage = 0x05;

        let mut secondary = usb_device(DUALSENSE_PRODUCT_ID);
        secondary.path = "secondary".to_owned();
        secondary.usage_page = 0xff00;
        secondary.usage = 0x0001;

        assert!(input_priority(&primary) > input_priority(&secondary));
        assert!(primary.is_primary_input_interface());
        assert!(!secondary.is_primary_input_interface());
    }

    #[test]
    fn stale_preferred_path_falls_back_to_best_available_device() {
        let mut primary = usb_device(DUALSENSE_PRODUCT_ID);
        primary.path = "primary".to_owned();
        primary.usage_page = 0x01;
        primary.usage = 0x05;

        let mut secondary = usb_device(DUALSENSE_PRODUCT_ID);
        secondary.path = "secondary".to_owned();
        secondary.usage_page = 0xff00;
        secondary.usage = 0x0001;

        let supported = vec![secondary.clone(), primary.clone()];
        let selected = Some("missing")
            .and_then(|path| supported.iter().find(|info| info.path == path).cloned())
            .or_else(|| {
                supported.into_iter().max_by(|a, b| {
                    input_priority(a)
                        .cmp(&input_priority(b))
                        .then_with(|| b.path.cmp(&a.path))
                })
            });

        assert_eq!(selected, Some(primary));
    }

    #[test]
    fn parses_neutral_usb_report() {
        let report = [
            0x01, 0x7e, 0x81, 0x84, 0x84, 0x00, 0x00, 0x4b, 0x08, 0x00, 0x00, 0x00,
        ];
        let parsed = InputParser::SonyDualSense {
            connection_mode: ConnectionMode::Usb,
        }
        .parse_report(&report)
        .expect("report should parse");
        assert_eq!(parsed.left_stick.raw_x, 0x7e);
        assert_eq!(parsed.left_stick.raw_y, 0x81);
        assert_eq!(parsed.l2, 0);
        assert_eq!(parsed.r2, 0);
        assert_eq!(parsed.dpad, DpadDirection::Neutral);
        assert!(!parsed.buttons.cross);
    }

    #[test]
    fn parses_cross_usb_report() {
        let report = [
            0x01, 0x80, 0x80, 0x80, 0x80, 0x00, 0x00, 0x00, 0x28, 0x00, 0x00, 0x00,
        ];
        let parsed = InputParser::SonyDualSense {
            connection_mode: ConnectionMode::Usb,
        }
        .parse_report(&report)
        .expect("report should parse");
        assert!(parsed.buttons.cross);
        assert!(!parsed.buttons.circle);
    }

    #[test]
    fn parses_circle_bluetooth_raw_report() {
        let report = [
            0x31, 0x01, 0x80, 0x80, 0x80, 0x80, 0x00, 0x00, 0x00, 0x48, 0x00, 0x00,
        ];
        let parsed = InputParser::SonyDualSense {
            connection_mode: ConnectionMode::Bluetooth,
        }
        .parse_report(&report)
        .expect("raw bluetooth report should parse");
        assert!(parsed.buttons.circle);
        assert!(!parsed.buttons.cross);
    }

    #[test]
    fn parses_compact_bluetooth_report() {
        let report = [0x01, 0x7d, 0x7e, 0x83, 0x82, 0x08, 0x00, 0x00, 0x00, 0x00];
        let parsed = InputParser::SonyDualSense {
            connection_mode: ConnectionMode::Bluetooth,
        }
        .parse_report(&report)
        .expect("compact bluetooth report should parse");
        assert_eq!(parsed.left_stick.raw_x, 0x7d);
        assert_eq!(parsed.left_stick.raw_y, 0x7e);
        assert_eq!(parsed.dpad, DpadDirection::Neutral);
    }

    #[test]
    fn parses_dualsense_edge_using_same_layout() {
        let report = [
            0x01, 0x80, 0x80, 0x80, 0x80, 0x00, 0x00, 0x00, 0x48, 0x00, 0x00, 0x00,
        ];
        let parsed = InputParser::SonyDualSense {
            connection_mode: ConnectionMode::Usb,
        }
        .parse_report(&report)
        .expect("edge report should parse");
        assert!(parsed.buttons.circle);
        assert!(!parsed.buttons.cross);
    }

    #[test]
    fn generic_layout_parses_axes_buttons_and_hat() {
        let descriptor = [
            0x05, 0x01, 0x09, 0x05, 0xA1, 0x01, 0x15, 0x00, 0x26, 0xFF, 0x00, 0x75,
            0x08, 0x95, 0x04, 0x09, 0x30, 0x09, 0x31, 0x09, 0x33, 0x09, 0x34, 0x81,
            0x02, 0x95, 0x02, 0x09, 0x32, 0x09, 0x35, 0x81, 0x02, 0x05, 0x01, 0x15,
            0x00, 0x25, 0x07, 0x75, 0x04, 0x95, 0x01, 0x09, 0x39, 0x81, 0x42, 0x05,
            0x09, 0x15, 0x00, 0x25, 0x01, 0x75, 0x01, 0x95, 0x0A, 0x19, 0x01, 0x29,
            0x0A, 0x81, 0x02, 0x75, 0x01, 0x95, 0x02, 0x81, 0x03, 0xC0,
        ];

        let parsed = parse_report_descriptor(&descriptor).expect("descriptor should parse");
        let layout = GenericReportLayout::from_hid_report(
            parsed
                .input_reports
                .into_iter()
                .next()
                .expect("one input report expected"),
        )
        .expect("layout should be recognized");

        let report = [0x20, 0xE0, 0x80, 0x10, 0x40, 0x03, 0b0011_0011, 0b0001_0000];
        let state = layout.parse(&report).expect("report should parse");

        assert_eq!(state.left_stick.raw_x, 0x20);
        assert_eq!(state.left_stick.raw_y, 0xE0);
        assert_eq!(state.right_stick.raw_x, 0x80);
        assert_eq!(state.right_stick.raw_y, 0x10);
        assert_eq!(state.l2, 0x40);
        assert_eq!(state.r2, 0x03);
        assert_eq!(state.dpad, DpadDirection::SouthEast);
        assert!(state.buttons.cross);
        assert!(state.buttons.circle);
        assert!(state.buttons.create);
        assert!(!state.buttons.options);
    }

    #[test]
    fn generic_layout_accepts_report_id_prefixed_payloads() {
        let descriptor = [
            0x05, 0x01, 0x09, 0x05, 0xA1, 0x01, 0x85, 0x01, 0x15, 0x00, 0x26, 0xFF,
            0x00, 0x75, 0x08, 0x95, 0x02, 0x09, 0x30, 0x09, 0x31, 0x81, 0x02, 0x05,
            0x09, 0x15, 0x00, 0x25, 0x01, 0x75, 0x01, 0x95, 0x04, 0x19, 0x01, 0x29,
            0x04, 0x81, 0x02, 0x75, 0x01, 0x95, 0x04, 0x81, 0x03, 0xC0,
        ];

        let parsed = parse_report_descriptor(&descriptor).expect("descriptor should parse");
        let layout = GenericReportLayout::from_hid_report(
            parsed
                .input_reports
                .into_iter()
                .next()
                .expect("one input report expected"),
        )
        .expect("layout should be recognized");

        let report = [0x01, 0x55, 0xAA, 0b0000_0101];
        let state = layout.parse(&report).expect("report should parse");

        assert_eq!(state.left_stick.raw_x, 0x55);
        assert_eq!(state.left_stick.raw_y, 0xAA);
        assert!(state.buttons.cross);
        assert!(state.buttons.square);
        assert!(!state.buttons.circle);
    }

    #[test]
    fn generic_devices_are_marked_supported() {
        let device = InputDeviceInfo {
            path: "generic".to_owned(),
            vendor_id: 0x1234,
            product_id: 0x5678,
            usage_page: 0x01,
            usage: 0x05,
            interface_number: 1,
            serial_number: None,
            manufacturer: "Test".to_owned(),
            product: "Arcade Pad".to_owned(),
            backend: Some(InputBackend::GenericGamepad),
            connection_mode: ConnectionMode::Usb,
        };

        assert!(device.is_supported());
        assert_eq!(device.backend_label(), "Generic Gamepad USB");
    }

    #[test]
    fn bluetooth_device_metadata_is_preserved() {
        let device = bluetooth_device(DUALSHOCK4_V2_PRODUCT_ID);
        assert_eq!(device.connection_mode, ConnectionMode::Bluetooth);
        assert_eq!(device.backend, Some(InputBackend::DualShock4V2));
        assert_eq!(device.product_label(), "DualShock 4");
    }

    #[test]
    fn sticky_match_prefers_same_device_identity() {
        let reference = usb_device(DUALSHOCK4_V2_PRODUCT_ID);

        let mut same_device = reference.clone();
        same_device.path = "usb-2".to_owned();

        let mut other_device = reference.clone();
        other_device.path = "usb-3".to_owned();
        other_device.serial_number = Some("other-pad".to_owned());

        assert!(sticky_match_score(&same_device, &reference).is_some());
        assert!(sticky_match_score(&other_device, &reference).is_none());
    }

    #[test]
    fn parses_switch_pro_bluetooth_full_report() {
        let report = [
            0x30, 0xE3, 0x91, 0x00, 0x80, 0x00, 0xCF, 0xF7, 0x76, 0x9C, 0x27, 0x7B,
        ];

        let parsed = parse_nintendo_switch_full_input_report(&report)
            .expect("switch pro bluetooth report should parse");

        assert_eq!(parsed.dpad, DpadDirection::Neutral);
        assert!(!parsed.buttons.cross);
        assert!(!parsed.buttons.circle);
        assert!(!parsed.buttons.square);
        assert!(!parsed.buttons.triangle);
    }

    #[test]
    fn parses_switch_pro_face_buttons_by_position() {
        let report = [
            0x30, 0x01, 0x91, 0x0F, 0x03, 0xCA, 0x00, 0x80, 0x80, 0x00, 0x80, 0x80,
        ];

        let parsed = parse_nintendo_switch_full_input_report(&report)
            .expect("custom switch report should parse");

        assert!(parsed.buttons.square);
        assert!(parsed.buttons.triangle);
        assert!(parsed.buttons.cross);
        assert!(parsed.buttons.circle);
        assert!(parsed.buttons.create);
        assert!(parsed.buttons.options);
        assert!(parsed.buttons.l1);
        assert!(parsed.buttons.l2_button);
        assert_eq!(parsed.l2, 255);
        assert_eq!(parsed.dpad, DpadDirection::NorthWest);
    }

    #[test]
    fn parses_switch_pro_usb_full_report_before_generic_fallback() {
        let report = [
            0x30, 0x01, 0x91, 0x0F, 0x03, 0xCA, 0x00, 0x80, 0x80, 0x00, 0x80, 0x80,
        ];
        let generic = GenericHidParser { layouts: Vec::new() };

        let parsed = parse_nintendo_switch_input_report(ConnectionMode::Usb, &generic, &report)
            .expect("usb switch pro report should parse via the dedicated parser");

        assert!(parsed.buttons.cross);
        assert_eq!(parsed.dpad, DpadDirection::NorthWest);
    }

    #[test]
    fn builds_switch_pro_usb_mode_report_with_full_input_request() {
        let report = build_switch_usb_subcommand_report(
            7,
            SWITCH_SUBCOMMAND_SET_INPUT_REPORT_MODE,
            &[SWITCH_INPUT_REPORT_MODE_FULL],
        );

        assert_eq!(report[0], SWITCH_USB_REPORT_ID);
        assert_eq!(report[1], SWITCH_USB_SUBCOMMAND_WRAPPER);
        assert_eq!(report[3], SWITCH_USB_SUBCOMMAND_HEADER);
        assert_eq!(report[8], SWITCH_SUBCOMMAND_REPORT_ID);
        assert_eq!(report[9], 7);
        assert_eq!(&report[10..18], &SWITCH_NEUTRAL_RUMBLE);
        assert_eq!(report[18], SWITCH_SUBCOMMAND_SET_INPUT_REPORT_MODE);
        assert_eq!(report[19], SWITCH_INPUT_REPORT_MODE_FULL);
    }

    #[test]
    fn pads_switch_usb_reports_to_fixed_output_size() {
        let report = pad_switch_usb_report(&[SWITCH_USB_REPORT_ID, SWITCH_USB_HANDSHAKE]);

        assert_eq!(report.len(), SWITCH_USB_OUTPUT_REPORT_LEN);
        assert_eq!(report[0], SWITCH_USB_REPORT_ID);
        assert_eq!(report[1], SWITCH_USB_HANDSHAKE);
        assert!(report[2..].iter().all(|byte| *byte == 0));
    }

    #[test]
    fn inverts_switch_y_axes_to_match_existing_controller_convention() {
        assert_eq!(invert_switch_axis_byte(0), 255);
        assert_eq!(invert_switch_axis_byte(128), 127);
        assert_eq!(invert_switch_axis_byte(255), 0);
    }

    #[test]
    fn infers_switch_pro_usb_when_windows_hid_path_lacks_usb_marker() {
        let path =
            r"\\?\HID#VID_057E&PID_2009#9&35c399e5&0&0000#{4d1e55b2-f16f-11cf-88cb-001111000030}";

        let mode = infer_connection_mode(path, NINTENDO_VENDOR_ID, SWITCH_PRO_PRODUCT_ID);

        assert_eq!(mode, ConnectionMode::Usb);
    }

    #[test]
    fn ignores_xbox_360_for_windows_virtual_pad() {
        assert!(should_ignore_device_identity(
            MICROSOFT_VENDOR_ID,
            XBOX360_WINDOWS_PRODUCT_ID,
            Some("Microsoft"),
            Some("Controller (XBOX 360 For Windows)"),
        ));
    }

    #[test]
    fn ignores_razer_xbox_360_shadow_device() {
        assert!(should_ignore_device_identity(
            0x046D,
            0x0000,
            Some("Microsoft"),
            Some("Controller (Razer Xbox 360 Controller)"),
        ));
    }

    #[test]
    fn keeps_switch_pro_visible_to_the_selector() {
        assert!(!should_ignore_device_identity(
            NINTENDO_VENDOR_ID,
            SWITCH_PRO_PRODUCT_ID,
            Some("Nintendo Co., Ltd."),
            Some("Pro Controller"),
        ));
    }
}
