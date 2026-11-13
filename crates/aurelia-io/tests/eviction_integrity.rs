//! Undo-history eviction must not damage a document that is subsequently saved (#51).
//!
//! The core tests prove undo/redo stay byte-exact after eviction; this proves the surviving
//! document state is still a valid AUR v3 file that reloads identically, including 16-bit
//! storage and a painted layer mask.

use aurelia_core::document::Document;
use aurelia_core::history::{HistoryStack, LayerPixelsDeltaCommand, MaskPixelsCommand};
use aurelia_core::layer::LayerBuffer;
use aurelia_io::project::{load_project_file, save_project_file};
use std::path::PathBuf;

fn temp_path(tag: &str) -> PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join(format!(
            "aurelia-eviction-{}-{}.aur",
            tag,
            std::process::id()
        ))
}

fn sixteen_bit_document() -> (Document, u64) {
    let mut doc = Document::new(96, 64);
    let id = doc.active_layer_id;
    if let Some(layer) = doc.get_layer_mut(id) {
        let mut pixels = vec![0u16; 96 * 64 * 4];
        for (index, px) in pixels.chunks_exact_mut(4).enumerate() {
            px[0] = ((index * 811) % 65535) as u16;
            px[1] = ((index * 277) % 65535) as u16;
            px[2] = ((index * 131) % 65535) as u16;
            px[3] = 65535;
        }
        *layer.buffer_mut() = LayerBuffer::Srgb16(pixels);
    }
    (doc, id)
}

#[test]
fn save_and_reload_survive_history_eviction() {
    let (mut doc, id) = sixteen_bit_document();
    doc.add_layer_mask(id, None);
    // A tiny budget so the fixture actually evicts while being saved afterwards.
    let mut history = HistoryStack::with_budget(32, 4000);

    for step in 0..25u16 {
        let before = doc.get_layer(id).expect("layer").storage_bytes();
        if let Some(channels) = doc.get_layer_mut(id).and_then(|l| l.as_srgb16_mut()) {
            channels[(step as usize * 37) % channels.len()] ^= 0x00FF;
        }
        let after = doc.get_layer(id).expect("layer").storage_bytes();
        history.push(Box::new(LayerPixelsDeltaCommand::new(
            id, "paint", before, after,
        )));

        let before_mask = doc
            .get_layer(id)
            .and_then(|l| l.mask.as_ref())
            .map(|m| m.pixels.clone())
            .expect("mask");
        let enabled_before = doc
            .get_layer(id)
            .and_then(|l| l.mask.as_ref())
            .map(|m| m.enabled)
            .expect("mask");
        if let Some(mask) = doc.get_layer_mut(id).and_then(|l| l.mask.as_mut()) {
            let at = (step as usize * 53) % mask.pixels.len();
            mask.pixels[at] = 255u8.saturating_sub(step as u8);
        }
        let after_mask = doc
            .get_layer(id)
            .and_then(|l| l.mask.as_ref())
            .map(|m| m.pixels.clone())
            .expect("mask");
        history.push(Box::new(MaskPixelsCommand {
            layer_id: id,
            description: "mask",
            before_pixels: before_mask,
            after_pixels: after_mask,
            before_enabled: enabled_before,
            after_enabled: enabled_before,
        }));
    }

    assert!(history.len() < 50, "the fixture must have evicted steps");
    while history.can_undo() {
        history.undo(&mut doc);
    }
    while history.can_redo() {
        history.redo(&mut doc);
    }

    let expected_pixels = doc.get_layer(id).expect("layer").storage_bytes();
    let expected_mask = doc
        .get_layer(id)
        .and_then(|l| l.mask.as_ref())
        .map(|m| m.pixels.clone())
        .expect("mask");

    let path = temp_path("roundtrip");
    save_project_file(&doc, &path).expect("save after eviction");
    let loaded = load_project_file(&path).expect("reload after eviction");
    let _ = std::fs::remove_file(&path);

    let layer = loaded.get_layer(id).expect("layer survives the round trip");
    assert_eq!(layer.format(), aurelia_core::layer::PixelFormat::Srgb16);
    assert_eq!(
        layer.storage_bytes(),
        expected_pixels,
        "16-bit bytes must round-trip exactly"
    );
    let mask = layer.mask.as_ref().expect("mask survives the round trip");
    assert_eq!(
        mask.pixels, expected_mask,
        "mask bytes must round-trip exactly"
    );
}
