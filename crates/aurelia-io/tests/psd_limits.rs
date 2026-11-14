//! PSD decode-budget and channel-selection tests.
//!
//! `Reader::decoded` only counts bytes consumed from the file. PackBits RLE expands up to 64x,
//! so the importer must charge the *decoded* size as well or a small file can request an
//! unbounded amount of memory by declaring many channels.

use aurelia_io::psd::{parse_psd, PsdError, MAX_PSD_DECODED_BYTES};

fn be16(out: &mut Vec<u8>, value: u16) {
    out.extend_from_slice(&value.to_be_bytes());
}

fn be32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_be_bytes());
}

/// Composite-image PSD (no layer records) whose rows are PackBits repeat runs.
fn composite_psd(width: u32, height: u32, channels: u16, value: u8) -> Vec<u8> {
    assert_eq!(width % 128, 0, "fixture helper uses 128-pixel runs");
    let mut out = Vec::new();
    out.extend_from_slice(b"8BPS");
    be16(&mut out, 1); // version 1
    out.extend_from_slice(&[0u8; 6]); // reserved
    be16(&mut out, channels);
    be32(&mut out, height);
    be32(&mut out, width);
    be16(&mut out, 8); // depth
    be16(&mut out, 3); // RGB
    be32(&mut out, 0); // colour mode data
    be32(&mut out, 0); // image resources
    be32(&mut out, 0); // layer/mask info -> composite fallback

    be16(&mut out, 1); // RLE

    let runs_per_row = (width / 128) as usize;
    let row_bytes = runs_per_row * 2;
    for _ in 0..channels {
        for _ in 0..height {
            be16(&mut out, row_bytes as u16);
        }
    }
    for _ in 0..channels {
        for row in 0..height {
            for _ in 0..runs_per_row {
                out.push(0x81); // -127: repeat the next byte 128 times
                out.push(value.wrapping_add(row as u8));
            }
        }
    }
    out
}

#[test]
fn rle_expansion_is_charged_to_the_decoded_budget() {
    // 16384 x 12288 = 192 Mi pixels. Four decoded channels are 768 MiB, and the RGBA buffer is
    // another 768 MiB, so the document cannot fit the 1 GiB decoded budget. Before the budget
    // existed this small file forced the channel allocations regardless.
    let width = 16384;
    let height = 12288;
    let psd = composite_psd(width, height, 4, 0x40);
    assert!(
        (psd.len() as u64) < 16 * 1024 * 1024,
        "fixture should stay small: {} bytes",
        psd.len()
    );

    match parse_psd(&psd) {
        Err(PsdError::TooLarge(message)) => {
            assert!(
                message.contains("decoded"),
                "budget error should name the decoded budget: {message}"
            );
        }
        Err(other) => panic!("expected TooLarge, got {other:?}"),
        Ok(document) => panic!(
            "a 192 Mi-pixel, 4-channel composite must not fit {} MiB; parsed {} layer(s)",
            MAX_PSD_DECODED_BYTES / (1024 * 1024),
            document.into_document().layers.len()
        ),
    }
}

#[test]
fn unused_composite_channels_are_consumed_without_being_decoded() {
    // 56 channels at 4096 x 4096 is 3.7 GiB if every channel is decompressed and retained, but
    // only the first four are merged into RGBA. This must parse, and stay small.
    let psd = composite_psd(4096, 4096, 56, 0x11);
    let document = parse_psd(&psd).expect("extra channels are skipped, not fatal");
    assert_eq!(document.width, 4096);
    assert_eq!(document.height, 4096);
}

#[test]
fn small_rle_composite_still_decodes_pixels() {
    let width = 256;
    let height = 4;
    let psd = composite_psd(width, height, 3, 0x7F);

    let document = parse_psd(&psd)
        .expect("small RLE composite parses")
        .into_document();
    let layer = document
        .layers
        .first()
        .expect("composite fallback produces one layer");
    assert_eq!(layer.width, width);
    assert_eq!(layer.height, height);
    // Row 0 of every channel is 0x7F; alpha defaults to opaque.
    let pixels = layer.as_srgb8_exact();
    assert_eq!(pixels.len(), (width * height * 4) as usize);
    assert_eq!(&pixels[..4], &[0x7F, 0x7F, 0x7F, 0xFF]);
    // Row 1 carries 0x80 (0x7F + 1).
    let row = (width * 4) as usize;
    assert_eq!(&pixels[row..row + 4], &[0x80, 0x80, 0x80, 0xFF]);
}
