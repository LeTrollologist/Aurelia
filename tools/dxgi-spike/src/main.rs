//! DXGI 10-bit presentation feasibility spike (no external crates).
//!
//! Answers, on the current machine:
//!   1. Can a D3D11 device be created (hardware, then WARP)?
//!   2. Can a composition swapchain be created with `R10G10B10A2_UNORM` and presented?
//!   3. Does the `B8G8R8A8_UNORM` fallback still work?
//!
//! Run with:
//!   cargo run --manifest-path tools/dxgi-spike/Cargo.toml
//!
//! This creates no window. A composition swapchain cannot show a visible 10-bit image without
//! DirectComposition and a 10-bit display chain, so the spike proves device/format/present-call
//! support and the fallback, not visual banding. That gate still needs real hardware.
//!
//! The Win32/COM calls are declared directly instead of adding a `windows` crate dependency: this
//! is a throwaway diagnostic, and keeping the workspace's dependency graph unchanged matters more
//! than ergonomics here.

#![cfg(windows)]

use std::ffi::c_void;
use std::ptr::{null, null_mut};

const S_OK: i32 = 0;
const D3D11_SDK_VERSION: u32 = 7;
const D3D11_CREATE_DEVICE_BGRA_SUPPORT: u32 = 0x20;
const D3D_DRIVER_TYPE_HARDWARE: u32 = 1;
const D3D_DRIVER_TYPE_WARP: u32 = 5;
const D3D_FEATURE_LEVEL_11_1: u32 = 0xb100;
const D3D_FEATURE_LEVEL_11_0: u32 = 0xb000;
const D3D_FEATURE_LEVEL_10_1: u32 = 0xa100;
const D3D_FEATURE_LEVEL_10_0: u32 = 0xa000;

const DXGI_FORMAT_R10G10B10A2_UNORM: u32 = 24;
const DXGI_FORMAT_B8G8R8A8_UNORM: u32 = 87;
const DXGI_USAGE_RENDER_TARGET_OUTPUT: u32 = 0x20;
const DXGI_SCALING_STRETCH: u32 = 0;
const DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL: u32 = 3;
const DXGI_ALPHA_MODE_IGNORE: u32 = 3;

#[repr(C)]
struct Guid {
    data1: u32,
    data2: u16,
    data3: u16,
    data4: [u8; 8],
}

const IID_IDXGIFACTORY2: Guid = Guid {
    data1: 0x50c8_3a1c,
    data2: 0xe072,
    data3: 0x4c48,
    data4: [0x87, 0xb0, 0x36, 0x30, 0xfa, 0x36, 0xa6, 0xd0],
};

const IID_IDXGIDEVICE: Guid = Guid {
    data1: 0x54ec_77fa,
    data2: 0x1377,
    data3: 0x44e6,
    data4: [0x8c, 0x32, 0x88, 0xfd, 0x5f, 0x44, 0xc8, 0x4c],
};

const IID_IDXGIOUTPUT6: Guid = Guid {
    data1: 0x0683_46e8,
    data2: 0xaaec,
    data3: 0x4b84,
    data4: [0xad, 0xd7, 0x13, 0x7f, 0x51, 0x3f, 0x77, 0xa1],
};

#[repr(C)]
struct SampleDesc {
    count: u32,
    quality: u32,
}

#[repr(C)]
struct SwapChainDesc1 {
    width: u32,
    height: u32,
    format: u32,
    stereo: i32,
    sample_desc: SampleDesc,
    buffer_usage: u32,
    buffer_count: u32,
    scaling: u32,
    swap_effect: u32,
    alpha_mode: u32,
    flags: u32,
}

#[repr(C)]
struct Rect {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

#[repr(C)]
struct OutputDesc1 {
    device_name: [u16; 32],
    desktop_coordinates: Rect,
    attached_to_desktop: i32,
    rotation: u32,
    monitor: isize,
    bits_per_color: u32,
    color_space: u32,
    red_primary: [f32; 2],
    green_primary: [f32; 2],
    blue_primary: [f32; 2],
    white_point: [f32; 2],
    min_luminance: f32,
    max_luminance: f32,
    max_full_frame_luminance: f32,
}

#[link(name = "d3d11")]
extern "system" {
    fn D3D11CreateDevice(
        adapter: *mut c_void,
        driver_type: u32,
        software: *mut c_void,
        flags: u32,
        feature_levels: *const u32,
        feature_level_count: u32,
        sdk_version: u32,
        device: *mut *mut c_void,
        feature_level: *mut u32,
        context: *mut *mut c_void,
    ) -> i32;
}

#[link(name = "dxgi")]
extern "system" {
    fn CreateDXGIFactory2(flags: u32, riid: *const Guid, factory: *mut *mut c_void) -> i32;
}

#[link(name = "user32")]
extern "system" {
    fn CreateWindowExW(
        ex_style: u32,
        class_name: *const u16,
        window_name: *const u16,
        style: u32,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        parent: isize,
        menu: isize,
        instance: isize,
        param: *mut c_void,
    ) -> isize;
    fn DestroyWindow(window: isize) -> i32;
}

type ReleaseFn = unsafe extern "system" fn(*mut c_void) -> u32;
type QueryInterfaceFn =
    unsafe extern "system" fn(*mut c_void, *const Guid, *mut *mut c_void) -> i32;
type GetAdapterFn = unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> i32;
type EnumOutputsFn = unsafe extern "system" fn(*mut c_void, u32, *mut *mut c_void) -> i32;
type GetDesc1Fn = unsafe extern "system" fn(*mut c_void, *mut OutputDesc1) -> i32;
type CreateSwapChainForCompositionFn = unsafe extern "system" fn(
    *mut c_void,
    *mut c_void,
    *const SwapChainDesc1,
    *mut c_void,
    *mut *mut c_void,
) -> i32;
type CreateSwapChainForHwndFn = unsafe extern "system" fn(
    *mut c_void,
    *mut c_void,
    isize,
    *const SwapChainDesc1,
    *const c_void,
    *mut c_void,
    *mut *mut c_void,
) -> i32;
type PresentFn = unsafe extern "system" fn(*mut c_void, u32, u32) -> i32;

unsafe fn vtable_entry<T: Copy>(object: *mut c_void, index: usize) -> T {
    let vtable = *(object as *mut *mut usize);
    let function = *vtable.add(index);
    std::mem::transmute_copy(&function)
}

unsafe fn release(object: *mut c_void) {
    if !object.is_null() {
        let release: ReleaseFn = vtable_entry(object, 2);
        let _ = release(object);
    }
}

unsafe fn create_device(driver: u32) -> Option<(*mut c_void, *mut c_void, u32)> {
    let levels = [
        D3D_FEATURE_LEVEL_11_1,
        D3D_FEATURE_LEVEL_11_0,
        D3D_FEATURE_LEVEL_10_1,
        D3D_FEATURE_LEVEL_10_0,
    ];
    let mut device: *mut c_void = null_mut();
    let mut context: *mut c_void = null_mut();
    let mut level = 0u32;
    let hr = D3D11CreateDevice(
        null_mut(),
        driver,
        null_mut(),
        D3D11_CREATE_DEVICE_BGRA_SUPPORT,
        levels.as_ptr(),
        levels.len() as u32,
        D3D11_SDK_VERSION,
        &mut device,
        &mut level,
        &mut context,
    );
    if hr == S_OK && !device.is_null() {
        Some((device, context, level))
    } else {
        println!("  d3d11 device failed: hr=0x{hr:08x}");
        None
    }
}

unsafe fn create_factory() -> Option<*mut c_void> {
    let mut factory: *mut c_void = null_mut();
    let hr = CreateDXGIFactory2(0, &IID_IDXGIFACTORY2, &mut factory);
    if hr == S_OK && !factory.is_null() {
        Some(factory)
    } else {
        println!("  CreateDXGIFactory2 failed: hr=0x{hr:08x}");
        None
    }
}

unsafe fn query_interface(object: *mut c_void, iid: &Guid) -> Option<*mut c_void> {
    let query: QueryInterfaceFn = vtable_entry(object, 0);
    let mut result: *mut c_void = null_mut();
    let hr = query(object, iid, &mut result);
    if hr == S_OK && !result.is_null() {
        Some(result)
    } else {
        println!("  QueryInterface failed: hr=0x{hr:08x}");
        None
    }
}

unsafe fn report_outputs(dxgi_device: *mut c_void) -> bool {
    let get_adapter: GetAdapterFn = vtable_entry(dxgi_device, 7);
    let mut adapter: *mut c_void = null_mut();
    let hr = get_adapter(dxgi_device, &mut adapter);
    if hr != S_OK || adapter.is_null() {
        println!("  output query: GetAdapter failed: hr=0x{hr:08x}");
        return false;
    }

    let mut wide = false;
    let enum_outputs: EnumOutputsFn = vtable_entry(adapter, 7);
    let mut index = 0u32;
    loop {
        let mut output: *mut c_void = null_mut();
        let hr = enum_outputs(adapter, index, &mut output);
        if hr != S_OK || output.is_null() {
            if index == 0 {
                println!("  output query: no outputs (hr=0x{hr:08x})");
            }
            break;
        }
        index += 1;

        if let Some(output6) = query_interface(output, &IID_IDXGIOUTPUT6) {
            let get_desc1: GetDesc1Fn = vtable_entry(output6, 27);
            let mut desc: OutputDesc1 = std::mem::zeroed();
            let hr = get_desc1(output6, &mut desc);
            if hr == S_OK {
                let name: String = desc
                    .device_name
                    .iter()
                    .take_while(|&&ch| ch != 0)
                    .map(|&ch| char::from_u32(ch as u32).unwrap_or('?'))
                    .collect();
                println!(
                    "display {name}: bits_per_color={} color_space={} max_luminance={:.1}",
                    desc.bits_per_color, desc.color_space, desc.max_luminance
                );
                wide |= desc.bits_per_color > 8;
            } else {
                println!("  output {index}: GetDesc1 failed: hr=0x{hr:08x}");
            }
            release(output6);
        } else {
            println!("  output {index}: IDXGIOutput6 unavailable");
        }
        release(output);
    }

    release(adapter);
    wide
}

unsafe fn create_swapchain(factory: *mut c_void, device: *mut c_void, format: u32) -> Option<*mut c_void> {
    let desc = SwapChainDesc1 {
        width: 320,
        height: 200,
        format,
        stereo: 0,
        sample_desc: SampleDesc { count: 1, quality: 0 },
        buffer_usage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
        buffer_count: 2,
        scaling: DXGI_SCALING_STRETCH,
        swap_effect: DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL,
        alpha_mode: DXGI_ALPHA_MODE_IGNORE,
        flags: 0,
    };
    let mut swapchain: *mut c_void = null_mut();
    // IDXGIFactory2::CreateSwapChainForComposition is vtable slot 24 in the Windows SDK
    // (IUnknown 0-2, IDXGIObject 3-6, IDXGIFactory 7-11, IDXGIFactory1 12-13,
    // IDXGIFactory2 14-24 with composition last).
    let create: CreateSwapChainForCompositionFn = vtable_entry(factory, 24);
    let hr = create(factory, device, &desc, null_mut(), &mut swapchain);
    if hr == S_OK && !swapchain.is_null() {
        Some(swapchain)
    } else {
        println!("  swapchain format {format} failed: hr=0x{hr:08x}");
        None
    }
}

unsafe fn present(swapchain: *mut c_void) -> i32 {
    let present: PresentFn = vtable_entry(swapchain, 8); // IDXGISwapChain::Present
    present(swapchain, 0, 0)
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

/// A hidden top-level window for the HWND swapchain attempt. It is never shown.
unsafe fn create_hidden_window() -> isize {
    let class = wide("STATIC");
    let title = wide("Aurelia DXGI spike");
    CreateWindowExW(
        0,
        class.as_ptr(),
        title.as_ptr(),
        0,
        0,
        0,
        320,
        200,
        0,
        0,
        0,
        null_mut(),
    )
}

unsafe fn create_swapchain_hwnd(
    factory: *mut c_void,
    device: *mut c_void,
    window: isize,
    format: u32,
) -> Option<*mut c_void> {
    if window == 0 {
        return None;
    }
    let desc = SwapChainDesc1 {
        width: 320,
        height: 200,
        format,
        stereo: 0,
        sample_desc: SampleDesc { count: 1, quality: 0 },
        buffer_usage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
        buffer_count: 2,
        scaling: DXGI_SCALING_STRETCH,
        swap_effect: DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL,
        alpha_mode: DXGI_ALPHA_MODE_IGNORE,
        flags: 0,
    };
    let mut swapchain: *mut c_void = null_mut();
    // IDXGIFactory2::CreateSwapChainForHwnd is vtable slot 15.
    let create: CreateSwapChainForHwndFn = vtable_entry(factory, 15);
    let hr = create(
        factory,
        device,
        window,
        &desc,
        null(),
        null_mut(),
        &mut swapchain,
    );
    if hr == S_OK && !swapchain.is_null() {
        Some(swapchain)
    } else {
        println!("  hwnd swapchain format {format} failed: hr=0x{hr:08x}");
        None
    }
}

fn main() {
    println!("DXGI 10-bit presentation spike");
    let mut device_created = false;
    let mut wide_swapchain = false;
    let mut fallback_swapchain = false;
    let mut wide_display = false;

    for (label, driver) in [("hardware", D3D_DRIVER_TYPE_HARDWARE), ("warp", D3D_DRIVER_TYPE_WARP)] {
        unsafe {
            let Some((device, context, level)) = create_device(driver) else {
                continue;
            };
            device_created = true;
            println!("d3d11: ok (driver={label}, feature_level=0x{level:04x})");
            let Some(factory) = create_factory() else {
                release(context);
                release(device);
                continue;
            };
            println!("dxgi: factory2 ok");
            let Some(dxgi_device) = query_interface(device, &IID_IDXGIDEVICE) else {
                release(factory);
                release(context);
                release(device);
                continue;
            };
            wide_display = report_outputs(dxgi_device);

            if let Some(swapchain) = create_swapchain(factory, dxgi_device, DXGI_FORMAT_R10G10B10A2_UNORM) {
                let hr = present(swapchain);
                println!(
                    "composition swapchain 10-bit: created (present hr=0x{hr:08x}{})",
                    if hr == S_OK { "" } else { " — composition swapchains may need a commit" }
                );
                wide_swapchain = true;
                release(swapchain);
            }
            if let Some(swapchain) = create_swapchain(factory, dxgi_device, DXGI_FORMAT_B8G8R8A8_UNORM) {
                let hr = present(swapchain);
                println!("composition swapchain 8-bit fallback: created (present hr=0x{hr:08x})");
                fallback_swapchain = true;
                release(swapchain);
            }

            let window = create_hidden_window();
            if window != 0 {
                if let Some(swapchain) =
                    create_swapchain_hwnd(factory, dxgi_device, window, DXGI_FORMAT_R10G10B10A2_UNORM)
                {
                    let hr = present(swapchain);
                    println!(
                        "hwnd swapchain 10-bit: created (present hr=0x{hr:08x})"
                    );
                    wide_swapchain = true;
                    release(swapchain);
                }
                if let Some(swapchain) =
                    create_swapchain_hwnd(factory, dxgi_device, window, DXGI_FORMAT_B8G8R8A8_UNORM)
                {
                    let hr = present(swapchain);
                    println!("hwnd swapchain 8-bit fallback: created (present hr=0x{hr:08x})");
                    fallback_swapchain = true;
                    release(swapchain);
                }
                DestroyWindow(window);
            } else {
                println!("  hidden window creation failed");
            }

            release(dxgi_device);
            release(factory);
            release(context);
            release(device);
            break;
        }
    }

    if !device_created {
        eprintln!("result: no D3D11 device; GDI is the only path on this machine");
        std::process::exit(1);
    }
    println!(
        "result: device=ok 10-bit-swapchain={} 8-bit-fallback={} 10-bit-display={}",
        if wide_swapchain { "ok" } else { "failed" },
        if fallback_swapchain { "ok" } else { "failed" },
        if wide_display { "yes" } else { "no" }
    );
    println!("note: no window is shown; the visible-banding gate still needs a real window and a 10-bit display chain");
    let _ = null::<u8>();
    std::process::exit(if wide_swapchain { 0 } else { 2 });
}
