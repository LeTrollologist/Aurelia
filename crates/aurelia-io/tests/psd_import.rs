use aurelia_io::load_psd_document;
use aurelia_io::psd::{parse_psd, PsdError};

fn be16(out: &mut Vec<u8>, value: u16) {
    out.extend_from_slice(&value.to_be_bytes());
}

fn be32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_be_bytes());
}

fn i32be(out: &mut Vec<u8>, value: i32) {
    out.extend_from_slice(&value.to_be_bytes());
}

fn name_block(out: &mut Vec<u8>, name: &str) {
    let len = name.len().min(255) as u8;
    out.push(len);
    out.extend_from_slice(&name.as_bytes()[..len as usize]);
    let total = 1 + len as usize;
    out.extend(std::iter::repeat_n(0, (4 - total % 4) % 4));
}

#[derive(Clone)]
struct Rect {
    top: i32,
    left: i32,
    bottom: i32,
    right: i32,
}

impl Rect {
    fn width(&self) -> u32 {
        (self.right - self.left) as u32
    }
    fn height(&self) -> u32 {
        (self.bottom - self.top) as u32
    }
}

struct Layer {
    name: String,
    rect: Rect,
    channels: Vec<(i16, Vec<u8>)>,
    opacity: u8,
    flags: u8,
    section: Option<u32>,
    mask: Option<(Rect, Vec<u8>)>,
    unsupported: Option<[u8; 4]>,
}

impl Layer {
    fn raster(name: &str, rect: Rect) -> Self {
        let pixels = vec![255u8; (rect.width() * rect.height()) as usize];
        Self {
            name: name.into(),
            rect,
            channels: vec![
                (0, vec![255; pixels.len()]),
                (1, vec![0; pixels.len()]),
                (2, vec![0; pixels.len()]),
                (-1, pixels),
            ],
            opacity: 255,
            flags: 0,
            section: None,
            mask: None,
            unsupported: None,
        }
    }

    fn group(name: &str, section: u32) -> Self {
        Self {
            name: name.into(),
            rect: Rect {
                top: 0,
                left: 0,
                bottom: 0,
                right: 0,
            },
            channels: Vec::new(),
            opacity: 255,
            flags: 0,
            section: Some(section),
            mask: None,
            unsupported: None,
        }
    }
}

fn write_extra(out: &mut Vec<u8>, layer: &Layer) {
    if let Some((rect, pixels)) = &layer.mask {
        be32(out, 18);
        i32be(out, rect.top);
        i32be(out, rect.left);
        i32be(out, rect.bottom);
        i32be(out, rect.right);
        out.push(0);
        out.push(0);
        let _ = pixels;
    } else {
        be32(out, 0);
    }
    be32(out, 0); // blending ranges
    name_block(out, &layer.name);
    if let Some(section) = layer.section {
        out.extend_from_slice(b"8BIM");
        out.extend_from_slice(b"lsct");
        be32(out, 4);
        be32(out, section);
    }
    if let Some(key) = layer.unsupported {
        out.extend_from_slice(b"8BIM");
        out.extend_from_slice(&key);
        be32(out, 0);
    }
}

fn build(width: u32, height: u32, layers: &[Layer], composite: Option<Vec<u8>>) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(b"8BPS");
    be16(&mut out, 1);
    out.extend_from_slice(&[0; 6]);
    be16(&mut out, 4);
    be32(&mut out, height);
    be32(&mut out, width);
    be16(&mut out, 8);
    be16(&mut out, 3);
    be32(&mut out, 0); // color mode data
    be32(&mut out, 0); // image resources

    let mut payload = Vec::new();
    be16(&mut payload, layers.len() as u16);
    for layer in layers {
        i32be(&mut payload, layer.rect.top);
        i32be(&mut payload, layer.rect.left);
        i32be(&mut payload, layer.rect.bottom);
        i32be(&mut payload, layer.rect.right);
        be16(&mut payload, layer.channels.len() as u16);
        for (id, data) in &layer.channels {
            be16(&mut payload, *id as u16);
            be32(&mut payload, (data.len() + 2) as u32);
        }
        payload.extend_from_slice(b"8BIM");
        payload.extend_from_slice(b"norm");
        payload.push(layer.opacity);
        payload.push(0);
        payload.push(layer.flags);
        payload.push(0);
        let mut extra = Vec::new();
        write_extra(&mut extra, layer);
        be32(&mut payload, extra.len() as u32);
        payload.extend_from_slice(&extra);
    }
    for layer in layers {
        for (_, data) in &layer.channels {
            be16(&mut payload, 0); // raw compression
            payload.extend_from_slice(data);
        }
    }

    let mut section = Vec::new();
    be32(&mut section, payload.len() as u32);
    section.extend_from_slice(&payload);
    be32(&mut section, 0); // global layer mask info
    be32(&mut out, section.len() as u32);
    out.extend_from_slice(&section);

    if let Some(composite) = composite {
        be16(&mut out, 0); // raw compression
        out.extend_from_slice(&composite);
    }
    out
}

fn rle_channel(pixels: &[u8], width: usize, height: usize) -> Vec<u8> {
    let mut out = Vec::new();
    be16(&mut out, 1); // RLE
    for row in 0..height {
        be16(&mut out, (width + 1) as u16);
        out.push((width - 1) as u8);
        out.extend_from_slice(&pixels[row * width..(row + 1) * width]);
    }
    out
}

#[test]
fn empty_psd_imports_the_composite() {
    let mut composite = Vec::new();
    for channel in 0..4 {
        for _ in 0..4 {
            composite.push(if channel == 3 { 255 } else { 40 });
        }
    }
    let data = build(2, 2, &[], Some(composite));
    let doc = parse_psd(&data).expect("parse").into_document();
    assert_eq!((doc.width, doc.height), (2, 2));
    assert_eq!(doc.layers.len(), 1);
    assert_eq!(doc.layers[0].name, "Composite");
}

#[test]
fn one_layer_and_multiple_layers_keep_names_and_pixels() {
    let rect = Rect {
        top: 0,
        left: 0,
        bottom: 2,
        right: 2,
    };
    let mut one = Layer::raster("Layer", rect.clone());
    one.opacity = 128;
    let doc = parse_psd(&build(2, 2, &[one], None))
        .expect("parse")
        .into_document();
    assert_eq!(doc.layers.len(), 1);
    assert_eq!(doc.layers[0].name, "Layer");
    assert_eq!(doc.layers[0].opacity, 128.0 / 255.0);
    assert_eq!(doc.layers[0].offset_x, 0);
}

#[test]
fn nested_groups_and_masks_are_preserved() {
    let rect = Rect {
        top: 0,
        left: 0,
        bottom: 2,
        right: 2,
    };
    let mut child = Layer::raster("Child", rect.clone());
    child.mask = Some((rect.clone(), vec![128; 4]));
    child.channels.push((-2, vec![128; 4]));
    let group = vec![
        Layer::group("Outer end", 2),
        Layer::group("Inner end", 2),
        child,
        Layer::group("Inner", 1),
        Layer::group("Outer", 1),
    ];
    let doc = parse_psd(&build(2, 2, &group, None))
        .expect("parse")
        .into_document();
    assert!(doc
        .layers
        .iter()
        .any(|layer| layer.name == "Outer" && layer.is_group()));
    assert!(doc
        .layers
        .iter()
        .any(|layer| layer.name == "Inner" && layer.is_group()));
    let child = doc
        .layers
        .iter()
        .find(|layer| layer.name == "Child")
        .expect("child");
    assert!(child.mask.is_some());
}

#[test]
fn text_and_adjustment_layers_are_rejected_explicitly() {
    let rect = Rect {
        top: 0,
        left: 0,
        bottom: 1,
        right: 1,
    };
    let mut text = Layer::raster("Text", rect.clone());
    text.unsupported = Some(*b"TySh");
    assert!(matches!(
        parse_psd(&build(1, 1, &[text], None)),
        Err(PsdError::Unsupported(_))
    ));

    let mut adjustment = Layer::raster("Adjustment", rect);
    adjustment.unsupported = Some(*b"SoCo");
    assert!(matches!(
        parse_psd(&build(1, 1, &[adjustment], None)),
        Err(PsdError::Unsupported(_))
    ));
}

#[test]
fn malformed_and_oversized_psd_are_rejected() {
    assert!(matches!(parse_psd(b"not a psd"), Err(PsdError::Corrupt(_))));
    let mut data = build(1, 1, &[], None);
    data.truncate(20);
    assert!(matches!(parse_psd(&data), Err(PsdError::Corrupt(_))));

    let mut header = build(1, 1, &[], None);
    header[14..18].copy_from_slice(&200_000u32.to_be_bytes());
    assert!(matches!(parse_psd(&header), Err(PsdError::TooLarge(_))));
}

#[test]
fn rle_layer_channels_are_decoded() {
    let rect = Rect {
        top: 0,
        left: 0,
        bottom: 2,
        right: 2,
    };
    let mut layer = Layer::raster("RLE", rect);
    for (_, data) in &mut layer.channels {
        *data = rle_channel(data, 2, 2);
    }
    let doc = parse_psd(&build(2, 2, &[layer], None))
        .expect("parse")
        .into_document();
    assert_eq!(doc.layers.len(), 1);
    assert_eq!(doc.layers[0].name, "RLE");
}

#[test]
fn real_psd_fixtures_parse() {
    for (name, bytes) in [
        (
            "1layer.psd",
            include_bytes!("fixtures/1layer.psd").as_slice(),
        ),
        (
            "2layers.psd",
            include_bytes!("fixtures/2layers.psd").as_slice(),
        ),
        (
            "0layers.psd",
            include_bytes!("fixtures/0layers.psd").as_slice(),
        ),
        (
            "empty-group.psd",
            include_bytes!("fixtures/empty-group.psd").as_slice(),
        ),
    ] {
        match parse_psd(bytes) {
            Ok(doc) => {
                let imported = doc.into_document();
                assert!(imported.width > 0 && imported.height > 0, "{name}");
            }
            Err(error) => panic!("{name}: {error}"),
        }
    }
}

#[test]
fn imported_document_can_be_loaded_through_the_public_path() {
    let rect = Rect {
        top: 0,
        left: 0,
        bottom: 1,
        right: 1,
    };
    let path = std::env::temp_dir().join(format!("aurelia-psd-import-{}.psd", std::process::id()));
    std::fs::write(&path, build(1, 1, &[Layer::raster("One", rect)], None)).expect("write");
    let doc = load_psd_document(&path).expect("load");
    assert_eq!(doc.layers.len(), 1);
    let _ = std::fs::remove_file(path);
}
