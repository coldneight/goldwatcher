#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! 金瞳 GoldWatcher — 全球黄金市场实时监控与告警工具。

mod alerts;
mod app;
mod data;
mod model;
mod notify;
mod storage;
mod theme;
#[cfg(any(target_os = "linux", target_os = "windows"))]
mod tray;

use std::sync::Arc;

fn load_icon() -> eframe::egui::IconData {
    eframe::icon_data::from_png_bytes(include_bytes!("../assets/icon-256.png"))
        .expect("内置应用图标必须有效")
}

fn main() -> eframe::Result<()> {
    let icon = load_icon();
    let viewport = eframe::egui::ViewportBuilder::default()
        .with_title("金瞳 GoldWatcher · 全球黄金监控")
        .with_app_id("io.github.goldwatcher")
        .with_inner_size([1360.0, 850.0])
        .with_min_inner_size([920.0, 640.0])
        .with_icon(Arc::new(icon.clone()));

    let options = eframe::NativeOptions {
        viewport,
        vsync: true,
        ..Default::default()
    };

    eframe::run_native(
        "金瞳 GoldWatcher",
        options,
        Box::new(move |cc| Ok(Box::new(app::GoldWatcherApp::new(cc, icon)))),
    )
}
