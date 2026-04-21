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
        self, Align, Align2, Color32, CornerRadius, FontData, FontDefinitions, FontFamily,
        FontId, Frame, Layout, Margin, RichText, ScrollArea, Sense, Stroke, StrokeKind, Vec2,
        ViewportBuilder, ViewportId, pos2,
    },
};
use parking_lot::Mutex;

use crate::{
    assist::{AssistConfig, JumpButton},
    dualsense::InputDeviceInfo,
    runtime::{RuntimeSnapshot, ServiceState, SharedState, spawn_runtime},
};

#[derive(Clone, Copy, Debug)]
struct JumpCaptureState {
    blocked_mask: u32,
    started_at: Instant,
}

#[derive(Clone, Copy, Debug)]
enum IconButtonKind {
    Menu,
    Close,
}

pub struct BhopApp {
    shared: Arc<Mutex<SharedState>>,
    runtime_handle: Option<JoinHandle<()>>,
    jump_capture: Option<JumpCaptureState>,
    options_open: bool,
}

impl BhopApp {
    pub fn new(cc: &CreationContext<'_>) -> Self {
        configure_fonts(&cc.egui_ctx);
        configure_theme(&cc.egui_ctx);
        let shared = Arc::new(Mutex::new(SharedState::default()));
        let runtime_handle = Some(spawn_runtime(shared.clone()));
        Self {
            shared,
            runtime_handle,
            jump_capture: None,
            options_open: false,
        }
    }

    fn show_options_viewport(
        &mut self,
        ctx: &egui::Context,
        snapshot: &RuntimeSnapshot,
        preferred_device_path: &mut Option<String>,
        config: &mut AssistConfig,
        raw_capture_mask: u32,
    ) {
        if !self.options_open {
            return;
        }

        let mut close_requested = false;
        let viewport_id = ViewportId::from_hash_of("options-viewport");
        let builder = ViewportBuilder::default()
            .with_title("オプション")
            .with_inner_size([400.0, 430.0])
            .with_min_inner_size([340.0, 340.0])
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
                                                .size(9.5)
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
                                self.jump_section(ui, config, raw_capture_mask);
                                ui.add_space(8.0);
                                self.threshold_section(ui, config);
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

    fn jump_section(
        &mut self,
        ui: &mut egui::Ui,
        config: &mut AssistConfig,
        raw_capture_mask: u32,
    ) {
        full_width_section(ui, |ui| {
            section_title(ui, "ジャンプボタン設定");
            ui.add_space(10.0);

            ui.horizontal_wrapped(|ui| {
                value_chip(ui, config.jump_button.label(), accent_soft());

                let listening = self.jump_capture.is_some();
                if listening {
                    value_chip(ui, "入力待ち", warning_soft());
                }

                let label = if listening {
                    "入力中..."
                } else {
                    "入力から設定"
                };

                if pill_button(
                    ui,
                    label,
                    control_fill_hovered(),
                    Stroke::new(1.0, accent_dim()),
                    13.5,
                )
                .clicked()
                {
                    self.jump_capture = Some(JumpCaptureState {
                        blocked_mask: raw_capture_mask,
                        started_at: Instant::now(),
                    });
                }

                if listening
                    && pill_button(
                        ui,
                        "キャンセル",
                        control_fill_hovered(),
                        Stroke::new(1.0, accent_dim()),
                        13.5,
                    )
                    .clicked()
                {
                    self.jump_capture = None;
                }
            });

            ui.add_space(8.0);
            ui.label(
                RichText::new("※BRIDGEをオンにした状態で設定してください。")
                    .size(10.5)
                    .color(text_muted()),
            );
            ui.add_space(6.0);
            ui.add(egui::Checkbox::new(
                &mut config.only_assist_while_moving,
                RichText::new("移動入力があるときだけ補助する")
                    .size(11.5)
                    .color(text_primary()),
            ));
        });
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
                .filter(|device| device.is_supported())
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
        });
    }
}

impl Drop for BhopApp {
    fn drop(&mut self) {
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
    style.visuals.window_corner_radius = CornerRadius::same(20);
    style.visuals.menu_corner_radius = CornerRadius::same(16);

    style.visuals.widgets.noninteractive.weak_bg_fill = surface_fill();
    style.visuals.widgets.noninteractive.bg_fill = surface_fill();
    style.visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, border_color());
    style.visuals.widgets.noninteractive.fg_stroke.color = text_primary();
    style.visuals.widgets.noninteractive.corner_radius = CornerRadius::same(14);

    style.visuals.widgets.inactive.weak_bg_fill = control_fill();
    style.visuals.widgets.inactive.bg_fill = control_fill();
    style.visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, border_color());
    style.visuals.widgets.inactive.fg_stroke.color = text_primary();
    style.visuals.widgets.inactive.corner_radius = CornerRadius::same(14);

    style.visuals.widgets.hovered.weak_bg_fill = control_fill_hovered();
    style.visuals.widgets.hovered.bg_fill = control_fill_hovered();
    style.visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, accent_dim());
    style.visuals.widgets.hovered.fg_stroke.color = text_primary();
    style.visuals.widgets.hovered.corner_radius = CornerRadius::same(14);

    style.visuals.widgets.active.weak_bg_fill = accent_soft();
    style.visuals.widgets.active.bg_fill = accent_soft();
    style.visuals.widgets.active.bg_stroke = Stroke::new(1.0, accent_color());
    style.visuals.widgets.active.fg_stroke.color = text_primary();
    style.visuals.widgets.active.corner_radius = CornerRadius::same(14);

    style.visuals.widgets.open.weak_bg_fill = control_fill_hovered();
    style.visuals.widgets.open.bg_fill = control_fill_hovered();
    style.visuals.widgets.open.bg_stroke = Stroke::new(1.0, accent_dim());
    style.visuals.widgets.open.fg_stroke.color = text_primary();
    style.visuals.widgets.open.corner_radius = CornerRadius::same(14);

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

fn warning_soft() -> Color32 {
    Color32::from_rgba_unmultiplied(138, 98, 30, 236)
}

fn warning_color() -> Color32 {
    Color32::from_rgb(255, 204, 107)
}

fn danger_soft() -> Color32 {
    Color32::from_rgba_unmultiplied(144, 58, 70, 236)
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
        .corner_radius(CornerRadius::same(18))
        .inner_margin(Margin::same(14))
}

fn section_frame() -> Frame {
    Frame::new()
        .fill(Color32::from_rgba_unmultiplied(16, 21, 28, 224))
        .stroke(Stroke::new(1.0, border_color()))
        .corner_radius(CornerRadius::same(14))
        .inner_margin(Margin::same(12))
}

fn metric_frame() -> Frame {
    Frame::new()
        .fill(Color32::from_rgba_unmultiplied(14, 19, 26, 236))
        .stroke(Stroke::new(1.0, border_color()))
        .corner_radius(CornerRadius::same(14))
        .inner_margin(Margin::same(12))
}

fn error_frame() -> Frame {
    Frame::new()
        .fill(Color32::from_rgba_unmultiplied(52, 18, 24, 224))
        .stroke(Stroke::new(1.0, warning_color()))
        .corner_radius(CornerRadius::same(14))
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
    ui.label(
        RichText::new(title)
            .size(9.5)
            .strong()
            .color(text_muted()),
    );
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

fn value_chip(ui: &mut egui::Ui, text: &str, fill: Color32) {
    Frame::new()
        .fill(fill)
        .stroke(Stroke::new(1.0, Color32::from_rgba_unmultiplied(255, 255, 255, 12)))
        .corner_radius(CornerRadius::same(255))
        .inner_margin(Margin::symmetric(10, 5))
        .show(ui, |ui| {
            ui.label(
                RichText::new(text)
                    .size(10.5)
                    .strong()
                    .color(text_primary()),
            );
        });
}

fn fixed_value_chip(ui: &mut egui::Ui, rect: egui::Rect, text: &str, fill: Color32) {
    if ui.is_rect_visible(rect) {
        ui.painter().rect(
            rect,
            CornerRadius::same(255),
            fill,
            Stroke::new(1.0, Color32::from_rgba_unmultiplied(255, 255, 255, 12)),
            StrokeKind::Middle,
        );
        ui.painter().text(
            rect.center(),
            Align2::CENTER_CENTER,
            text,
            FontId::proportional(10.5),
            text_primary(),
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
            CornerRadius::same(12),
            if response.hovered() {
                hovered_fill(fill)
            } else {
                fill
            },
            stroke,
            StrokeKind::Middle,
        );
        ui.painter().text(
            rect.center(),
            Align2::CENTER_CENTER,
            label,
            font,
            text_primary(),
        );
    }

    response
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
            CornerRadius::same(10),
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
            CornerRadius::same(14),
            if response.hovered() {
                hovered_fill(fill)
            } else {
                fill
            },
            stroke,
            StrokeKind::Middle,
        );
        ui.painter().text(
            rect.center(),
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
    let fill = if selected { accent_soft() } else { control_fill_hovered() };
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
            CornerRadius::same(14),
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
            if selected { text_primary() } else { text_muted() },
        );
    }

    response
}

fn status_accent(state: ServiceState) -> Color32 {
    match state {
        ServiceState::Running => accent_soft(),
        ServiceState::Searching => warning_soft(),
        ServiceState::DriverMissing | ServiceState::Error => danger_soft(),
        ServiceState::Stopped => control_fill_hovered(),
    }
}

fn service_state_text(state: ServiceState) -> &'static str {
    match state {
        ServiceState::Stopped => "停止中",
        ServiceState::Searching => "検出中",
        ServiceState::Running => "動作中",
        ServiceState::DriverMissing => "ViGEmBus 未検出",
        ServiceState::Error => "入力エラー",
    }
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

const THRESHOLD_LEADING_WIDTH: f32 = 150.0;
const THRESHOLD_VALUE_WIDTH: f32 = 76.0;
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
    let (row_rect, _) = ui.allocate_exact_size(
        Vec2::new(row_width, THRESHOLD_ROW_HEIGHT),
        Sense::hover(),
    );
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
        Vec2::new((slider_right - slider_left).max(110.0), THRESHOLD_SLIDER_HEIGHT),
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

    let info_response =
        info_icon_button(ui, info_rect, ui.id().with(("threshold-info", label)));
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
    threshold_row(ui, label, help_text, &format!("{value} {suffix}"), |ui, rect| {
        ui.put(
            rect,
            egui::Slider::new(value, range)
                .show_value(false)
                .trailing_fill(true),
        );
    });
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
                    ui.label(
                        RichText::new(help_text)
                            .size(11.0)
                            .color(text_primary()),
                    );
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
        let (mut enabled, mut preferred_device_path, mut config, snapshot) = {
            let state = self.shared.lock();
            (
                state.enabled,
                state.preferred_device_path.clone(),
                state.config,
                state.runtime.clone(),
            )
        };

        let raw_capture_mask = snapshot.raw_state.buttons.capture_mask();
        if let Some(capture) = &mut self.jump_capture {
            capture.blocked_mask &= raw_capture_mask;
            let candidate_mask = raw_capture_mask & !capture.blocked_mask;
            if let Some(button) = JumpButton::from_capture_mask(candidate_mask) {
                config.jump_button = button;
                self.jump_capture = None;
            } else if capture.started_at.elapsed() > Duration::from_secs(8) {
                self.jump_capture = None;
            }
        }

        let current_device = current_detected_device(&snapshot, preferred_device_path.as_deref());
        let controller_title = current_device
            .map(|device| device.product_label())
            .unwrap_or_else(|| "未検出".to_owned());
        let controller_backend = current_device
            .map(|device| device.connection_mode.label().to_owned())
            .unwrap_or_else(|| "--".to_owned());
        let runtime_error = snapshot.last_error.clone();

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

                            ui.add_space(12.0);

                            let gap = 8.0;
                            let total_width = ui.available_width();
                            let bridge_width = (total_width * 0.24).clamp(84.0, 120.0);
                            let controller_width = (total_width - gap - bridge_width).max(0.0);
                            let card_height = 106.0;
                            let primary_height = 34.0;

                            ui.horizontal_top(|ui| {
                                ui.allocate_ui_with_layout(
                                    Vec2::new(bridge_width, card_height),
                                    Layout::top_down(Align::Min),
                                    |ui| {
                                        metric_frame().show(ui, |ui| {
                                            ui.set_min_height(card_height - 24.0);
                                            metric_label(ui, "BRIDGE");
                                            ui.add_space(8.0);
                                            ui.allocate_ui_with_layout(
                                                Vec2::new(ui.available_width(), primary_height),
                                                Layout::top_down(Align::Center),
                                                |ui| {
                                                    ui.label(
                                                        RichText::new(if enabled { "オン" } else { "オフ" })
                                                            .size(17.0)
                                                            .strong()
                                                            .color(if enabled {
                                                                accent_color()
                                                            } else {
                                                                text_primary()
                                                            }),
                                                    );
                                                },
                                            );
                                            ui.add_space(6.0);
                                            let remaining = ui.available_height();
                                            ui.allocate_ui_with_layout(
                                                Vec2::new(ui.available_width(), remaining.max(0.0)),
                                                Layout::bottom_up(Align::Center),
                                                |ui| {
                                                    value_chip(
                                                        ui,
                                                        service_state_text(snapshot.service_state),
                                                        status_accent(snapshot.service_state),
                                                    );
                                                },
                                            );
                                        });
                                    },
                                );

                                ui.add_space(gap);

                                ui.allocate_ui_with_layout(
                                    Vec2::new(controller_width, card_height),
                                    Layout::top_down(Align::Min),
                                    |ui| {
                                        metric_frame().show(ui, |ui| {
                                            ui.set_min_height(card_height - 24.0);
                                            metric_label(ui, "CONTROLLER");
                                            ui.add_space(8.0);
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
                                            ui.add_space(6.0);
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
                            });

                            ui.add_space(12.0);

                            let (button_label, button_fill, button_stroke) = if enabled {
                                (
                                    "停止",
                                    danger_soft(),
                                    Stroke::new(1.0, warning_color()),
                                )
                            } else {
                                (
                                    "開始",
                                    accent_soft(),
                                    Stroke::new(1.0, accent_color()),
                                )
                            };

                            if wide_button(
                                ui,
                                button_label,
                                36.0,
                                button_fill,
                                button_stroke,
                                14.5,
                            )
                            .clicked()
                            {
                                enabled = !enabled;
                            }

                            if let Some(error_text) = runtime_error.as_deref() {
                                ui.add_space(14.0);
                                error_panel(ui, error_text);
                            }
                        });
                    });
            });

        self.show_options_viewport(
            &ctx,
            &snapshot,
            &mut preferred_device_path,
            &mut config,
            raw_capture_mask,
        );

        {
            let mut state = self.shared.lock();
            state.enabled = enabled;
            state.preferred_device_path = preferred_device_path;
            state.config = AssistConfig { ..config };
        }

        let repaint_ms = if self.jump_capture.is_some() {
            16
        } else if self.options_open || enabled {
            90
        } else {
            220
        };
        ctx.request_repaint_after(Duration::from_millis(repaint_ms));
    }
}
