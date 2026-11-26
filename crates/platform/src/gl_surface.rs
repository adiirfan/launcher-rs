//! EGL display, OpenGL ES 3.0 context and the window surface.
//! The context outlives the surface, so textures survive the screen
//! turning off or the window being rebuilt.

use std::ffi::{CString, c_void};

use khronos_egl as egl;
use ndk::native_window::NativeWindow;

type Egl = egl::Instance<egl::Static>;

pub struct GlSurface {
    egl: Egl,
    display: egl::Display,
    config: egl::Config,
    context: egl::Context,
    surface: Option<egl::Surface>,
    /// The window size the surface was built for. The surface follows the
    /// window on its own, but keeps buffers of its first size around.
    built_for: (i32, i32),
    gl: glow::Context,
}

impl GlSurface {
    pub fn new(window: &NativeWindow) -> Result<GlSurface, String> {
        let egl = egl::Instance::new(egl::Static);
        let display = unsafe { egl.get_display(egl::DEFAULT_DISPLAY) }
            .ok_or("no EGL display")?;
        egl.initialize(display).map_err(|e| format!("eglInitialize: {e}"))?;

        let attrs = [
            egl::RED_SIZE, 8,
            egl::GREEN_SIZE, 8,
            egl::BLUE_SIZE, 8,
            egl::ALPHA_SIZE, 8,
            egl::DEPTH_SIZE, 0,
            egl::STENCIL_SIZE, 0,
            egl::RENDERABLE_TYPE, egl::OPENGL_ES3_BIT,
            egl::SURFACE_TYPE, egl::WINDOW_BIT,
            egl::NONE,
        ];
        let config = egl
            .choose_first_config(display, &attrs)
            .map_err(|e| format!("eglChooseConfig: {e}"))?
            .ok_or("no RGBA8888 ES3 config")?;
        let context = egl
            .create_context(display, config, None, &[egl::CONTEXT_MAJOR_VERSION, 3, egl::NONE])
            .map_err(|e| format!("eglCreateContext: {e}"))?;

        // glow reads GL_VERSION while loading, so a context must be current first.
        let surface = make_surface(&egl, display, config, context, window)?;
        let gl = unsafe { glow::Context::from_loader_function(|name| load_gl(&egl, name)) };
        let built_for = (window.width(), window.height());
        let me = GlSurface { egl, display, config, context, surface: Some(surface), built_for, gl };
        me.log_size();
        Ok(me)
    }

    /// Make a window surface for `window` and make it current.
    pub fn resume(&mut self, window: &NativeWindow) -> Result<(), String> {
        self.suspend();
        self.surface = Some(make_surface(&self.egl, self.display, self.config, self.context, window)?);
        self.built_for = (window.width(), window.height());
        self.log_size();
        Ok(())
    }

    fn log_size(&self) {
        let (w, h) = self.size();
        log::info!("GlSurface: surface rebuilt {w}x{h}");
    }

    /// Drop the window surface; keep the context.
    pub fn suspend(&mut self) {
        if let Some(surface) = self.surface.take() {
            let _ = self.egl.make_current(self.display, None, None, None);
            let _ = self.egl.destroy_surface(self.display, surface);
            log::info!("GlSurface: surface dropped");
        }
    }

    /// The window is no longer the size this surface was built for.
    pub fn outgrown(&self, window: &NativeWindow) -> bool {
        self.surface.is_some() && (window.width(), window.height()) != self.built_for
    }

    pub fn has_surface(&self) -> bool {
        self.surface.is_some()
    }

    pub fn gl(&self) -> &glow::Context {
        &self.gl
    }

    pub fn size(&self) -> (i32, i32) {
        let Some(surface) = self.surface else { return (0, 0) };
        let w = self.egl.query_surface(self.display, surface, egl::WIDTH).unwrap_or(0);
        let h = self.egl.query_surface(self.display, surface, egl::HEIGHT).unwrap_or(0);
        (w, h)
    }

    pub fn swap(&self) -> Result<(), String> {
        let surface = self.surface.ok_or("no surface")?;
        self.egl.swap_buffers(self.display, surface).map_err(|e| format!("eglSwapBuffers: {e}"))
    }
}

impl Drop for GlSurface {
    fn drop(&mut self) {
        self.suspend();
        let _ = self.egl.destroy_context(self.display, self.context);
        let _ = self.egl.terminate(self.display);
    }
}

fn make_surface(
    egl: &Egl,
    display: egl::Display,
    config: egl::Config,
    context: egl::Context,
    window: &NativeWindow,
) -> Result<egl::Surface, String> {
    let surface = unsafe {
        egl.create_window_surface(display, config, window.ptr().as_ptr() as egl::NativeWindowType, None)
    }
    .map_err(|e| format!("eglCreateWindowSurface: {e}"))?;
    egl.make_current(display, Some(surface), Some(surface), Some(context))
        .map_err(|e| format!("eglMakeCurrent: {e}"))?;
    Ok(surface)
}

/// Core GLES functions come from libGLESv3 directly: older Android
/// versions only hand out extensions through eglGetProcAddress.
fn load_gl(egl: &Egl, name: &str) -> *const c_void {
    static LIB: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    let lib = *LIB.get_or_init(|| unsafe {
        libc::dlopen(c"libGLESv3.so".as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL) as usize
    });
    let cname = CString::new(name).unwrap();
    if lib != 0 {
        let p = unsafe { libc::dlsym(lib as *mut c_void, cname.as_ptr()) };
        if !p.is_null() {
            return p;
        }
    }
    egl.get_proc_address(name).map_or(std::ptr::null(), |p| p as *const c_void)
}
