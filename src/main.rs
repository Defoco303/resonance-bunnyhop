#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

mod app;
mod assist;
mod calibration;
mod diagnostics;
mod dualsense;
mod runtime;
mod settings;
mod x360;
mod xinput;

use eframe::{egui, icon_data};

fn app_icon() -> egui::IconData {
    icon_data::from_png_bytes(include_bytes!("../assets/icon.png"))
        .expect("embedded icon png should be valid")
}

fn main() -> eframe::Result<()> {
    let viewport = egui::ViewportBuilder::default()
        .with_inner_size([442.0, 290.0])
        .with_min_inner_size([408.0, 260.0])
        .with_resizable(true)
        .with_transparent(true)
        .with_icon(app_icon())
        .with_title("RESONANCE BUNNYHOP");

    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };

    eframe::run_native(
        "RESONANCE BUNNYHOP",
        options,
        Box::new(|cc| Ok(Box::new(app::BhopApp::new(cc)))),
    )
}
