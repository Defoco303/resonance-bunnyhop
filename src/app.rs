use std::{
    fs,
    path::Path,
    sync::Arc,
    thread::JoinHandle,
    time::{Duration, Instant},
};

use eframe::{
    CreationContext,
    egui::{
        self, Align, Align2, Color32, ColorImage, CornerRadius, FontData, FontDefinitions,
        FontFamily, FontId, Frame, Layout, Margin, Pos2, RichText, ScrollArea, Sense, Stroke,
        StrokeKind, TextureHandle, TextureOptions, Vec2, ViewportBuilder, ViewportId, pos2,
    },
};
use parking_lot::Mutex;

use crate::{
    assist::{AssistConfig, JumpButton},
    calibration::{
        CalibrationStep, CalibrationStore, DeviceCalibrationProfile, detect_button_source,
        detect_trigger_source, device_calibration_key, is_capture_idle,
        supports_manual_calibration,
    },
    dualsense::InputDeviceInfo,
    runtime::{RuntimeSnapshot, SharedState, spawn_runtime},
    settings::{PersistedSettings, load_settings, save_settings},
};

#[derive(Clone, Copy, Debug)]
enum IconButtonKind {
    Menu,
    Close,
}

#[derive(Clone, Debug)]
struct CalibrationSession {
    working: DeviceCalibrationProfile,
    baseline: crate::assist::ControllerState,
    step_index: usize,
    waiting_for_idle: bool,
}

impl CalibrationSession {
    fn new(
        current_device: &InputDeviceInfo,
        existing: &CalibrationStore,
        raw_state: crate::assist::ControllerState,
    ) -> Self {
        Self {
            working: existing.profile_or_default(current_device),
            baseline: raw_state,
            step_index: 0,
            waiting_for_idle: !is_capture_idle(raw_state),
        }
    }

    fn current_step(&self) -> Option<CalibrationStep> {
        CalibrationStep::ALL.get(self.step_index).copied()
    }
}

#[derive(Clone, Debug)]
struct PendingSettingsSave {
    settings: PersistedSettings,
    changed_at: Instant,
}

#[derive(Clone, Copy, Debug)]
struct CalibrationVisualHint {
    normalized_position: (f32, f32),
    location_label: &'static str,
    alias_label: &'static str,
}

pub struct BhopApp {
    shared: Arc<Mutex<SharedState>>,
    runtime_handle: Option<JoinHandle<()>>,
    calibration_session: Option<CalibrationSession>,
    calibration_guide_texture: Option<TextureHandle>,
    calibration_guide_error: Option<String>,
    options_open: bool,
    last_saved_settings: PersistedSettings,
    pending_settings_save: Option<PendingSettingsSave>,
    persist_error: Option<String>,
    warning_expanded_main_window: bool,
}

impl BhopApp {
    pub fn new(cc: &CreationContext<'_>) -> Self {
        configure_fonts(&cc.egui_ctx);
        configure_theme(&cc.egui_ctx);
        let (calibration_guide_texture, calibration_guide_error) =
            match load_calibration_guide_texture(&cc.egui_ctx) {
                Ok(texture) => (Some(texture), None),
                Err(error) => (None, Some(error)),
            };
        let (initial_settings, persist_error) = match load_settings() {
            Ok(settings) => (settings, None),
            Err(error) => (PersistedSettings::default(), Some(error.to_string())),
        };
        let initial_settings = PersistedSettings {
            assist: normalize_assist_config(initial_settings.assist),
            preferred_device_path: sanitize_preferred_device_path(
                initial_settings.preferred_device_path,
            ),
            calibrations: initial_settings.calibrations,
        };

        let shared = Arc::new(Mutex::new(SharedState {
            preferred_device_path: initial_settings.preferred_device_path.clone(),
            config: initial_settings.assist,
            calibrations: initial_settings.calibrations.clone(),
            ..SharedState::default()
        }));
        let runtime_handle = Some(spawn_runtime(shared.clone()));
        Self {
            shared,
            runtime_handle,
            calibration_session: None,
            calibration_guide_texture,
            calibration_guide_error,
            options_open: false,
            last_saved_settings: initial_settings,
            pending_settings_save: None,
            persist_error,
            warning_expanded_main_window: false,
        }
    }

    fn current_settings(
        &self,
        preferred_device_path: Option<String>,
        config: AssistConfig,
        calibrations: CalibrationStore,
    ) -> PersistedSettings {
        PersistedSettings {
            assist: normalize_assist_config(config),
            preferred_device_path: sanitize_preferred_device_path(preferred_device_path),
            calibrations,
        }
    }

    fn queue_settings_save(&mut self, settings: PersistedSettings) {
        if settings == self.last_saved_settings {
            self.pending_settings_save = None;
            return;
        }

        match &mut self.pending_settings_save {
            Some(pending) if pending.settings == settings => {}
            Some(pending) => {
                pending.settings = settings;
                pending.changed_at = Instant::now();
            }
            None => {
                self.pending_settings_save = Some(PendingSettingsSave {
                    settings,
                    changed_at: Instant::now(),
                });
            }
        }
    }

    fn flush_pending_settings_if_due(&mut self) {
        let should_flush = self
            .pending_settings_save
            .as_ref()
            .is_some_and(|pending| pending.changed_at.elapsed() >= Duration::from_millis(300));
        if should_flush {
            self.flush_pending_settings_now();
        }
    }

    fn flush_pending_settings_now(&mut self) {
        let Some(pending) = self.pending_settings_save.clone() else {
            return;
        };

        match save_settings(&pending.settings) {
            Ok(()) => {
                self.last_saved_settings = pending.settings;
                self.pending_settings_save = None;
                self.persist_error = None;
            }
            Err(error) => {
                if let Some(pending) = &mut self.pending_settings_save {
                    pending.changed_at = Instant::now();
                }
                self.persist_error = Some(error.to_string());
            }
        }
    }

    fn show_options_viewport(
        &mut self,
        ctx: &egui::Context,
        snapshot: &RuntimeSnapshot,
        preferred_device_path: &mut Option<String>,
        config: &mut AssistConfig,
        calibrations: &mut CalibrationStore,
        current_device: Option<InputDeviceInfo>,
        bridge_enabled: bool,
    ) {
        if !self.options_open {
            return;
        }

        let mut close_requested = false;
        let viewport_id = ViewportId::from_hash_of("options-viewport");
        let builder = ViewportBuilder::default()
            .with_title("オプション")
            .with_inner_size([400.0, 760.0])
            .with_min_inner_size([340.0, 700.0])
            .with_resizable(true);

        ctx.show_viewport_immediate(viewport_id, builder, |ui, _class| {
            if ui.input(|input| input.viewport().close_requested()) {
                close_requested = true;
            }

            paint_backdrop(ui, false);

            Frame::new()
                .fill(Color32::from_rgba_unmultiplied(7, 10, 15, 170))
                .inner_margin(Margin::same(8))
                .show(ui, |ui| {
                    ScrollArea::vertical()
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            main_shell_frame().show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    ui.vertical(|ui| {
                                        ui.label(
                                            RichText::new("OPTIONS")
                                                .size(14.5)
                                                .strong()
                                                .color(text_muted()),
                                        );
                                    });

                                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                        if icon_button(ui, IconButtonKind::Close).clicked() {
                                            close_requested = true;
                                        }
                                    });
                                });

                                ui.add_space(12.0);
                                self.threshold_section(ui, config);
                                ui.add_space(8.0);
                                self.calibration_section(
                                    ui,
                                    current_device.as_ref(),
                                    snapshot.raw_state,
                                    calibrations,
                                    bridge_enabled,
                                );
                                ui.add_space(8.0);
                                self.jump_button_section(ui, config);
                                ui.add_space(8.0);
                                self.device_section(ui, snapshot, preferred_device_path);
                            });
                        });
                });
        });

        if close_requested {
            self.options_open = false;
        }
    }

    fn threshold_section(&mut self, ui: &mut egui::Ui, config: &mut AssistConfig) {
        full_width_section(ui, |ui| {
            section_title(ui, "閾値設定");
            ui.add_space(10.0);

            slider_row_u64(
                ui,
                "ジャンプ重なり",
                "ジャンプ入力の直後に、移動入力を少しだけ残す時間です。",
                &mut config.jump_overlap_ms,
                0..=30,
                "ms",
            );
            slider_row_u64(
                ui,
                "ニュートラル時間",
                "移動入力をニュートラルにする時間です。長いほど一瞬止める時間が伸びます。",
                &mut config.movement_release_ms,
                0..=40,
                "ms",
            );
            slider_row_u64(
                ui,
                "再入力ガード",
                "補助の終了後、すぐ次の補助に入らないようにする待機時間です。",
                &mut config.retrigger_guard_ms,
                0..=30,
                "ms",
            );
            slider_row_f32(
                ui,
                "移動デッドゾーン",
                "これ未満のスティック入力は移動中として扱いません。",
                &mut config.movement_deadzone,
                0.05..=0.60,
            );
        });
    }

    fn calibration_section(
        &mut self,
        ui: &mut egui::Ui,
        current_device: Option<&InputDeviceInfo>,
        raw_state: crate::assist::ControllerState,
        calibrations: &mut CalibrationStore,
        bridge_enabled: bool,
    ) {
        full_width_section(ui, |ui| {
            section_title(ui, "入力キャリブレーション");
            ui.add_space(10.0);

            let Some(device) = current_device else {
                ui.label(
                    RichText::new("BRIDGEをオンにしてコントローラーを検出すると、このデバイス用の手動校正が使えます。")
                        .size(10.5)
                        .color(text_muted()),
                );
                return;
            };

            full_width_chip(
                ui,
                &device.product_label(),
                accent_soft(),
                Stroke::new(1.0, accent_color()),
                text_primary(),
                30.0,
                10.5,
            );
            ui.add_space(8.0);

            if !supports_manual_calibration(device) {
                ui.label(
                    RichText::new("この入力元では手動キャリブレーションを使えません。")
                        .size(10.5)
                        .color(text_muted()),
                );
                return;
            }

            let session_active = self.calibration_session.as_ref().is_some_and(|session| {
                session.working.device_key == device_calibration_key(device)
            });

            ui.horizontal_wrapped(|ui| {
                if session_active {
                    if pill_button(
                        ui,
                        "キャンセル",
                        control_fill_hovered(),
                        Stroke::new(1.0, accent_dim()),
                        13.5,
                    )
                    .clicked()
                    {
                        self.calibration_session = None;
                    }
                } else {
                    if bridge_enabled {
                        if pill_button(
                            ui,
                            "キャリブレーション開始",
                            control_fill_hovered(),
                            Stroke::new(1.0, accent_dim()),
                            13.5,
                        )
                        .clicked()
                        {
                            self.calibration_session =
                                Some(CalibrationSession::new(device, calibrations, raw_state));
                        }
                    } else {
                        pill_label(
                            ui,
                            "キャリブレーション開始",
                            control_fill(),
                            Stroke::new(1.0, border_color()),
                            text_muted(),
                            13.5,
                        );
                    }

                    if pill_button(
                        ui,
                        "初期化",
                        control_fill_hovered(),
                        Stroke::new(1.0, accent_dim()),
                        13.5,
                    )
                    .clicked()
                    {
                        calibrations.reset_for(device);
                        self.calibration_session = None;
                    }
                }
            });

            ui.add_space(8.0);
            if !bridge_enabled {
                ui.label(
                    RichText::new("※キャリブレーション中はBRIDGEをオンにしてください。")
                        .size(10.5)
                        .color(text_muted()),
                );
            }

            if let Some(session) = self.calibration_session.as_ref() {
                if session.working.device_key == device_calibration_key(device) {
                    ui.add_space(8.0);
                    if let Some(step) = session.current_step() {
                        let hint = calibration_visual_hint(step);
                        ui.label(
                            RichText::new(format!(
                                "手順 {}/{}: {} を 1 回入力してください。",
                                session.step_index + 1,
                                CalibrationStep::ALL.len(),
                                hint.location_label
                            ))
                            .size(11.0)
                            .color(text_primary()),
                        );
                        ui.label(
                            RichText::new(hint.alias_label)
                                .size(10.5)
                                .color(accent_color()),
                        );
                        ui.label(
                            RichText::new(if session.waiting_for_idle {
                                "いったん全てのボタンを離したら、次の入力を待ちます。"
                            } else {
                                "狙ったボタンだけを単独で入力すると認識しやすいです。"
                            })
                            .size(10.5)
                            .color(text_muted()),
                        );
                        ui.add_space(8.0);
                        render_calibration_guide(
                            ui,
                            step,
                            self.calibration_guide_texture.as_ref(),
                            self.calibration_guide_error.as_deref(),
                        );
                    } else {
                        ui.label(
                            RichText::new("キャリブレーションを保存しました。")
                                .size(11.0)
                                .color(accent_color()),
                        );
                    }
                }
            }
        });
    }

    fn process_calibration_session(
        &mut self,
        current_device: Option<&InputDeviceInfo>,
        raw_state: crate::assist::ControllerState,
        calibrations: &mut CalibrationStore,
    ) {
        let Some(session) = &mut self.calibration_session else {
            return;
        };
        let Some(device) = current_device else {
            return;
        };
        if session.working.device_key != device_calibration_key(device) {
            return;
        }

        if session.current_step().is_none() {
            calibrations.upsert(session.working.clone());
            self.calibration_session = None;
            return;
        }

        if session.waiting_for_idle {
            if is_capture_idle(raw_state) {
                session.baseline = raw_state;
                session.waiting_for_idle = false;
            }
            return;
        }

        match session.current_step() {
            Some(CalibrationStep::Button(target)) => {
                if let Some(source) = detect_button_source(session.baseline, raw_state) {
                    session.working.set_button_source(target, source);
                    session.step_index += 1;
                    session.waiting_for_idle = true;
                }
            }
            Some(CalibrationStep::Trigger(side)) => {
                if let Some(source) = detect_trigger_source(session.baseline, raw_state) {
                    session.working.set_trigger_source(side, source);
                    session.step_index += 1;
                    session.waiting_for_idle = true;
                }
            }
            None => {}
        }
    }

    fn device_section(
        &mut self,
        ui: &mut egui::Ui,
        snapshot: &RuntimeSnapshot,
        preferred_device_path: &mut Option<String>,
    ) {
        full_width_section(ui, |ui| {
            section_title(ui, "入力元");
            ui.add_space(10.0);

            if selection_button(
                ui,
                preferred_device_path.is_none(),
                "自動",
                "最初に見つかった対応コントローラー",
            )
            .clicked()
            {
                *preferred_device_path = None;
            }

            for device in snapshot
                .available_devices
                .iter()
                .filter(|device| device.is_manual_selectable())
            {
                if selection_button(
                    ui,
                    preferred_device_path
                        .as_ref()
                        .is_some_and(|path| path == &device.path),
                    &device.product_label(),
                    &device.backend_label(),
                )
                .clicked()
                {
                    *preferred_device_path = Some(device.path.clone());
                }
            }

            if snapshot
                .available_devices
                .iter()
                .any(|device| device.is_xinput())
            {
                ui.add_space(8.0);
                ui.label(
                    RichText::new("XInputコントローラーは非対応です。背面スイッチやモード切替で DInput / HID 側にしてから使ってください。")
                        .size(10.5)
                        .color(text_muted()),
                );
            }
        });
    }

    fn jump_button_section(&mut self, ui: &mut egui::Ui, config: &mut AssistConfig) {
        full_width_section(ui, |ui| {
            section_title(ui, "ジャンプ入力");
            ui.add_space(10.0);
            ui.label(
                RichText::new("補助の発火に使うボタンを選んでください。")
                    .size(10.5)
                    .color(text_muted()),
            );
            ui.add_space(8.0);

            ui.horizontal_wrapped(|ui| {
                for (button, label) in jump_button_choices() {
                    let selected = config.jump_button == *button;
                    let fill = if selected {
                        accent_soft()
                    } else {
                        control_fill_hovered()
                    };
                    let stroke = if selected {
                        Stroke::new(1.0, accent_color())
                    } else {
                        Stroke::new(1.0, accent_dim())
                    };

                    if pill_button(ui, label, fill, stroke, 13.0).clicked() {
                        config.jump_button = *button;
                    }
                }
            });
        });
    }
}

impl Drop for BhopApp {
    fn drop(&mut self) {
        self.flush_pending_settings_now();
        {
            let mut shared = self.shared.lock();
            shared.shutdown = true;
            shared.enabled = false;
        }

        if let Some(handle) = self.runtime_handle.take() {
            let _ = handle.join();
        }
    }
}

fn configure_fonts(ctx: &egui::Context) {
    let candidates = [
        r"C:\Windows\Fonts\YuGothM.ttc",
        r"C:\Windows\Fonts\YuGothR.ttc",
        r"C:\Windows\Fonts\meiryo.ttc",
        r"C:\Windows\Fonts\msgothic.ttc",
    ];

    let Some(path) = candidates.iter().find(|path| Path::new(path).exists()) else {
        return;
    };

    let Ok(bytes) = fs::read(path) else {
        return;
    };

    let mut fonts = FontDefinitions::default();
    fonts
        .font_data
        .insert("jp-ui".into(), FontData::from_owned(bytes).into());

    if let Some(family) = fonts.families.get_mut(&FontFamily::Proportional) {
        family.insert(0, "jp-ui".into());
    }

    if let Some(family) = fonts.families.get_mut(&FontFamily::Monospace) {
        family.push("jp-ui".into());
    }

    ctx.set_fonts(fonts);
}

fn configure_theme(ctx: &egui::Context) {
    let mut style = (*ctx.global_style()).clone();
    style.spacing.item_spacing = Vec2::new(6.0, 6.0);
    style.spacing.button_padding = Vec2::new(9.0, 6.0);
    style.spacing.interact_size = Vec2::new(40.0, 26.0);
    style.spacing.slider_width = 150.0;

    style.visuals = egui::Visuals::dark();
    style.visuals.window_fill = Color32::from_rgba_unmultiplied(6, 10, 14, 234);
    style.visuals.panel_fill = Color32::TRANSPARENT;
    style.visuals.extreme_bg_color = surface_fill();
    style.visuals.faint_bg_color = Color32::from_rgba_unmultiplied(18, 24, 30, 250);
    style.visuals.window_stroke = Stroke::new(1.0, border_color());
    style.visuals.window_corner_radius = CornerRadius::same(18);
    style.visuals.menu_corner_radius = CornerRadius::same(14);

    style.visuals.widgets.noninteractive.weak_bg_fill = surface_fill();
    style.visuals.widgets.noninteractive.bg_fill = surface_fill();
    style.visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, border_color());
    style.visuals.widgets.noninteractive.fg_stroke.color = text_primary();
    style.visuals.widgets.noninteractive.corner_radius = CornerRadius::same(12);

    style.visuals.widgets.inactive.weak_bg_fill = control_fill();
    style.visuals.widgets.inactive.bg_fill = control_fill();
    style.visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, border_color());
    style.visuals.widgets.inactive.fg_stroke.color = text_primary();
    style.visuals.widgets.inactive.corner_radius = CornerRadius::same(12);

    style.visuals.widgets.hovered.weak_bg_fill = control_fill_hovered();
    style.visuals.widgets.hovered.bg_fill = control_fill_hovered();
    style.visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, accent_dim());
    style.visuals.widgets.hovered.fg_stroke.color = text_primary();
    style.visuals.widgets.hovered.corner_radius = CornerRadius::same(12);

    style.visuals.widgets.active.weak_bg_fill = accent_soft();
    style.visuals.widgets.active.bg_fill = accent_soft();
    style.visuals.widgets.active.bg_stroke = Stroke::new(1.0, accent_color());
    style.visuals.widgets.active.fg_stroke.color = text_primary();
    style.visuals.widgets.active.corner_radius = CornerRadius::same(12);

    style.visuals.widgets.open.weak_bg_fill = control_fill_hovered();
    style.visuals.widgets.open.bg_fill = control_fill_hovered();
    style.visuals.widgets.open.bg_stroke = Stroke::new(1.0, accent_dim());
    style.visuals.widgets.open.fg_stroke.color = text_primary();
    style.visuals.widgets.open.corner_radius = CornerRadius::same(12);

    style.visuals.selection.bg_fill = accent_soft();
    style.visuals.selection.stroke = Stroke::new(1.0, accent_color());
    style.visuals.override_text_color = Some(text_primary());
    ctx.set_global_style(style);
}

fn text_primary() -> Color32 {
    Color32::from_rgb(237, 242, 247)
}

fn text_muted() -> Color32 {
    Color32::from_rgb(118, 132, 145)
}

fn brand_color() -> Color32 {
    Color32::from_rgb(176, 120, 255)
}

fn accent_color() -> Color32 {
    Color32::from_rgb(94, 232, 255)
}

fn accent_dim() -> Color32 {
    Color32::from_rgba_unmultiplied(94, 232, 255, 120)
}

fn accent_soft() -> Color32 {
    Color32::from_rgba_unmultiplied(33, 126, 145, 236)
}

fn danger_color() -> Color32 {
    Color32::from_rgb(255, 118, 132)
}

fn warning_color() -> Color32 {
    Color32::from_rgb(255, 204, 107)
}

fn border_color() -> Color32 {
    Color32::from_rgba_unmultiplied(255, 255, 255, 24)
}

fn surface_fill() -> Color32 {
    Color32::from_rgba_unmultiplied(13, 17, 23, 248)
}

fn control_fill() -> Color32 {
    Color32::from_rgba_unmultiplied(28, 38, 49, 248)
}

fn control_fill_hovered() -> Color32 {
    Color32::from_rgba_unmultiplied(40, 53, 68, 248)
}

fn glass_fill() -> Color32 {
    Color32::from_rgba_unmultiplied(9, 12, 18, 210)
}

fn main_shell_frame() -> Frame {
    Frame::new()
        .fill(glass_fill())
        .stroke(Stroke::new(1.0, border_color()))
        .corner_radius(CornerRadius::same(16))
        .inner_margin(Margin::same(12))
}

fn section_frame() -> Frame {
    Frame::new()
        .fill(Color32::from_rgba_unmultiplied(16, 21, 28, 224))
        .stroke(Stroke::new(1.0, border_color()))
        .corner_radius(CornerRadius::same(12))
        .inner_margin(Margin::same(12))
}

fn metric_frame() -> Frame {
    Frame::new()
        .fill(Color32::from_rgba_unmultiplied(14, 19, 26, 236))
        .stroke(Stroke::new(1.0, border_color()))
        .corner_radius(CornerRadius::same(12))
        .inner_margin(Margin::same(10))
}

fn error_frame() -> Frame {
    Frame::new()
        .fill(Color32::from_rgba_unmultiplied(52, 18, 24, 224))
        .stroke(Stroke::new(1.0, warning_color()))
        .corner_radius(CornerRadius::same(12))
        .inner_margin(Margin::same(12))
}

fn section_title(ui: &mut egui::Ui, title: &str) {
    ui.label(
        RichText::new(title)
            .size(12.5)
            .strong()
            .color(text_primary()),
    );
}

fn metric_label(ui: &mut egui::Ui, title: &str) {
    ui.label(RichText::new(title).size(9.5).strong().color(text_muted()));
}

fn full_width_section(ui: &mut egui::Ui, add_contents: impl FnOnce(&mut egui::Ui)) {
    let width = ui.available_width();
    ui.allocate_ui_with_layout(Vec2::new(width, 0.0), Layout::top_down(Align::Min), |ui| {
        section_frame().show(ui, |ui| {
            ui.set_min_width((width - 24.0).max(0.0));
            add_contents(ui);
        });
    });
}

fn error_panel(ui: &mut egui::Ui, error_text: &str) {
    error_frame().show(ui, |ui| {
        ui.label(
            RichText::new("ERROR")
                .size(11.0)
                .strong()
                .color(warning_color()),
        );
        ui.add_space(8.0);
        ui.add(
            egui::Label::new(
                RichText::new(error_text)
                    .size(12.5)
                    .monospace()
                    .color(text_primary()),
            )
            .wrap(),
        );
    });
}

fn warning_panel(ui: &mut egui::Ui, title: &str, body: &str) {
    error_frame().show(ui, |ui| {
        ui.label(
            RichText::new(title)
                .size(11.0)
                .strong()
                .color(danger_color()),
        );
        ui.add_space(8.0);
        ui.label(RichText::new(body).size(11.5).color(text_primary()));
    });
}

fn render_calibration_guide(
    ui: &mut egui::Ui,
    step: CalibrationStep,
    texture: Option<&TextureHandle>,
    load_error: Option<&str>,
) {
    let Some(texture) = texture else {
        ui.label(
            RichText::new(load_error.unwrap_or("ガイド画像を読み込めませんでした。"))
                .size(10.5)
                .color(text_muted()),
        );
        return;
    };

    let hint = calibration_visual_hint(step);
    let max_width = ui.available_width().min(340.0);
    let texture_size = texture.size();
    let aspect = texture_size[1] as f32 / texture_size[0] as f32;
    let desired_size = Vec2::new(max_width, max_width * aspect);
    let response = ui.add(egui::Image::new((texture.id(), desired_size)));
    let rect = response.rect;
    let marker_center = pos2(
        rect.left() + rect.width() * hint.normalized_position.0,
        rect.top() + rect.height() * hint.normalized_position.1,
    );
    let radius = (rect.width() * 0.024).clamp(8.0, 14.0);
    let painter = ui.painter();

    painter.circle_filled(
        marker_center,
        radius * 1.35,
        Color32::from_rgba_unmultiplied(94, 232, 255, 28),
    );
    painter.circle_stroke(marker_center, radius, Stroke::new(2.5, accent_color()));
    painter.circle_filled(marker_center, radius * 0.45, accent_color());
}

fn load_calibration_guide_texture(ctx: &egui::Context) -> Result<TextureHandle, String> {
    let image =
        image::load_from_memory(include_bytes!("../assets/controller-calibration-base.png"))
            .map_err(|error| format!("ガイド画像のデコードに失敗しました: {error}"))?
            .to_rgba8();
    let size = [image.width() as usize, image.height() as usize];
    let pixels = image.into_raw();
    let color_image = ColorImage::from_rgba_unmultiplied(size, &pixels);
    Ok(ctx.load_texture(
        "controller-calibration-guide",
        color_image,
        TextureOptions::LINEAR,
    ))
}

fn calibration_visual_hint(step: CalibrationStep) -> CalibrationVisualHint {
    match step {
        CalibrationStep::Button(crate::calibration::ButtonTarget::Cross) => CalibrationVisualHint {
            normalized_position: (0.649, 0.635),
            location_label: "右側4ボタンの下",
            alias_label: "Xbox A / PlayStation Cross 相当",
        },
        CalibrationStep::Button(crate::calibration::ButtonTarget::Circle) => {
            CalibrationVisualHint {
                normalized_position: (0.685, 0.574),
                location_label: "右側4ボタンの右",
                alias_label: "Xbox B / PlayStation Circle 相当",
            }
        }
        CalibrationStep::Button(crate::calibration::ButtonTarget::Square) => {
            CalibrationVisualHint {
                normalized_position: (0.618, 0.573),
                location_label: "右側4ボタンの左",
                alias_label: "Xbox X / PlayStation Square 相当",
            }
        }
        CalibrationStep::Button(crate::calibration::ButtonTarget::Triangle) => {
            CalibrationVisualHint {
                normalized_position: (0.650, 0.513),
                location_label: "右側4ボタンの上",
                alias_label: "Xbox Y / PlayStation Triangle 相当",
            }
        }
        CalibrationStep::Button(crate::calibration::ButtonTarget::L1) => CalibrationVisualHint {
            normalized_position: (0.348, 0.172),
            location_label: "上面の左バンパー",
            alias_label: "L1 / LB",
        },
        CalibrationStep::Button(crate::calibration::ButtonTarget::R1) => CalibrationVisualHint {
            normalized_position: (0.652, 0.172),
            location_label: "上面の右バンパー",
            alias_label: "R1 / RB",
        },
        CalibrationStep::Trigger(crate::calibration::TriggerSide::Left) => CalibrationVisualHint {
            normalized_position: (0.336, 0.078),
            location_label: "上面の左トリガー",
            alias_label: "L2 / LT",
        },
        CalibrationStep::Trigger(crate::calibration::TriggerSide::Right) => CalibrationVisualHint {
            normalized_position: (0.664, 0.078),
            location_label: "上面の右トリガー",
            alias_label: "R2 / RT",
        },
        CalibrationStep::Button(crate::calibration::ButtonTarget::Create) => {
            CalibrationVisualHint {
                normalized_position: (0.388, 0.476),
                location_label: "中央左の小ボタン",
                alias_label: "Create / Back / Share 相当",
            }
        }
        CalibrationStep::Button(crate::calibration::ButtonTarget::Options) => {
            CalibrationVisualHint {
                normalized_position: (0.612, 0.476),
                location_label: "中央右の小ボタン",
                alias_label: "Options / Start / Menu 相当",
            }
        }
        CalibrationStep::Button(crate::calibration::ButtonTarget::L3) => CalibrationVisualHint {
            normalized_position: (0.432, 0.704),
            location_label: "左スティック押し込み",
            alias_label: "L3",
        },
        CalibrationStep::Button(crate::calibration::ButtonTarget::R3) => CalibrationVisualHint {
            normalized_position: (0.582, 0.690),
            location_label: "右スティック押し込み",
            alias_label: "R3",
        },
    }
}

const PILL_TEXT_Y_OFFSET: f32 = 1.0;

fn centered_pill_text(rect: egui::Rect) -> Pos2 {
    pos2(rect.center().x, rect.center().y + PILL_TEXT_Y_OFFSET)
}

fn left_pill_text(rect: egui::Rect, left_padding: f32) -> Pos2 {
    pos2(
        rect.left() + left_padding,
        rect.center().y + PILL_TEXT_Y_OFFSET,
    )
}

fn value_chip(ui: &mut egui::Ui, text: &str, fill: Color32) {
    let font = FontId::proportional(10.5);
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_owned(), font.clone(), text_primary());
    let desired = Vec2::new((galley.size().x + 20.0).max(54.0), 24.0);
    let (rect, _) = ui.allocate_exact_size(desired, Sense::hover());

    if ui.is_rect_visible(rect) {
        ui.painter().rect(
            rect,
            CornerRadius::same(10),
            fill,
            Stroke::new(1.0, Color32::from_rgba_unmultiplied(255, 255, 255, 12)),
            StrokeKind::Middle,
        );
        ui.painter().text(
            centered_pill_text(rect),
            Align2::CENTER_CENTER,
            text,
            font,
            text_primary(),
        );
    }
}

fn fixed_value_chip(ui: &mut egui::Ui, rect: egui::Rect, text: &str, fill: Color32) {
    if ui.is_rect_visible(rect) {
        ui.painter().rect(
            rect,
            CornerRadius::same(10),
            fill,
            Stroke::new(1.0, Color32::from_rgba_unmultiplied(255, 255, 255, 12)),
            StrokeKind::Middle,
        );
        ui.painter().text(
            centered_pill_text(rect),
            Align2::CENTER_CENTER,
            text,
            FontId::proportional(10.5),
            text_primary(),
        );
    }
}

fn full_width_chip(
    ui: &mut egui::Ui,
    text: &str,
    fill: Color32,
    stroke: Stroke,
    text_color: Color32,
    height: f32,
    font_size: f32,
) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), height), Sense::hover());

    if ui.is_rect_visible(rect) {
        ui.painter().rect(
            rect,
            CornerRadius::same(10),
            fill,
            stroke,
            StrokeKind::Middle,
        );
        ui.painter().text(
            left_pill_text(rect, 12.0),
            Align2::LEFT_CENTER,
            text,
            FontId::proportional(font_size),
            text_color,
        );
    }
}

fn hovered_fill(fill: Color32) -> Color32 {
    Color32::from_rgba_unmultiplied(
        fill.r().saturating_add(10),
        fill.g().saturating_add(10),
        fill.b().saturating_add(10),
        fill.a(),
    )
}

fn pill_button(
    ui: &mut egui::Ui,
    label: &str,
    fill: Color32,
    stroke: Stroke,
    font_size: f32,
) -> egui::Response {
    let font = FontId::proportional(font_size);
    let galley = ui
        .painter()
        .layout_no_wrap(label.to_owned(), font.clone(), text_primary());
    let desired = Vec2::new((galley.size().x + 22.0).max(68.0), 30.0);
    let (rect, response) = ui.allocate_exact_size(desired, Sense::click());

    if ui.is_rect_visible(rect) {
        ui.painter().rect(
            rect,
            CornerRadius::same(9),
            if response.hovered() {
                hovered_fill(fill)
            } else {
                fill
            },
            stroke,
            StrokeKind::Middle,
        );
        ui.painter().text(
            centered_pill_text(rect),
            Align2::CENTER_CENTER,
            label,
            font,
            text_primary(),
        );
    }

    response
}

fn pill_label(
    ui: &mut egui::Ui,
    label: &str,
    fill: Color32,
    stroke: Stroke,
    text_color: Color32,
    font_size: f32,
) {
    let font = FontId::proportional(font_size);
    let galley = ui
        .painter()
        .layout_no_wrap(label.to_owned(), font.clone(), text_color);
    let desired = Vec2::new((galley.size().x + 22.0).max(68.0), 30.0);
    let (rect, _) = ui.allocate_exact_size(desired, Sense::hover());

    if ui.is_rect_visible(rect) {
        ui.painter().rect(
            rect,
            CornerRadius::same(9),
            fill,
            stroke,
            StrokeKind::Middle,
        );
        ui.painter().text(
            centered_pill_text(rect),
            Align2::CENTER_CENTER,
            label,
            font,
            text_color,
        );
    }
}

fn icon_button(ui: &mut egui::Ui, kind: IconButtonKind) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(30.0), Sense::click());

    if ui.is_rect_visible(rect) {
        let fill = if response.hovered() {
            hovered_fill(control_fill_hovered())
        } else {
            control_fill()
        };
        ui.painter().rect(
            rect,
            CornerRadius::same(9),
            fill,
            Stroke::new(1.0, accent_dim()),
            StrokeKind::Middle,
        );

        let stroke = Stroke::new(1.6, text_primary());
        match kind {
            IconButtonKind::Menu => {
                for offset in [-5.0, 0.0, 5.0] {
                    ui.painter().line_segment(
                        [
                            pos2(rect.left() + 8.0, rect.center().y + offset),
                            pos2(rect.right() - 8.0, rect.center().y + offset),
                        ],
                        stroke,
                    );
                }
            }
            IconButtonKind::Close => {
                ui.painter().line_segment(
                    [
                        pos2(rect.left() + 9.0, rect.top() + 9.0),
                        pos2(rect.right() - 9.0, rect.bottom() - 9.0),
                    ],
                    stroke,
                );
                ui.painter().line_segment(
                    [
                        pos2(rect.right() - 9.0, rect.top() + 9.0),
                        pos2(rect.left() + 9.0, rect.bottom() - 9.0),
                    ],
                    stroke,
                );
            }
        }
    }

    response
}

fn wide_button(
    ui: &mut egui::Ui,
    label: &str,
    height: f32,
    fill: Color32,
    stroke: Stroke,
    font_size: f32,
) -> egui::Response {
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), height), Sense::click());

    if ui.is_rect_visible(rect) {
        ui.painter().rect(
            rect,
            CornerRadius::same(10),
            if response.hovered() {
                hovered_fill(fill)
            } else {
                fill
            },
            stroke,
            StrokeKind::Middle,
        );
        ui.painter().text(
            centered_pill_text(rect),
            Align2::CENTER_CENTER,
            label,
            FontId::proportional(font_size),
            text_primary(),
        );
    }

    response
}

fn selection_button(
    ui: &mut egui::Ui,
    selected: bool,
    title: &str,
    subtitle: &str,
) -> egui::Response {
    let fill = if selected {
        accent_soft()
    } else {
        control_fill_hovered()
    };
    let stroke = if selected {
        Stroke::new(1.0, accent_color())
    } else {
        Stroke::new(1.0, border_color())
    };

    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), 46.0), Sense::click());

    if ui.is_rect_visible(rect) {
        ui.painter().rect(
            rect,
            CornerRadius::same(10),
            if response.hovered() {
                hovered_fill(fill)
            } else {
                fill
            },
            stroke,
            StrokeKind::Middle,
        );

        ui.painter().text(
            pos2(rect.left() + 12.0, rect.top() + 10.0),
            Align2::LEFT_TOP,
            title,
            FontId::proportional(12.0),
            text_primary(),
        );
        ui.painter().text(
            pos2(rect.left() + 12.0, rect.top() + 25.0),
            Align2::LEFT_TOP,
            subtitle,
            FontId::proportional(10.0),
            if selected {
                text_primary()
            } else {
                text_muted()
            },
        );
    }

    response
}

fn current_detected_device<'a>(
    snapshot: &'a RuntimeSnapshot,
    preferred_device_path: Option<&str>,
) -> Option<&'a InputDeviceInfo> {
    if let Some(active_device) = snapshot.active_device.as_ref() {
        return Some(active_device);
    }

    if let Some(path) = preferred_device_path {
        return snapshot
            .available_devices
            .iter()
            .find(|device| device.path == path);
    }

    snapshot
        .available_devices
        .iter()
        .find(|device| device.is_supported())
}

fn sanitize_preferred_device_path(preferred_device_path: Option<String>) -> Option<String> {
    preferred_device_path.filter(|path| !path.starts_with("xinput:"))
}

fn normalize_assist_config(mut config: AssistConfig) -> AssistConfig {
    config.only_assist_while_moving = true;
    config
}

fn jump_button_choices() -> &'static [(JumpButton, &'static str)] {
    &[
        (JumpButton::Cross, "A / Cross"),
        (JumpButton::Circle, "B / Circle"),
        (JumpButton::Square, "X / Square"),
        (JumpButton::Triangle, "Y / Triangle"),
        (JumpButton::L1, "LB / L1"),
        (JumpButton::R1, "RB / R1"),
        (JumpButton::L2Button, "LT / L2"),
        (JumpButton::R2Button, "RT / R2"),
        (JumpButton::L3, "L3"),
        (JumpButton::R3, "R3"),
        (JumpButton::Create, "Back / Create"),
        (JumpButton::Options, "Start / Options"),
        (JumpButton::Touchpad, "Touchpad"),
        (JumpButton::Ps, "PS / Home"),
    ]
}

const THRESHOLD_LEADING_WIDTH: f32 = 150.0;
const THRESHOLD_VALUE_WIDTH: f32 = 64.0;
const THRESHOLD_COLUMN_GAP: f32 = 8.0;
const THRESHOLD_ROW_HEIGHT: f32 = 28.0;
const THRESHOLD_SLIDER_HEIGHT: f32 = 18.0;

fn threshold_row(
    ui: &mut egui::Ui,
    label: &str,
    help_text: &str,
    value_text: &str,
    add_slider: impl FnOnce(&mut egui::Ui, egui::Rect),
) {
    let row_width = ui.available_width();
    let (row_rect, _) =
        ui.allocate_exact_size(Vec2::new(row_width, THRESHOLD_ROW_HEIGHT), Sense::hover());
    let value_rect = egui::Rect::from_min_size(
        pos2(row_rect.right() - THRESHOLD_VALUE_WIDTH, row_rect.top()),
        Vec2::new(THRESHOLD_VALUE_WIDTH, THRESHOLD_ROW_HEIGHT),
    );
    let slider_left = row_rect.left() + THRESHOLD_LEADING_WIDTH + THRESHOLD_COLUMN_GAP;
    let slider_right = value_rect.left() - THRESHOLD_COLUMN_GAP;
    let slider_rect = egui::Rect::from_min_size(
        pos2(
            slider_left,
            row_rect.center().y - (THRESHOLD_SLIDER_HEIGHT * 0.5),
        ),
        Vec2::new(
            (slider_right - slider_left).max(110.0),
            THRESHOLD_SLIDER_HEIGHT,
        ),
    );
    let info_rect = egui::Rect::from_center_size(
        pos2(
            row_rect.left() + THRESHOLD_LEADING_WIDTH - 8.0,
            row_rect.center().y,
        ),
        Vec2::splat(16.0),
    );

    ui.painter().text(
        pos2(row_rect.left(), row_rect.center().y),
        Align2::LEFT_CENTER,
        label,
        FontId::proportional(11.5),
        text_primary(),
    );

    let info_response = info_icon_button(ui, info_rect, ui.id().with(("threshold-info", label)));
    show_threshold_help_popup(&info_response, help_text);

    add_slider(ui, slider_rect);
    fixed_value_chip(ui, value_rect, value_text, accent_soft());
    ui.add_space(8.0);
}

fn slider_row_u64(
    ui: &mut egui::Ui,
    label: &str,
    help_text: &str,
    value: &mut u64,
    range: std::ops::RangeInclusive<u64>,
    suffix: &str,
) {
    threshold_row(
        ui,
        label,
        help_text,
        &format!("{value} {suffix}"),
        |ui, rect| {
            ui.put(
                rect,
                egui::Slider::new(value, range)
                    .show_value(false)
                    .trailing_fill(true),
            );
        },
    );
}

fn slider_row_f32(
    ui: &mut egui::Ui,
    label: &str,
    help_text: &str,
    value: &mut f32,
    range: std::ops::RangeInclusive<f32>,
) {
    threshold_row(ui, label, help_text, &format!("{value:.2}"), |ui, rect| {
        ui.put(
            rect,
            egui::Slider::new(value, range)
                .show_value(false)
                .trailing_fill(true),
        );
    });
}

fn show_threshold_help_popup(response: &egui::Response, help_text: &str) {
    egui::Popup::from_toggle_button_response(response)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .width(220.0)
        .gap(6.0)
        .show(|ui| {
            Frame::new()
                .fill(surface_fill())
                .stroke(Stroke::new(1.0, border_color()))
                .corner_radius(CornerRadius::same(12))
                .inner_margin(Margin::same(10))
                .show(ui, |ui| {
                    ui.set_max_width(220.0);
                    ui.label(RichText::new(help_text).size(11.0).color(text_primary()));
                });
        });
}

fn info_icon_button(ui: &mut egui::Ui, rect: egui::Rect, id: egui::Id) -> egui::Response {
    let response = ui.interact(rect, id, Sense::click());

    if ui.is_rect_visible(rect) {
        let fill = if response.hovered() {
            hovered_fill(control_fill_hovered())
        } else {
            control_fill()
        };
        let radius = 7.0;
        ui.painter().circle_filled(rect.center(), radius, fill);
        ui.painter()
            .circle_stroke(rect.center(), radius, Stroke::new(1.0, accent_dim()));
        ui.painter().circle_filled(
            pos2(rect.center().x, rect.center().y - 3.3),
            0.95,
            text_primary(),
        );
        ui.painter().line_segment(
            [
                pos2(rect.center().x, rect.center().y - 1.0),
                pos2(rect.center().x, rect.center().y + 3.2),
            ],
            Stroke::new(1.1, text_primary()),
        );
    }

    response
}

fn paint_backdrop(ui: &mut egui::Ui, strong: bool) {
    let rect = ui.max_rect();
    let painter = ui.painter();
    let base_alpha = if strong { 176 } else { 132 };

    painter.rect_filled(
        rect,
        CornerRadius::ZERO,
        Color32::from_rgba_unmultiplied(4, 7, 11, base_alpha),
    );
    painter.circle_filled(
        pos2(rect.left() + 88.0, rect.top() + 58.0),
        140.0,
        Color32::from_rgba_unmultiplied(36, 183, 219, 28),
    );
    painter.circle_filled(
        pos2(rect.right() - 92.0, rect.bottom() - 86.0),
        170.0,
        Color32::from_rgba_unmultiplied(19, 122, 255, 22),
    );
    painter.line_segment(
        [
            pos2(rect.left() + 12.0, rect.top() + 12.0),
            pos2(rect.right() - 12.0, rect.top() + 12.0),
        ],
        Stroke::new(1.0, accent_dim()),
    );
}

impl eframe::App for BhopApp {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        Color32::TRANSPARENT.to_normalized_gamma_f32()
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let (mut enabled, mut preferred_device_path, mut config, mut calibrations, snapshot) = {
            let state = self.shared.lock();
            (
                state.enabled,
                sanitize_preferred_device_path(state.preferred_device_path.clone()),
                normalize_assist_config(state.config),
                state.calibrations.clone(),
                state.runtime.clone(),
            )
        };

        let current_device =
            current_detected_device(&snapshot, preferred_device_path.as_deref()).cloned();
        self.process_calibration_session(
            current_device.as_ref(),
            snapshot.raw_state,
            &mut calibrations,
        );

        let controller_title = current_device
            .as_ref()
            .map(|device| device.product_label())
            .unwrap_or_else(|| "未検出".to_owned());
        let controller_backend = current_device
            .as_ref()
            .map(|device| device.transport_label().to_owned())
            .unwrap_or_else(|| "--".to_owned());
        let xinput_warning = current_device
            .as_ref()
            .filter(|device| device.is_xinput())
            .map(|_| "XInputコントローラーはこのツールでは非対応です。DInput/HIDモードへ切り替えるか、DualSense / DualShock 4 / Switch Pro などの対応入力を使ってください。");
        let runtime_error = snapshot.last_error.clone();
        let persist_error = self.persist_error.clone();
        let base_height = 290.0;
        let warning_height = 372.0;
        if let Some(inner_rect) = ctx.input(|input| input.viewport().inner_rect) {
            if xinput_warning.is_some() {
                if !self.warning_expanded_main_window || inner_rect.height() < warning_height {
                    ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(Vec2::new(
                        inner_rect.width(),
                        warning_height,
                    )));
                    self.warning_expanded_main_window = true;
                }
            } else {
                if self.warning_expanded_main_window {
                    ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(Vec2::new(
                        inner_rect.width(),
                        base_height,
                    )));
                    self.warning_expanded_main_window = false;
                } else if inner_rect.height() < base_height {
                    ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(Vec2::new(
                        inner_rect.width(),
                        base_height,
                    )));
                }
            }
        }

        paint_backdrop(ui, true);

        Frame::new()
            .fill(Color32::from_rgba_unmultiplied(7, 10, 15, 148))
            .inner_margin(Margin::same(6))
            .show(ui, |ui| {
                ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        main_shell_frame().show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.label(
                                    RichText::new("RESONANCE BUNNYHOP")
                                        .size(14.5)
                                        .strong()
                                        .color(brand_color()),
                                );

                                ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                                    if icon_button(ui, IconButtonKind::Menu).clicked() {
                                        self.options_open = true;
                                    }
                                });
                            });

                            ui.add_space(8.0);

                            let gap = 6.0;
                            let total_width = ui.available_width();
                            let half_width = ((total_width - gap) * 0.5).max(0.0);
                            let top_card_height = 90.0;
                            let controller_card_height = 86.0;
                            let primary_height = 28.0;
                            let toggle_button_fill = control_fill_hovered();
                            let toggle_button_stroke = Stroke::new(1.0, accent_dim());

                            ui.horizontal_top(|ui| {
                                ui.allocate_ui_with_layout(
                                    Vec2::new(half_width, top_card_height),
                                    Layout::top_down(Align::Min),
                                    |ui| {
                                        metric_frame().show(ui, |ui| {
                                            ui.set_min_height(top_card_height - 20.0);
                                            metric_label(ui, "BRIDGE");
                                            ui.add_space(4.0);
                                            ui.allocate_ui_with_layout(
                                                Vec2::new(ui.available_width(), primary_height),
                                                Layout::top_down(Align::Center),
                                                |ui| {
                                                    ui.label(
                                                        RichText::new(if enabled {
                                                            "ON"
                                                        } else {
                                                            "OFF"
                                                        })
                                                        .size(17.0)
                                                        .strong()
                                                        .color(if enabled {
                                                            accent_color()
                                                        } else {
                                                            danger_color()
                                                        }),
                                                    );
                                                },
                                            );
                                            ui.add_space(2.0);
                                            if wide_button(
                                                ui,
                                                if enabled { "■" } else { "▶" },
                                                primary_height,
                                                toggle_button_fill,
                                                toggle_button_stroke,
                                                14.5,
                                            )
                                            .clicked()
                                            {
                                                enabled = !enabled;
                                            }
                                        });
                                    },
                                );

                                ui.add_space(gap);

                                ui.allocate_ui_with_layout(
                                    Vec2::new(half_width, top_card_height),
                                    Layout::top_down(Align::Min),
                                    |ui| {
                                        metric_frame().show(ui, |ui| {
                                            ui.set_min_height(top_card_height - 20.0);
                                            metric_label(ui, "ASSIST");
                                            ui.add_space(4.0);
                                            ui.allocate_ui_with_layout(
                                                Vec2::new(ui.available_width(), primary_height),
                                                Layout::top_down(Align::Center),
                                                |ui| {
                                                    ui.label(
                                                        RichText::new(if config.assist_enabled {
                                                            "ON"
                                                        } else {
                                                            "OFF"
                                                        })
                                                        .size(17.0)
                                                        .strong()
                                                        .color(if config.assist_enabled {
                                                            accent_color()
                                                        } else {
                                                            danger_color()
                                                        }),
                                                    );
                                                },
                                            );
                                            ui.add_space(2.0);
                                            if wide_button(
                                                ui,
                                                if config.assist_enabled { "■" } else { "▶" },
                                                primary_height,
                                                toggle_button_fill,
                                                toggle_button_stroke,
                                                14.5,
                                            )
                                            .clicked()
                                            {
                                                config.assist_enabled = !config.assist_enabled;
                                            }
                                        });
                                    },
                                );
                            });

                            ui.add_space(gap);

                            ui.allocate_ui_with_layout(
                                Vec2::new(total_width, controller_card_height),
                                Layout::top_down(Align::Min),
                                |ui| {
                                    metric_frame().show(ui, |ui| {
                                        ui.set_min_height(controller_card_height - 20.0);
                                        metric_label(ui, "CONTROLLER");
                                        ui.add_space(4.0);
                                        ui.allocate_ui_with_layout(
                                            Vec2::new(ui.available_width(), primary_height),
                                            Layout::top_down(Align::Center),
                                            |ui| {
                                                ui.label(
                                                    RichText::new(controller_title)
                                                        .size(13.5)
                                                        .strong()
                                                        .color(text_primary()),
                                                );
                                            },
                                        );
                                        ui.add_space(4.0);
                                        let remaining = ui.available_height();
                                        ui.allocate_ui_with_layout(
                                            Vec2::new(ui.available_width(), remaining.max(0.0)),
                                            Layout::bottom_up(Align::Center),
                                            |ui| {
                                                value_chip(ui, &controller_backend, accent_soft());
                                            },
                                        );
                                    });
                                },
                            );

                            if let Some(error_text) = runtime_error.as_deref() {
                                ui.add_space(14.0);
                                error_panel(ui, error_text);
                            }

                            if let Some(error_text) = persist_error.as_deref() {
                                ui.add_space(14.0);
                                error_panel(ui, error_text);
                            }

                            if let Some(notice_text) = xinput_warning {
                                ui.add_space(14.0);
                                warning_panel(ui, "XINPUT UNSUPPORTED", notice_text);
                            }
                        });
                    });
            });

        self.show_options_viewport(
            &ctx,
            &snapshot,
            &mut preferred_device_path,
            &mut config,
            &mut calibrations,
            current_device.clone(),
            enabled,
        );

        let settings_preferred_device_path = preferred_device_path.clone();
        {
            let mut state = self.shared.lock();
            state.enabled = enabled;
            state.preferred_device_path = preferred_device_path;
            state.config = normalize_assist_config(config);
            state.calibrations = calibrations.clone();
        }

        self.queue_settings_save(self.current_settings(
            settings_preferred_device_path,
            config,
            calibrations,
        ));
        self.flush_pending_settings_if_due();

        let repaint_ms = if self.calibration_session.is_some() {
            16
        } else if self.options_open || enabled {
            90
        } else {
            220
        };
        ctx.request_repaint_after(Duration::from_millis(repaint_ms));
    }
}
