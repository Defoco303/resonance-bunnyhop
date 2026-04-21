use std::{
    sync::Arc,
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use hidapi::{HidApi, HidDevice};
use parking_lot::Mutex;
use vigem_rust::target::Xbox360;
use vigem_rust::{Client, TargetHandle};

use crate::{
    assist::{AssistConfig, AssistEngine, AssistPhase, ControllerState, JumpButton},
    diagnostics::{EnvironmentDiagnostics, collect_environment_diagnostics},
    dualsense::{InputDeviceInfo, InputParser, open_preferred_input_device, scan_input_devices},
    x360::map_to_x360_report,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ServiceState {
    Stopped,
    Searching,
    Running,
    DriverMissing,
    Error,
}

// Snapshot-only live values retained for future UI/diagnostic surfaces.
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, Default)]
pub struct LiveView {
    pub left_x: f32,
    pub left_y: f32,
    pub right_x: f32,
    pub right_y: f32,
    pub tracked_jump_pressed: bool,
    pub movement_magnitude: f32,
}

impl LiveView {
    fn from_state(state: ControllerState, jump_button: JumpButton) -> Self {
        Self {
            left_x: state.left_stick.normalized_x(),
            left_y: state.left_stick.normalized_y(),
            right_x: state.right_stick.normalized_x(),
            right_y: state.right_stick.normalized_y(),
            tracked_jump_pressed: state.buttons.pressed(jump_button),
            movement_magnitude: state.left_stick.magnitude(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct RuntimeSnapshot {
    pub service_state: ServiceState,
    pub physical_connected: bool,
    pub virtual_connected: bool,
    pub assist_phase: AssistPhase,
    pub jump_count: u64,
    pub last_sequence_age_ms: u128,
    pub active_device: Option<InputDeviceInfo>,
    pub available_devices: Vec<InputDeviceInfo>,
    pub raw_state: ControllerState,
    pub raw_live: LiveView,
    pub assisted_live: LiveView,
    pub output_differs: bool,
    pub diagnostics: EnvironmentDiagnostics,
    pub last_error: Option<String>,
}

impl Default for RuntimeSnapshot {
    fn default() -> Self {
        Self {
            service_state: ServiceState::Stopped,
            physical_connected: false,
            virtual_connected: false,
            assist_phase: AssistPhase::Idle,
            jump_count: 0,
            last_sequence_age_ms: 0,
            active_device: None,
            available_devices: Vec::new(),
            raw_state: ControllerState::default(),
            raw_live: LiveView::default(),
            assisted_live: LiveView::default(),
            output_differs: false,
            diagnostics: EnvironmentDiagnostics::default(),
            last_error: None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct SharedState {
    pub enabled: bool,
    pub preferred_device_path: Option<String>,
    pub config: AssistConfig,
    pub runtime: RuntimeSnapshot,
    pub shutdown: bool,
}

impl Default for SharedState {
    fn default() -> Self {
        Self {
            enabled: false,
            preferred_device_path: None,
            config: AssistConfig::default(),
            runtime: RuntimeSnapshot::default(),
            shutdown: false,
        }
    }
}

pub fn spawn_runtime(shared: Arc<Mutex<SharedState>>) -> JoinHandle<()> {
    thread::spawn(move || {
        let mut engine = BridgeEngine::default();
        engine.run(shared);
    })
}

#[derive(Default)]
struct BridgeEngine {
    hid_api: Option<HidApi>,
    physical: Option<HidDevice>,
    active_device: Option<InputDeviceInfo>,
    active_parser: Option<InputParser>,
    device_hint: Option<InputDeviceInfo>,
    client: Option<Client>,
    virtual_pad: Option<TargetHandle<Xbox360>>,
    assist: AssistEngine,
    last_input: Option<ControllerState>,
    last_output: Option<ControllerState>,
    next_controller_scan: Option<Instant>,
    next_driver_retry: Option<Instant>,
    next_inventory_refresh: Option<Instant>,
    next_diagnostics_refresh: Option<Instant>,
    preferred_device_path: Option<String>,
}

impl BridgeEngine {
    fn run(&mut self, shared: Arc<Mutex<SharedState>>) {
        loop {
            let (enabled, preferred_device_path, config, shutdown) = {
                let state = shared.lock();
                (
                    state.enabled,
                    state.preferred_device_path.clone(),
                    state.config,
                    state.shutdown,
                )
            };

            if shutdown {
                self.stop_bridge(&shared);
                break;
            }

            self.sync_preferences(preferred_device_path, &shared);

            if !enabled {
                if let Err(error) = self.refresh_inventory_if_due(&shared) {
                    self.update_snapshot(&shared, |runtime| {
                        runtime.last_error = Some(error.to_string());
                    });
                }

                if let Err(error) = self.refresh_diagnostics_if_due(&shared) {
                    self.update_snapshot(&shared, |runtime| {
                        runtime.last_error = Some(error.to_string());
                    });
                }

                self.stop_bridge(&shared);
                thread::sleep(Duration::from_millis(80));
                continue;
            }

            if self.physical.is_none() {
                if let Err(error) = self.refresh_inventory_if_due(&shared) {
                    self.update_snapshot(&shared, |runtime| {
                        runtime.last_error = Some(error.to_string());
                    });
                }
            }

            if let Err(error) = self.tick(config, &shared) {
                self.record_error(&shared, error);
                thread::sleep(Duration::from_millis(120));
            }
        }
    }

    fn sync_preferences(
        &mut self,
        preferred_device_path: Option<String>,
        shared: &Arc<Mutex<SharedState>>,
    ) {
        if self.preferred_device_path != preferred_device_path {
            self.preferred_device_path = preferred_device_path;
            self.physical = None;
            self.active_device = None;
            self.active_parser = None;
            self.device_hint = None;
            self.last_input = None;
            self.last_output = None;
            self.next_controller_scan = None;
            self.update_snapshot(shared, |runtime| {
                runtime.active_device = None;
                runtime.physical_connected = false;
                runtime.output_differs = false;
            });
        }
    }

    fn tick(&mut self, config: AssistConfig, shared: &Arc<Mutex<SharedState>>) -> Result<()> {
        self.ensure_virtual_pad(shared)?;
        self.ensure_controller(shared)?;

        if self.physical.is_none() || self.active_device.is_none() || self.active_parser.is_none() {
            return Ok(());
        }

        let mut report = [0_u8; 128];
        self.active_device
            .as_ref()
            .context("controller metadata missing")?;
        let parser = self
            .active_parser
            .as_ref()
            .context("controller parser missing")?;
        let physical = self
            .physical
            .as_ref()
            .context("controller disappeared unexpectedly")?;
        let mut latest_physical_state = None;

        loop {
            let bytes = physical.read(&mut report)?;
            if bytes == 0 {
                break;
            }

            if let Some(state) = parser.parse_report(&report[..bytes]) {
                latest_physical_state = Some(state);
            }
        }

        let now = Instant::now();

        if let Some(physical_state) = latest_physical_state {
            self.last_input = Some(physical_state);
            self.publish_state(physical_state, config, shared, now)?;
        } else if self.assist.has_pending_sequence() {
            if let Some(physical_state) = self.last_input {
                self.publish_state(physical_state, config, shared, now)?;
            } else {
                self.update_snapshot(shared, |runtime| {
                    runtime.service_state = ServiceState::Running;
                    runtime.physical_connected = true;
                    runtime.virtual_connected = true;
                    runtime.assist_phase = self.assist.phase();
                    runtime.jump_count = self.assist.jump_count();
                    runtime.last_sequence_age_ms = self.assist.last_sequence_age_ms();
                    runtime.output_differs = false;
                });
            }
        }

        Ok(())
    }

    fn publish_state(
        &mut self,
        physical_state: ControllerState,
        config: AssistConfig,
        shared: &Arc<Mutex<SharedState>>,
        now: Instant,
    ) -> Result<()> {
        let output_state = self.assist.apply(physical_state, config, now);
        if self.last_output != Some(output_state) {
            let report = map_to_x360_report(output_state);
            self.virtual_pad
                .as_ref()
                .context("virtual pad disappeared unexpectedly")?
                .update(&report)?;
            self.last_output = Some(output_state);
        }

        self.update_snapshot(shared, |runtime| {
            runtime.service_state = ServiceState::Running;
            runtime.physical_connected = true;
            runtime.virtual_connected = true;
            runtime.assist_phase = self.assist.phase();
            runtime.jump_count = self.assist.jump_count();
            runtime.last_sequence_age_ms = self.assist.last_sequence_age_ms();
            runtime.active_device = self.active_device.clone();
            runtime.raw_state = physical_state;
            runtime.raw_live = LiveView::from_state(physical_state, config.jump_button);
            runtime.assisted_live = LiveView::from_state(output_state, config.jump_button);
            runtime.output_differs = physical_state != output_state;
            runtime.last_error = None;
        });

        Ok(())
    }

    fn ensure_controller(&mut self, shared: &Arc<Mutex<SharedState>>) -> Result<()> {
        if self.physical.is_some() && self.active_device.is_some() && self.active_parser.is_some() {
            return Ok(());
        }

        if self.physical.is_some() || self.active_device.is_some() || self.active_parser.is_some() {
            self.physical = None;
            self.active_device = None;
            self.active_parser = None;
            self.last_input = None;
        }

        let now = Instant::now();
        if self.next_controller_scan.is_some_and(|deadline| now < deadline) {
            self.update_snapshot(shared, |runtime| {
                runtime.service_state = ServiceState::Searching;
                runtime.physical_connected = false;
            });
            thread::sleep(Duration::from_millis(50));
            return Ok(());
        }

        let api = self.hid_api.get_or_insert(HidApi::new()?);
        match open_preferred_input_device(
            api,
            self.preferred_device_path.as_deref(),
            self.device_hint.as_ref(),
        )? {
            Some((device, info, parser)) => {
                device.set_blocking_mode(false)?;
                self.physical = Some(device);
                self.active_device = Some(info.clone());
                self.active_parser = Some(parser);
                self.device_hint = Some(info.clone());
                self.last_input = None;
                self.next_controller_scan = None;
                self.update_snapshot(shared, |runtime| {
                    runtime.service_state = ServiceState::Running;
                    runtime.physical_connected = true;
                    runtime.active_device = Some(info.clone());
                    runtime.last_error = None;
                });
                Ok(())
            }
            None => {
                self.active_device = None;
                self.active_parser = None;
                self.next_controller_scan = Some(now + Duration::from_millis(900));
                self.update_snapshot(shared, |runtime| {
                    runtime.service_state = ServiceState::Searching;
                    runtime.physical_connected = false;
                    runtime.active_device = None;
                });
                thread::sleep(Duration::from_millis(50));
                Ok(())
            }
        }
    }

    fn ensure_virtual_pad(&mut self, shared: &Arc<Mutex<SharedState>>) -> Result<()> {
        if self.virtual_pad.is_some() {
            return Ok(());
        }

        let now = Instant::now();
        if self.next_driver_retry.is_some_and(|deadline| now < deadline) {
            self.update_snapshot(shared, |runtime| {
                runtime.service_state = ServiceState::DriverMissing;
                runtime.virtual_connected = false;
            });
            thread::sleep(Duration::from_millis(50));
            return Ok(());
        }

        let client = Client::connect().context("ViGEmBus driver is not available")?;
        let x360 = client
            .new_x360_target()
            .plugin()
            .context("failed to create a virtual Xbox 360 pad")?;
        x360.wait_for_ready()?;

        self.client = Some(client);
        self.virtual_pad = Some(x360);
        self.next_driver_retry = None;

        self.update_snapshot(shared, |runtime| {
            runtime.virtual_connected = true;
            runtime.last_error = None;
        });

        Ok(())
    }

    fn refresh_inventory_if_due(&mut self, shared: &Arc<Mutex<SharedState>>) -> Result<()> {
        let now = Instant::now();
        if self.next_inventory_refresh.is_some_and(|deadline| now < deadline) {
            return Ok(());
        }

        let api = self.hid_api.get_or_insert(HidApi::new()?);
        let devices = scan_input_devices(api)?;
        self.next_inventory_refresh = Some(now + Duration::from_millis(1000));

        self.update_snapshot(shared, |runtime| {
            runtime.available_devices = devices.clone();
        });

        Ok(())
    }

    fn refresh_diagnostics_if_due(&mut self, shared: &Arc<Mutex<SharedState>>) -> Result<()> {
        let now = Instant::now();
        if self
            .next_diagnostics_refresh
            .is_some_and(|deadline| now < deadline)
        {
            return Ok(());
        }

        let diagnostics = collect_environment_diagnostics();
        self.next_diagnostics_refresh = Some(now + Duration::from_secs(4));

        self.update_snapshot(shared, |runtime| {
            runtime.diagnostics = diagnostics.clone();
        });

        Ok(())
    }

    fn stop_bridge(&mut self, shared: &Arc<Mutex<SharedState>>) {
        self.physical = None;
        self.active_device = None;
        self.active_parser = None;
        self.virtual_pad = None;
        self.client = None;
        self.last_input = None;
        self.last_output = None;
        self.next_controller_scan = None;
        self.next_driver_retry = None;
        self.assist.reset();

        self.update_snapshot(shared, |runtime| {
            runtime.service_state = ServiceState::Stopped;
            runtime.physical_connected = false;
            runtime.virtual_connected = false;
            runtime.assist_phase = AssistPhase::Idle;
            runtime.last_sequence_age_ms = 0;
            runtime.active_device = None;
            runtime.raw_state = ControllerState::default();
            runtime.raw_live = LiveView::default();
            runtime.assisted_live = LiveView::default();
            runtime.output_differs = false;
        });
    }

    fn record_error(&mut self, shared: &Arc<Mutex<SharedState>>, error: anyhow::Error) {
        let message = format_error_chain(&error);

        if message.contains("ViGEmBus") {
            self.virtual_pad = None;
            self.client = None;
            self.next_driver_retry = Some(Instant::now() + Duration::from_secs(2));
            self.update_snapshot(shared, |runtime| {
                runtime.service_state = ServiceState::DriverMissing;
                runtime.virtual_connected = false;
                runtime.last_error = Some(message.clone());
            });
            return;
        }

        self.physical = None;
        self.active_device = None;
        self.active_parser = None;
        self.last_input = None;
        self.last_output = None;
        self.next_controller_scan = Some(Instant::now() + Duration::from_millis(700));
        self.update_snapshot(shared, |runtime| {
            runtime.service_state = ServiceState::Error;
            runtime.physical_connected = false;
            runtime.active_device = None;
            runtime.last_error = Some(message.clone());
        });
    }

    fn update_snapshot(
        &self,
        shared: &Arc<Mutex<SharedState>>,
        mut apply: impl FnMut(&mut RuntimeSnapshot),
    ) {
        let mut state = shared.lock();
        apply(&mut state.runtime);
    }
}

fn format_error_chain(error: &anyhow::Error) -> String {
    let mut chain = error.chain();
    let Some(first) = chain.next() else {
        return "unknown error".to_owned();
    };

    let mut message = first.to_string();
    for cause in chain {
        message.push_str("\ncaused by: ");
        message.push_str(&cause.to_string());
    }

    message
}
