pub mod app;
pub mod colors;
pub mod table;
pub mod search;
pub mod treemap;
pub mod dialogs;

/// Entry point: launch the native GUI window
pub fn run() -> crate::Result<()> {
    let native_options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("EmFit — Ultra-fast NTFS File Scanner")
            .with_inner_size([1280.0, 800.0])
            .with_min_inner_size([640.0, 400.0]),
        ..Default::default()
    };

    eframe::run_native(
        "EmFit",
        native_options,
        Box::new(|cc| Ok(Box::new(app::GuiApp::new(cc)))),
    )
    .map_err(|e| crate::EmFitError::WindowsError(format!("GUI error: {}", e)))
}
