//! ICC input: every format Aurelia embeds a profile into must also return it from the decoder.

use aurelia_core::document::Document;
use aurelia_io::export::{export_flat_image, ExportFormat};
use image::ImageDecoder;
use std::path::PathBuf;

fn temp_path(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("aurelia-icc-input-{}-{name}", std::process::id()))
}

fn decoded_profile(path: &std::path::Path) -> Option<Vec<u8>> {
    let reader = image::ImageReader::open(path)
        .expect("open")
        .with_guessed_format()
        .expect("guess");
    let mut decoder = reader.into_decoder().expect("decoder");
    decoder.icc_profile().expect("icc read")
}

#[test]
fn png_jpeg_and_webp_profiles_are_readable_on_import() {
    let icc = moxcms::ColorProfile::new_srgb().encode().expect("encode sRGB");
    let mut doc = Document::new(8, 8);
    doc.set_icc_profile(icc.clone()).expect("attach");

    for (format, extension) in [
        (ExportFormat::Png, "png"),
        (ExportFormat::Jpeg, "jpg"),
        (ExportFormat::WebP, "webp"),
    ] {
        let path = temp_path(&format!("profile.{extension}"));
        export_flat_image(&doc, &path, format, false).expect("export");
        assert_eq!(
            decoded_profile(&path).as_deref(),
            Some(icc.as_slice()),
            "{extension} must return the embedded profile"
        );
        let _ = std::fs::remove_file(path);
    }
}

#[test]
fn an_untagged_image_returns_no_profile() {
    let doc = Document::new(4, 4);
    let path = temp_path("untagged.png");
    export_flat_image(&doc, &path, ExportFormat::Png, false).expect("export");
    assert_eq!(decoded_profile(&path), None);
    let _ = std::fs::remove_file(path);
}
