//! ICC profile persistence: the optional AICC section must round-trip exact bytes and degrade to
//! untagged when absent or damaged.

use aurelia_core::document::Document;
use aurelia_io::project::{load_project_from_reader, save_project_to_writer};
use std::io::Cursor;

fn srgb_icc() -> Vec<u8> {
    moxcms::ColorProfile::new_srgb().encode().expect("encode sRGB")
}

fn round_trip(doc: &Document) -> Document {
    let mut bytes = Vec::new();
    save_project_to_writer(doc, &mut bytes).expect("save");
    load_project_from_reader(Cursor::new(bytes)).expect("load")
}

#[test]
fn attached_profile_round_trips_exactly() {
    let icc = srgb_icc();
    let mut doc = Document::new(4, 4);
    doc.set_icc_profile(icc.clone()).expect("attach");
    let loaded = round_trip(&doc);
    assert_eq!(loaded.icc_profile(), Some(icc.as_slice()));
}

#[test]
fn absent_or_damaged_profiles_load_as_untagged() {
    let untagged = Document::new(4, 4);
    assert!(round_trip(&untagged).icc_profile().is_none());

    let icc = srgb_icc();
    let mut doc = Document::new(4, 4);
    doc.set_icc_profile(icc).expect("attach");
    let mut bytes = Vec::new();
    save_project_to_writer(&doc, &mut bytes).expect("save");
    let marker = bytes
        .windows(4)
        .position(|window| window == b"AICC")
        .expect("AICC present");
    // Cut inside the payload: the reader must ignore the section, not fail the document.
    bytes.truncate(marker + 8);
    let loaded = load_project_from_reader(Cursor::new(bytes)).expect("load");
    assert!(loaded.icc_profile().is_none());
}

#[test]
fn a_profile_the_reader_cannot_parse_is_ignored() {
    let mut doc = Document::new(4, 4);
    // Bypass the setter deliberately: this is the "damaged optional section" shape.
    doc.icc_profile = Some(vec![0u8; 200]);
    let loaded = round_trip(&doc);
    assert!(loaded.icc_profile().is_none());
}
