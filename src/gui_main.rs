//! EmFit GUI Entry Point
//!
//! Launches the Everything-like GUI for file searching.

#![cfg(windows)]
#![windows_subsystem = "windows"]

use eframe::egui;
use emfit::gui::EmFitApp;

fn main() -> eframe::Result<()> {
    // Configure native options
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1024.0, 768.0])
            .with_min_inner_size([640.0, 480.0])
            .with_title("EmFit - File Search"),
        ..Default::default()
    };

    // Run the app
    eframe::run_native(
        "EmFit",
        native_options,
        Box::new(|cc| Ok(Box::new(EmFitApp::new(cc)))),
    )
}
