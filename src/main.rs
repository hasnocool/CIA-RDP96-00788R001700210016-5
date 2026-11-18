// Filename: src/main.rs

mod app;
mod audio;
mod dsp;
mod export;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size([1040.0, 820.0])
            .with_min_inner_size([780.0, 620.0]),
        ..Default::default()
    };

    eframe::run_native(
        "Gateway Audio Lab",
        options,
        Box::new(|creation_context| {
            Ok(Box::new(app::GatewayApp::new(creation_context)))
        }),
    )
}
