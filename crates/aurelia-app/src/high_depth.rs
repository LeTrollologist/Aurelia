//! 10-bit canvas presentation helpers.
//!
//! The compositor still produces an 8-bit frame for the existing GDI path. This module adds
//! the packed 10-bit pixel format and a D3D11/DXGI flip-model presenter used only when the
//! containing output actually reports at least 10 bits per colour. Any failure returns `None`
//! so the caller keeps the byte-compatible GDI blit.

use windows::core::{Interface, BOOL};
use windows::Win32::Foundation::{HMODULE, HWND};
use windows::Win32::Graphics::Direct3D::{D3D_DRIVER_TYPE_HARDWARE, D3D_FEATURE_LEVEL_11_1};
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D, D3D11_CPU_ACCESS_WRITE,
    D3D11_MAP_WRITE_DISCARD, D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC, D3D11_USAGE_DYNAMIC,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_ALPHA_MODE_IGNORE, DXGI_FORMAT_R10G10B10A2_UNORM, DXGI_SAMPLE_DESC,
};
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory2, IDXGIFactory2, IDXGIOutput, IDXGIOutput6, IDXGISwapChain1,
    DXGI_CREATE_FACTORY_FLAGS, DXGI_SCALING_STRETCH, DXGI_SWAP_CHAIN_DESC1, DXGI_SWAP_CHAIN_FLAG,
    DXGI_SWAP_EFFECT_FLIP_DISCARD, DXGI_USAGE_RENDER_TARGET_OUTPUT,
};

/// Convert one straight 16-bit channel to the rounded 10-bit range.
#[allow(dead_code)]
#[inline]
pub fn channel16_to_10(value: u16) -> u32 {
    ((value as u32 * 1023 + 32_767) / 65_535).min(1023)
}

/// Convert one 8-bit channel to the rounded 10-bit range.
#[inline]
pub fn channel8_to_10(value: u8) -> u32 {
    ((value as u32 * 1023 + 127) / 255).min(1023)
}

/// Pack straight RGBA16 into DXGI `R10G10B10A2_UNORM` bit order.
#[allow(dead_code)]
pub fn rgba16_to_px10(r: u16, g: u16, b: u16, a: u16) -> u32 {
    let a2 = ((a as u32 * 3 + 32_767) / 65_535).min(3);
    channel16_to_10(r) | (channel16_to_10(g) << 10) | (channel16_to_10(b) << 20) | (a2 << 30)
}

/// Pack a `0xAARRGGBB` pixel into `R10G10B10A2_UNORM` bit order.
pub fn rgba8_to_px10(color: u32) -> u32 {
    let r = ((color >> 16) & 0xFF) as u8;
    let g = ((color >> 8) & 0xFF) as u8;
    let b = (color & 0xFF) as u8;
    let a2 = if (color >> 24) == 0 { 0 } else { 3 };
    channel8_to_10(r) | (channel8_to_10(g) << 10) | (channel8_to_10(b) << 20) | (a2 << 30)
}

/// Convert a packed 10-bit pixel back to `0xAARRGGBB` for the 8-bit overlay/canvas buffer.
pub fn px10_to_rgba8(pixel: u32) -> u32 {
    let q = |value: u32| ((value.min(1023) * 255 + 511) / 1023) as u32;
    let r = q(pixel & 1023);
    let g = q((pixel >> 10) & 1023);
    let b = q((pixel >> 20) & 1023);
    0xFF00_0000 | (r << 16) | (g << 8) | b
}

/// A D3D11 flip-model swapchain presenting packed 10-bit frames to an existing HWND.
pub struct TenBitPresenter {
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    swapchain: IDXGISwapChain1,
    texture: Option<ID3D11Texture2D>,
    width: u32,
    height: u32,
}

impl TenBitPresenter {
    /// Create the presenter only when the containing output is genuinely 10-bit.
    ///
    /// `force` is for hardware diagnostics; it bypasses the output-depth check but still takes
    /// the same fallback path if device/swapchain/texture creation fails.
    pub fn new(hwnd: HWND, width: u32, height: u32, force: bool) -> Option<Self> {
        if width == 0 || height == 0 {
            return None;
        }
        unsafe {
            let levels = [D3D_FEATURE_LEVEL_11_1];
            let mut device: Option<ID3D11Device> = None;
            let mut context: Option<ID3D11DeviceContext> = None;
            let mut level = Default::default();
            D3D11CreateDevice(
                None,
                D3D_DRIVER_TYPE_HARDWARE,
                HMODULE::default(),
                windows::Win32::Graphics::Direct3D11::D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                Some(&levels),
                D3D11_SDK_VERSION,
                Some(&mut device),
                Some(&mut level),
                Some(&mut context),
            )
            .ok()?;
            let device = device?;
            let context = context?;

            let factory: IDXGIFactory2 = CreateDXGIFactory2(DXGI_CREATE_FACTORY_FLAGS(0)).ok()?;
            let desc = DXGI_SWAP_CHAIN_DESC1 {
                Width: width,
                Height: height,
                Format: DXGI_FORMAT_R10G10B10A2_UNORM,
                Stereo: BOOL::from(false),
                SampleDesc: DXGI_SAMPLE_DESC {
                    Count: 1,
                    Quality: 0,
                },
                BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
                BufferCount: 2,
                Scaling: DXGI_SCALING_STRETCH,
                SwapEffect: DXGI_SWAP_EFFECT_FLIP_DISCARD,
                AlphaMode: DXGI_ALPHA_MODE_IGNORE,
                Flags: 0,
            };
            let swapchain = factory
                .CreateSwapChainForHwnd(&device, hwnd, &desc, None, None::<&IDXGIOutput>)
                .ok()?;

            if !force {
                let output = swapchain.GetContainingOutput().ok()?;
                let output = output.cast::<IDXGIOutput6>().ok()?;
                let desc = output.GetDesc1().ok()?;
                if desc.BitsPerColor < 10 {
                    return None;
                }
            }

            let mut presenter = Self {
                device,
                context,
                swapchain,
                texture: None,
                width,
                height,
            };
            presenter.ensure_texture()?;
            Some(presenter)
        }
    }

    fn ensure_texture(&mut self) -> Option<()> {
        if self.texture.is_some() {
            return Some(());
        }
        let desc = D3D11_TEXTURE2D_DESC {
            Width: self.width,
            Height: self.height,
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_R10G10B10A2_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Usage: D3D11_USAGE_DYNAMIC,
            BindFlags: 0,
            CPUAccessFlags: D3D11_CPU_ACCESS_WRITE.0 as u32,
            MiscFlags: 0,
        };
        let mut texture: Option<ID3D11Texture2D> = None;
        unsafe {
            self.device
                .CreateTexture2D(&desc, None, Some(&mut texture))
                .ok()?;
        }
        self.texture = texture;
        Some(())
    }

    /// Resize the swapchain; returns false so the caller can fall back permanently.
    pub fn resize(&mut self, width: u32, height: u32) -> bool {
        if width == 0 || height == 0 {
            return false;
        }
        if width == self.width && height == self.height && self.texture.is_some() {
            return true;
        }
        self.texture = None;
        unsafe {
            if self
                .swapchain
                .ResizeBuffers(
                    2,
                    width,
                    height,
                    DXGI_FORMAT_R10G10B10A2_UNORM,
                    DXGI_SWAP_CHAIN_FLAG(0),
                )
                .is_err()
            {
                return false;
            }
        }
        self.width = width;
        self.height = height;
        self.ensure_texture().is_some()
    }

    /// Upload one packed frame and present it. Any failure returns false; the caller falls back.
    pub fn present(&mut self, pixels: &[u32]) -> bool {
        let expected = self.width as usize * self.height as usize;
        if pixels.len() < expected || self.texture.is_none() {
            return false;
        }
        unsafe {
            let texture = self.texture.as_ref().unwrap();
            let mut mapped = Default::default();
            if self
                .context
                .Map(texture, 0, D3D11_MAP_WRITE_DISCARD, 0, Some(&mut mapped))
                .is_err()
            {
                return false;
            }
            let dst = mapped.pData as *mut u32;
            if dst.is_null() {
                let _ = self.context.Unmap(texture, 0);
                return false;
            }
            let row_pitch = mapped.RowPitch as usize / 4;
            for y in 0..self.height as usize {
                let src = &pixels[y * self.width as usize..(y + 1) * self.width as usize];
                std::ptr::copy_nonoverlapping(
                    src.as_ptr(),
                    dst.add(y * row_pitch),
                    self.width as usize,
                );
            }
            let _ = self.context.Unmap(texture, 0);

            let backbuffer: ID3D11Texture2D = match self.swapchain.GetBuffer(0) {
                Ok(buffer) => buffer,
                Err(_) => return false,
            };
            self.context.CopyResource(&backbuffer, texture);
            self.swapchain.Present(1, Default::default()).is_ok()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conversions_cover_black_white_and_midpoints() {
        assert_eq!(channel8_to_10(0), 0);
        assert_eq!(channel8_to_10(255), 1023);
        assert_eq!(channel16_to_10(0), 0);
        assert_eq!(channel16_to_10(65_535), 1023);
        assert_eq!(rgba16_to_px10(0, 0, 0, 65_535) & 1023, 0);
        assert_eq!(rgba16_to_px10(65_535, 0, 0, 65_535) & 1023, 1023);
        assert_eq!(rgba16_to_px10(65_535, 0, 0, 65_535) >> 30, 3);
    }

    #[test]
    fn packed_pixels_round_trip_to_the_8_bit_range() {
        for value in [0u8, 1, 64, 127, 128, 200, 254, 255] {
            let color = 0xFF00_0000 | ((value as u32) << 16);
            let packed = rgba8_to_px10(color);
            let back = px10_to_rgba8(packed);
            assert_eq!((back >> 16) & 0xFF, value as u32);
            assert_eq!(back >> 24, 0xFF);
        }
    }
}
