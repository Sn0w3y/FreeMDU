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

fn load_icon() -> egui::IconData {
    // Simple default icon - a wrench/tool symbol
    let size = 32;
    let mut rgba = vec![0u8; size * size * 4];

    // Create a simple blue circle icon
    for y in 0..size {
        for x in 0..size {
            let idx = (y * size + x) * 4;
            let cx = x as f32 - size as f32 / 2.0;
            let cy = y as f32 - size as f32 / 2.0;
            let dist = (cx * cx + cy * cy).sqrt();

            if dist < size as f32 / 2.0 - 2.0 {
                // Blue fill
                rgba[idx] = 66; // R
                rgba[idx + 1] = 133; // G
                rgba[idx + 2] = 244; // B
                rgba[idx + 3] = 255; // A
            } else if dist < size as f32 / 2.0 {
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
        width: size as u32,
        height: size as u32,
    }
}
