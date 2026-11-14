//! Read-only Photoshop PSD import (safe subset).
//!
//! This is a deliberately narrow parser, not a full Photoshop compatibility layer. It accepts
//! 8-bit RGB PSD/PSB-version-1 files with bounded layer trees, raster layers, groups, layer masks,
//! opacity, visibility and common blend modes. Unsupported features are rejected with an explicit
//! error rather than producing a silently flattened or partially imported document.

use std::io::Read;
use std::path::Path;

use aurelia_core::blend::BlendMode;
use aurelia_core::document::Document;
use aurelia_core::layer::{Layer, LayerId, LayerMask, PixelFormat};
use thiserror::Error;

pub const PSD_SIGNATURE: [u8; 4] = *b"8BPS";
pub const MAX_PSD_SIDE: u32 = 32_768;
pub const MAX_PSD_PIXELS: u64 = 512 * 1024 * 1024;
pub const MAX_PSD_LAYERS: u32 = 4096;
pub const MAX_PSD_DECODED_BYTES: u64 = 1024 * 1024 * 1024;
pub const MAX_PSD_FILE_BYTES: u64 = 512 * 1024 * 1024;

#[derive(Debug, Error)]
pub enum PsdError {
    #[error("PSD I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("PSD is too large: {0}")]
    TooLarge(String),
    #[error("unsupported PSD feature: {0}")]
    Unsupported(String),
    #[error("corrupt PSD: {0}")]
    Corrupt(String),
}

/// Aggregate budget for *decoded* PSD bytes.
///
/// `Reader::decoded` only counts bytes consumed from the file, which for PackBits RLE can be
/// up to 64x smaller than the buffer it expands into. Every canvas-sized buffer the importer
/// allocates is therefore charged here as well, so a small file cannot request an unbounded
/// amount of memory by multiplying many channels or layers.
#[derive(Debug, Default)]
struct DecodedBudget {
    bytes: u64,
}

impl DecodedBudget {
    fn charge(&mut self, bytes: u64) -> Result<(), PsdError> {
        self.bytes = self
            .bytes
            .checked_add(bytes)
            .ok_or_else(|| PsdError::TooLarge("decoded byte total overflow".into()))?;
        if self.bytes > MAX_PSD_DECODED_BYTES {
            return Err(PsdError::TooLarge(format!(
                "decoded PSD data ({} bytes, budget {MAX_PSD_DECODED_BYTES})",
                self.bytes
            )));
        }
        Ok(())
    }
}

/// Allocates a zeroed buffer, reporting exhaustion as a parse error instead of aborting.
fn zeroed_buffer(len: usize) -> Result<Vec<u8>, PsdError> {
    let mut buffer = Vec::new();
    buffer
        .try_reserve_exact(len)
        .map_err(|_| PsdError::TooLarge(format!("{len}-byte buffer cannot be allocated")))?;
    buffer.resize(len, 0);
    Ok(buffer)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Rect {
    top: i32,
    left: i32,
    bottom: i32,
    right: i32,
}

impl Rect {
    fn width(self) -> u32 {
        self.right.saturating_sub(self.left).max(0) as u32
    }

    fn height(self) -> u32 {
        self.bottom.saturating_sub(self.top).max(0) as u32
    }
}

#[derive(Debug, Clone)]
struct PsdMask {
    rect: Rect,
    flags: u8,
    pixels: Option<Vec<u8>>,
}

#[derive(Debug, Clone)]
enum PsdNode {
    Group {
        name: String,
        opacity: f32,
        visible: bool,
        children: Vec<PsdNode>,
    },
    Raster {
        name: String,
        rect: Rect,
        pixels: Vec<u8>,
        opacity: f32,
        blend: BlendMode,
        visible: bool,
        clipping: bool,
        mask: Option<PsdMask>,
    },
}

#[derive(Debug, Clone)]
pub struct PsdDocument {
    pub width: u32,
    pub height: u32,
    nodes: Vec<PsdNode>,
}

impl PsdDocument {
    /// Convert the parsed subset into an Aurelia document. Groups become real groups; raster
    /// layers keep their offset, opacity, visibility, clipping and mask.
    pub fn into_document(self) -> Document {
        let mut doc = Document::new_with_format(self.width, self.height, PixelFormat::Srgb8);
        doc.layers.clear();
        doc.next_layer_id = 1;
        doc.is_transparent = true;
        let mut next_id: LayerId = 1;
        for node in &self.nodes {
            add_node(&mut doc, node, None, &mut next_id);
        }
        if let Some(first) = doc.layers.first() {
            doc.active_layer_id = first.id;
        } else {
            doc.add_layer(Some("Imported PSD".to_string()));
        }
        doc
    }
}

fn add_node(doc: &mut Document, node: &PsdNode, parent: Option<LayerId>, next_id: &mut LayerId) {
    match node {
        PsdNode::Group {
            name,
            opacity,
            visible,
            children,
        } => {
            let id = *next_id;
            *next_id += 1;
            let mut group = Layer::new_group(id, name);
            group.parent_id = parent;
            group.opacity = *opacity;
            group.visible = *visible;
            doc.layers.push(group);
            for child in children {
                add_node(doc, child, Some(id), next_id);
            }
        }
        PsdNode::Raster {
            name,
            rect,
            pixels,
            opacity,
            blend,
            visible,
            clipping,
            mask,
        } => {
            let width = rect.width();
            let height = rect.height();
            if width == 0 || height == 0 {
                return;
            }
            let id = *next_id;
            *next_id += 1;
            // AUR stores every raster layer at canvas size, so expand the PSD layer rectangle
            // into a canvas-sized buffer. A layer smaller than the canvas would otherwise be
            // unrepresentable: saving it writes a short payload that the loader rejects.
            let (canvas_w, canvas_h) = (doc.width, doc.height);
            let expanded_len = (canvas_w as usize) * (canvas_h as usize) * 4;
            let mut expanded = vec![0u8; expanded_len];
            for y in 0..height as usize {
                let canvas_y = rect.top as usize + y;
                if canvas_y >= canvas_h as usize {
                    break;
                }
                for x in 0..width as usize {
                    let canvas_x = rect.left as usize + x;
                    if canvas_x >= canvas_w as usize {
                        break;
                    }
                    let source = (y * width as usize + x) * 4;
                    let destination = (canvas_y * canvas_w as usize + canvas_x) * 4;
                    if source + 3 < pixels.len() && destination + 3 < expanded.len() {
                        expanded[destination..destination + 4]
                            .copy_from_slice(&pixels[source..source + 4]);
                    }
                }
            }
            let mut layer = Layer::new(id, name, canvas_w, canvas_h);
            layer.parent_id = parent;
            layer.offset_x = 0;
            layer.offset_y = 0;
            layer.opacity = *opacity;
            layer.blend_mode = *blend;
            layer.visible = *visible;
            layer.clipping_mask = *clipping;
            layer.set_pixels(expanded);
            if let Some(mask) = mask {
                if let Some(pixels) = &mask.pixels {
                    // Aurelia keeps one canvas-sized mask per layer; the PSD mask is scoped to
                    // the layer rectangle. Place it in a canvas buffer so resizing, compositing
                    // and saving keep it instead of dropping it as a dimension mismatch.
                    let (canvas_w, canvas_h) = (doc.width, doc.height);
                    let mask_w = mask.rect.width();
                    let mask_h = mask.rect.height();
                    let invert = mask.flags & 0x04 != 0;
                    let mut canvas_mask = LayerMask::full(canvas_w, canvas_h, 255);
                    for y in 0..mask_h {
                        let canvas_y = mask.rect.top + y as i32;
                        if canvas_y < 0 || canvas_y >= canvas_h as i32 {
                            continue;
                        }
                        for x in 0..mask_w {
                            let canvas_x = mask.rect.left + x as i32;
                            if canvas_x < 0 || canvas_x >= canvas_w as i32 {
                                continue;
                            }
                            let Some(&value) =
                                pixels.get((y as usize) * (mask_w as usize) + x as usize)
                            else {
                                continue;
                            };
                            canvas_mask.pixels
                                [(canvas_y as usize) * (canvas_w as usize) + canvas_x as usize] =
                                if invert { 255 - value } else { value };
                        }
                    }
                    layer.mask = Some(LayerMask {
                        enabled: mask.flags & 0x02 == 0,
                        ..canvas_mask
                    });
                }
            }
            doc.layers.push(layer);
        }
    }
}

/// Number of raster layers in the parsed tree, which is how many canvas-sized buffers
/// `into_document` will allocate.
fn count_raster_layers(nodes: &[PsdNode]) -> u64 {
    nodes
        .iter()
        .map(|node| match node {
            PsdNode::Raster { .. } => 1,
            PsdNode::Group { children, .. } => count_raster_layers(children),
        })
        .sum()
}

struct Reader<'a> {
    data: &'a [u8],
    position: usize,
    decoded: u64,
}

impl<'a> Reader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            position: 0,
            decoded: 0,
        }
    }

    fn remaining(&self) -> usize {
        self.data.len() - self.position
    }

    fn take(&mut self, count: usize) -> Result<&'a [u8], PsdError> {
        let end = self
            .position
            .checked_add(count)
            .ok_or_else(|| PsdError::Corrupt("offset overflow".into()))?;
        if end > self.data.len() {
            return Err(PsdError::Corrupt("truncated PSD".into()));
        }
        self.decoded = self
            .decoded
            .checked_add(count as u64)
            .ok_or_else(|| PsdError::TooLarge("decoded budget overflow".into()))?;
        if self.decoded > MAX_PSD_DECODED_BYTES {
            return Err(PsdError::TooLarge("decoded PSD budget exceeded".into()));
        }
        let slice = &self.data[self.position..end];
        self.position = end;
        Ok(slice)
    }

    fn u8(&mut self) -> Result<u8, PsdError> {
        Ok(self.take(1)?[0])
    }

    fn u16(&mut self) -> Result<u16, PsdError> {
        let b = self.take(2)?;
        Ok(u16::from_be_bytes([b[0], b[1]]))
    }

    fn u32(&mut self) -> Result<u32, PsdError> {
        let b = self.take(4)?;
        Ok(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn i16(&mut self) -> Result<i16, PsdError> {
        Ok(self.u16()? as i16)
    }

    fn i32(&mut self) -> Result<i32, PsdError> {
        Ok(self.u32()? as i32)
    }

    fn rect(&mut self) -> Result<Rect, PsdError> {
        Ok(Rect {
            top: self.i32()?,
            left: self.i32()?,
            bottom: self.i32()?,
            right: self.i32()?,
        })
    }

    fn skip(&mut self, count: usize) -> Result<(), PsdError> {
        self.take(count).map(|_| ())
    }
}

#[derive(Debug, Clone)]
struct LayerRecord {
    name: String,
    rect: Rect,
    channels: Vec<(i16, usize)>,
    blend: BlendMode,
    opacity: f32,
    visible: bool,
    clipping: bool,
    section: Option<u32>,
    mask: Option<PsdMask>,
    channel_data: Vec<(i16, Vec<u8>)>,
}

struct ParsedHeader {
    width: u32,
    height: u32,
    channels: u16,
}

fn parse_header(reader: &mut Reader<'_>) -> Result<ParsedHeader, PsdError> {
    if reader.take(4)? != PSD_SIGNATURE {
        return Err(PsdError::Corrupt("bad 8BPS signature".into()));
    }
    let version = reader.u16()?;
    if version != 1 {
        return Err(PsdError::Unsupported(format!(
            "PSB/version {version}; only PSD version 1 is supported"
        )));
    }
    reader.skip(6)?; // reserved
    let channels = reader.u16()?;
    let height = reader.u32()?;
    let width = reader.u32()?;
    let depth = reader.u16()?;
    let mode = reader.u16()?;
    if width == 0 || height == 0 || width > MAX_PSD_SIDE || height > MAX_PSD_SIDE {
        return Err(PsdError::TooLarge(format!("canvas {width}x{height}")));
    }
    let pixels = width as u64 * height as u64;
    if pixels > MAX_PSD_PIXELS {
        return Err(PsdError::TooLarge(format!("canvas {width}x{height}")));
    }
    if depth != 8 {
        return Err(PsdError::Unsupported(format!(
            "{depth}-bit document; only 8-bit RGB is supported"
        )));
    }
    if mode != 3 {
        return Err(PsdError::Unsupported(format!(
            "colour mode {mode}; only RGB is supported"
        )));
    }
    if !(3..=56).contains(&channels) {
        return Err(PsdError::Corrupt(format!("channel count {channels}")));
    }
    Ok(ParsedHeader {
        width,
        height,
        channels,
    })
}

fn parse_mask(reader: &mut Reader<'_>) -> Result<Option<PsdMask>, PsdError> {
    let len = reader.u32()? as usize;
    if len == 0 {
        return Ok(None);
    }
    if len < 18 {
        return Err(PsdError::Corrupt("layer mask record too short".into()));
    }
    let rect = reader.rect()?;
    let _default_color = reader.u8()?;
    let flags = reader.u8()?;
    reader.skip(len - 18)?;
    Ok(Some(PsdMask {
        rect,
        flags,
        pixels: None,
    }))
}

fn parse_name(reader: &mut Reader<'_>) -> Result<String, PsdError> {
    let len = reader.u8()? as usize;
    let bytes = reader.take(len)?;
    let name = String::from_utf8_lossy(bytes).to_string();
    let total = 1 + len;
    let pad = (4 - total % 4) % 4;
    reader.skip(pad)?;
    Ok(name)
}

fn map_blend(key: &[u8]) -> Result<BlendMode, PsdError> {
    match key {
        b"norm" | b"pass" => Ok(BlendMode::Normal),
        b"mul " => Ok(BlendMode::Multiply),
        b"scrn" => Ok(BlendMode::Screen),
        b"over" => Ok(BlendMode::Overlay),
        b"dark" => Ok(BlendMode::Darken),
        b"lite" => Ok(BlendMode::Lighten),
        b"div " => Ok(BlendMode::ColorDodge),
        b"idiv" => Ok(BlendMode::ColorBurn),
        b"hLit" => Ok(BlendMode::HardLight),
        b"sLit" => Ok(BlendMode::SoftLight),
        b"diff" => Ok(BlendMode::Difference),
        b"smud" => Ok(BlendMode::Exclusion),
        b"lum " => Ok(BlendMode::Luminosity),
        other => Err(PsdError::Unsupported(format!(
            "blend mode {:?}",
            String::from_utf8_lossy(other)
        ))),
    }
}

fn parse_extra(
    reader: &mut Reader<'_>,
    extra_len: usize,
) -> Result<(String, Option<PsdMask>, Option<u32>), PsdError> {
    let extra = reader.take(extra_len)?;
    let mut extra = Reader::new(extra);
    let mask = parse_mask(&mut extra)?;
    let ranges_len = extra.u32()? as usize;
    extra.skip(ranges_len)?;
    let name = parse_name(&mut extra)?;
    let mut unicode_name = None;
    let mut section = None;
    while extra.remaining() >= 12 {
        let signature = extra.take(4)?;
        if signature != b"8BIM" && signature != b"8B64" {
            return Err(PsdError::Corrupt("bad additional-layer signature".into()));
        }
        let key = extra.take(4)?.to_vec();
        let len = extra.u32()? as usize;
        let data = extra.take(len)?;
        match key.as_slice() {
            b"lsct" => {
                if data.len() < 4 {
                    return Err(PsdError::Corrupt("short lsct block".into()));
                }
                section = Some(u32::from_be_bytes([data[0], data[1], data[2], data[3]]));
            }
            b"luni" => {
                if data.len() >= 4 {
                    let chars = data[4..]
                        .chunks_exact(2)
                        .map(|pair| u16::from_be_bytes([pair[0], pair[1]]))
                        .collect::<Vec<_>>();
                    if let Ok(value) = String::from_utf16(&chars) {
                        let value = value.trim_end_matches('\0').to_string();
                        if !value.trim().is_empty() {
                            unicode_name = Some(value);
                        }
                    }
                }
            }
            b"lnsr" | b"lyid" | b"lclr" | b"shmd" | b"clbl" => {}
            b"vmsk" | b"vscg" => {
                return Err(PsdError::Unsupported("vector layer mask".into()));
            }
            b"TySh" => return Err(PsdError::Unsupported("text layer".into())),
            b"SoCo" | b"GdFl" | b"brit" | b"levl" | b"curv" | b"hue " | b"blnc" | b"phfl"
            | b"expA" | b"vibA" | b"blwh" | b"nvrt" | b"post" | b"thrs" | b"mixr" | b"clrL" => {
                return Err(PsdError::Unsupported("adjustment/fill layer".into()));
            }
            other => {
                return Err(PsdError::Unsupported(format!(
                    "unknown additional-layer block {:?}",
                    String::from_utf8_lossy(other)
                )));
            }
        }
    }
    Ok((unicode_name.unwrap_or(name), mask, section))
}

fn decode_packbits(data: &[u8], expected: usize) -> Result<Vec<u8>, PsdError> {
    let mut out = Vec::with_capacity(expected);
    let mut pos = 0;
    while out.len() < expected {
        let header = *data
            .get(pos)
            .ok_or_else(|| PsdError::Corrupt("truncated RLE row".into()))?
            as i8;
        pos += 1;
        if header >= 0 {
            let count = header as usize + 1;
            let end = pos
                .checked_add(count)
                .ok_or_else(|| PsdError::Corrupt("RLE offset overflow".into()))?;
            let slice = data
                .get(pos..end)
                .ok_or_else(|| PsdError::Corrupt("truncated RLE literal".into()))?;
            let take = count.min(expected - out.len());
            out.extend_from_slice(&slice[..take]);
            pos = end;
        } else if header != -128 {
            let count = (1i16 - header as i16) as usize;
            let value = *data
                .get(pos)
                .ok_or_else(|| PsdError::Corrupt("truncated RLE repeat".into()))?;
            pos += 1;
            let take = count.min(expected - out.len());
            out.extend(std::iter::repeat_n(value, take));
        }
    }
    Ok(out)
}

fn decode_channel(
    data: &[u8],
    width: usize,
    height: usize,
    budget: &mut DecodedBudget,
) -> Result<Vec<u8>, PsdError> {
    let mut reader = Reader::new(data);
    let compression = reader.u16()?;
    let expected = width
        .checked_mul(height)
        .ok_or_else(|| PsdError::TooLarge("channel dimensions overflow".into()))?;
    // Charge the decoded size, not the bytes on disk: RLE expands up to 64x.
    budget.charge(expected as u64)?;
    match compression {
        0 => {
            let bytes = reader.take(expected)?;
            Ok(bytes.to_vec())
        }
        1 => {
            let mut rows = Vec::with_capacity(height);
            let mut total = 0usize;
            for _ in 0..height {
                let count = reader.u16()? as usize;
                total = total
                    .checked_add(count)
                    .ok_or_else(|| PsdError::TooLarge("RLE row total overflow".into()))?;
                rows.push(count);
            }
            let mut out = Vec::new();
            out.try_reserve_exact(expected)
                .map_err(|_| PsdError::TooLarge(format!("{expected}-byte channel buffer")))?;
            for count in rows {
                let row = reader.take(count)?;
                out.extend(decode_packbits(row, width)?);
            }
            if out.len() != expected {
                return Err(PsdError::Corrupt("RLE channel length mismatch".into()));
            }
            Ok(out)
        }
        2 | 3 => Err(PsdError::Unsupported(
            "ZIP-compressed layer/channel data".into(),
        )),
        other => Err(PsdError::Unsupported(format!("compression method {other}"))),
    }
}

fn parse_layer_info(
    reader: &mut Reader<'_>,
    header: &ParsedHeader,
) -> Result<Vec<PsdNode>, PsdError> {
    let layer_info_len = reader.u32()? as usize;
    if layer_info_len == 0 {
        return Ok(Vec::new());
    }
    let end = reader
        .position
        .checked_add(layer_info_len)
        .ok_or_else(|| PsdError::Corrupt("layer info length overflow".into()))?;
    if end > reader.data.len() {
        return Err(PsdError::Corrupt("truncated layer info".into()));
    }
    let count_raw = reader.i16()?;
    let count = count_raw.unsigned_abs() as u32;
    if count > MAX_PSD_LAYERS {
        return Err(PsdError::TooLarge(format!("{count} layers")));
    }
    let mut records = Vec::with_capacity(count as usize);
    let mut layer_pixels = 0u64;
    let mut budget = DecodedBudget::default();

    for _ in 0..count {
        let rect = reader.rect()?;
        if rect.left < 0
            || rect.top < 0
            || rect.right < rect.left
            || rect.bottom < rect.top
            || rect.right as u32 > header.width
            || rect.bottom as u32 > header.height
        {
            return Err(PsdError::Unsupported(format!(
                "layer outside canvas ({},{} to {},{})",
                rect.left, rect.top, rect.right, rect.bottom
            )));
        }
        let width = rect.width();
        let height = rect.height();
        layer_pixels = layer_pixels
            .checked_add(width as u64 * height as u64)
            .ok_or_else(|| PsdError::TooLarge("layer pixel total overflow".into()))?;
        if layer_pixels > MAX_PSD_PIXELS {
            return Err(PsdError::TooLarge("aggregate layer pixels".into()));
        }
        let channel_count = reader.u16()?;
        if channel_count > 56 {
            return Err(PsdError::Corrupt("layer channel count".into()));
        }
        let mut channels = Vec::with_capacity(channel_count as usize);
        for _ in 0..channel_count {
            let id = reader.i16()?;
            let len = reader.u32()? as usize;
            channels.push((id, len));
        }
        let signature = reader.take(4)?;
        if signature != b"8BIM" {
            return Err(PsdError::Corrupt("bad layer blend signature".into()));
        }
        let blend = map_blend(reader.take(4)?)?;
        let opacity = reader.u8()? as f32 / 255.0;
        let clipping = reader.u8()? != 0;
        let flags = reader.u8()?;
        reader.skip(1)?;
        let extra_len = reader.u32()? as usize;
        let (name, mask, section) = parse_extra(reader, extra_len)?;
        records.push(LayerRecord {
            name,
            rect,
            channels,
            blend,
            opacity,
            visible: flags & 0x02 == 0,
            clipping,
            section,
            mask,
            channel_data: Vec::new(),
        });
    }

    for record in &mut records {
        let width = record.rect.width() as usize;
        let height = record.rect.height() as usize;
        for (id, len) in &record.channels {
            if width == 0 || height == 0 || *len == 0 {
                record.channel_data.push((*id, Vec::new()));
                continue;
            }
            let data = reader.take(*len)?;
            // Only the channels the importer consumes are worth decompressing. PackBits can
            // expand 64x, so decoding an unused channel is pure amplification.
            if !matches!(id, 0 | 1 | 2 | -1 | -2) {
                record.channel_data.push((*id, Vec::new()));
                continue;
            }
            record
                .channel_data
                .push((*id, decode_channel(data, width, height, &mut budget)?));
        }
    }

    let mut stack: Vec<(Vec<PsdNode>, Option<(String, f32, bool)>)> = vec![(Vec::new(), None)];
    records.reverse();
    for record in records {
        match record.section {
            Some(1) => {
                stack.push((
                    Vec::new(),
                    Some((record.name.clone(), record.opacity, record.visible)),
                ));
            }
            Some(2) | Some(3) => {
                if stack.len() <= 1 {
                    return Err(PsdError::Corrupt("group end without start".into()));
                }
                let (mut children, meta) = stack.pop().unwrap();
                children.reverse();
                let (name, opacity, visible) =
                    meta.ok_or_else(|| PsdError::Corrupt("group missing metadata".into()))?;
                stack.last_mut().unwrap().0.push(PsdNode::Group {
                    name,
                    opacity,
                    visible,
                    children,
                });
            }
            Some(other) => {
                return Err(PsdError::Unsupported(format!(
                    "layer section divider type {other}"
                )));
            }
            None => {
                if record.rect.width() == 0 || record.rect.height() == 0 {
                    continue;
                }
                let width = record.rect.width() as usize;
                let height = record.rect.height() as usize;
                budget.charge((width * height * 4) as u64)?;
                let mut rgba = zeroed_buffer(width * height * 4)?;
                for pixel in rgba.chunks_exact_mut(4) {
                    pixel[3] = 255;
                }
                let mut mask_pixels = None;
                let mut seen_rgb = [false; 3];
                for (id, data) in record.channel_data {
                    if data.len() != width * height {
                        continue;
                    }
                    match id {
                        0 | 1 | 2 => {
                            let offset = id as usize;
                            seen_rgb[offset] = true;
                            for (index, value) in data.into_iter().enumerate() {
                                rgba[index * 4 + offset] = value;
                            }
                        }
                        -1 => {
                            for (index, value) in data.into_iter().enumerate() {
                                rgba[index * 4 + 3] = value;
                            }
                        }
                        -2 => mask_pixels = Some(data),
                        _ => {}
                    }
                }
                if !seen_rgb.iter().all(|seen| *seen) {
                    return Err(PsdError::Unsupported(
                        "raster layer without R, G and B channels".into(),
                    ));
                }
                let mask = match record.mask {
                    Some(mut mask) => {
                        if mask.rect.left < 0
                            || mask.rect.top < 0
                            || mask.rect.right > header.width as i32
                            || mask.rect.bottom > header.height as i32
                            || mask.rect.width() != width as u32
                            || mask.rect.height() != height as u32
                        {
                            return Err(PsdError::Unsupported(
                                "layer mask outside its layer bounds".into(),
                            ));
                        }
                        let pixels = mask_pixels.ok_or_else(|| {
                            PsdError::Unsupported("layer mask without a mask channel".into())
                        })?;
                        let expected = mask.rect.width() as usize * mask.rect.height() as usize;
                        if pixels.len() != expected {
                            return Err(PsdError::Unsupported(
                                "layer mask dimensions do not match its pixel channel".into(),
                            ));
                        }
                        mask.pixels = Some(pixels);
                        Some(mask)
                    }
                    None => None,
                };
                stack.last_mut().unwrap().0.push(PsdNode::Raster {
                    name: record.name,
                    rect: record.rect,
                    pixels: rgba,
                    opacity: record.opacity,
                    blend: record.blend,
                    visible: record.visible,
                    clipping: record.clipping,
                    mask,
                });
            }
        }
    }
    if stack.len() != 1 {
        return Err(PsdError::Corrupt("unclosed layer group".into()));
    }
    if reader.position > end {
        return Err(PsdError::Corrupt("layer data overran its section".into()));
    }
    reader.skip(end - reader.position)?;
    let mut roots = stack.pop().unwrap().0;
    roots.reverse();
    Ok(roots)
}

fn parse_composite(
    reader: &mut Reader<'_>,
    header: &ParsedHeader,
    budget: &mut DecodedBudget,
) -> Result<PsdNode, PsdError> {
    let width = header.width as usize;
    let height = header.height as usize;
    let compression = reader.u16()?;
    let mut channels: Vec<Vec<u8>> = Vec::new();
    if compression == 1 {
        // Composite PSD RLE stores all row byte counts for all channels first, then
        // the compressed rows for channel 0, channel 1, ...
        let mut counts = Vec::with_capacity(height * header.channels as usize);
        for _ in 0..height * header.channels as usize {
            counts.push(reader.u16()? as usize);
        }
        for channel in 0..header.channels as usize {
            if channel > 3 {
                // Alpha/spot channels past the first four are never merged into the RGBA
                // buffer; consume their rows without decompressing them.
                for row in 0..height {
                    let count = counts[channel * height + row];
                    reader.take(count)?;
                }
                channels.push(Vec::new());
                continue;
            }
            budget.charge((width * height) as u64)?;
            let mut out = Vec::new();
            out.try_reserve_exact(width * height).map_err(|_| {
                PsdError::TooLarge(format!("{}-byte channel buffer", width * height))
            })?;
            for row in 0..height {
                let count = counts[channel * height + row];
                out.extend(decode_packbits(reader.take(count)?, width)?);
            }
            channels.push(out);
        }
    } else {
        for channel in 0..header.channels as usize {
            match compression {
                0 if channel > 3 => {
                    reader.take(width * height)?;
                    channels.push(Vec::new());
                }
                0 => {
                    budget.charge((width * height) as u64)?;
                    channels.push(reader.take(width * height)?.to_vec());
                }
                other => {
                    return Err(PsdError::Unsupported(format!(
                        "composite compression method {other}"
                    )))
                }
            }
        }
    }
    budget.charge((width * height * 4) as u64)?;
    let mut rgba = zeroed_buffer(width * height * 4)?;
    for pixel in rgba.chunks_exact_mut(4) {
        pixel[3] = 255;
    }
    for (channel_index, data) in channels.into_iter().enumerate() {
        match channel_index {
            0 | 1 | 2 => {
                for (index, value) in data.into_iter().enumerate() {
                    rgba[index * 4 + channel_index] = value;
                }
            }
            3 => {
                for (index, value) in data.into_iter().enumerate() {
                    rgba[index * 4 + 3] = value;
                }
            }
            _ => {}
        }
    }
    Ok(PsdNode::Raster {
        name: "Composite".to_string(),
        rect: Rect {
            top: 0,
            left: 0,
            bottom: header.height as i32,
            right: header.width as i32,
        },
        pixels: rgba,
        opacity: 1.0,
        blend: BlendMode::Normal,
        visible: true,
        clipping: false,
        mask: None,
    })
}

/// Parse a PSD from a bounded byte slice.
pub fn parse_psd(data: &[u8]) -> Result<PsdDocument, PsdError> {
    if data.len() as u64 > MAX_PSD_FILE_BYTES {
        return Err(PsdError::TooLarge("PSD file byte budget".into()));
    }
    let mut reader = Reader::new(data);
    let header = parse_header(&mut reader)?;
    let color_len = reader.u32()? as usize;
    reader.skip(color_len)?;
    let resource_len = reader.u32()? as usize;
    reader.skip(resource_len)?;
    let layer_mask_len = reader.u32()? as usize;
    let layer_mask_start = reader.position;
    let nodes = if layer_mask_len > 0 {
        parse_layer_info(&mut reader, &header)?
    } else {
        Vec::new()
    };
    reader.position = layer_mask_start
        .checked_add(layer_mask_len)
        .ok_or_else(|| PsdError::Corrupt("layer/mask length overflow".into()))?;
    if reader.position > data.len() {
        return Err(PsdError::Corrupt("truncated layer/mask section".into()));
    }
    let nodes = if nodes.is_empty() {
        let mut budget = DecodedBudget::default();
        vec![parse_composite(&mut reader, &header, &mut budget)?]
    } else {
        nodes
    };

    // Layers are expanded to canvas size (`into_document`), so the decoded budget must cover
    // that expansion, not just the sum of the layer rectangles.
    let canvas_pixels = header.width as u64 * header.height as u64;
    let raster_layers = count_raster_layers(&nodes);
    if canvas_pixels
        .checked_mul(raster_layers)
        .is_none_or(|expanded| expanded > MAX_PSD_PIXELS)
    {
        return Err(PsdError::TooLarge(format!(
            "{raster_layers} layer(s) at canvas size exceed the pixel budget"
        )));
    }
    Ok(PsdDocument {
        width: header.width,
        height: header.height,
        nodes,
    })
}

/// Read and parse a PSD file.
pub fn load_psd_file(path: &Path) -> Result<PsdDocument, PsdError> {
    let metadata = std::fs::metadata(path)?;
    if metadata.len() > MAX_PSD_FILE_BYTES {
        return Err(PsdError::TooLarge(format!(
            "file is {} bytes; maximum {MAX_PSD_FILE_BYTES}",
            metadata.len()
        )));
    }
    let file = std::fs::File::open(path)?;
    let mut data = Vec::new();
    file.take(MAX_PSD_FILE_BYTES + 1).read_to_end(&mut data)?;
    parse_psd(&data)
}

/// Import a PSD into a document in one step.
pub fn load_psd_document(path: &Path) -> Result<Document, PsdError> {
    Ok(load_psd_file(path)?.into_document())
}
