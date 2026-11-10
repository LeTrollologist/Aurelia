//! Canvas present-path benchmark: GDI-compatible RGBA8 vs native RGBA16 -> 10-bit.
//!
//! Run:
//!   cargo run --release -p aurelia-render --example bench_canvas_present -- 1920 1080
//!   cargo run --release -p aurelia-render --example bench_canvas_present -- 3840 2160
//!
//! It reports the render half of each path. DXGI upload/present is measured separately by the
//! app smoke test; this keeps the comparison dependency-free and repeatable.

use aurelia_core::document::Document;
use aurelia_core::layer::PixelFormat;
use glam::Vec2;
use std::time::{Duration, Instant};

const REPEATS: u32 = 5;

fn song(value: u16, y: u32, h: u32) -> u16 {
    let ramp = ((y as u32 * 65_535) / h.max(1)) as u16;
    value.wrapping_add(ramp)
}

fn build_doc(w: u32, h: u32) -> Document {
    let mut doc = Document::new_with_format(w, h, PixelFormat::Srgb16);
    for layer_index in 0..3 {
        if layer_index > 0 {
            doc.add_layer(Some(format!("Layer {layer_index}")));
        }
    }
    for (layer_index, layer) in doc.layers.iter_mut().enumerate() {
        if let Some(pixels) = layer.as_srgb16_mut() {
            for y in 0..h {
                for x in 0..w {
                    let i = ((y * w + x) * 4) as usize;
                    let value = 8_000 + (layer_index as u16 * 3_000);
                    pixels[i] = song(value, y, h);
                    pixels[i + 1] = song(value / 2, x, w);
                    pixels[i + 2] = 12_000;
                    pixels[i + 3] = 65_535;
                }
            }
        }
    }
    doc
}

fn main() {
    let mut args = std::env::args().skip(1);
    let w: u32 = args
        .next()
        .and_then(|value| value.parse().ok())
        .unwrap_or(1920);
    let h: u32 = args
        .next()
        .and_then(|value| value.parse().ok())
        .unwrap_or(1080);
    let doc = build_doc(w, h);
    let mut renderer = aurelia_render::SoftwareRenderer::new();
    let mut bitmap = vec![0u32; (w as usize) * (h as usize)];
    let mut tenbit = vec![0u32; (w as usize) * (h as usize)];
    let viewport = [0, 0, w as usize, h as usize];

    let mut best8 = Duration::MAX;
    let mut best10 = Duration::MAX;
    for _ in 0..REPEATS {
        let start = Instant::now();
        renderer.render_canvas(
            &mut bitmap,
            w as usize,
            h as usize,
            &doc,
            Vec2::ZERO,
            1.0,
            Vec2::ZERO,
            viewport,
            None,
            &[],
            None,
        );
        best8 = best8.min(start.elapsed());
        let start = Instant::now();
        assert!(renderer.render_canvas_srgb16(
            &mut tenbit,
            w as usize,
            h as usize,
            &doc,
            Vec2::ZERO,
            1.0,
            Vec2::ZERO,
            viewport,
            None,
            &[],
            None,
        ));
        best10 = best10.min(start.elapsed());
    }

    let mpix = (w as f64 * h as f64) / 1_000_000.0;
    println!("canvas {w}x{h} ({mpix:.1} MPix), best of {REPEATS}");
    println!(
        "RGBA8 composite -> GDI frame: {:?} ({:.1} MPix/s)",
        best8,
        mpix / best8.as_secs_f64()
    );
    println!(
        "RGBA16 composite -> 10-bit: {:?} ({:.1} MPix/s)",
        best10,
        mpix / best10.as_secs_f64()
    );
    println!(
        "10-bit overhead: {:.2}x",
        best10.as_secs_f64() / best8.as_secs_f64()
    );
}
