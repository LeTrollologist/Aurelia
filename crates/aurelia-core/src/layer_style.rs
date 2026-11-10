//! Layer styles: stroke, drop shadow and outer glow, evaluated at composite time.
//!
//! A style is a function of the layer's own alpha and colour. Nothing is baked into the
//! layer's pixels: the compositor asks for a styled copy of the layer each time it draws it,
//! exactly like a blend mode or an adjustment. That keeps styles non-destructive, undoable by
//! changing the style record only, and consistent between previews and exports.
//!
//! The renderer works in the layer's own pixel space (not the canvas), so offsets, masks,
//! clipping, opacity and blend modes keep their existing meaning. Effects that extend past the
//! layer's bounds are clipped to them; editor-created layers are canvas-sized, so in practice
//! that only clips at the canvas edge.
//!
//! Working precision is `f32` per channel for the masks and the composite; the result is
//! quantised once, into the layer's storage width (8 or 16 bits).

use crate::color::Color;
use crate::layer::{Layer, LayerBuffer};
use crate::pixel::Channel;
use serde::{Deserialize, Serialize};

/// One layer's style stack. `Default` (all `None`) is the common case and costs nothing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct LayerStyles {
    #[serde(default)]
    pub stroke: Option<StrokeStyle>,
    #[serde(default)]
    pub drop_shadow: Option<DropShadowStyle>,
    #[serde(default)]
    pub outer_glow: Option<OuterGlowStyle>,
}

impl LayerStyles {
    pub fn is_empty(&self) -> bool {
        self.stroke.is_none() && self.drop_shadow.is_none() && self.outer_glow.is_none()
    }

    /// Scales pixel lengths with a canvas resize. Offsets scale per axis; widths, blurs and
    /// radii use the geometric mean of the two axes because the effects are isotropic.
    pub fn scaled(&self, scale_x: f32, scale_y: f32) -> Self {
        let uniform = (scale_x * scale_y).sqrt();
        if !uniform.is_finite() || uniform <= 0.0 {
            return self.clone();
        }
        Self {
            stroke: self.stroke.as_ref().map(|stroke| StrokeStyle {
                width: (stroke.width * uniform).max(0.0),
                color: stroke.color,
                opacity: stroke.opacity,
            }),
            drop_shadow: self.drop_shadow.as_ref().map(|shadow| DropShadowStyle {
                offset_x: shadow.offset_x * scale_x,
                offset_y: shadow.offset_y * scale_y,
                blur: (shadow.blur * uniform).max(0.0),
                color: shadow.color,
                opacity: shadow.opacity,
            }),
            outer_glow: self.outer_glow.as_ref().map(|glow| OuterGlowStyle {
                radius: (glow.radius * uniform).max(0.0),
                color: glow.color,
                opacity: glow.opacity,
            }),
        }
    }

    /// Styles that would actually draw: present, with non-zero width/radius and opacity.
    pub fn has_visible_effect(&self) -> bool {
        self.stroke
            .as_ref()
            .is_some_and(|s| s.width > 0.0 && s.opacity > 0.0)
            || self.drop_shadow.as_ref().is_some_and(|s| {
                s.opacity > 0.0 && (s.blur > 0.0 || s.offset_x != 0.0 || s.offset_y != 0.0)
            })
            || self
                .outer_glow
                .as_ref()
                .is_some_and(|s| s.radius > 0.0 && s.opacity > 0.0)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StrokeStyle {
    /// Stroke width in pixels, measured outward from the silhouette.
    pub width: f32,
    pub color: Color,
    pub opacity: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DropShadowStyle {
    pub offset_x: f32,
    pub offset_y: f32,
    /// Gaussian-ish blur radius in pixels; 0 is a hard shadow.
    pub blur: f32,
    pub color: Color,
    pub opacity: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OuterGlowStyle {
    pub radius: f32,
    pub color: Color,
    pub opacity: f32,
}

impl Default for StrokeStyle {
    fn default() -> Self {
        Self {
            width: 3.0,
            color: Color::BLACK,
            opacity: 1.0,
        }
    }
}

impl Default for DropShadowStyle {
    fn default() -> Self {
        Self {
            offset_x: 6.0,
            offset_y: 6.0,
            blur: 6.0,
            color: Color::BLACK,
            opacity: 0.6,
        }
    }
}

impl Default for OuterGlowStyle {
    fn default() -> Self {
        Self {
            radius: 8.0,
            color: Color::BRAND_GOLD,
            opacity: 0.8,
        }
    }
}

/// Renders the layer plus its styles into a new buffer of the layer's storage width.
///
/// Only the rectangle that can contain the effects is evaluated; every effect's support is
/// bounded, so pixels outside the rectangle would be fully transparent anyway. This is the
/// same computation as [`render`] restricted to that rectangle.
pub fn render_styled(layer: &Layer) -> LayerBuffer {
    match layer.buffer() {
        LayerBuffer::Srgb8(source) => LayerBuffer::Srgb8(render_bounded(
            source,
            layer.width,
            layer.height,
            &layer.styles,
        )),
        LayerBuffer::Srgb16(source) => LayerBuffer::Srgb16(render_bounded(
            source,
            layer.width,
            layer.height,
            &layer.styles,
        )),
    }
}

/// Renders styles at 16-bit precision regardless of the layer's storage width.
///
/// The mixed-document compositor promotes 8-bit layers to 16 bits before compositing; styles
/// must be evaluated at that width too, or a glow would be quantised to the 8-bit lattice
/// before it ever reaches the working-precision path.
pub fn render_styled_srgb16(layer: &Layer) -> Vec<u16> {
    match layer.buffer() {
        LayerBuffer::Srgb16(source) => {
            render_bounded(source, layer.width, layer.height, &layer.styles)
        }
        LayerBuffer::Srgb8(source) => {
            let promoted: Vec<u16> = source
                .iter()
                .map(|&v| crate::pixel::srgb8_to_srgb16(v))
                .collect();
            render_bounded(&promoted, layer.width, layer.height, &layer.styles)
        }
    }
}

/// Full-canvas renderer. Kept as the reference for the bounded path and for direct tests.
pub fn render<C: Channel>(source: &[C], width: u32, height: u32, styles: &LayerStyles) -> Vec<C> {
    render_rect(source, width, height, styles, 0, 0, width, height)
}

/// Bounds of the region that can contain any styled pixel, or `None` when the layer is empty.
///
/// Content is the set of pixels with non-zero alpha. Each effect's maximum reach is known:
///
/// - blur: three box passes, each with `kernel = max(1, round(radius / 1.5))`, so `3 * kernel`;
/// - drop shadow: the blur support plus the offset;
/// - outer glow: the blur support;
/// - stroke: the band width plus one pixel of chamfer slack.
///
/// The rectangle is expanded symmetrically by the largest reach and clamped to the canvas.
/// Outside it every effect mask is exactly zero, which is what makes cropping lossless.
pub fn styled_rect<C: Channel>(
    source: &[C],
    width: u32,
    height: u32,
    styles: &LayerStyles,
) -> Option<(u32, u32, u32, u32)> {
    let w = width as usize;
    let h = height as usize;
    let mut min_x = w;
    let mut min_y = h;
    let mut max_x = 0usize;
    let mut max_y = 0usize;
    let mut found = false;
    for y in 0..h {
        for x in 0..w {
            let index = (y * w + x) * 4 + 3;
            if index < source.len() && source[index].to_scaled_f32() > 0.0 {
                found = true;
                min_x = min_x.min(x);
                min_y = min_y.min(y);
                max_x = max_x.max(x);
                max_y = max_y.max(y);
            }
        }
    }
    if !found {
        return None;
    }

    let support = effect_support(styles) as i64;
    let x0 = (min_x as i64 - support).max(0) as u32;
    let y0 = (min_y as i64 - support).max(0) as u32;
    let x1 = (max_x as i64 + 1 + support).min(width as i64) as u32;
    let y1 = (max_y as i64 + 1 + support).min(height as i64) as u32;
    Some((x0, y0, x1, y1))
}

fn blur_support(radius: f32) -> u32 {
    if radius < 0.5 {
        return 0;
    }
    let kernel = ((radius / 1.5).round() as u32).max(1);
    3 * kernel
}

fn effect_support(styles: &LayerStyles) -> u32 {
    let mut support = 0;
    if let Some(stroke) = &styles.stroke {
        if stroke.width > 0.0 && stroke.opacity > 0.0 {
            support = support.max(stroke.width.ceil() as u32 + 1);
        }
    }
    if let Some(glow) = &styles.outer_glow {
        if glow.radius > 0.0 && glow.opacity > 0.0 {
            support = support.max(blur_support(glow.radius));
        }
    }
    if let Some(shadow) = &styles.drop_shadow {
        if shadow.opacity > 0.0
            && (shadow.blur > 0.0 || shadow.offset_x != 0.0 || shadow.offset_y != 0.0)
        {
            let offset = shadow.offset_x.abs().max(shadow.offset_y.abs()).ceil() as u32;
            support = support.max(offset + blur_support(shadow.blur));
        }
    }
    support
}

fn render_bounded<C: Channel>(
    source: &[C],
    width: u32,
    height: u32,
    styles: &LayerStyles,
) -> Vec<C> {
    let Some((x0, y0, x1, y1)) = styled_rect(source, width, height, styles) else {
        return vec![C::from_scaled_f32(0.0); source.len().min((width * height * 4) as usize)];
    };
    render_rect(source, width, height, styles, x0, y0, x1, y1)
}

/// Generic renderer: straight RGBA in the channel's own scale out, same size as the source.
///
/// The rectangle is `[x0, x1) × [y0, y1)`; pixels outside it stay transparent. The effect maths
/// is identical for any rectangle, including the full canvas, which is what the equivalence
/// tests rely on.
#[allow(clippy::too_many_arguments)]
fn render_rect<C: Channel>(
    source: &[C],
    width: u32,
    height: u32,
    styles: &LayerStyles,
    x0: u32,
    y0: u32,
    x1: u32,
    y1: u32,
) -> Vec<C> {
    let needed = (width as usize * height as usize * 4).min(source.len());
    let mut out = vec![C::from_scaled_f32(0.0); needed];
    if width == 0 || height == 0 || out.len() < (width as usize * height as usize * 4) {
        return out;
    }
    let w = width as usize;
    let x0 = x0.min(width);
    let y0 = y0.min(height);
    let x1 = x1.min(width).max(x0);
    let y1 = y1.min(height).max(y0);
    let sw = (x1 - x0) as usize;
    let sh = (y1 - y0) as usize;
    let n = sw * sh;
    if n == 0 {
        return out;
    }

    // Alpha silhouette of the rectangle in working precision.
    let mut alpha = vec![0.0_f32; n];
    for ly in 0..sh {
        let gy = y0 as usize + ly;
        for lx in 0..sw {
            let gx = x0 as usize + lx;
            alpha[ly * sw + lx] =
                source[(gy * w + gx) * 4 + 3].to_scaled_f32() / C::MAX_SCALED as f32;
        }
    }

    // Composite canvas for the rectangle, straight alpha.
    let mut canvas = vec![[0.0_f32; 4]; n];

    if let Some(shadow) = &styles.drop_shadow {
        if shadow.opacity > 0.0 {
            let mut mask = vec![0.0_f32; n];
            let dx = shadow.offset_x.round() as i32;
            let dy = shadow.offset_y.round() as i32;
            for ly in 0..sh as i32 {
                for lx in 0..sw as i32 {
                    // Local coordinates mirror the global sample (gx + lx) - dx.
                    let sx = lx - dx;
                    let sy = ly - dy;
                    if sx < 0 || sy < 0 || sx >= sw as i32 || sy >= sh as i32 {
                        continue;
                    }
                    mask[(ly as usize) * sw + lx as usize] =
                        alpha[(sy as usize) * sw + sx as usize];
                }
            }
            if shadow.blur > 0.0 {
                blur(&mut mask, sw as u32, sh as u32, shadow.blur);
            }
            let color = shadow.color.to_rgba8();
            let opacity = shadow.opacity.clamp(0.0, 1.0);
            for (index, coverage) in mask.iter().enumerate() {
                let a = coverage.clamp(0.0, 1.0) * opacity;
                if a > 0.0 {
                    blend_over(
                        &mut canvas[index],
                        [
                            color[0] as f32 / 255.0,
                            color[1] as f32 / 255.0,
                            color[2] as f32 / 255.0,
                            a,
                        ],
                    );
                }
            }
        }
    }

    if let Some(glow) = &styles.outer_glow {
        if glow.opacity > 0.0 && glow.radius > 0.0 {
            let mut mask = alpha.clone();
            blur(&mut mask, sw as u32, sh as u32, glow.radius);
            let color = glow.color.to_rgba8();
            let opacity = glow.opacity.clamp(0.0, 1.0);
            for (index, coverage) in mask.iter().enumerate() {
                // Outside only: the layer itself is painted over the glow afterwards, and
                // multiplying by (1 - alpha) keeps a bright glow from washing out the art.
                let outside = (1.0 - alpha[index]).clamp(0.0, 1.0);
                let a = coverage.clamp(0.0, 1.0) * outside * opacity;
                if a > 0.0 {
                    blend_over(
                        &mut canvas[index],
                        [
                            color[0] as f32 / 255.0,
                            color[1] as f32 / 255.0,
                            color[2] as f32 / 255.0,
                            a,
                        ],
                    );
                }
            }
        }
    }

    if let Some(stroke) = &styles.stroke {
        if stroke.opacity > 0.0 && stroke.width > 0.0 {
            let distance = outside_distance(&alpha, sw as u32, sh as u32);
            let color = stroke.color.to_rgba8();
            let opacity = stroke.opacity.clamp(0.0, 1.0);
            for index in 0..n {
                if alpha[index] > 0.5 {
                    continue;
                }
                let coverage = (stroke.width - distance[index] + 0.5).clamp(0.0, 1.0);
                let a = coverage * opacity;
                if a > 0.0 {
                    blend_over(
                        &mut canvas[index],
                        [
                            color[0] as f32 / 255.0,
                            color[1] as f32 / 255.0,
                            color[2] as f32 / 255.0,
                            a,
                        ],
                    );
                }
            }
        }
    }

    // The layer itself, painted straight over the effects.
    for ly in 0..sh {
        let gy = y0 as usize + ly;
        for lx in 0..sw {
            let gx = x0 as usize + lx;
            let index = ly * sw + lx;
            let pixel = &source[(gy * w + gx) * 4..(gy * w + gx) * 4 + 4];
            blend_over(
                &mut canvas[index],
                [
                    pixel[0].to_scaled_f32() / C::MAX_SCALED as f32,
                    pixel[1].to_scaled_f32() / C::MAX_SCALED as f32,
                    pixel[2].to_scaled_f32() / C::MAX_SCALED as f32,
                    pixel[3].to_scaled_f32() / C::MAX_SCALED as f32,
                ],
            );
        }
    }

    for ly in 0..sh {
        let gy = y0 as usize + ly;
        for lx in 0..sw {
            let gx = x0 as usize + lx;
            let index = ly * sw + lx;
            let pixel = canvas[index];
            let out_index = (gy * w + gx) * 4;
            out[out_index] = C::from_scaled_f32(pixel[0] * C::MAX_SCALED as f32);
            out[out_index + 1] = C::from_scaled_f32(pixel[1] * C::MAX_SCALED as f32);
            out[out_index + 2] = C::from_scaled_f32(pixel[2] * C::MAX_SCALED as f32);
            out[out_index + 3] = C::from_scaled_f32(pixel[3] * C::MAX_SCALED as f32);
        }
    }
    out
}

/// Straight-alpha source-over into a straight-alpha destination.
#[inline]
fn blend_over(dst: &mut [f32; 4], src: [f32; 4]) {
    let sa = src[3].clamp(0.0, 1.0);
    if sa <= 0.0 {
        return;
    }
    let da = dst[3];
    let out_a = sa + da * (1.0 - sa);
    if out_a <= 0.0 {
        return;
    }
    // One reciprocal, three multiplies: the three divisions here dominated the style pass on
    // large canvases (millions of divisions per effect).
    let inv_out_a = 1.0 / out_a;
    for channel in 0..3 {
        dst[channel] = (src[channel] * sa + dst[channel] * da * (1.0 - sa)) * inv_out_a;
    }
    dst[3] = out_a;
}

/// Three-pass separable box blur, which is close enough to a Gaussian for an effect mask.
fn blur(mask: &mut [f32], width: u32, height: u32, radius: f32) {
    let radius = radius.max(0.0);
    if radius < 0.5 || width == 0 || height == 0 {
        return;
    }
    let kernel = ((radius / 1.5).round() as u32).max(1);
    let mut temp = vec![0.0_f32; mask.len()];
    for _ in 0..3 {
        blur_horizontal(mask, &mut temp, width, height, kernel);
        blur_vertical(&temp, mask, width, height, kernel);
    }
}

/// Boundary policy: **transparent extension**. Samples outside the mask contribute zero, and
/// the divisor is always the full kernel width. A uniform field therefore loses intensity near
/// the mask border, which is the expected result for a blur of a layer whose coverage ends
/// there (the same model as an effect computed on a transparent-padded canvas). This is
/// deliberate, not an artefact: interior pixels are unaffected, and both borders behave
/// symmetrically. The policy is pinned by tests; changing it is a rendering change.
fn blur_horizontal(src: &[f32], dst: &mut [f32], width: u32, height: u32, radius: u32) {
    let w = width as usize;
    let window = (radius as usize * 2 + 1) as f32;
    let inv = 1.0 / window;
    for y in 0..height as usize {
        let row = y * w;
        let mut sum: f32 = 0.0;
        for x in 0..=(radius as usize).min(w.saturating_sub(1)) {
            sum += src[row + x];
        }
        for x in 0..w {
            dst[row + x] = sum * inv;
            if x >= radius as usize {
                sum -= src[row + (x - radius as usize)];
            }
            let in_index = x + radius as usize + 1;
            if in_index < w {
                sum += src[row + in_index];
            }
        }
    }
}

/// Same transparent-extension policy as [`blur_horizontal`], but row-major: the per-column
/// sums live in one small array and every pass walks rows sequentially. The column-major
/// version it replaces missed the cache on essentially every sample at 1080p and above, which
/// made a styled layer's shadow/glow pass the dominant viewport cost.
fn blur_vertical(src: &[f32], dst: &mut [f32], width: u32, height: u32, radius: u32) {
    let w = width as usize;
    let h = height as usize;
    let radius = radius as usize;
    let inv = 1.0 / ((radius * 2 + 1) as f32);
    let mut sums = vec![0.0_f32; w];
    for y in 0..=radius.min(h.saturating_sub(1)) {
        let row = y * w;
        for x in 0..w {
            sums[x] += src[row + x];
        }
    }
    for y in 0..h {
        let row = y * w;
        for x in 0..w {
            dst[row + x] = sums[x] * inv;
        }
        if y >= radius {
            let out = (y - radius) * w;
            for x in 0..w {
                sums[x] -= src[out + x];
            }
        }
        let next = y + radius + 1;
        if next < h {
            let input = next * w;
            for x in 0..w {
                sums[x] += src[input + x];
            }
        }
    }
}

/// Chamfer distance from every transparent pixel to the nearest covered pixel (> 0.5 alpha).
///
/// Three-to-four chamfer weights approximate Euclidean distance closely enough for a stroke
/// band, and the two-pass sweep is linear in pixels regardless of stroke width.
fn outside_distance(alpha: &[f32], width: u32, height: u32) -> Vec<f32> {
    let w = width as usize;
    let h = height as usize;
    let big = (w + h + 2) as f32 * 4.0;
    let mut distance = vec![big; w * h];
    for (index, value) in alpha.iter().enumerate() {
        if *value > 0.5 {
            distance[index] = 0.0;
        }
    }
    fn at(distance: &[f32], w: usize, h: usize, big: f32, x: i32, y: i32) -> f32 {
        if x < 0 || y < 0 || x >= w as i32 || y >= h as i32 {
            big
        } else {
            distance[(y as usize) * w + x as usize]
        }
    }
    for y in 0..h as i32 {
        for x in 0..w as i32 {
            let index = (y as usize) * w + x as usize;
            let mut best = distance[index];
            best = best.min(at(&distance, w, h, big, x - 1, y) + 3.0);
            best = best.min(at(&distance, w, h, big, x, y - 1) + 3.0);
            best = best.min(at(&distance, w, h, big, x - 1, y - 1) + 4.0);
            best = best.min(at(&distance, w, h, big, x + 1, y - 1) + 4.0);
            distance[index] = best;
        }
    }
    for y in (0..h as i32).rev() {
        for x in (0..w as i32).rev() {
            let index = (y as usize) * w + x as usize;
            let mut best = distance[index];
            best = best.min(at(&distance, w, h, big, x + 1, y) + 3.0);
            best = best.min(at(&distance, w, h, big, x, y + 1) + 3.0);
            best = best.min(at(&distance, w, h, big, x + 1, y + 1) + 4.0);
            best = best.min(at(&distance, w, h, big, x - 1, y + 1) + 4.0);
            distance[index] = best;
        }
    }
    for value in &mut distance {
        *value /= 3.0;
    }
    distance
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer::Layer;

    fn layer_with_square(size: u32) -> Layer {
        let mut layer = Layer::new(1, "Square", size, size);
        if let Some(pixels) = layer.as_srgb8_mut() {
            for y in 10..12 {
                for x in 10..12 {
                    let at = ((y * size + x) * 4) as usize;
                    pixels[at] = 255;
                    pixels[at + 1] = 0;
                    pixels[at + 2] = 0;
                    pixels[at + 3] = 255;
                }
            }
        }
        layer
    }

    fn pixel(pixels: &[u8], width: u32, x: u32, y: u32) -> [u8; 4] {
        let at = ((y * width + x) * 4) as usize;
        [pixels[at], pixels[at + 1], pixels[at + 2], pixels[at + 3]]
    }

    #[test]
    fn no_styles_returns_the_source_exactly() {
        let layer = layer_with_square(32);
        let styled = render_styled(&layer);
        match (styled, layer.buffer()) {
            (LayerBuffer::Srgb8(a), LayerBuffer::Srgb8(b)) => assert_eq!(&a, b),
            _ => panic!("format mismatch"),
        }
    }

    #[test]
    fn drop_shadow_is_offset_and_coloured() {
        let mut layer = layer_with_square(32);
        layer.styles.drop_shadow = Some(DropShadowStyle {
            offset_x: 5.0,
            offset_y: 0.0,
            blur: 0.0,
            color: Color::BLACK,
            opacity: 1.0,
        });
        let LayerBuffer::Srgb8(out) = render_styled(&layer) else {
            panic!("8-bit");
        };
        assert_eq!(
            pixel(&out, 32, 16, 10),
            [0, 0, 0, 255],
            "shadow follows the offset"
        );
        assert_eq!(
            pixel(&out, 32, 10, 10),
            [255, 0, 0, 255],
            "layer paints over its shadow"
        );
        assert_eq!(
            pixel(&out, 32, 5, 5)[3],
            0,
            "no shadow where the shifted silhouette is absent"
        );
    }

    #[test]
    fn glow_and_stroke_extend_beyond_the_silhouette() {
        let mut layer = layer_with_square(32);
        layer.styles.outer_glow = Some(OuterGlowStyle {
            radius: 3.0,
            color: Color::from_hex("#00ff00").expect("green"),
            opacity: 1.0,
        });
        layer.styles.stroke = Some(StrokeStyle {
            width: 1.5,
            color: Color::from_hex("#0000ff").expect("blue"),
            opacity: 1.0,
        });
        let LayerBuffer::Srgb8(out) = render_styled(&layer) else {
            panic!("8-bit");
        };
        // The stroke band sits immediately outside the square and is drawn above the glow.
        let edge = pixel(&out, 32, 9, 10);
        assert!(
            edge[3] > 0,
            "stroke must cover the pixel just outside the silhouette"
        );
        assert!(
            edge[2] > edge[1],
            "stroke colour should dominate at the edge: {edge:?}"
        );
        let glow = pixel(&out, 32, 9, 6);
        assert!(glow[3] > 0, "glow reaches a few pixels out");
        assert!(glow[1] > glow[2], "glow is green, not blue: {glow:?}");
    }

    #[test]
    fn sixteen_bit_effects_keep_sub_lattice_alpha() {
        let mut layer = Layer::new(1, "Wide", 16, 16);
        layer.convert_format(crate::layer::PixelFormat::Srgb16);
        if let Some(pixels) = layer.as_srgb16_mut() {
            for y in 4..8 {
                for x in 4..8 {
                    let at = ((y * 16 + x) * 4) as usize;
                    pixels[at] = 65535;
                    pixels[at + 3] = 65535;
                }
            }
        }
        layer.styles.outer_glow = Some(OuterGlowStyle {
            radius: 2.0,
            color: Color::WHITE,
            opacity: 0.5,
        });
        let LayerBuffer::Srgb16(out) = render_styled(&layer) else {
            panic!("16-bit");
        };
        let at = ((6 * 16 + 6) * 4) as usize;
        assert_eq!(out[at + 3], 65535, "layer alpha stays exact");
        let outside = ((3 * 16 + 6) * 4) as usize;
        let alpha = out[outside + 3];
        assert!(alpha > 0, "glow must be present");
        assert_ne!(
            alpha % 257,
            0,
            "16-bit glow alpha must not collapse to the 8-bit lattice"
        );
    }

    #[test]
    fn styles_are_clipped_to_the_layer_buffer() {
        let mut layer = layer_with_square(12);
        layer.styles.drop_shadow = Some(DropShadowStyle {
            offset_x: 40.0,
            offset_y: 40.0,
            blur: 0.0,
            color: Color::BLACK,
            opacity: 1.0,
        });
        let LayerBuffer::Srgb8(out) = render_styled(&layer) else {
            panic!("8-bit");
        };
        // The shadow falls entirely outside a 12x12 buffer; nothing may panic or wrap.
        assert!(out.chunks_exact(4).all(|px| px[3] == 0 || px[0] == 255));
    }
}

#[cfg(test)]
mod blur_policy {
    use super::blur;

    /// Transparent extension: a uniform field stays uniform everywhere the kernel fits inside
    /// the mask, and only the border bands fade. This pins the boundary policy, not a
    /// uniformity guarantee at the edges.
    #[test]
    fn interior_is_unaffected_and_edges_fade_symmetrically() {
        let (w, h) = (64u32, 64u32);
        let mut mask = vec![1.0f32; (w * h) as usize];
        blur(&mut mask, w, h, 6.0);
        let at = |x: u32, y: u32| mask[(y * w + x) as usize];
        for y in 18..46 {
            for x in 18..46 {
                assert!(
                    (at(x, y) - 1.0).abs() < 0.002,
                    "interior ({x},{y}) = {}",
                    at(x, y)
                );
            }
        }
        let mid = h / 2;
        let left = at(0, mid);
        let right = at(w - 1, mid);
        assert!(
            (left - right).abs() < 0.002,
            "left {left} and right {right} must mirror"
        );
        assert!(
            left < 0.9,
            "the border must fade under transparent extension, got {left}"
        );
        assert!(at(w / 2, mid) > 0.99, "the centre must not fade");
        assert!(
            at(0, 0) <= left + 0.002,
            "corners fade at least as much as edges"
        );
    }

    /// A dimension smaller than the kernel is legal (thin strips, 1-pixel masks) and must not
    /// panic or produce values outside 0..=1.
    #[test]
    fn thin_masks_are_defined() {
        for (w, h) in [(1u32, 1u32), (1, 32), (32, 1), (3, 9)] {
            let mut mask = vec![1.0f32; (w * h) as usize];
            blur(&mut mask, w, h, 8.0);
            assert!(
                mask.iter()
                    .all(|v| v.is_finite() && (0.0..=1.0).contains(v)),
                "{w}x{h} produced an out-of-range value"
            );
        }
    }
}

#[cfg(test)]
mod bounded_equivalence {
    use super::*;
    use crate::layer::Layer;

    fn layer_with(width: u32, height: u32, art: &[(u32, u32)]) -> Layer {
        let mut layer = Layer::new(1, "art", width, height);
        for &(x, y) in art {
            if x < width && y < height {
                let index = ((y as usize) * width as usize + x as usize) * 4;
                let pixels = layer.as_srgb8_mut().expect("8-bit");
                pixels[index] = (x * 37 % 256) as u8;
                pixels[index + 1] = (y * 53 % 256) as u8;
                pixels[index + 2] = ((x + y) * 11 % 256) as u8;
                pixels[index + 3] = 255;
            }
        }
        layer
    }

    fn styles() -> LayerStyles {
        LayerStyles {
            stroke: Some(StrokeStyle {
                width: 4.0,
                color: Color::BLACK,
                opacity: 0.8,
            }),
            drop_shadow: Some(DropShadowStyle {
                offset_x: -9.0,
                offset_y: 7.0,
                blur: 12.0,
                color: Color::BLACK,
                opacity: 0.5,
            }),
            outer_glow: Some(OuterGlowStyle {
                radius: 10.0,
                color: Color::WHITE,
                opacity: 0.4,
            }),
        }
    }

    fn assert_equivalent(layer: &Layer) {
        let width = layer.width;
        let height = layer.height;
        let source = layer.as_srgb8_exact();
        let reference = super::render(source, width, height, &layer.styles);
        let bounded = match super::render_styled(layer) {
            LayerBuffer::Srgb8(pixels) => pixels,
            _ => panic!("expected 8-bit"),
        };
        assert_eq!(reference.len(), bounded.len());
        let mut checked = 0u32;
        for index in 0..width as usize * height as usize {
            let r = &reference[index * 4..index * 4 + 4];
            let b = &bounded[index * 4..index * 4 + 4];
            assert_eq!(r[3], b[3], "alpha differs at {index}");
            if r[3] > 0 {
                assert_eq!(r, b, "RGBA differs at {index}");
                checked += 1;
            }
        }
        assert!(checked > 0);

        // The same rectangle at 16-bit must also be exact against the full renderer.
        let promoted: Vec<u16> = source
            .iter()
            .map(|&v| crate::pixel::srgb8_to_srgb16(v))
            .collect();
        let reference16 = super::render(&promoted, width, height, &layer.styles);
        let bounded16 = super::render_styled_srgb16(layer);
        for index in 0..width as usize * height as usize {
            let r = &reference16[index * 4..index * 4 + 4];
            let b = &bounded16[index * 4..index * 4 + 4];
            assert_eq!(r[3], b[3], "16-bit alpha differs at {index}");
            if r[3] > 0 {
                assert_eq!(r, b, "16-bit RGBA differs at {index}");
            }
        }
    }

    #[test]
    fn bounded_render_matches_the_full_renderer_for_many_shapes() {
        let width = 96;
        let height = 64;
        let fixtures: Vec<Vec<(u32, u32)>> = vec![
            vec![(0, 0)],
            vec![(95, 63)],
            vec![(48, 32)],
            (0..96).map(|x| (x, 0)).collect(),
            (0..96).map(|x| (x, 63)).collect(),
            (0..96).flat_map(|x| (0..64).map(move |y| (x, y))).collect(),
            (30..50)
                .flat_map(|x| (20..40).map(move |y| (x, y)))
                .collect(),
        ];
        for art in fixtures {
            let mut layer = layer_with(width, height, &art);
            layer.styles = styles();
            assert_equivalent(&layer);

            layer.styles = LayerStyles {
                stroke: None,
                drop_shadow: None,
                outer_glow: None,
            };
            assert_equivalent(&layer);
        }
    }

    #[test]
    fn bounds_cover_the_effect_and_stay_inside_the_canvas() {
        let layer = layer_with(96, 64, &[(40, 30), (41, 30), (40, 31)]);
        let (x0, y0, x1, y1) =
            super::styled_rect(layer.as_srgb8_exact(), layer.width, layer.height, &styles())
                .expect("content");
        assert!(x0 > 0 && x1 < layer.width, "the horizontal rect is cropped");
        assert!(y0 <= 30 && y1 >= 32);
        assert!(x0 <= 40 && y0 <= 30 && x1 > 41 && y1 > 31);
        // Shadow offset (-9, +7) and blur support are inside the expanded rect.
        assert!(x0 < 40 - 9);
        assert!(y1 > 31 + 7);
    }

    #[test]
    fn an_empty_layer_has_no_bounds_and_renders_transparent() {
        let layer = Layer::new(1, "empty", 32, 32);
        assert!(super::styled_rect(layer.as_srgb8_exact(), 32, 32, &styles()).is_none());
        let rendered = super::render_styled(&layer);
        let LayerBuffer::Srgb8(pixels) = rendered else {
            panic!("8-bit")
        };
        assert!(pixels.iter().all(|&v| v == 0));
    }
}

#[cfg(test)]
mod bounded_randomized {
    use super::*;

    /// Deterministic LCG so failures reproduce from the seed alone.
    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u32 {
            self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1);
            (self.0 >> 33) as u32
        }
        fn range(&mut self, lo: u32, hi: u32) -> u32 {
            lo + self.next() % (hi - lo + 1)
        }
        fn unit(&mut self) -> f32 {
            self.next() as f32 / u32::MAX as f32
        }
    }

    #[test]
    fn randomized_fixtures_match_the_full_renderer() {
        let mut rng = Rng(0x5EED_1234);
        for case in 0..24 {
            let width = rng.range(1, 72);
            let height = rng.range(1, 48);
            let mut layer = Layer::new(1, "random", width, height);
            {
                let pixels = layer.as_srgb8_mut().expect("8-bit");
                let blobs = rng.range(1, 5);
                for _ in 0..blobs {
                    let cx = rng.range(0, width - 1) as i32;
                    let cy = rng.range(0, height - 1) as i32;
                    let r = rng.range(0, 6) as i32;
                    for y in (cy - r).max(0)..=(cy + r).min(height as i32 - 1) {
                        for x in (cx - r).max(0)..=(cx + r).min(width as i32 - 1) {
                            let index = ((y as usize) * width as usize + x as usize) * 4;
                            pixels[index] = (x * 31 % 256) as u8;
                            pixels[index + 1] = (y * 17 % 256) as u8;
                            pixels[index + 2] = case as u8;
                            pixels[index + 3] = rng.range(1, 255) as u8;
                        }
                    }
                }
            }
            let styles = LayerStyles {
                stroke: (rng.next().is_multiple_of(2)).then(|| StrokeStyle {
                    width: rng.unit() * 12.0,
                    color: Color::BLACK,
                    opacity: rng.unit(),
                }),
                drop_shadow: (rng.next().is_multiple_of(2)).then(|| DropShadowStyle {
                    offset_x: (rng.unit() - 0.5) * 40.0,
                    offset_y: (rng.unit() - 0.5) * 40.0,
                    blur: rng.unit() * 20.0,
                    color: Color::BLACK,
                    opacity: rng.unit(),
                }),
                outer_glow: (rng.next().is_multiple_of(2)).then(|| OuterGlowStyle {
                    radius: rng.unit() * 24.0,
                    color: Color::WHITE,
                    opacity: rng.unit(),
                }),
            };
            layer.styles = styles;

            let source = layer.as_srgb8_exact();
            let reference = super::render(source, width, height, &layer.styles);
            let bounded = match super::render_styled(&layer) {
                LayerBuffer::Srgb8(pixels) => pixels,
                _ => panic!("expected 8-bit"),
            };
            for index in 0..width as usize * height as usize {
                let r = &reference[index * 4..index * 4 + 4];
                let b = &bounded[index * 4..index * 4 + 4];
                assert_eq!(r[3], b[3], "case {case}: alpha at {index}");
                if r[3] > 0 {
                    assert_eq!(r, b, "case {case}: RGBA at {index}");
                }
            }
        }
    }
}
