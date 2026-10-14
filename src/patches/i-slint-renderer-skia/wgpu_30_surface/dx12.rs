// Copyright © SixtyFPS GmbH <info@slint.dev>
// SPDX-License-Identifier: GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0

use windows::Win32::Graphics::Direct3D12::{D3D12_RESOURCE_STATE_PRESENT, ID3D12Resource};
use windows::Win32::Graphics::Dxgi::Common::DXGI_STANDARD_MULTISAMPLE_QUALITY_PATTERN;
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_FORMAT_R8G8B8A8_UNORM, DXGI_FORMAT_R8G8B8A8_UNORM_SRGB,
};

use wgpu_30 as wgpu;

/// # Safety
/// `resource` must be a valid D3D12 resource for the lifetime of the returned Surface.
unsafe fn wrap_dx12_texture(
    width: i32,
    height: i32,
    gr_context: &mut skia_safe::gpu::DirectContext,
    resource: ID3D12Resource,
    dxgi_format: windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT,
    color_type: skia_safe::ColorType,
    color_space: skia_safe::ColorSpace,
    surface_props: Option<&skia_safe::SurfaceProps>,
) -> Option<skia_safe::Surface> {
    let texture_info = skia_safe::gpu::d3d::TextureResourceInfo {
        resource,
        alloc: None,
        resource_state: D3D12_RESOURCE_STATE_PRESENT,
        format: dxgi_format,
        sample_count: 1,
        level_count: 1,
        sample_quality_pattern: DXGI_STANDARD_MULTISAMPLE_QUALITY_PATTERN,
        protected: skia_safe::gpu::Protected::No,
    };
    let backend_render_target =
        skia_safe::gpu::backend_render_targets::make_d3d((width, height), &texture_info);
    skia_safe::gpu::surfaces::wrap_backend_render_target(
        gr_context,
        &backend_render_target,
        skia_safe::gpu::SurfaceOrigin::TopLeft,
        color_type,
        color_space,
        surface_props,
    )
}

// Concat: the window's text is drawn with ClearType, as every other program
// on a Windows desk is, when the desk has it on (jub0t/concat#278). Greyscale
// coverage on a 1x screen reads soft and grey beside the system's own text;
// LCD coverage triples the horizontal resolution a stem is drawn at.
//
// The settings are the ones the Control Panel's "Adjust ClearType text"
// writes, read per frame so turning ClearType off takes effect at once.
// SystemParametersInfoW is declared here rather than through the `windows`
// crate so the copy's dependency features stay as published.
#[link(name = "user32")]
unsafe extern "system" {
    fn SystemParametersInfoW(
        action: u32,
        param: u32,
        pv_param: *mut core::ffi::c_void,
        win_ini: u32,
    ) -> i32;
}

/// The surface properties for the window's swapchain: the subpixel order of
/// the screen when ClearType is on, so Skia draws LCD text, and `None`, its
/// greyscale default, when font smoothing is off or not ClearType.
pub fn system_text_props() -> Option<skia_safe::SurfaceProps> {
    const SPI_GETFONTSMOOTHING: u32 = 0x004A;
    const SPI_GETFONTSMOOTHINGTYPE: u32 = 0x200A;
    const SPI_GETFONTSMOOTHINGORIENTATION: u32 = 0x2012;
    const FE_FONTSMOOTHINGCLEARTYPE: u32 = 0x0002;
    const FE_FONTSMOOTHINGORIENTATIONBGR: u32 = 0x0000;

    let get = |action: u32| -> Option<u32> {
        let mut value: u32 = 0;
        // SAFETY: each of these actions writes one UINT (or BOOL) to pv_param.
        let ok = unsafe { SystemParametersInfoW(action, 0, (&raw mut value).cast(), 0) };
        (ok != 0).then_some(value)
    };

    let smoothing = get(SPI_GETFONTSMOOTHING)? != 0;
    if !smoothing || get(SPI_GETFONTSMOOTHINGTYPE)? != FE_FONTSMOOTHINGCLEARTYPE {
        return None;
    }
    let geometry = match get(SPI_GETFONTSMOOTHINGORIENTATION) {
        Some(FE_FONTSMOOTHINGORIENTATIONBGR) => skia_safe::PixelGeometry::BGRH,
        _ => skia_safe::PixelGeometry::RGBH,
    };
    Some(skia_safe::SurfaceProps::new(Default::default(), geometry))
}

/// # Safety
/// The caller must ensure `texture` was created by a DX12-backed wgpu device and remains
/// valid for the lifetime of the returned `skia_safe::Surface`.
pub unsafe fn make_dx12_surface(
    gr_context: &mut skia_safe::gpu::DirectContext,
    texture: &wgpu::Texture,
    surface_props: Option<&skia_safe::SurfaceProps>,
) -> Option<skia_safe::Surface> {
    // SAFETY: texture is borrowed for the duration of this call; the D3D12 resource is
    // ref-counted (COM) and cloned into Skia's internal BackendRenderTarget via wrap_dx12_texture.
    unsafe {
        let dx12_texture = texture.as_hal::<wgpu::wgc::api::Dx12>()?;
        let resource = windows_core::Interface::from_raw(windows_core::Interface::into_raw(
            dx12_texture.raw_resource().clone(),
        ));
        let size = texture.size();
        let (dxgi_format, color_type) = match texture.format() {
            wgpu::TextureFormat::Rgba8Unorm => {
                (DXGI_FORMAT_R8G8B8A8_UNORM, skia_safe::ColorType::RGBA8888)
            }
            wgpu::TextureFormat::Rgba8UnormSrgb => {
                (DXGI_FORMAT_R8G8B8A8_UNORM_SRGB, skia_safe::ColorType::SRGBA8888)
            }
            wgpu::TextureFormat::Bgra8Unorm => {
                (DXGI_FORMAT_B8G8R8A8_UNORM, skia_safe::ColorType::BGRA8888)
            }
            _ => return None,
        };
        wrap_dx12_texture(
            size.width as i32,
            size.height as i32,
            gr_context,
            resource,
            dxgi_format,
            color_type,
            super::attachment_color_space(texture),
            surface_props,
        )
    }
}

#[cfg_attr(not(feature = "unstable-wgpu-30"), allow(dead_code))]
#[allow(non_snake_case)]
pub unsafe fn import_dx12_texture(
    canvas: &skia_safe::Canvas,
    texture: wgpu::Texture,
) -> Option<skia_safe::Image> {
    unsafe {
        let color_space = super::sampled_texture_color_space(&texture);
        let dx12_texture = texture.as_hal::<wgpu::wgc::api::Dx12>();

        let resource: ID3D12Resource = windows_core::Interface::from_raw(
            windows_core::Interface::into_raw(dx12_texture.unwrap().raw_resource().clone()),
        );

        let dxgi_texture_format = resource.GetDesc().Format;

        let color_type = match dxgi_texture_format {
            DXGI_FORMAT_R8G8B8A8_UNORM => skia_safe::ColorType::RGBA8888,
            DXGI_FORMAT_R8G8B8A8_UNORM_SRGB => skia_safe::ColorType::SRGBA8888,
            DXGI_FORMAT_B8G8R8A8_UNORM => skia_safe::ColorType::BGRA8888,
            _ => return None,
        };

        let texture_info = skia_safe::gpu::d3d::TextureResourceInfo {
            resource,
            alloc: None,
            resource_state: D3D12_RESOURCE_STATE_PRESENT,
            format: dxgi_texture_format,
            sample_count: 1,
            level_count: 1,
            sample_quality_pattern: DXGI_STANDARD_MULTISAMPLE_QUALITY_PATTERN,
            protected: skia_safe::gpu::Protected::No,
        };
        let size = texture.size();

        let backend_texture = skia_safe::gpu::backend_textures::make_d3d(
            (size.width as i32, size.height as i32),
            &texture_info,
            "slint_imported_wgpu_texture",
        );

        Some(
            skia_safe::image::Image::from_texture(
                canvas.recording_context().as_mut().unwrap(),
                &backend_texture,
                skia_safe::gpu::SurfaceOrigin::TopLeft,
                color_type,
                skia_safe::AlphaType::Unpremul,
                color_space,
            )
            .unwrap(),
        )
    }
}

pub unsafe fn make_dx12_context(
    adapter: &wgpu::Adapter,
    _device: &wgpu::Device,
    queue: &wgpu::Queue,
) -> Option<skia_safe::gpu::DirectContext> {
    let backend = unsafe {
        let maybe_dx12_queue = queue.as_hal::<wgpu::wgc::api::Dx12>();
        let dx12_adapter = adapter.as_hal::<wgpu::wgc::api::Dx12>().unwrap();

        maybe_dx12_queue.map(|dx12_queue| {
            let dx12_queue_raw = dx12_queue.as_raw();
            let mut dx12_device_old: Option<windows::Win32::Graphics::Direct3D12::ID3D12Device> =
                None;
            dx12_queue_raw.GetDevice(&mut dx12_device_old as _).unwrap();
            let dx12_device_old = dx12_device_old.unwrap();
            let dx12_device = windows_core::Interface::from_raw(windows_core::Interface::into_raw(
                dx12_device_old,
            ));

            let idxgiadapter_3: windows::Win32::Graphics::Dxgi::IDXGIAdapter3 =
                dx12_adapter.as_raw().clone().into();

            skia_safe::gpu::d3d::BackendContext {
                adapter: windows_core::Interface::from_raw(windows_core::Interface::into_raw(
                    idxgiadapter_3,
                )),
                device: dx12_device,
                queue: windows_core::Interface::from_raw(windows_core::Interface::into_raw(
                    dx12_queue_raw.clone(),
                )),
                memory_allocator: None,
                protected_context: skia_safe::gpu::Protected::No,
            }
        })
    };

    unsafe { skia_safe::gpu::direct_contexts::make_d3d(&backend.unwrap(), None) }
}
