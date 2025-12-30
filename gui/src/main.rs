#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod worker;

use anyhow::Result;
use app::FreeMduApp;

fn main() -> Result<()> {
    env_logger::init();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([900.0, 700.0])
            .with_min_inner_size([600.0, 400.0])
            .with_icon(load_icon()),
        ..Default::default()
    };

    eframe::run_native(
        "FreeMDU",
        options,
        Box::new(|cc| Ok(Box::new(FreeMduApp::new(cc)))),
    )
    .map_err(|e| anyhow::anyhow!("Failed to run application: {e}"))
}

#[allow(clippy::cast_precision_loss)]
fn load_icon() -> egui::IconData {
    // Simple default icon - a blue circle
    const SIZE: u32 = 32;
    const SIZE_F: f32 = SIZE as f32;
    let mut rgba = vec![0u8; (SIZE * SIZE * 4) as usize];

    // Create a simple blue circle icon
    for y in 0..SIZE {
        for x in 0..SIZE {
            let idx = ((y * SIZE + x) * 4) as usize;
            let cx = x as f32 - SIZE_F / 2.0;
            let cy = y as f32 - SIZE_F / 2.0;
            let dist = (cx * cx + cy * cy).sqrt();

            if dist < SIZE_F / 2.0 - 2.0 {
                // Blue fill
                rgba[idx] = 66; // R
                rgba[idx + 1] = 133; // G
                rgba[idx + 2] = 244; // B
                rgba[idx + 3] = 255; // A
            } else if dist < SIZE_F / 2.0 {
                // White border
                rgba[idx] = 255;
                rgba[idx + 1] = 255;
                rgba[idx + 2] = 255;
                rgba[idx + 3] = 255;
            }
        }
    }

    egui::IconData {
        rgba,
        width: SIZE,
        height: SIZE,
    }
}
