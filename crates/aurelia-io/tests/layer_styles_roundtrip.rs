//! AUR v3 round-trip for per-layer styles (v3.8.0).
//!
//! Styles live in an optional trailing section rather than the layer record, so the checks are:
//! present styles survive exactly, absent styles load as empty, and a damaged optional section
//! is ignored instead of failing the file.

use aurelia_core::color::Color;
use aurelia_core::document::Document;
use aurelia_core::layer_style::{DropShadowStyle, OuterGlowStyle, StrokeStyle};
use aurelia_io::project::{load_project_file, save_project_file};
use std::path::PathBuf;

fn temp_path(tag: &str) -> PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join(format!("aurelia-styles-{}-{}.aur", tag, std::process::id()))
}

#[test]
fn styles_round_trip_and_absent_styles_stay_empty() {
    let mut doc = Document::new(32, 24);
    let id = doc.active_layer_id;
    let layer = doc.get_layer_mut(id).expect("layer");
    layer.styles.stroke = Some(StrokeStyle {
        width: 4.5,
        color: Color::from_hex("#3366cc").expect("colour"),
        opacity: 0.75,
    });
    layer.styles.drop_shadow = Some(DropShadowStyle {
        offset_x: 7.0,
        offset_y: -3.5,
        blur: 9.0,
        color: Color::from_hex("#102030").expect("colour"),
        opacity: 0.4,
    });
    layer.styles.outer_glow = Some(OuterGlowStyle {
        radius: 12.0,
        color: Color::from_hex("#ffcc00").expect("colour"),
        opacity: 0.9,
    });

    let path = temp_path("styled");
    save_project_file(&doc, &path).expect("save");
    let loaded = load_project_file(&path).expect("load");
    let _ = std::fs::remove_file(&path);

    let restored = loaded.get_layer(id).expect("layer survives");
    assert_eq!(restored.styles, doc.get_layer(id).expect("source").styles);

    // A document without styles does not grow a section and reloads with defaults.
    let plain = Document::new(16, 16);
    let path = temp_path("plain");
    save_project_file(&plain, &path).expect("save plain");
    let loaded = load_project_file(&path).expect("load plain");
    let _ = std::fs::remove_file(&path);
    assert!(loaded.layers.iter().all(|layer| layer.styles.is_empty()));
}

#[test]
fn damaged_optional_section_is_ignored_not_fatal() {
    let mut doc = Document::new(16, 16);
    let id = doc.active_layer_id;
    doc.get_layer_mut(id).expect("layer").styles.stroke = Some(StrokeStyle {
        width: 2.0,
        color: Color::BLACK,
        opacity: 1.0,
    });
    let path = temp_path("damaged");
    save_project_file(&doc, &path).expect("save");
    let mut bytes = std::fs::read(&path).expect("read");
    // Cut into the trailing style section: the file must still load, with the optional
    // section simply absent.
    bytes.truncate(bytes.len().saturating_sub(6));
    std::fs::write(&path, &bytes).expect("write truncated");
    let loaded = load_project_file(&path).expect("truncated optional section must not fail");
    let _ = std::fs::remove_file(&path);
    assert!(loaded.get_layer(id).expect("layer").styles.is_empty());
}

#[test]
fn inner_effects_and_bevel_round_trip() {
    use aurelia_core::layer_style::{BevelStyle, InnerGlowStyle, InnerShadowStyle};

    let mut doc = Document::new(24, 24);
    let id = doc.active_layer_id;
    let layer = doc.get_layer_mut(id).expect("layer");
    layer.styles.inner_shadow = Some(InnerShadowStyle {
        offset_x: 3.0,
        offset_y: -2.0,
        blur: 5.0,
        color: Color::from_hex("#203040").expect("colour"),
        opacity: 0.6,
    });
    layer.styles.inner_glow = Some(InnerGlowStyle {
        radius: 4.0,
        color: Color::from_hex("#ffeeaa").expect("colour"),
        opacity: 0.35,
    });
    layer.styles.bevel = Some(BevelStyle {
        size: 6.5,
        angle_deg: 225.0,
        depth: 0.7,
        highlight: Color::WHITE,
        shadow: Color::BLACK,
        opacity: 0.55,
    });

    let path = temp_path("inner");
    save_project_file(&doc, &path).expect("save");
    let loaded = load_project_file(&path).expect("load");
    let _ = std::fs::remove_file(&path);
    assert_eq!(
        loaded.get_layer(id).expect("layer").styles,
        doc.get_layer(id).expect("source").styles
    );
}
