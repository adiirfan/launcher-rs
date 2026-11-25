//! SPIKE: Vulkan instead of OpenGL ES, chosen with
//! `adb shell setprop debug.launcherrs.gpu vulkan` (read at start).
//! Same single instanced draw, same shaders (shaders/quad.*), same atlases.
//! `debug.launcherrs.vkimages N` asks for N screen buffers.

use std::ffi::c_void;
use std::io::Cursor;

use ash::{khr, vk};
use ndk::native_window::NativeWindow;

/// Which atlas an upload goes to (the icon one is RGBA, the glyph one is one byte).
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum AtlasId {
    Icons = 0,
    Glyphs = 1,
}

struct Upload {
    atlas: AtlasId,
    x: u32,
    y: u32,
    w: u32,
    h: u32,
    pixels: Vec<u8>,
}

struct HostBuffer {
    buf: vk::Buffer,
    mem: vk::DeviceMemory,
    ptr: *mut u8,
    size: u64,
}

struct Image {
    image: vk::Image,
    mem: vk::DeviceMemory,
    view: vk::ImageView,
}

struct Chain {
    surface: vk::SurfaceKHR,
    swapchain: vk::SwapchainKHR,
    extent: vk::Extent2D,
    views: Vec<vk::ImageView>,
    framebuffers: Vec<vk::Framebuffer>,
    /// One per screen buffer: signalled when drawing into it is done.
    rendered: Vec<vk::Semaphore>,
    /// The screen buffer drawn into this frame, waiting for `swap`.
    current: Option<u32>,
    stale: bool,
}

pub struct VkSurface {
    _entry: ash::Entry,
    instance: ash::Instance,
    surface_fn: khr::surface::Instance,
    android_fn: khr::android_surface::Instance,
    swap_fn: khr::swapchain::Device,
    pdev: vk::PhysicalDevice,
    device: ash::Device,
    queue: vk::Queue,
    mem_props: vk::PhysicalDeviceMemoryProperties,
    format: vk::Format,
    render_pass: vk::RenderPass,
    set_layout: vk::DescriptorSetLayout,
    layout: vk::PipelineLayout,
    pipeline: vk::Pipeline,
    pool: vk::CommandPool,
    cmd: vk::CommandBuffer,
    desc_pool: vk::DescriptorPool,
    set: vk::DescriptorSet,
    sampler: vk::Sampler,
    atlases: [Image; 2],
    acquired: vk::Semaphore,
    done: vk::Fence,
    instances: Option<HostBuffer>,
    staging: Option<HostBuffer>,
    uploads: Vec<Upload>,
    chain: Option<Chain>,
    images_wanted: u32,
}

fn err(what: &str) -> impl Fn(vk::Result) -> String + '_ {
    move |e| format!("{what}: {e:?}")
}

const RANGE: vk::ImageSubresourceRange = vk::ImageSubresourceRange {
    aspect_mask: vk::ImageAspectFlags::COLOR,
    base_mip_level: 0,
    level_count: 1,
    base_array_layer: 0,
    layer_count: 1,
};

impl VkSurface {
    pub fn new(window: &NativeWindow, icon_atlas: (u32, u32), glyph_atlas: (u32, u32), images_wanted: u32) -> Result<VkSurface, String> {
        unsafe {
            let entry = ash::Entry::load().map_err(|e| format!("no Vulkan library: {e}"))?;
            let app = vk::ApplicationInfo::default().api_version(vk::API_VERSION_1_0);
            let exts = [khr::surface::NAME.as_ptr(), khr::android_surface::NAME.as_ptr()];
            let instance = entry
                .create_instance(&vk::InstanceCreateInfo::default().application_info(&app).enabled_extension_names(&exts), None)
                .map_err(err("vkCreateInstance"))?;
            let surface_fn = khr::surface::Instance::new(&entry, &instance);
            let android_fn = khr::android_surface::Instance::new(&entry, &instance);

            let pdev = *instance
                .enumerate_physical_devices()
                .map_err(err("enumerate devices"))?
                .first()
                .ok_or("no Vulkan device")?;
            let qfi = instance
                .get_physical_device_queue_family_properties(pdev)
                .iter()
                .position(|q| q.queue_flags.contains(vk::QueueFlags::GRAPHICS))
                .ok_or("no graphics queue")? as u32;
            let prio = [1.0];
            let qinfo = [vk::DeviceQueueCreateInfo::default().queue_family_index(qfi).queue_priorities(&prio)];
            let dexts = [khr::swapchain::NAME.as_ptr()];
            let device = instance
                .create_device(pdev, &vk::DeviceCreateInfo::default().queue_create_infos(&qinfo).enabled_extension_names(&dexts), None)
                .map_err(err("vkCreateDevice"))?;
            let queue = device.get_device_queue(qfi, 0);
            let swap_fn = khr::swapchain::Device::new(&instance, &device);
            let mem_props = instance.get_physical_device_memory_properties(pdev);

            // The window decides the format; ask it once, up front.
            let surface = make_surface(&android_fn, window)?;
            if !surface_fn.get_physical_device_surface_support(pdev, qfi, surface).unwrap_or(false) {
                return Err("queue can't show on this window".into());
            }
            let formats = surface_fn.get_physical_device_surface_formats(pdev, surface).map_err(err("surface formats"))?;
            let format = formats
                .iter()
                .find(|f| f.format == vk::Format::R8G8B8A8_UNORM)
                .or(formats.first())
                .ok_or("no surface format")?
                .format;
            surface_fn.destroy_surface(surface, None);

            let render_pass = make_render_pass(&device, format)?;
            let bindings = [0, 1].map(|b| {
                vk::DescriptorSetLayoutBinding::default()
                    .binding(b)
                    .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
                    .descriptor_count(1)
                    .stage_flags(vk::ShaderStageFlags::FRAGMENT)
            });
            let set_layout = device
                .create_descriptor_set_layout(&vk::DescriptorSetLayoutCreateInfo::default().bindings(&bindings), None)
                .map_err(err("set layout"))?;
            let push = [vk::PushConstantRange { stage_flags: vk::ShaderStageFlags::VERTEX, offset: 0, size: 8 }];
            let set_layouts = [set_layout];
            let layout = device
                .create_pipeline_layout(&vk::PipelineLayoutCreateInfo::default().set_layouts(&set_layouts).push_constant_ranges(&push), None)
                .map_err(err("pipeline layout"))?;
            let pipeline = make_pipeline(&device, render_pass, layout)?;

            let pool = device
                .create_command_pool(
                    &vk::CommandPoolCreateInfo::default()
                        .queue_family_index(qfi)
                        .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER),
                    None,
                )
                .map_err(err("command pool"))?;
            let cmd = device
                .allocate_command_buffers(
                    &vk::CommandBufferAllocateInfo::default().command_pool(pool).level(vk::CommandBufferLevel::PRIMARY).command_buffer_count(1),
                )
                .map_err(err("command buffer"))?[0];

            let sampler = device
                .create_sampler(
                    &vk::SamplerCreateInfo::default()
                        .mag_filter(vk::Filter::LINEAR)
                        .min_filter(vk::Filter::LINEAR)
                        .address_mode_u(vk::SamplerAddressMode::CLAMP_TO_EDGE)
                        .address_mode_v(vk::SamplerAddressMode::CLAMP_TO_EDGE)
                        .address_mode_w(vk::SamplerAddressMode::CLAMP_TO_EDGE),
                    None,
                )
                .map_err(err("sampler"))?;

            let mut me_atlases = Vec::new();
            for (size, fmt) in [(icon_atlas, vk::Format::R8G8B8A8_UNORM), (glyph_atlas, vk::Format::R8_UNORM)] {
                me_atlases.push(make_image(&device, &mem_props, size, fmt)?);
            }
            let atlases: [Image; 2] = me_atlases.try_into().map_err(|_| "atlases")?;

            let sizes = [vk::DescriptorPoolSize { ty: vk::DescriptorType::COMBINED_IMAGE_SAMPLER, descriptor_count: 2 }];
            let desc_pool = device
                .create_descriptor_pool(&vk::DescriptorPoolCreateInfo::default().max_sets(1).pool_sizes(&sizes), None)
                .map_err(err("descriptor pool"))?;
            let set = device
                .allocate_descriptor_sets(&vk::DescriptorSetAllocateInfo::default().descriptor_pool(desc_pool).set_layouts(&set_layouts))
                .map_err(err("descriptor set"))?[0];
            let infos = atlases.each_ref().map(|a| {
                [vk::DescriptorImageInfo { sampler, image_view: a.view, image_layout: vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL }]
            });
            let writes = [0usize, 1].map(|i| {
                vk::WriteDescriptorSet::default()
                    .dst_set(set)
                    .dst_binding(i as u32)
                    .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
                    .image_info(&infos[i])
            });
            device.update_descriptor_sets(&writes, &[]);

            let acquired = device.create_semaphore(&vk::SemaphoreCreateInfo::default(), None).map_err(err("semaphore"))?;
            let done = device
                .create_fence(&vk::FenceCreateInfo::default().flags(vk::FenceCreateFlags::SIGNALED), None)
                .map_err(err("fence"))?;

            let mut me = VkSurface {
                _entry: entry,
                instance,
                surface_fn,
                android_fn,
                swap_fn,
                pdev,
                device,
                queue,
                mem_props,
                format,
                render_pass,
                set_layout,
                layout,
                pipeline,
                pool,
                cmd,
                desc_pool,
                set,
                sampler,
                atlases,
                acquired,
                done,
                instances: None,
                staging: None,
                uploads: Vec::new(),
                chain: None,
                images_wanted,
            };
            me.prepare_atlases()?;
            me.resume(window)?;
            Ok(me)
        }
    }

    /// Move both atlases into the layout the shader reads from, once.
    unsafe fn prepare_atlases(&mut self) -> Result<(), String> {
        unsafe {
            let d = &self.device;
            d.begin_command_buffer(self.cmd, &vk::CommandBufferBeginInfo::default()).map_err(err("begin"))?;
            for a in &self.atlases {
                barrier(d, self.cmd, a.image, vk::ImageLayout::UNDEFINED, vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL);
            }
            d.end_command_buffer(self.cmd).map_err(err("end"))?;
            let cmds = [self.cmd];
            d.reset_fences(&[self.done]).map_err(err("reset fence"))?;
            d.queue_submit(self.queue, &[vk::SubmitInfo::default().command_buffers(&cmds)], self.done).map_err(err("submit"))?;
            d.wait_for_fences(&[self.done], true, 1_000_000_000).map_err(err("wait"))?;
            Ok(())
        }
    }

    pub fn resume(&mut self, window: &NativeWindow) -> Result<(), String> {
        self.suspend();
        unsafe {
            let surface = make_surface(&self.android_fn, window)?;
            self.chain = Some(Chain {
                surface,
                swapchain: vk::SwapchainKHR::null(),
                extent: vk::Extent2D::default(),
                views: Vec::new(),
                framebuffers: Vec::new(),
                rendered: Vec::new(),
                current: None,
                stale: true,
            });
            self.rebuild_chain()?;
        }
        Ok(())
    }

    unsafe fn rebuild_chain(&mut self) -> Result<(), String> {
        unsafe {
            let d = &self.device;
            let _ = d.device_wait_idle();
            let Some(chain) = self.chain.as_mut() else { return Ok(()) };
            for fb in chain.framebuffers.drain(..) {
                d.destroy_framebuffer(fb, None);
            }
            for v in chain.views.drain(..) {
                d.destroy_image_view(v, None);
            }
            for s in chain.rendered.drain(..) {
                d.destroy_semaphore(s, None);
            }
            let caps = self
                .surface_fn
                .get_physical_device_surface_capabilities(self.pdev, chain.surface)
                .map_err(err("surface caps"))?;
            let mut count = self.images_wanted.max(caps.min_image_count);
            if caps.max_image_count > 0 {
                count = count.min(caps.max_image_count);
            }
            let alpha = [
                vk::CompositeAlphaFlagsKHR::INHERIT,
                vk::CompositeAlphaFlagsKHR::PRE_MULTIPLIED,
                vk::CompositeAlphaFlagsKHR::OPAQUE,
            ]
            .into_iter()
            .find(|a| caps.supported_composite_alpha.contains(*a))
            .unwrap_or(vk::CompositeAlphaFlagsKHR::OPAQUE);
            let old = chain.swapchain;
            let info = vk::SwapchainCreateInfoKHR::default()
                .surface(chain.surface)
                .min_image_count(count)
                .image_format(self.format)
                .image_color_space(vk::ColorSpaceKHR::SRGB_NONLINEAR)
                .image_extent(caps.current_extent)
                .image_array_layers(1)
                .image_usage(vk::ImageUsageFlags::COLOR_ATTACHMENT)
                .image_sharing_mode(vk::SharingMode::EXCLUSIVE)
                .pre_transform(vk::SurfaceTransformFlagsKHR::IDENTITY)
                .composite_alpha(alpha)
                .present_mode(vk::PresentModeKHR::FIFO)
                .clipped(true)
                .old_swapchain(old);
            chain.swapchain = self.swap_fn.create_swapchain(&info, None).map_err(err("vkCreateSwapchainKHR"))?;
            if old != vk::SwapchainKHR::null() {
                self.swap_fn.destroy_swapchain(old, None);
            }
            chain.extent = caps.current_extent;
            let images = self.swap_fn.get_swapchain_images(chain.swapchain).map_err(err("swapchain images"))?;
            log::info!(
                "vulkan: {} screen buffers (asked {count}, allowed {}..{}), {}x{}, alpha {alpha:?}",
                images.len(), caps.min_image_count, caps.max_image_count, chain.extent.width, chain.extent.height
            );
            for img in images {
                let view = d
                    .create_image_view(
                        &vk::ImageViewCreateInfo::default()
                            .image(img)
                            .view_type(vk::ImageViewType::TYPE_2D)
                            .format(self.format)
                            .subresource_range(RANGE),
                        None,
                    )
                    .map_err(err("view"))?;
                let atts = [view];
                let fb = d
                    .create_framebuffer(
                        &vk::FramebufferCreateInfo::default()
                            .render_pass(self.render_pass)
                            .attachments(&atts)
                            .width(chain.extent.width)
                            .height(chain.extent.height)
                            .layers(1),
                        None,
                    )
                    .map_err(err("framebuffer"))?;
                chain.views.push(view);
                chain.framebuffers.push(fb);
                chain.rendered.push(d.create_semaphore(&vk::SemaphoreCreateInfo::default(), None).map_err(err("semaphore"))?);
            }
            chain.stale = false;
            chain.current = None;
            Ok(())
        }
    }

    /// Drop the window's screen buffers; keep the device and atlases.
    pub fn suspend(&mut self) {
        let Some(mut chain) = self.chain.take() else { return };
        unsafe {
            let d = &self.device;
            let _ = d.device_wait_idle();
            for fb in chain.framebuffers.drain(..) {
                d.destroy_framebuffer(fb, None);
            }
            for v in chain.views.drain(..) {
                d.destroy_image_view(v, None);
            }
            for s in chain.rendered.drain(..) {
                d.destroy_semaphore(s, None);
            }
            if chain.swapchain != vk::SwapchainKHR::null() {
                self.swap_fn.destroy_swapchain(chain.swapchain, None);
            }
            self.surface_fn.destroy_surface(chain.surface, None);
        }
    }

    pub fn has_surface(&self) -> bool {
        self.chain.is_some()
    }

    pub fn size(&self) -> (i32, i32) {
        self.chain.as_ref().map_or((0, 0), |c| (c.extent.width as i32, c.extent.height as i32))
    }

    /// Queue pixels for an atlas; copied in at the start of the next draw.
    pub fn upload(&mut self, atlas: AtlasId, x: u32, y: u32, w: u32, h: u32, pixels: &[u8]) {
        self.uploads.push(Upload { atlas, x, y, w, h, pixels: pixels.to_vec() });
    }

    /// Record and submit one frame: atlas copies, then the one instanced draw.
    pub fn draw(&mut self, instances: &[u8], count: u32, screen: (i32, i32)) -> Result<(), String> {
        unsafe {
            if self.chain.as_ref().is_some_and(|c| c.stale) {
                self.rebuild_chain()?;
            }
            let d = self.device.clone();
            d.wait_for_fences(&[self.done], true, 1_000_000_000).map_err(err("wait for last frame"))?;
            let Some(chain) = self.chain.as_ref() else { return Ok(()) };
            let (index, _) = match self.swap_fn.acquire_next_image(chain.swapchain, 1_000_000_000, self.acquired, vk::Fence::null()) {
                Ok(r) => r,
                Err(vk::Result::ERROR_OUT_OF_DATE_KHR | vk::Result::SUBOPTIMAL_KHR) => {
                    if let Some(c) = self.chain.as_mut() {
                        c.stale = true;
                    }
                    return Ok(());
                }
                Err(e) => return Err(format!("acquire: {e:?}")),
            };
            d.reset_fences(&[self.done]).map_err(err("reset fence"))?;

            // Pixels waiting for the atlases go through one staging buffer.
            let mut regions: [Vec<vk::BufferImageCopy>; 2] = [Vec::new(), Vec::new()];
            if !self.uploads.is_empty() {
                let total: usize = self.uploads.iter().map(|u| u.pixels.len().next_multiple_of(4)).sum();
                let staging = ensure(&d, &self.mem_props, &mut self.staging, total as u64, vk::BufferUsageFlags::TRANSFER_SRC)?;
                let mut at = 0usize;
                for u in self.uploads.drain(..) {
                    std::ptr::copy_nonoverlapping(u.pixels.as_ptr(), staging.ptr.add(at), u.pixels.len());
                    regions[u.atlas as usize].push(vk::BufferImageCopy {
                        buffer_offset: at as u64,
                        buffer_row_length: 0,
                        buffer_image_height: 0,
                        image_subresource: vk::ImageSubresourceLayers {
                            aspect_mask: vk::ImageAspectFlags::COLOR,
                            mip_level: 0,
                            base_array_layer: 0,
                            layer_count: 1,
                        },
                        image_offset: vk::Offset3D { x: u.x as i32, y: u.y as i32, z: 0 },
                        image_extent: vk::Extent3D { width: u.w, height: u.h, depth: 1 },
                    });
                    at += u.pixels.len().next_multiple_of(4);
                }
            }
            if count > 0 {
                let buf = ensure(&d, &self.mem_props, &mut self.instances, instances.len() as u64, vk::BufferUsageFlags::VERTEX_BUFFER)?;
                std::ptr::copy_nonoverlapping(instances.as_ptr(), buf.ptr, instances.len());
            }

            let cmd = self.cmd;
            d.begin_command_buffer(cmd, &vk::CommandBufferBeginInfo::default().flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT))
                .map_err(err("begin"))?;
            for (i, r) in regions.iter().enumerate() {
                if r.is_empty() {
                    continue;
                }
                let img = self.atlases[i].image;
                barrier(&d, cmd, img, vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL, vk::ImageLayout::TRANSFER_DST_OPTIMAL);
                let src = self.staging.as_ref().expect("filled above").buf;
                d.cmd_copy_buffer_to_image(cmd, src, img, vk::ImageLayout::TRANSFER_DST_OPTIMAL, r);
                barrier(&d, cmd, img, vk::ImageLayout::TRANSFER_DST_OPTIMAL, vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL);
            }
            let clear = [vk::ClearValue { color: vk::ClearColorValue { float32: [0.0; 4] } }];
            let area = vk::Rect2D { offset: vk::Offset2D::default(), extent: chain.extent };
            d.cmd_begin_render_pass(
                cmd,
                &vk::RenderPassBeginInfo::default()
                    .render_pass(self.render_pass)
                    .framebuffer(chain.framebuffers[index as usize])
                    .render_area(area)
                    .clear_values(&clear),
                vk::SubpassContents::INLINE,
            );
            if count > 0 {
                let (w, h) = (chain.extent.width as f32, chain.extent.height as f32);
                d.cmd_set_viewport(cmd, 0, &[vk::Viewport { x: 0.0, y: 0.0, width: w, height: h, min_depth: 0.0, max_depth: 1.0 }]);
                d.cmd_set_scissor(cmd, 0, &[area]);
                d.cmd_bind_pipeline(cmd, vk::PipelineBindPoint::GRAPHICS, self.pipeline);
                d.cmd_bind_descriptor_sets(cmd, vk::PipelineBindPoint::GRAPHICS, self.layout, 0, &[self.set], &[]);
                let screen = [screen.0 as f32, screen.1 as f32];
                d.cmd_push_constants(cmd, self.layout, vk::ShaderStageFlags::VERTEX, 0, bytemuck::cast_slice(&screen));
                d.cmd_bind_vertex_buffers(cmd, 0, &[self.instances.as_ref().expect("filled above").buf], &[0]);
                d.cmd_draw(cmd, 4, count, 0, 0);
            }
            d.cmd_end_render_pass(cmd);
            d.end_command_buffer(cmd).map_err(err("end"))?;

            let waits = [self.acquired];
            let stages = [vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT];
            let cmds = [cmd];
            let signals = [chain.rendered[index as usize]];
            d.queue_submit(
                self.queue,
                &[vk::SubmitInfo::default().wait_semaphores(&waits).wait_dst_stage_mask(&stages).command_buffers(&cmds).signal_semaphores(&signals)],
                self.done,
            )
            .map_err(err("submit"))?;
            if let Some(c) = self.chain.as_mut() {
                c.current = Some(index);
            }
            Ok(())
        }
    }

    /// Show the frame drawn by `draw`.
    pub fn swap(&mut self) -> Result<(), String> {
        let Some(chain) = self.chain.as_mut() else { return Err("no surface".into()) };
        let Some(index) = chain.current.take() else { return Ok(()) };
        let waits = [chain.rendered[index as usize]];
        let chains = [chain.swapchain];
        let indices = [index];
        let info = vk::PresentInfoKHR::default().wait_semaphores(&waits).swapchains(&chains).image_indices(&indices);
        match unsafe { self.swap_fn.queue_present(self.queue, &info) } {
            Ok(false) => Ok(()),
            Ok(true) | Err(vk::Result::ERROR_OUT_OF_DATE_KHR) => {
                chain.stale = true;
                Ok(())
            }
            Err(e) => Err(format!("present: {e:?}")),
        }
    }
}

impl Drop for VkSurface {
    fn drop(&mut self) {
        self.suspend();
        unsafe {
            let d = &self.device;
            let _ = d.device_wait_idle();
            for b in [self.instances.take(), self.staging.take()].into_iter().flatten() {
                d.destroy_buffer(b.buf, None);
                d.free_memory(b.mem, None);
            }
            for a in &self.atlases {
                d.destroy_image_view(a.view, None);
                d.destroy_image(a.image, None);
                d.free_memory(a.mem, None);
            }
            d.destroy_sampler(self.sampler, None);
            d.destroy_descriptor_pool(self.desc_pool, None);
            d.destroy_semaphore(self.acquired, None);
            d.destroy_fence(self.done, None);
            d.destroy_command_pool(self.pool, None);
            d.destroy_pipeline(self.pipeline, None);
            d.destroy_pipeline_layout(self.layout, None);
            d.destroy_descriptor_set_layout(self.set_layout, None);
            d.destroy_render_pass(self.render_pass, None);
            d.destroy_device(None);
            self.instance.destroy_instance(None);
        }
    }
}

unsafe fn make_surface(android_fn: &khr::android_surface::Instance, window: &NativeWindow) -> Result<vk::SurfaceKHR, String> {
    let info = vk::AndroidSurfaceCreateInfoKHR::default().window(window.ptr().as_ptr() as *mut c_void);
    unsafe { android_fn.create_android_surface(&info, None) }.map_err(err("vkCreateAndroidSurfaceKHR"))
}

fn memory_type(props: &vk::PhysicalDeviceMemoryProperties, bits: u32, want: vk::MemoryPropertyFlags) -> Option<u32> {
    (0..props.memory_type_count).find(|&i| bits & (1 << i) != 0 && props.memory_types[i as usize].property_flags.contains(want))
}

/// A mapped buffer the processor writes and the graphics chip reads; grown when too small.
unsafe fn ensure<'a>(
    d: &ash::Device,
    props: &vk::PhysicalDeviceMemoryProperties,
    slot: &'a mut Option<HostBuffer>,
    size: u64,
    usage: vk::BufferUsageFlags,
) -> Result<&'a HostBuffer, String> {
    unsafe {
        if slot.as_ref().is_some_and(|b| b.size < size) {
            let b = slot.take().expect("checked");
            d.destroy_buffer(b.buf, None);
            d.free_memory(b.mem, None);
        }
        if slot.is_none() {
            let size = size.next_power_of_two().max(64 * 1024);
            let buf = d
                .create_buffer(&vk::BufferCreateInfo::default().size(size).usage(usage).sharing_mode(vk::SharingMode::EXCLUSIVE), None)
                .map_err(err("buffer"))?;
            let req = d.get_buffer_memory_requirements(buf);
            let ty = memory_type(props, req.memory_type_bits, vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT)
                .ok_or("no host memory")?;
            let mem = d
                .allocate_memory(&vk::MemoryAllocateInfo::default().allocation_size(req.size).memory_type_index(ty), None)
                .map_err(err("allocate"))?;
            d.bind_buffer_memory(buf, mem, 0).map_err(err("bind"))?;
            let ptr = d.map_memory(mem, 0, vk::WHOLE_SIZE, vk::MemoryMapFlags::empty()).map_err(err("map"))? as *mut u8;
            *slot = Some(HostBuffer { buf, mem, ptr, size });
        }
        Ok(slot.as_ref().expect("just made"))
    }
}

unsafe fn make_image(d: &ash::Device, props: &vk::PhysicalDeviceMemoryProperties, (w, h): (u32, u32), format: vk::Format) -> Result<Image, String> {
    unsafe {
        let image = d
            .create_image(
                &vk::ImageCreateInfo::default()
                    .image_type(vk::ImageType::TYPE_2D)
                    .format(format)
                    .extent(vk::Extent3D { width: w, height: h, depth: 1 })
                    .mip_levels(1)
                    .array_layers(1)
                    .samples(vk::SampleCountFlags::TYPE_1)
                    .tiling(vk::ImageTiling::OPTIMAL)
                    .usage(vk::ImageUsageFlags::SAMPLED | vk::ImageUsageFlags::TRANSFER_DST)
                    .initial_layout(vk::ImageLayout::UNDEFINED),
                None,
            )
            .map_err(err("image"))?;
        let req = d.get_image_memory_requirements(image);
        let ty = memory_type(props, req.memory_type_bits, vk::MemoryPropertyFlags::DEVICE_LOCAL).ok_or("no device memory")?;
        let mem = d
            .allocate_memory(&vk::MemoryAllocateInfo::default().allocation_size(req.size).memory_type_index(ty), None)
            .map_err(err("allocate image"))?;
        d.bind_image_memory(image, mem, 0).map_err(err("bind image"))?;
        let view = d
            .create_image_view(
                &vk::ImageViewCreateInfo::default().image(image).view_type(vk::ImageViewType::TYPE_2D).format(format).subresource_range(RANGE),
                None,
            )
            .map_err(err("image view"))?;
        Ok(Image { image, mem, view })
    }
}

unsafe fn barrier(d: &ash::Device, cmd: vk::CommandBuffer, image: vk::Image, from: vk::ImageLayout, to: vk::ImageLayout) {
    let (src_access, src_stage) = match from {
        vk::ImageLayout::TRANSFER_DST_OPTIMAL => (vk::AccessFlags::TRANSFER_WRITE, vk::PipelineStageFlags::TRANSFER),
        vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL => (vk::AccessFlags::SHADER_READ, vk::PipelineStageFlags::FRAGMENT_SHADER),
        _ => (vk::AccessFlags::empty(), vk::PipelineStageFlags::TOP_OF_PIPE),
    };
    let (dst_access, dst_stage) = match to {
        vk::ImageLayout::TRANSFER_DST_OPTIMAL => (vk::AccessFlags::TRANSFER_WRITE, vk::PipelineStageFlags::TRANSFER),
        _ => (vk::AccessFlags::SHADER_READ, vk::PipelineStageFlags::FRAGMENT_SHADER),
    };
    let b = vk::ImageMemoryBarrier::default()
        .src_access_mask(src_access)
        .dst_access_mask(dst_access)
        .old_layout(from)
        .new_layout(to)
        .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
        .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
        .image(image)
        .subresource_range(RANGE);
    unsafe { d.cmd_pipeline_barrier(cmd, src_stage, dst_stage, vk::DependencyFlags::empty(), &[], &[], &[b]) };
}

unsafe fn make_render_pass(d: &ash::Device, format: vk::Format) -> Result<vk::RenderPass, String> {
    let att = [vk::AttachmentDescription::default()
        .format(format)
        .samples(vk::SampleCountFlags::TYPE_1)
        .load_op(vk::AttachmentLoadOp::CLEAR)
        .store_op(vk::AttachmentStoreOp::STORE)
        .stencil_load_op(vk::AttachmentLoadOp::DONT_CARE)
        .stencil_store_op(vk::AttachmentStoreOp::DONT_CARE)
        .initial_layout(vk::ImageLayout::UNDEFINED)
        .final_layout(vk::ImageLayout::PRESENT_SRC_KHR)];
    let refs = [vk::AttachmentReference { attachment: 0, layout: vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL }];
    let sub = [vk::SubpassDescription::default().pipeline_bind_point(vk::PipelineBindPoint::GRAPHICS).color_attachments(&refs)];
    let dep = [vk::SubpassDependency::default()
        .src_subpass(vk::SUBPASS_EXTERNAL)
        .dst_subpass(0)
        .src_stage_mask(vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT)
        .dst_stage_mask(vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT)
        .dst_access_mask(vk::AccessFlags::COLOR_ATTACHMENT_WRITE)];
    unsafe { d.create_render_pass(&vk::RenderPassCreateInfo::default().attachments(&att).subpasses(&sub).dependencies(&dep), None) }
        .map_err(err("render pass"))
}

unsafe fn make_pipeline(d: &ash::Device, render_pass: vk::RenderPass, layout: vk::PipelineLayout) -> Result<vk::Pipeline, String> {
    unsafe {
        let module = |bytes: &[u8]| -> Result<vk::ShaderModule, String> {
            let words = ash::util::read_spv(&mut Cursor::new(bytes)).map_err(|e| format!("spv: {e}"))?;
            d.create_shader_module(&vk::ShaderModuleCreateInfo::default().code(&words), None).map_err(err("shader"))
        };
        let vs = module(include_bytes!("../shaders/quad.vert.spv"))?;
        let fs = module(include_bytes!("../shaders/quad.frag.spv"))?;
        let stages = [
            vk::PipelineShaderStageCreateInfo::default().stage(vk::ShaderStageFlags::VERTEX).module(vs).name(c"main"),
            vk::PipelineShaderStageCreateInfo::default().stage(vk::ShaderStageFlags::FRAGMENT).module(fs).name(c"main"),
        ];
        let stride = std::mem::size_of::<launcher_draw::batch::Instance>() as u32;
        let bindings = [vk::VertexInputBindingDescription { binding: 0, stride, input_rate: vk::VertexInputRate::INSTANCE }];
        let attrs: [vk::VertexInputAttributeDescription; 5] = std::array::from_fn(|i| vk::VertexInputAttributeDescription {
            location: i as u32,
            binding: 0,
            format: vk::Format::R32G32B32A32_SFLOAT,
            offset: 16 * i as u32,
        });
        let vertex = vk::PipelineVertexInputStateCreateInfo::default()
            .vertex_binding_descriptions(&bindings)
            .vertex_attribute_descriptions(&attrs);
        let assembly = vk::PipelineInputAssemblyStateCreateInfo::default().topology(vk::PrimitiveTopology::TRIANGLE_STRIP);
        let viewport = vk::PipelineViewportStateCreateInfo::default().viewport_count(1).scissor_count(1);
        let raster = vk::PipelineRasterizationStateCreateInfo::default()
            .polygon_mode(vk::PolygonMode::FILL)
            .cull_mode(vk::CullModeFlags::NONE)
            .line_width(1.0);
        let multisample = vk::PipelineMultisampleStateCreateInfo::default().rasterization_samples(vk::SampleCountFlags::TYPE_1);
        let blend_att = [vk::PipelineColorBlendAttachmentState::default()
            .blend_enable(true)
            .src_color_blend_factor(vk::BlendFactor::ONE)
            .dst_color_blend_factor(vk::BlendFactor::ONE_MINUS_SRC_ALPHA)
            .color_blend_op(vk::BlendOp::ADD)
            .src_alpha_blend_factor(vk::BlendFactor::ONE)
            .dst_alpha_blend_factor(vk::BlendFactor::ONE_MINUS_SRC_ALPHA)
            .alpha_blend_op(vk::BlendOp::ADD)
            .color_write_mask(vk::ColorComponentFlags::RGBA)];
        let blend = vk::PipelineColorBlendStateCreateInfo::default().attachments(&blend_att);
        let dynamic = [vk::DynamicState::VIEWPORT, vk::DynamicState::SCISSOR];
        let dynamic = vk::PipelineDynamicStateCreateInfo::default().dynamic_states(&dynamic);
        let info = vk::GraphicsPipelineCreateInfo::default()
            .stages(&stages)
            .vertex_input_state(&vertex)
            .input_assembly_state(&assembly)
            .viewport_state(&viewport)
            .rasterization_state(&raster)
            .multisample_state(&multisample)
            .color_blend_state(&blend)
            .dynamic_state(&dynamic)
            .layout(layout)
            .render_pass(render_pass)
            .subpass(0);
        let p = d
            .create_graphics_pipelines(vk::PipelineCache::null(), &[info], None)
            .map_err(|(_, e)| format!("pipeline: {e:?}"))?[0];
        d.destroy_shader_module(vs, None);
        d.destroy_shader_module(fs, None);
        Ok(p)
    }
}
