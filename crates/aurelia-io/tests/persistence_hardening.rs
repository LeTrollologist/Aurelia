//! Persistence hardening: truncated, mutated and over-claimed optional sections must never panic,
//! and a failed atomic save must leave the previous file untouched.

use aurelia_core::color::Color;
use aurelia_core::document::{Document, EditorState, GuideLine, StraightRulerState};
use aurelia_core::layer_style::{DropShadowStyle, LayerStyles, OuterGlowStyle, StrokeStyle};
use aurelia_io::project::{
    load_project_from_reader, save_project_file, save_project_to_writer,
};
use std::io::Cursor;

fn fixture_document() -> Document {
    let mut doc = Document::new(24, 16);
    let id = doc.add_layer(Some("Styled".into()));
    let layer = doc.get_layer_mut(id).expect("layer");
    layer.styles = LayerStyles {
        stroke: Some(StrokeStyle {
            width: 2.0,
            color: Color::BLACK,
            opacity: 0.8,
        }),
        drop_shadow: Some(DropShadowStyle {
            offset_x: 3.0,
            offset_y: 2.0,
            blur: 4.0,
            color: Color::BLACK,
            opacity: 0.5,
        }),
        outer_glow: Some(OuterGlowStyle {
            radius: 4.0,
            color: Color::WHITE,
            opacity: 0.4,
        }),
    };
    doc.editor = EditorState {
        guides: vec![
            GuideLine {
                vertical: true,
                position: 8.0,
            },
            GuideLine {
                vertical: false,
                position: 4.0,
            },
        ],
        grid_origin: [1.5, -2.5],
        straight_ruler: Some(StraightRulerState {
            origin: [2.0, 3.0],
            angle_rad: 0.5,
            length: 20.0,
        }),
    };
    doc
}

fn fixture_bytes() -> Vec<u8> {
    let mut bytes = Vec::new();
    save_project_to_writer(&fixture_document(), &mut bytes).expect("serialize");
    bytes
}

fn find(bytes: &[u8], needle: &[u8]) -> usize {
    bytes
        .windows(needle.len())
        .position(|window| window == needle)
        .unwrap_or_else(|| panic!("missing section {:?}", String::from_utf8_lossy(needle)))
}

fn write_u32(bytes: &mut [u8], at: usize, value: u32) {
    bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
}

fn load(bytes: &[u8]) -> Result<Document, aurelia_io::project::ProjectError> {
    load_project_from_reader(Cursor::new(bytes))
}

#[test]
fn every_truncation_is_handled_without_panicking() {
    let bytes = fixture_bytes();
    for cut in 0..=bytes.len() {
        let _ = load(&bytes[..cut]);
    }
    // From the editor section onward the document itself is complete, so truncating an optional
    // section must degrade to defaults rather than fail the load.
    let editor_start = find(&bytes, b"AED1");
    let styles_start = find(&bytes, b"ASTY");
    for cut in editor_start..=bytes.len() {
        let doc = load(&bytes[..cut]).unwrap_or_else(|error| {
            panic!("cut {cut} of {} failed: {error}", bytes.len())
        });
        assert!(doc.width == 24 && doc.height == 16);
        if cut >= styles_start {
            assert!(!doc.editor.guides.is_empty(), "editor survives a cut at {cut}");
        }
    }
}

#[test]
fn optional_section_length_lies_are_bounded() {
    let bytes = fixture_bytes();
    let editor_start = find(&bytes, b"AED1");
    let styles_start = find(&bytes, b"ASTY");

    for (offset, label) in [
        (editor_start + 4, "editor length"),
        (styles_start + 4, "style length"),
    ] {
        for length in [u32::MAX, 0, 1024 * 1024 + 1] {
            let mut mutated = bytes.clone();
            write_u32(&mut mutated, offset, length);
            let result = load(&mutated);
            assert!(
                result.is_ok(),
                "{label} = {length} must not fail the document load"
            );
        }
    }

    // A count that claims more entries than the payload can hold is rejected by the parser, not
    // by allocating for it.
    let mut styles = bytes.clone();
    write_u32(&mut styles, styles_start + 9, u32::MAX);
    assert!(load(&styles).is_ok());

    let mut editor = bytes.clone();
    write_u32(&mut editor, editor_start + 9, 4096);
    assert!(load(&editor).is_ok());
}

#[test]
fn bit_flips_in_optional_sections_never_panic() {
    let bytes = fixture_bytes();
    let editor_start = find(&bytes, b"AED1");
    for offset in editor_start..bytes.len() {
        let mut mutated = bytes.clone();
        mutated[offset] ^= 0xff;
        let _ = load(&mutated);
    }
}

#[test]
fn save_load_save_is_stable() {
    let first = fixture_bytes();
    let loaded = load(&first).expect("load");
    let mut second = Vec::new();
    save_project_to_writer(&loaded, &mut second).expect("re-save");
    assert_eq!(first, second, "save -> load -> save must be byte-stable");

    let reloaded = load(&second).expect("reload");
    assert_eq!(reloaded.editor, loaded.editor);
    assert_eq!(reloaded.layers[0].styles, loaded.layers[0].styles);
}

#[test]
fn a_failed_atomic_save_keeps_the_previous_file() {
    let dir = std::env::temp_dir().join(format!("aurelia-atomic-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp dir");
    let path = dir.join("atomic.aur");
    save_project_file(&fixture_document(), &path).expect("initial save");
    let original = std::fs::read(&path).expect("read original");

    // Make the sibling temp path impossible to create; a failed save must not touch `path`.
    let tmp = dir.join("atomic.aur.tmp");
    std::fs::create_dir(&tmp).expect("blocking temp dir");
    assert!(save_project_file(&fixture_document(), &path).is_err());
    assert_eq!(std::fs::read(&path).expect("read original"), original);

    let _ = std::fs::remove_dir_all(&dir);
}
