//! SPIKE: the window's drawing target, OpenGL ES (default) or Vulkan
//! (`adb shell setprop debug.launcherrs.gpu vulkan`, read at start; falls
//! back to OpenGL ES if Vulkan can't start).

use std::cell::RefCell;

use ndk::native_window::NativeWindow;

use crate::gl_surface::GlSurface;
use crate::vk_surface::VkSurface;

pub enum Gpu {
    Gl(GlSurface),
    /// Borrowed mutably for uploads and drawing; nothing holds it across calls.
    Vk(Box<RefCell<VkSurface>>),
}

impl Gpu {
    pub fn new(window: &NativeWindow, icon_atlas: (u32, u32), glyph_atlas: (u32, u32)) -> Result<Gpu, String> {
        if crate::property(c"debug.launcherrs.gpu") == "vulkan" {
            let images = crate::property(c"debug.launcherrs.vkimages").parse().unwrap_or(2);
            match VkSurface::new(window, icon_atlas, glyph_atlas, images) {
                Ok(v) => {
                    log::info!("drawing with Vulkan");
                    return Ok(Gpu::Vk(Box::new(RefCell::new(v))));
                }
                Err(e) => log::warn!("Vulkan failed ({e}); using OpenGL ES"),
            }
        }
        GlSurface::new(window).map(Gpu::Gl)
    }

    pub fn resume(&mut self, window: &NativeWindow) -> Result<(), String> {
        match self {
            Gpu::Gl(g) => g.resume(window),
            Gpu::Vk(v) => v.get_mut().resume(window),
        }
    }

    pub fn suspend(&mut self) {
        match self {
            Gpu::Gl(g) => g.suspend(),
            Gpu::Vk(v) => v.get_mut().suspend(),
        }
    }

    pub fn has_surface(&self) -> bool {
        match self {
            Gpu::Gl(g) => g.has_surface(),
            Gpu::Vk(v) => v.borrow().has_surface(),
        }
    }

    pub fn size(&self) -> (i32, i32) {
        match self {
            Gpu::Gl(g) => g.size(),
            Gpu::Vk(v) => v.borrow().size(),
        }
    }

    pub fn swap(&self) -> Result<(), String> {
        match self {
            Gpu::Gl(g) => g.swap(),
            Gpu::Vk(v) => v.borrow_mut().swap(),
        }
    }

    pub fn gl(&self) -> Option<&glow::Context> {
        match self {
            Gpu::Gl(g) => Some(g.gl()),
            Gpu::Vk(_) => None,
        }
    }
}
