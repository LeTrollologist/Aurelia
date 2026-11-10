//! Styled-layer composite cost, for the v3.9.0 content-bounds milestone.
//!
//! Run with:
//!   cargo run --release -p aurelia-render --example bench_styles
//!
//! The workload is deliberately identical for every layer count: same art generator, same effect
//! parameters, same number of effects. Only the number of stacked styled layers changes. Warm-up
//! runs are discarded; raw samples are written next to the medians so a later comparison does not
//! depend on a single summary number.

use aurelia_core::color::Color;
use aurelia_core::document::Document;
use aurelia_core::layer_style::{DropShadowStyle, LayerStyles, OuterGlowStyle, StrokeStyle};
use aurelia_render::SoftwareRenderer;
use glam::Vec2;
use std::fmt::Write as _;

const WARMUP: u32 = 3;
const ITERS: u32 = 20;
const LAYER_COUNTS: [usize; 4] = [1, 2, 4, 8];

fn styles() -> LayerStyles {
    LayerStyles {
        stroke: Some(StrokeStyle {
            width: 3.0,
            color: Color::BLACK,
            opacity: 0.8,
        }),
        drop_shadow: Some(DropShadowStyle {
            offset_x: 6.0,
            offset_y: 6.0,
            blur: 8.0,
            color: Color::BLACK,
            opacity: 0.5,
        }),
        outer_glow: Some(OuterGlowStyle {
            radius: 6.0,
            color: Color::WHITE,
            opacity: 0.4,
        }),
    }
}

/// One art pattern, independent of the layer index and of how many layers exist.
fn styled_doc(width: u32, height: u32, layers: usize, small: bool) -> Document {
    let mut doc = Document::new(width, height);
    for index in 0..layers {
        let id = if index == 0 {
            doc.active_layer_id
        } else {
            doc.add_layer(Some(format!("Styled {index}")))
        };
        if let Some(layer) = doc.get_layer_mut(id) {
            for (i, px) in layer
                .as_srgb8_mut()
                .expect("8-bit")
                .chunks_exact_mut(4)
                .enumerate()
            {
                let x = (i as u32) % width;
                let y = (i as u32) / width;
                // `small` puts a 10%-wide object in the centre; support is still inside the
                // canvas, so a content-bounds renderer should touch far fewer pixels.
                let inset_x = if small { width * 9 / 20 } else { width / 8 };
                let inset_y = if small { height * 9 / 20 } else { height / 8 };
                let inside = x > inset_x
                    && x < width - inset_x
                    && y > inset_y
                    && y < height - inset_y
                    && ((x + y) % 97) < 70;
                px[3] = if inside { 255 } else { 0 };
                px[0] = (x % 256) as u8;
                px[1] = (y % 256) as u8;
                px[2] = 90;
            }
            layer.styles = styles();
        }
    }
    doc
}

fn percentile(sorted: &[f64], fraction: f64) -> f64 {
    let index = ((sorted.len() - 1) as f64 * fraction).round() as usize;
    sorted[index]
}

struct Samples {
    label: String,
    style: Vec<f64>,
    composite: Vec<f64>,
    viewport: Vec<f64>,
    navigator: Vec<f64>,
}

impl Samples {
    fn summary(values: &[f64]) -> (f64, f64, f64) {
        let mut sorted = values.to_vec();
        sorted.sort_by(|a, b| a.total_cmp(b));
        (
            sorted[0],
            percentile(&sorted, 0.5),
            percentile(&sorted, 0.95),
        )
    }

    fn report(&self) {
        let (min, median, p95) = Self::summary(&self.composite);
        let (vmin, vmedian, vp95) = Self::summary(&self.viewport);
        let (smin, smedian, sp95) = Self::summary(&self.style);
        println!(
            "{label}: composite {median:.2} ms (min {min:.2}, p95 {p95:.2}) | viewport {vmedian:.2} ms (min {vmin:.2}, p95 {vp95:.2}) | style {smedian:.2} ms (min {smin:.2}, p95 {sp95:.2})",
            label = self.label
        );
    }

    fn json(&self) -> String {
        let mut out = String::new();
        let _ = write!(
            out,
            "{{\"label\":\"{}\",\"style\":{},\"composite\":{},\"viewport\":{},\"navigator\":{}}}",
            self.label,
            json_numbers(&self.style),
            json_numbers(&self.composite),
            json_numbers(&self.viewport),
            json_numbers(&self.navigator)
        );
        out
    }
}

fn json_numbers(values: &[f64]) -> String {
    let mut out = String::from("[");
    for (index, value) in values.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        let _ = write!(out, "{value:.4}");
    }
    out.push(']');
    out
}

fn measure(width: u32, height: u32, layers: usize, small: bool) -> Samples {
    let doc = styled_doc(width, height, layers, small);
    let mut renderer = SoftwareRenderer::new();
    let mut buffer = vec![0u32; (width * height) as usize];
    let viewport = [0, 0, width as usize, height as usize];
    let mut samples = Samples {
        label: format!(
            "{width}x{height}-{layers}layer{}",
            if small { "-small" } else { "" }
        ),
        style: Vec::new(),
        composite: Vec::new(),
        viewport: Vec::new(),
        navigator: Vec::new(),
    };

    for iteration in 0..WARMUP + ITERS {
        let start = std::time::Instant::now();
        let pixels = doc.composite_layers(false);
        std::hint::black_box(&pixels);
        let composite = start.elapsed().as_secs_f64() * 1000.0;

        let start = std::time::Instant::now();
        if let Some(layer) = doc.layers.first() {
            let styled = aurelia_core::layer_style::render_styled(layer);
            std::hint::black_box(&styled);
        }
        let style = start.elapsed().as_secs_f64() * 1000.0;

        let start = std::time::Instant::now();
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
        let viewport_ms = start.elapsed().as_secs_f64() * 1000.0;

        let start = std::time::Instant::now();
        if let Some(image) = renderer.navigator_thumbnail(&doc, 320) {
            std::hint::black_box(&image);
        }
        let navigator = start.elapsed().as_secs_f64() * 1000.0;

        if iteration >= WARMUP {
            samples.composite.push(composite);
            samples.style.push(style);
            samples.viewport.push(viewport_ms);
            samples.navigator.push(navigator);
        }
    }
    samples.report();
    samples
}

fn main() {
    println!(
        "Aurelia styled-layer cost: identical workload per layer count, warm-up {WARMUP}, measured {ITERS}"
    );
    let mut all = Vec::new();
    for (width, height) in [(1920, 1080), (3840, 2160)] {
        for layers in LAYER_COUNTS {
            all.push(measure(width, height, layers, false));
        }
        all.push(measure(width, height, 1, true));
    }

    let json = format!(
        "{{\"warmup\":{WARMUP},\"iters\":{ITERS},\"samples\":[{}]}}",
        all.iter().map(Samples::json).collect::<Vec<_>>().join(",")
    );
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../docs/benchmarks/style-baseline.json");
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    match std::fs::write(&path, json) {
        Ok(()) => println!("raw samples: {}", path.display()),
        Err(error) => eprintln!("could not write raw samples: {error}"),
    }
}
