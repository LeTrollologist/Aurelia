//! ICC colour profiles and conversion.
//!
//! `moxcms` is the engine: pure Rust, no C toolchain, and it already supports the 8- and 16-bit
//! RGBA layouts Aurelia uses. This module is the seam that keeps its types out of the document
//! model and gives the rest of the editor one place to attach, validate and convert profiles.
//!
//! The load-bearing rule: **untagged documents are not converted**. Empty profiles preserve the
//! existing sRGB byte-for-byte behaviour, and every new conversion path has to prove that

use std::sync::Arc;

use moxcms::{ColorProfile, Layout, TransformOptions};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ColorProfileError {
    Empty,
    Invalid(String),
    Conversion(String),
}

impl std::fmt::Display for ColorProfileError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => write!(formatter, "ICC profile is empty"),
            Self::Invalid(reason) => write!(formatter, "invalid ICC profile: {reason}"),
            Self::Conversion(reason) => write!(formatter, "colour conversion failed: {reason}"),
        }
    }
}

impl std::error::Error for ColorProfileError {}

/// A validated ICC profile plus the bytes needed to persist it.
#[derive(Clone)]
pub struct IccProfile {
    bytes: Vec<u8>,
    inner: Arc<ColorProfile>,
}

impl IccProfile {
    /// Parses and validates an ICC profile. Bytes are retained for exact save/round-trip.
    pub fn from_icc(bytes: Vec<u8>) -> Result<Self, ColorProfileError> {
        if bytes.is_empty() {
            return Err(ColorProfileError::Empty);
        }
        let inner = ColorProfile::new_from_slice(&bytes)
            .map_err(|error| ColorProfileError::Invalid(error.to_string()))?;
        Ok(Self {
            bytes,
            inner: Arc::new(inner),
        })
    }

    /// The built-in sRGB profile. Used as the assumed working space for untagged documents.
    pub fn srgb() -> Self {
        Self {
            bytes: Vec::new(),
            inner: Arc::new(ColorProfile::new_srgb()),
        }
    }

    /// Original ICC bytes, empty for the synthetic sRGB profile.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub fn is_synthetic(&self) -> bool {
        self.bytes.is_empty()
    }

    /// Converts straight RGBA8 in place.
    pub fn convert_rgba8(
        &self,
        destination: &IccProfile,
        pixels: &mut [u8],
    ) -> Result<(), ColorProfileError> {
        if self.same_as(destination) {
            return Ok(());
        }
        let transform = self
            .inner
            .create_transform_8bit(
                Layout::Rgba,
                &destination.inner,
                Layout::Rgba,
                TransformOptions::default(),
            )
            .map_err(|error| ColorProfileError::Conversion(error.to_string()))?;
        let mut converted = vec![0u8; pixels.len()];
        transform
            .transform(pixels, &mut converted)
            .map_err(|error| ColorProfileError::Conversion(error.to_string()))?;
        pixels.copy_from_slice(&converted);
        Ok(())
    }

    /// Converts straight RGBA16 in place.
    pub fn convert_rgba16(
        &self,
        destination: &IccProfile,
        pixels: &mut [u16],
    ) -> Result<(), ColorProfileError> {
        if self.same_as(destination) {
            return Ok(());
        }
        let transform = self
            .inner
            .create_transform_16bit(
                Layout::Rgba,
                &destination.inner,
                Layout::Rgba,
                TransformOptions::default(),
            )
            .map_err(|error| ColorProfileError::Conversion(error.to_string()))?;
        let mut converted = vec![0u16; pixels.len()];
        transform
            .transform(pixels, &mut converted)
            .map_err(|error| ColorProfileError::Conversion(error.to_string()))?;
        pixels.copy_from_slice(&converted);
        Ok(())
    }

    /// Approximate soft proof for the 8-bit viewport: source -> proof -> sRGB display.
    ///
    /// This is a simulation preview, not a display claim. It intentionally routes through the
    /// proof profile in both directions so out-of-gamut colours pass through the proof gamut and
    /// clip, then are shown in the current 8-bit sRGB viewport.
    pub fn simulate_proof_rgba8(
        &self,
        proof: &IccProfile,
        display: &IccProfile,
        pixels: &mut [u8],
    ) -> Result<(), ColorProfileError> {
        self.convert_rgba8(proof, pixels)?;
        proof.convert_rgba8(display, pixels)
    }

    /// Approximate soft proof for the 16-bit viewport: source -> proof -> sRGB display.
    ///
    /// The 10-bit presentation path uses this variant so proofing does not force an 8-bit
    /// downconversion before the frame reaches the swapchain.
    pub fn simulate_proof_rgba16(
        &self,
        proof: &IccProfile,
        display: &IccProfile,
        pixels: &mut [u16],
    ) -> Result<(), ColorProfileError> {
        self.convert_rgba16(proof, pixels)?;
        proof.convert_rgba16(display, pixels)
    }

    /// Whether this profile describes the same colour space as `other`, by ICC bytes.
    pub fn same_as(&self, other: &IccProfile) -> bool {
        self.bytes == other.bytes
    }
}

impl std::fmt::Debug for IccProfile {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("IccProfile")
            .field("bytes", &self.bytes.len())
            .field("synthetic", &self.is_synthetic())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_and_garbage_profiles_are_rejected() {
        assert_eq!(
            IccProfile::from_icc(Vec::new()).unwrap_err(),
            ColorProfileError::Empty
        );
        assert!(matches!(
            IccProfile::from_icc(vec![0u8; 128]),
            Err(ColorProfileError::Invalid(_))
        ));
    }

    #[test]
    fn srgb_to_srgb_is_identity_for_8_and_16_bit() {
        let srgb = IccProfile::srgb();
        let mut rgba8 = vec![0u8, 1, 127, 255, 255, 128, 64, 0, 10, 20, 30, 40];
        let original8 = rgba8.clone();
        srgb.convert_rgba8(&srgb, &mut rgba8).expect("convert8");
        assert_eq!(
            rgba8, original8,
            "untagged/sRGB path must not move 8-bit bytes"
        );

        let mut rgba16 = vec![0u16, 257, 32768, 65535, 300, 1000, 40000, 12345];
        let original16 = rgba16.clone();
        srgb.convert_rgba16(&srgb, &mut rgba16).expect("convert16");
        assert_eq!(rgba16, original16, "sRGB 16-bit path must not quantise");
    }

    #[test]
    fn alpha_survives_conversion() {
        let srgb = IccProfile::srgb();
        let mut rgba = vec![12u8, 34, 56, 78, 90, 12, 34, 0];
        srgb.convert_rgba8(&srgb, &mut rgba).expect("convert");
        assert_eq!(rgba[3], 78);
        assert_eq!(rgba[7], 0);
    }
}

#[cfg(test)]
mod document_profile_tests {
    use super::*;
    use crate::document::Document;
    use crate::layer::LayerBuffer;

    fn srgb_icc() -> Vec<u8> {
        moxcms::ColorProfile::new_srgb()
            .encode()
            .expect("encode sRGB")
    }

    #[test]
    fn document_attaches_and_assigns_without_moving_pixels() {
        let icc = srgb_icc();
        let mut doc = Document::new(2, 2);
        doc.layers[0].as_srgb8_mut().expect("8-bit")[0] = 42;
        let before = doc.layers[0].as_srgb8_exact().to_vec();
        doc.set_icc_profile(icc.clone()).expect("attach");
        assert_eq!(doc.icc_profile(), Some(icc.as_slice()));
        assert_eq!(doc.layers[0].as_srgb8_exact(), before);

        doc.convert_to_profile(&IccProfile::from_icc(icc).expect("parse"))
            .expect("convert");
        assert_eq!(
            doc.layers[0].as_srgb8_exact(),
            before,
            "same-profile conversion is a no-op"
        );
    }

    #[test]
    fn untagged_conversion_to_srgb_is_a_no_op() {
        let mut doc = Document::new(2, 2);
        let before = doc.layers[0].as_srgb8_exact().to_vec();
        doc.convert_to_profile(&IccProfile::srgb())
            .expect("convert");
        assert_eq!(doc.layers[0].as_srgb8_exact(), before);
        assert!(doc.icc_profile().is_none());
    }

    #[test]
    fn invalid_profiles_are_refused_and_clearing_restores_untagged() {
        let mut doc = Document::new(1, 1);
        assert!(doc.set_icc_profile(vec![0u8; 128]).is_err());
        doc.set_icc_profile(srgb_icc()).expect("attach");
        doc.clear_icc_profile();
        assert!(doc.icc_profile().is_none());
        assert!(matches!(doc.layers[0].buffer(), LayerBuffer::Srgb8(_)));
    }
}

#[cfg(test)]
mod profile_undo_tests {
    use super::*;
    use crate::document::Document;
    use crate::history::{Command, LayerPixelsDeltaCommand, ProfileConversionCommand};

    #[test]
    fn conversion_undo_restores_pixels_and_the_stored_profile() {
        let srgb_bytes = moxcms::ColorProfile::new_srgb().encode().expect("sRGB");
        let p3_bytes = moxcms::ColorProfile::new_display_p3().encode().expect("P3");
        let mut doc = Document::new(4, 4);
        doc.layers[0].as_srgb8_mut().expect("8-bit")[..4].copy_from_slice(&[255, 0, 0, 255]);
        doc.set_icc_profile(srgb_bytes.clone()).expect("attach");
        let layer_id = doc.layers[0].id;
        let before_pixels = doc.layers[0].storage_bytes();
        let before_profile = doc.icc_profile().map(<[u8]>::to_vec);

        doc.convert_to_profile(&IccProfile::from_icc(p3_bytes.clone()).expect("p3"))
            .expect("convert");
        let after_pixels = doc.layers[0].storage_bytes();
        assert_ne!(before_pixels, after_pixels, "P3 conversion must move red");
        assert_eq!(doc.icc_profile(), Some(p3_bytes.as_slice()));

        let delta = LayerPixelsDeltaCommand::new(
            layer_id,
            "Convert",
            before_pixels.clone(),
            after_pixels.clone(),
        );
        let mut cmd = ProfileConversionCommand::new(
            "Convert to ICC",
            before_profile.clone(),
            Some(p3_bytes.clone()),
            vec![delta],
        );
        cmd.undo(&mut doc);
        assert_eq!(doc.layers[0].storage_bytes(), before_pixels);
        assert_eq!(doc.icc_profile(), before_profile.as_deref());

        cmd.redo(&mut doc);
        assert_eq!(doc.layers[0].storage_bytes(), after_pixels);
        assert_eq!(doc.icc_profile(), Some(p3_bytes.as_slice()));
    }
}

#[cfg(test)]
mod soft_proof_tests {
    use super::*;

    #[test]
    fn soft_proof_routes_through_the_proof_profile_and_back() {
        let srgb = IccProfile::srgb();
        let p3 = IccProfile::from_icc(moxcms::ColorProfile::new_display_p3().encode().expect("P3"))
            .expect("parse P3");
        let mut pixels = vec![255u8, 0, 0, 255, 0, 255, 0, 128];
        let original = pixels.clone();
        srgb.simulate_proof_rgba8(&p3, &srgb, &mut pixels)
            .expect("simulate");
        assert_ne!(pixels, original, "the proof round trip must be visible");
        assert_eq!(pixels[3], 255);
        assert_eq!(pixels[7], 128, "alpha is untouched by soft proof");
    }

    #[test]
    fn sixteen_bit_soft_proof_keeps_sixteen_bit_alpha() {
        let srgb = IccProfile::srgb();
        let p3 = IccProfile::from_icc(moxcms::ColorProfile::new_display_p3().encode().expect("P3"))
            .expect("parse P3");
        let mut pixels = vec![12_345u16, 40_000, 20_000, 65_535, 0, 65_535, 0, 32_768];
        let original = pixels.clone();
        srgb.simulate_proof_rgba16(&p3, &srgb, &mut pixels)
            .expect("simulate");
        assert_ne!(pixels, original, "the proof round trip must be visible");
        assert_eq!(pixels[3], 65_535);
        assert_eq!(pixels[7], 32_768, "alpha is untouched by soft proof");
    }
}
