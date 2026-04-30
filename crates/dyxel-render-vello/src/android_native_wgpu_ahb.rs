// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Android AHB-as-wgpu-texture diagnostics.
//!
//! This is still default-off.  It proves the next seam after the raw Vulkan
//! clear probe: wgpu can wrap an imported AHardwareBuffer image, write it with a
//! normal wgpu render pass, then export a sync-fd for SurfaceFlinger.

use super::android_native_presenter::{
    close_fd, create_wgpu_custom_vulkan_device_with_android_interop,
    default_native_presenter_wgpu_features, import_ahb_slot_to_vulkan_image,
    log_wgpu_vulkan_external_ahb_support, wgpu_vulkan_device_native_interop_enabled,
    AndroidHardwareBufferSlot, ImportedAhbVkImage,
};
use std::ffi::{c_char, CStr, CString};

fn android_property(name: &str) -> Option<String> {
    const PROP_VALUE_MAX: usize = 92;
    unsafe extern "C" {
        fn __system_property_get(name: *const c_char, value: *mut c_char) -> i32;
    }

    let name = CString::new(name).ok()?;
    let mut value = [0 as c_char; PROP_VALUE_MAX];
    let len = unsafe { __system_property_get(name.as_ptr(), value.as_mut_ptr()) };
    if len <= 0 {
        return None;
    }
    unsafe { CStr::from_ptr(value.as_ptr()) }
        .to_str()
        .ok()
        .map(|s| s.to_string())
}

fn setting_flag(env_name: &str, property_name: &str) -> bool {
    std::env::var(env_name)
        .ok()
        .or_else(|| android_property(property_name))
        .map(|value| !matches!(value.as_str(), "0" | "false" | "FALSE" | "no" | "NO" | ""))
        .unwrap_or(false)
}

pub(crate) fn android_native_presenter_wgpu_ahb_texture_probe_enabled() -> bool {
    setting_flag(
        "DYXEL_ANDROID_NATIVE_PRESENTER_WGPU_AHB_TEXTURE_PROBE",
        "debug.dyxel.native_presenter_wgpu_ahb_texture_probe",
    )
}

pub(crate) fn android_native_presenter_wgpu_ahb_frame_enabled() -> bool {
    setting_flag(
        "DYXEL_ANDROID_NATIVE_PRESENTER_WGPU_AHB_FRAME",
        "debug.dyxel.native_presenter_wgpu_ahb_frame",
    )
}

pub(crate) fn present_offscreen_texture_frame(
    presenter: &mut super::android_native_presenter::AndroidNativePresenterProbe,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    source_texture: &wgpu::Texture,
    width: u32,
    height: u32,
) -> anyhow::Result<f64> {
    let present_t0 = std::time::Instant::now();
    if presenter.width != width || presenter.height != height {
        return Err(anyhow::anyhow!(
            "native AHB frame size {}x{} does not match presenter {}x{}",
            width,
            height,
            presenter.width,
            presenter.height
        ));
    }
    if !wgpu_vulkan_device_native_interop_enabled(device)? {
        return Err(anyhow::anyhow!(
            "current wgpu Device lacks AHB/semaphore-fd interop; enable debug.dyxel.native_presenter_custom_device=1"
        ));
    }

    let slot_index = presenter.next_cpu_slot % presenter.buffers.len().max(1);
    let slot = presenter
        .buffers
        .get(slot_index)
        .ok_or_else(|| anyhow::anyhow!("native presenter has no AHB buffers"))?;
    let acquire_fence_fd =
        blit_texture_to_slot_and_export_sync_fd(device, queue, source_texture, slot)?;
    presenter.show_slot_with_buffer(slot, acquire_fence_fd)?;
    presenter.next_cpu_slot = (slot_index + 1) % presenter.buffers.len().max(1);
    presenter.presented_cpu_frames = presenter.presented_cpu_frames.saturating_add(1);

    let present_ms = present_t0.elapsed().as_secs_f64() * 1000.0;
    if presenter.presented_cpu_frames == 1
        || presenter.presented_cpu_frames % 60 == 0
        || present_ms >= 8.0
    {
        log::info!(
            "[DIAG-NATIVE-PRESENTER] presented wgpu AHB frame count={} slot={} size={}x{} present_ms={:.2}",
            presenter.presented_cpu_frames,
            slot_index,
            width,
            height,
            present_ms
        );
    }
    Ok(present_ms)
}

pub(crate) fn clear_slot_with_wgpu_texture_and_export_sync_fd(
    adapter: &wgpu::Adapter,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    slot: &AndroidHardwareBufferSlot,
) -> anyhow::Result<i32> {
    match wgpu_vulkan_device_native_interop_enabled(device) {
        Ok(true) => clear_slot_on_wgpu_device(adapter, device, queue, slot, "current-wgpu-device"),
        Ok(false) => {
            log::warn!(
                "[DIAG-NATIVE-PRESENTER] current wgpu Device lacks AHB interop; wgpu AHB texture probe using a temporary custom Vulkan Device"
            );
            clear_slot_with_temp_custom_device(adapter, slot)
        }
        Err(err) => Err(anyhow::anyhow!(
            "wgpu AHB texture probe unavailable on current Device: {:?}",
            err
        )),
    }
}

fn clear_slot_with_temp_custom_device(
    adapter: &wgpu::Adapter,
    slot: &AndroidHardwareBufferSlot,
) -> anyhow::Result<i32> {
    let features = default_native_presenter_wgpu_features(adapter);
    let (device, queue) = create_wgpu_custom_vulkan_device_with_android_interop(
        adapter,
        features,
        "Dyxel Android native presenter wgpu AHB texture probe Device",
    )?;
    log_wgpu_vulkan_external_ahb_support(adapter, &device);
    let result =
        clear_slot_on_wgpu_device(adapter, &device, &queue, slot, "custom-wgpu-ahb-device");
    let _ = device.poll(wgpu::PollType::wait_indefinitely());
    drop(queue);
    drop(device);
    result
}

fn clear_slot_on_wgpu_device(
    _adapter: &wgpu::Adapter,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    slot: &AndroidHardwareBufferSlot,
    label: &'static str,
) -> anyhow::Result<i32> {
    let Some(vk_device) = (unsafe { device.as_hal::<wgpu::hal::api::Vulkan>() }) else {
        return Err(anyhow::anyhow!("wgpu backend is not Vulkan"));
    };

    let (wgpu_texture, vk_format, memory_type_bits, memory_type_index) =
        unsafe { import_slot_as_wgpu_texture(device, &vk_device, slot)? };

    let view = wgpu_texture.create_view(&wgpu::TextureViewDescriptor {
        label: Some("Native Presenter wgpu AHB Probe View"),
        ..Default::default()
    });
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("Native Presenter wgpu AHB Probe Encoder"),
    });
    {
        let _rp = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Native Presenter wgpu AHB Probe Clear Pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: 0.02,
                        g: 0.20,
                        b: 0.95,
                        a: 1.0,
                    }),
                    store: wgpu::StoreOp::Store,
                },
                depth_slice: None,
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
    }
    let submission_index = queue.submit(Some(encoder.finish()));

    let exported_sync_fd = unsafe { export_sync_fd_after_current_queue_and_wait(&vk_device)? };
    log::info!(
        "[DIAG-NATIVE-PRESENTER] wgpu AHB texture probe ok label={} size={}x{} stride={} vk_format={:?} memory_type_bits=0x{:x} memory_type_index={} submission={:?} exported_sync_fd={} queue_family={}",
        label,
        slot.width,
        slot.height,
        slot.stride,
        vk_format,
        memory_type_bits,
        memory_type_index,
        submission_index,
        exported_sync_fd,
        vk_device.queue_family_index()
    );
    Ok(exported_sync_fd)
}

fn blit_texture_to_slot_and_export_sync_fd(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    source_texture: &wgpu::Texture,
    slot: &AndroidHardwareBufferSlot,
) -> anyhow::Result<i32> {
    let Some(vk_device) = (unsafe { device.as_hal::<wgpu::hal::api::Vulkan>() }) else {
        return Err(anyhow::anyhow!("wgpu backend is not Vulkan"));
    };
    let (dest_texture, vk_format, memory_type_bits, memory_type_index) =
        unsafe { import_slot_as_wgpu_texture(device, &vk_device, slot)? };
    let source_view = source_texture.create_view(&wgpu::TextureViewDescriptor {
        label: Some("Native Presenter AHB Frame Source View"),
        ..Default::default()
    });
    let dest_view = dest_texture.create_view(&wgpu::TextureViewDescriptor {
        label: Some("Native Presenter AHB Frame Dest View"),
        ..Default::default()
    });
    blit_texture_to_view(device, queue, &source_view, &dest_view)?;
    let exported_sync_fd = unsafe { export_sync_fd_after_current_queue_and_wait(&vk_device)? };
    log::info!(
        "[DIAG-NATIVE-PRESENTER] wgpu AHB frame blit ok size={}x{} stride={} vk_format={:?} memory_type_bits=0x{:x} memory_type_index={} exported_sync_fd={} queue_family={}",
        slot.width,
        slot.height,
        slot.stride,
        vk_format,
        memory_type_bits,
        memory_type_index,
        exported_sync_fd,
        vk_device.queue_family_index()
    );
    Ok(exported_sync_fd)
}

fn blit_texture_to_view(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    source_view: &wgpu::TextureView,
    dest_view: &wgpu::TextureView,
) -> anyhow::Result<()> {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("Native Presenter AHB Frame Blit Shader"),
        source: wgpu::ShaderSource::Wgsl(include_str!("blit.wgsl").into()),
    });
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("Native Presenter AHB Frame Blit BGL"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
        ],
    });
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("Native Presenter AHB Frame Blit Sampler"),
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        ..Default::default()
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("Native Presenter AHB Frame Blit Pipeline Layout"),
        bind_group_layouts: &[&layout],
        push_constant_ranges: &[],
    });
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("Native Presenter AHB Frame Blit Pipeline"),
        layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_main"),
            buffers: &[],
            compilation_options: Default::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs_main"),
            targets: &[Some(wgpu::ColorTargetState {
                format: wgpu::TextureFormat::Rgba8Unorm,
                blend: Some(wgpu::BlendState::REPLACE),
                write_mask: wgpu::ColorWrites::ALL,
            })],
            compilation_options: Default::default(),
        }),
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview: None,
        cache: None,
    });
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("Native Presenter AHB Frame Blit Bind Group"),
        layout: &layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(source_view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(&sampler),
            },
        ],
    });

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("Native Presenter AHB Frame Blit Encoder"),
    });
    {
        let mut rp = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Native Presenter AHB Frame Blit Pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: dest_view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
                depth_slice: None,
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        rp.set_pipeline(&pipeline);
        rp.set_bind_group(0, &bind_group, &[]);
        rp.draw(0..3, 0..1);
    }
    queue.submit(Some(encoder.finish()));
    Ok(())
}

unsafe fn import_slot_as_wgpu_texture(
    device: &wgpu::Device,
    vk_device: &wgpu::hal::vulkan::Device,
    slot: &AndroidHardwareBufferSlot,
) -> anyhow::Result<(wgpu::Texture, ash::vk::Format, u32, u32)> {
    let imported = unsafe { import_ahb_slot_to_vulkan_image(vk_device, slot)? };
    let vk_format = imported.format;
    let memory_type_bits = imported.memory_type_bits;
    let memory_type_index = imported.memory_type_index;
    let hal_texture = unsafe { imported_ahb_to_hal_texture(vk_device, imported, slot)? };
    let desc = wgpu::TextureDescriptor {
        label: Some("Native Presenter Imported AHB wgpu Texture"),
        size: wgpu::Extent3d {
            width: slot.width,
            height: slot.height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    };
    let texture =
        unsafe { device.create_texture_from_hal::<wgpu::hal::api::Vulkan>(hal_texture, &desc) };
    Ok((texture, vk_format, memory_type_bits, memory_type_index))
}

unsafe fn imported_ahb_to_hal_texture(
    vk_device: &wgpu::hal::vulkan::Device,
    imported: ImportedAhbVkImage,
    slot: &AndroidHardwareBufferSlot,
) -> anyhow::Result<wgpu::hal::vulkan::Texture> {
    let raw_device = vk_device.raw_device().clone();
    let image = imported.image;
    let memory = imported.memory;
    let hal_desc = wgpu::hal::TextureDescriptor {
        label: Some("Native Presenter Imported AHB HAL Texture"),
        size: wgpu::Extent3d {
            width: slot.width,
            height: slot.height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUses::COLOR_TARGET
            | wgpu::TextureUses::RESOURCE
            | wgpu::TextureUses::COPY_DST,
        memory_flags: wgpu::hal::MemoryFlags::empty(),
        view_formats: Vec::new(),
    };
    let drop_callback = Box::new(move || unsafe {
        raw_device.destroy_image(image, None);
        raw_device.free_memory(memory, None);
    });
    Ok(unsafe { vk_device.texture_from_raw(image, &hal_desc, Some(drop_callback)) })
}

unsafe fn export_sync_fd_after_current_queue_and_wait(
    vk_device: &wgpu::hal::vulkan::Device,
) -> anyhow::Result<i32> {
    let raw_device = vk_device.raw_device();
    let mut export_info = ash::vk::ExportSemaphoreCreateInfo::default()
        .handle_types(ash::vk::ExternalSemaphoreHandleTypeFlags::SYNC_FD);
    let semaphore_info = ash::vk::SemaphoreCreateInfo::default().push_next(&mut export_info);
    let signal_semaphore =
        unsafe { raw_device.create_semaphore(&semaphore_info, None) }.map_err(|err| {
            anyhow::anyhow!(
                "vkCreateSemaphore(wgpu AHB texture probe) failed: {:?}",
                err
            )
        })?;

    let fence = match unsafe { raw_device.create_fence(&ash::vk::FenceCreateInfo::default(), None) }
    {
        Ok(fence) => fence,
        Err(err) => {
            unsafe { raw_device.destroy_semaphore(signal_semaphore, None) };
            return Err(anyhow::anyhow!(
                "vkCreateFence(wgpu AHB texture probe) failed: {:?}",
                err
            ));
        }
    };

    let signal_semaphores = [signal_semaphore];
    let submit_info = ash::vk::SubmitInfo::default().signal_semaphores(&signal_semaphores);
    if let Err(err) =
        unsafe { raw_device.queue_submit(vk_device.raw_queue(), &[submit_info], fence) }
    {
        unsafe {
            raw_device.destroy_fence(fence, None);
            raw_device.destroy_semaphore(signal_semaphore, None);
        }
        return Err(anyhow::anyhow!(
            "vkQueueSubmit(sync-fd wgpu AHB texture probe) failed: {:?}",
            err
        ));
    }

    let semaphore_fd_ext = ash::khr::external_semaphore_fd::Device::new(
        vk_device.shared_instance().raw_instance(),
        raw_device,
    );
    let fd_info = ash::vk::SemaphoreGetFdInfoKHR::default()
        .semaphore(signal_semaphore)
        .handle_type(ash::vk::ExternalSemaphoreHandleTypeFlags::SYNC_FD);
    let exported_sync_fd = match unsafe { semaphore_fd_ext.get_semaphore_fd(&fd_info) } {
        Ok(fd) => fd,
        Err(err) => {
            let _ = unsafe { raw_device.wait_for_fences(&[fence], true, 5_000_000_000) };
            unsafe {
                raw_device.destroy_fence(fence, None);
                raw_device.destroy_semaphore(signal_semaphore, None);
            }
            return Err(anyhow::anyhow!(
                "vkGetSemaphoreFdKHR(wgpu AHB texture probe) failed: {:?}",
                err
            ));
        }
    };

    if let Err(err) = unsafe { raw_device.wait_for_fences(&[fence], true, 5_000_000_000) } {
        if exported_sync_fd >= 0 {
            close_fd(exported_sync_fd);
        }
        unsafe {
            raw_device.destroy_fence(fence, None);
            raw_device.destroy_semaphore(signal_semaphore, None);
        }
        return Err(anyhow::anyhow!(
            "vkWaitForFences(wgpu AHB texture probe) failed: {:?}",
            err
        ));
    }

    unsafe {
        raw_device.destroy_fence(fence, None);
        raw_device.destroy_semaphore(signal_semaphore, None);
    }
    Ok(exported_sync_fd)
}
