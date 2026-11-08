//! Styled-layer composite cost, for the v3.8.1 clone fix.
//!
//! Run with:
//!   cargo run --release -p aurelia-render --example bench_styles
//!
//! It times a full composite and a Navigator thumbnail for 1 and 4 styled layers at 1080p and
//! 4K. Compare the output before/after the metadata-only styled-layer copy.

use aurelia_core::color::Color;
use aurelia_core::document::Document;
use aurelia_core::layer_style::{DropShadowStyle, LayerStyles, OuterGlowStyle, StrokeStyle};
use aurelia_render::SoftwareRenderer;
use glam::Vec2;
use std::time::Instant;

const ITERS: u32 = 12;

fn styled_doc(width: u32, height: u32, layers: usize) -> Document {
    let mut doc = Document::new(width, height);
    for index in 0..layers {
        let id = if index == 0 {
            doc.active_layer_id
        } else {
            doc.add_layer(Some(format!("Styled {index}")))
        };
        if let Some(layer) = doc.get_layer_mut(id) {
            for (i, px) in layer.as_srgb8_mut().expect("8-bit").chunks_exact_mut(4).enumerate() {
                let x = (i as u32) % width;
                let y = (i as u32) / width;
                let inside = x > width / 8
                    && x < width * 7 / 8
                    && y > height / 8
                    && y < height * 7 / 8
                    && ((x + y + index as u32 * 37) % 97) < 70;
                px[3] = if inside { 255 } else { 0 };
                px[0] = (x % 256) as u8;
                px[1] = (y % 256) as u8;
                px[2] = 90;
            }
            layer.styles = LayerStyles {
                stroke: Some(StrokeStyle { width: 3.0, color: Color::BLACK, opacity: 0.8 }),
                drop_shadow: Some(DropShadowStyle {
                    offset_x: 6.0,
                    offset_y: 6.0,
                    blur: 8.0,
                    color: Color::BLACK,
                    opacity: 0.5,
                }),
                outer_glow: Some(OuterGlowStyle { radius: 6.0, color: Color::WHITE, opacity: 0.4 }),
            };
        }
    }
    doc
}

fn median(mut samples: Vec<f64>) -> f64 {
    samples.sort_by(|a, b| a.total_cmp(b));
    samples[samples.len() / 2]
}

fn measure(width: u32, height: u32, layers: usize) {
    let doc = styled_doc(width, height, layers);
    let mut renderer = SoftwareRenderer::new();
    let mut buffer = vec![0u32; (width * height) as usize];
    let viewport = [0, 0, width as usize, height as usize];

    let mut composite = Vec::new();
    let mut viewport_time = Vec::new();
    let mut thumbnail = Vec::new();
    let mut style_only = Vec::new();
    for _ in 0..ITERS {
        let start = Instant::now();
        let pixels = doc.composite_layers(false);
        std::hint::black_box(&pixels);
        composite.push(start.elapsed().as_secs_f64() * 1000.0);

        if let Some(layer) = doc.layers.first() {
            let start = Instant::now();
            let styled = aurelia_core::layer_style::render_styled(layer);
            std::hint::black_box(&styled);
            style_only.push(start.elapsed().as_secs_f64() * 1000.0);
        }

        let start = Instant::now();
        renderer.render_canvas(
            &mut buffer,
            width as usize,
            height as usize,
            &doc,
            Vec2::ZERO,
            1.0,
            Vec2::ZERO,
            viewport,
            None,
            &[],
            None,
        );
        viewport_time.push(start.elapsed().as_secs_f64() * 1000.0);

        let start = Instant::now();
        if let Some(image) = renderer.navigator_thumbnail(&doc, 320) {
            std::hint::black_box(&image);
        }
        thumbnail.push(start.elapsed().as_secs_f64() * 1000.0);
    }

    println!(
        "{width}x{height}, {layers} styled layer(s): style-render {:.2} ms, composite {:.2} ms, viewport {:.2} ms, navigator {:.2} ms",
        median(style_only),
        median(composite),
        median(viewport_time),
        median(thumbnail)
    );
}

fn main() {
    println!("Aurelia styled-layer cost (median of {ITERS} runs)");
    for (width, height) in [(1920, 1080), (3840, 2160)] {
        for layers in [1usize, 4] {
            measure(width, height, layers);
        }
    }
}
