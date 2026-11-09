//! 16-bit PNG export: the file boundary must preserve what the 16-bit compositor produced.

use aurelia_core::document::Document;
use aurelia_core::layer::PixelFormat;
use aurelia_io::export::{export_flat_image, export_flat_image_with_limits, ExportError, ExportFormat, ExportLimits};
use std::path::PathBuf;

fn temp_path(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "aurelia-png16-{}-{name}",
        std::process::id()
    ))
}

fn sixteen_bit_doc() -> Document {
    let mut doc = Document::new_with_format(4, 4, PixelFormat::Srgb16);
    doc.is_transparent = true;
    let id = doc.active_layer_id;
    {
        let pixels = doc
            .get_layer_mut(id)
            .expect("layer")
            .as_srgb16_mut()
            .expect("16-bit");
        let at = (4 + 1) * 4;
        pixels[at] = 301;
        pixels[at + 1] = 1001;
        pixels[at + 2] = 40001;
        pixels[at + 3] = 65535;
    }
    doc
}

#[test]
fn sixteen_bit_png_keeps_sub_lattice_values() {
    let path = temp_path("wide.png");
    export_flat_image(&sixteen_bit_doc(), &path, ExportFormat::Png, false).expect("export");
    let opened = image::open(&path).expect("decode");
    assert!(matches!(opened, image::DynamicImage::ImageRgba16(_)));
    let decoded = opened.to_rgba16();
    assert_eq!(decoded.dimensions(), (4, 4));
    let pixel = decoded.get_pixel(1, 1).0;
    assert_eq!(pixel, [301, 1001, 40001, 65535]);
    assert!(!pixel[0].is_multiple_of(257), "sub-lattice value survived");
    let _ = std::fs::remove_file(&path);
}

#[test]
fn eight_bit_png_is_unchanged() {
    let mut doc = Document::new(4, 4);
    doc.is_transparent = true;
    let id = doc.active_layer_id;
    {
        let pixels = doc.get_layer_mut(id).expect("layer").as_srgb8_mut().expect("8-bit");
        let at = (2 * 4 + 2) * 4;
        pixels[at] = 17;
        pixels[at + 1] = 34;
        pixels[at + 2] = 51;
        pixels[at + 3] = 255;
    }
    let path = temp_path("narrow.png");
    export_flat_image(&doc, &path, ExportFormat::Png, false).expect("export");
    let opened = image::open(&path).expect("decode");
    assert!(matches!(opened, image::DynamicImage::ImageRgba8(_)));
    let decoded = opened.to_rgba8();
    assert_eq!(decoded.get_pixel(2, 2).0, [17, 34, 51, 255]);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn a_mixed_document_exports_wide_png() {
    let mut doc = Document::new(4, 4);
    doc.is_transparent = true;
    let id = doc.add_layer(Some("Wide".into()));
    doc.get_layer_mut(id)
        .expect("layer")
        .convert_format(PixelFormat::Srgb16);
    {
        let pixels = doc.get_layer_mut(id).expect("layer").as_srgb16_mut().expect("16-bit");
        let at = (4 + 1) * 4;
        pixels[at] = 700;
        pixels[at + 3] = 65535;
    }
    let path = temp_path("mixed.png");
    export_flat_image(&doc, &path, ExportFormat::Png, false).expect("export");
    let opened = image::open(&path).expect("decode");
    assert!(matches!(opened, image::DynamicImage::ImageRgba16(_)));
    let decoded = opened.to_rgba16();
    assert_eq!(decoded.get_pixel(1, 1).0[0], 700);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn jpeg_stays_8bit_and_wide_png_respects_the_budget() {
    let path = temp_path("narrow.jpg");
    export_flat_image(&sixteen_bit_doc(), &path, ExportFormat::Jpeg, false).expect("jpeg");
    assert_eq!(image::open(&path).expect("decode").to_rgba8().dimensions(), (4, 4));
    let _ = std::fs::remove_file(&path);

    let limits = ExportLimits {
        max_dimension: 64,
        max_frames: 1,
        max_total_bytes: 64, // 4x4 RGBA16 is 128 bytes
    };
    let error = export_flat_image_with_limits(
        &sixteen_bit_doc(),
        temp_path("budget.png"),
        ExportFormat::Png,
        false,
        None,
        &limits,
    )
    .expect_err("budget must reject");
    assert!(matches!(error, ExportError::AggregateTooLarge { .. }));
}

#[test]
fn png_export_embeds_the_document_profile() {
    use image::ImageDecoder;

    let icc = moxcms::ColorProfile::new_srgb().encode().expect("encode sRGB");
    let mut doc = sixteen_bit_doc();
    doc.set_icc_profile(icc.clone()).expect("attach");
    let path = temp_path("profiled.png");
    export_flat_image(&doc, &path, ExportFormat::Png, false).expect("export");

    let file = std::fs::File::open(&path).expect("open png");
    let mut decoder =
        image::codecs::png::PngDecoder::new(std::io::BufReader::new(file)).expect("decoder");
    let embedded = decoder.icc_profile().expect("read icc").expect("icc present");
    assert_eq!(embedded, icc);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn jpeg_export_embeds_the_document_profile() {
    use image::ImageDecoder;

    let icc = moxcms::ColorProfile::new_srgb().encode().expect("encode sRGB");
    let mut doc = Document::new(8, 8);
    doc.set_icc_profile(icc.clone()).expect("attach");
    let path = temp_path("profiled.jpg");
    export_flat_image(&doc, &path, ExportFormat::Jpeg, false).expect("export");

    let file = std::fs::File::open(&path).expect("open jpeg");
    let mut decoder =
        image::codecs::jpeg::JpegDecoder::new(std::io::BufReader::new(file)).expect("decoder");
    let embedded = decoder.icc_profile().expect("read icc").expect("icc present");
    assert_eq!(embedded, icc);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn webp_embeds_the_profile_in_an_extended_container() {
    let icc = moxcms::ColorProfile::new_srgb().encode().expect("encode sRGB");
    let mut doc = Document::new(8, 8);
    doc.set_icc_profile(icc.clone()).expect("attach");
    let path = temp_path("profiled.webp");
    export_flat_image(&doc, &path, ExportFormat::WebP, false).expect("export");
    let data = std::fs::read(&path).expect("read webp");
    assert_eq!(&data[..4], b"RIFF");
    assert_eq!(&data[8..12], b"WEBP");
    assert_eq!(
        u32::from_le_bytes(data[4..8].try_into().expect("size")) as usize,
        data.len() - 8,
        "RIFF size must cover the inserted chunks"
    );

    let mut at = 12usize;
    let mut saw_vp8x = false;
    let mut iccp = None;
    while at + 8 <= data.len() {
        let size = u32::from_le_bytes(data[at + 4..at + 8].try_into().expect("chunk size")) as usize;
        match &data[at..at + 4] {
            b"VP8X" => {
                saw_vp8x = true;
                assert_eq!(data[at + 8] & 0x20, 0x20, "ICC flag set");
            }
            b"ICCP" => iccp = Some(data[at + 8..at + 8 + size].to_vec()),
            _ => {}
        }
        at += 8 + size + (size & 1);
    }
    assert!(saw_vp8x, "ICCP requires a VP8X header");
    assert_eq!(iccp.as_deref(), Some(icc.as_slice()));
    let decoded = image::open(&path).expect("extended WebP still decodes");
    assert_eq!((decoded.width(), decoded.height()), (8, 8));
    let _ = std::fs::remove_file(&path);
}
