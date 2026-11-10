//! Long-session soak coverage for the 16-bit editing pipeline (#50).
//!
//! Run explicitly, because it is a measurement rather than a unit test:
//!
//! ```powershell
//! cargo test -p aurelia-io --test soak --release -- --ignored --nocapture
//! ```
//!
//! It cycles the operations that retain memory in ordinary use - painting, a Gaussian filter,
//! a transform patch, undo/redo under a bounded budget, save/load and animation-frame work -
//! and records live heap at the start of the steady state and at the end. The assertion is
//! bounded retention, not "no growth at all": a session that keeps a fixed history budget is
//! allowed to sit at a plateau, but it must not climb cycle after cycle.
//!
//! The soak record printed at the end is the artifact to paste into `docs/PERFORMANCE.md`.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

use aurelia_core::animation::AnimationTimeline;
use aurelia_core::brush::{BrushPoint, BrushSettings};
use aurelia_core::document::Document;
use aurelia_core::filter::filter_gaussian_blur;
use aurelia_core::history::{HistoryStack, LayerPixelsDeltaCommand};
use aurelia_core::layer::{LayerBuffer, PixelFormat};
use aurelia_core::rasterizer::StrokeRasterizer;
use aurelia_core::symmetry::SymmetryConfig;
use aurelia_core::transform::{AffineTransform2D, InterpolationMode, RenderMode, TransformPatch};
use glam::Vec2;

struct CountingAllocator;

static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

fn record(size: usize) {
    let live = LIVE.fetch_add(size, Ordering::Relaxed) + size;
    PEAK.fetch_max(live, Ordering::Relaxed);
}

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let ptr = System.alloc(layout);
        if !ptr.is_null() {
            record(layout.size());
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        System.dealloc(ptr, layout);
        LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let new_ptr = System.realloc(ptr, layout, new_size);
        if !new_ptr.is_null() {
            if new_size >= layout.size() {
                record(new_size - layout.size());
            } else {
                LIVE.fetch_sub(layout.size() - new_size, Ordering::Relaxed);
            }
        }
        new_ptr
    }
}

#[global_allocator]
static GLOBAL: CountingAllocator = CountingAllocator;

const SIDE: u32 = 192;
const CYCLES: usize = 120;
const WARMUP: usize = 20;

fn sixteen_bit_document() -> (Document, u64) {
    let mut doc = Document::new(SIDE, SIDE);
    let id = doc.active_layer_id;
    if let Some(layer) = doc.get_layer_mut(id) {
        let mut pixels = vec![0u16; (SIDE * SIDE * 4) as usize];
        for (index, px) in pixels.chunks_exact_mut(4).enumerate() {
            px[0] = ((index * 811) % 65535) as u16;
            px[1] = ((index * 277) % 65535) as u16;
            px[2] = ((index * 131) % 65535) as u16;
            px[3] = 65535;
        }
        *layer.buffer_mut() = LayerBuffer::Srgb16(pixels);
    }
    doc.add_layer_mask(id, None);
    (doc, id)
}

fn temp_path() -> std::path::PathBuf {
    std::env::temp_dir().join(format!("aurelia-soak-{}.aur", std::process::id()))
}

#[test]
#[ignore = "long-session measurement; run explicitly with --ignored --nocapture"]
fn sixteen_bit_editing_session_stays_bounded() {
    let (mut doc, id) = sixteen_bit_document();
    let mut history = HistoryStack::with_budget(64, 8 * 1024 * 1024);
    let brush = BrushSettings::default();
    let symmetry = SymmetryConfig::default();
    let mut timeline = AnimationTimeline::new(SIDE, SIDE);
    let path = temp_path();

    let mut plateau: Option<usize> = None;
    let mut peak = 0usize;
    let mut undo_redo_rounds = 0usize;

    for cycle in 0..CYCLES {
        // Paint a stroke.
        let before = doc.get_layer(id).expect("layer").storage_bytes();
        let t = cycle as f32 / CYCLES as f32;
        let from = Vec2::new(20.0 + t * 120.0, 30.0);
        let to = Vec2::new(from.x + 18.0, 150.0);
        StrokeRasterizer::paint_dot(
            &mut doc,
            BrushPoint::new(from, 0.8),
            &brush,
            &symmetry,
            None,
        );
        StrokeRasterizer::paint_segment(
            &mut doc,
            BrushPoint::new(from, 0.8),
            BrushPoint::new(to, 0.9),
            &brush,
            &symmetry,
            None,
        );
        let after = doc.get_layer(id).expect("layer").storage_bytes();
        history.push(Box::new(LayerPixelsDeltaCommand::new(
            id,
            "soak stroke",
            before,
            after,
        )));

        // Filter the active layer natively.
        let before_filter = doc.get_layer(id).expect("layer").storage_bytes();
        if let Some(channels) = doc.get_layer_mut(id).and_then(|l| l.as_srgb16_mut()) {
            filter_gaussian_blur::<u16>(channels, SIDE, SIDE, 1.2, None);
        }
        let after_filter = doc.get_layer(id).expect("layer").storage_bytes();
        history.push(Box::new(LayerPixelsDeltaCommand::new(
            id,
            "soak blur",
            before_filter,
            after_filter,
        )));

        // Transform a patch and put it back (the free-transform retention shape).
        if let Some(layer) = doc.get_layer(id) {
            let patch = TransformPatch::extract_from_layer(layer, 0, 0, 48, 48, &|_, _| true);
            let transform = AffineTransform2D {
                translation: Vec2::new(2.0, 1.0),
                ..AffineTransform2D::default()
            };
            if let Some(layer) = doc.get_layer_mut(id) {
                patch.render_into(
                    layer,
                    48,
                    48,
                    Vec2::new(0.0, 0.0),
                    InterpolationMode::Bilinear,
                    RenderMode::Affine(&transform),
                );
            }
        }

        // Undo/redo a few steps, crossing the retained boundary as the budget fills.
        for _ in 0..3 {
            if history.undo(&mut doc).is_some() {
                undo_redo_rounds += 1;
            }
        }
        for _ in 0..3 {
            if history.redo(&mut doc).is_some() {
                undo_redo_rounds += 1;
            }
        }

        // Animation frame work.
        if cycle % 10 == 0 {
            let index = timeline.add_frame(SIDE, SIDE);
            timeline.current_frame_idx = index;
            timeline.sync_from_document(&doc);
            if let Some(frame) = timeline.current_frame() {
                let _ = frame.composite_layers(SIDE, SIDE, false, 0);
            }
            if timeline.frames.len() > 3 {
                timeline.current_frame_idx = 1;
                let _ = timeline.delete_current_frame();
            }
        }

        // Save/load every 15 cycles; the loaded document is dropped immediately.
        if cycle % 15 == 0 {
            aurelia_io::project::save_project_file(&doc, &path).expect("soak save");
            let loaded = aurelia_io::project::load_project_file(&path).expect("soak load");
            assert_eq!(
                loaded.get_layer(id).expect("layer").storage_bytes(),
                doc.get_layer(id).expect("layer").storage_bytes(),
                "cycle {cycle}: save/load changed the 16-bit bytes"
            );
        }

        let live = LIVE.load(Ordering::Relaxed);
        peak = peak.max(live);
        if cycle + 1 == WARMUP {
            plateau = Some(live);
        }

        assert!(history.len() <= 64, "history exceeded its step budget");
        assert!(
            history.retained_bytes() <= history.max_bytes(),
            "history exceeded its byte budget"
        );
    }
    let _ = std::fs::remove_file(&path);

    let end_live = LIVE.load(Ordering::Relaxed);
    let start_live = plateau.expect("warmup recorded");
    let growth = end_live.saturating_sub(start_live);
    let budget = start_live / 4 + 2 * 1024 * 1024;

    println!(
        "soak_record {{\"doc\":\"{}x{} Srgb16 + mask\",\"cycles\":{},\"undo_redo\":{},\"history_steps\":{},\"history_bytes\":{},\"history_budget\":{},\"live_start\":{},\"live_end\":{},\"peak\":{},\"growth\":{},\"growth_budget\":{}}}",
        SIDE, SIDE, CYCLES, undo_redo_rounds, history.len(), history.retained_bytes(),
        history.max_bytes(), start_live, end_live, peak, growth, budget
    );

    assert!(
        growth <= budget,
        "live heap grew by {growth} bytes after warmup (budget {budget}): \
         retained history is {}/{}, document is 16-bit",
        history.retained_bytes(),
        history.max_bytes()
    );
    assert_eq!(
        doc.active_layer().expect("layer").format(),
        PixelFormat::Srgb16,
        "the soak must stay on a 16-bit layer"
    );
}
