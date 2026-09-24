use khronos_egl as egl;

const EGL_PLATFORM_SURFACELESS_MESA: egl::Enum = 0x31DD;

pub(crate) const DRM_FORMAT_MOD_INVALID: u64 = 0x00ffffffffffffff;
pub(crate) const DRM_FORMAT_MOD_LINEAR: u64 = 0;

pub(crate) struct GlContext {
    pub(crate) egl: egl::DynamicInstance<egl::EGL1_4>,
    pub(crate) display: egl::Display,
    pub(crate) context: egl::Context,
    pub(crate) surface: Option<egl::Surface>,
    pub(crate) _gles_lib: Option<libloading::Library>,
    pub(crate) create_image: unsafe extern "C" fn(
        *mut std::ffi::c_void,
        *mut std::ffi::c_void,
        egl::Enum,
        *mut std::ffi::c_void,
        *const egl::Int,
    ) -> *mut std::ffi::c_void,
    pub(crate) destroy_image:
        unsafe extern "C" fn(*mut std::ffi::c_void, *mut std::ffi::c_void) -> egl::Boolean,
    pub(crate) image_target_texture_2d: unsafe extern "C" fn(u32, *mut std::ffi::c_void),
    pub(crate) bind_texture: unsafe extern "C" fn(u32, u32),
    pub(crate) delete_textures: unsafe extern "C" fn(i32, *const u32),
    pub(crate) bind_framebuffer: unsafe extern "C" fn(u32, u32),
    pub(crate) framebuffer_texture_2d: unsafe extern "C" fn(u32, u32, u32, u32, i32),
    pub(crate) check_framebuffer_status: unsafe extern "C" fn(u32) -> u32,
    pub(crate) delete_framebuffers: unsafe extern "C" fn(i32, *const u32),
    pub(crate) read_pixels:
        unsafe extern "C" fn(i32, i32, i32, i32, u32, u32, *mut std::ffi::c_void),
    pub(crate) get_error: unsafe extern "C" fn() -> u32,
    /// Texture + FBO reused across frames: creating and destroying the
    /// pair per frame is driver work the 60fps schedule cannot spare.
    /// The texture is re-pointed at each frame's EGLImage; the FBO
    /// stays attached to the texture.
    pub(crate) texture: u32,
    pub(crate) fbo: u32,
}

fn load_proc<T: Copy>(
    egl: &egl::DynamicInstance<egl::EGL1_4>,
    lib: Option<&libloading::Library>,
    name: &str,
) -> Result<T, String> {
    // Every T here is a GL/EGL function pointer, so it is exactly one
    // pointer wide; transmute_copy carries no size check, so assert the
    // invariant rather than let a future non-pointer T read out of
    // bounds.
    assert_eq!(
        std::mem::size_of::<T>(),
        std::mem::size_of::<*const ()>(),
        "load_proc target {name} is not pointer-sized"
    );
    if let Some(proc) = egl.get_proc_address(name) {
        return Ok(unsafe { std::mem::transmute_copy(&proc) });
    }
    if let Some(lib) = lib {
        let cname = std::ffi::CString::new(name).map_err(|e| e.to_string())?;
        // Symbol<T> derefs to T; the value is already the right type.
        if let Ok(sym) = unsafe { lib.get::<T>(cname.as_bytes_with_nul()) } {
            return Ok(*sym);
        }
    }
    Err(format!("could not resolve GL/EGL symbol {name}"))
}

impl GlContext {
    pub fn new() -> Result<Self, String> {
        let egl_inst = unsafe { egl::DynamicInstance::<egl::EGL1_4>::load_required() }
            .map_err(|e| format!("load EGL library: {e}"))?;

        type EglGetPlatformDisplayEXT = unsafe extern "C" fn(
            egl::Enum,
            *mut std::ffi::c_void,
            *const egl::Int,
        ) -> *mut std::ffi::c_void;

        let display = if let Some(get_platform_display) = egl_inst
            .get_proc_address("eglGetPlatformDisplayEXT")
            .or_else(|| egl_inst.get_proc_address("eglGetPlatformDisplay"))
        {
            let get_platform: EglGetPlatformDisplayEXT =
                unsafe { std::mem::transmute_copy(&get_platform_display) };
            let raw_display = unsafe {
                get_platform(
                    EGL_PLATFORM_SURFACELESS_MESA,
                    std::ptr::null_mut(),
                    [egl::NONE].as_ptr(),
                )
            };
            if raw_display.is_null() {
                unsafe { egl_inst.get_display(egl::DEFAULT_DISPLAY) }
                    .ok_or_else(|| "eglGetDisplay returned None".to_string())?
            } else {
                unsafe { egl::Display::from_ptr(raw_display) }
            }
        } else {
            unsafe { egl_inst.get_display(egl::DEFAULT_DISPLAY) }
                .ok_or_else(|| "eglGetDisplay returned None".to_string())?
        };

        egl_inst
            .initialize(display)
            .map_err(|e| format!("eglInitialize: {e}"))?;

        if let Err(e) = egl_inst.bind_api(egl::OPENGL_ES_API) {
            let _ = egl_inst.bind_api(egl::OPENGL_API);
            crate::ilog!("iris: record: EGL bind GLES failed ({e}), tried GL");
        }

        let config_attribs = [
            egl::RENDERABLE_TYPE,
            egl::OPENGL_ES2_BIT,
            egl::SURFACE_TYPE,
            egl::PBUFFER_BIT,
            egl::NONE,
        ];
        let mut configs = Vec::with_capacity(1);
        let _ = egl_inst.choose_config(display, &config_attribs, &mut configs);
        let config = configs.first().copied();

        // ES2 covers everything the readback uses: EGLImage textures,
        // a framebuffer, and glReadPixels.
        let ctx_attribs = [egl::CONTEXT_CLIENT_VERSION, 2, egl::NONE];
        let context = match config {
            Some(cfg) => egl_inst
                .create_context(display, cfg, None, &ctx_attribs)
                .map_err(|e| format!("eglCreateContext: {e}"))?,
            None => {
                let raw_ctx = unsafe {
                    type EglCreateContextRaw = unsafe extern "C" fn(
                        *mut std::ffi::c_void,
                        *mut std::ffi::c_void,
                        *mut std::ffi::c_void,
                        *const egl::Int,
                    )
                        -> *mut std::ffi::c_void;
                    if let Some(create_ctx_fn) = egl_inst.get_proc_address("eglCreateContext") {
                        let create_ctx: EglCreateContextRaw =
                            std::mem::transmute_copy(&create_ctx_fn);
                        create_ctx(
                            display.as_ptr(),
                            std::ptr::null_mut(),
                            std::ptr::null_mut(),
                            ctx_attribs.as_ptr(),
                        )
                    } else {
                        std::ptr::null_mut()
                    }
                };
                if raw_ctx.is_null() {
                    return Err("eglCreateContext failed without EGLConfig".to_string());
                }
                unsafe { egl::Context::from_ptr(raw_ctx) }
            }
        };

        let surface = match egl_inst.make_current(display, None, None, Some(context)) {
            Ok(()) => None,
            Err(_) => {
                let cfg = config.ok_or_else(|| "pbuffer surface needs EGLConfig".to_string())?;
                let pbuf_attribs = [egl::WIDTH, 1, egl::HEIGHT, 1, egl::NONE];
                let surf = egl_inst
                    .create_pbuffer_surface(display, cfg, &pbuf_attribs)
                    .map_err(|e| format!("eglCreatePbufferSurface: {e}"))?;
                egl_inst
                    .make_current(display, Some(surf), Some(surf), Some(context))
                    .map_err(|e| format!("eglMakeCurrent with pbuffer: {e}"))?;
                Some(surf)
            }
        };

        let gles_lib = unsafe {
            libloading::Library::new("libGLESv2.so.2")
                .or_else(|_| libloading::Library::new("libGLESv2.so"))
                .or_else(|_| libloading::Library::new("libGL.so.1"))
                .ok()
        };

        let create_image = load_proc(&egl_inst, gles_lib.as_ref(), "eglCreateImageKHR")
            .or_else(|_| load_proc(&egl_inst, gles_lib.as_ref(), "eglCreateImage"))?;
        let destroy_image = load_proc(&egl_inst, gles_lib.as_ref(), "eglDestroyImageKHR")
            .or_else(|_| load_proc(&egl_inst, gles_lib.as_ref(), "eglDestroyImage"))?;
        let image_target_texture_2d =
            load_proc(&egl_inst, gles_lib.as_ref(), "glEGLImageTargetTexture2DOES").or_else(
                |_| load_proc(&egl_inst, gles_lib.as_ref(), "glEGLImageTargetTexture2D"),
            )?;

        let gen_textures: unsafe extern "C" fn(i32, *mut u32) =
            load_proc(&egl_inst, gles_lib.as_ref(), "glGenTextures")?;
        let bind_texture = load_proc(&egl_inst, gles_lib.as_ref(), "glBindTexture")?;
        let delete_textures = load_proc(&egl_inst, gles_lib.as_ref(), "glDeleteTextures")?;
        let gen_framebuffers: unsafe extern "C" fn(i32, *mut u32) =
            load_proc(&egl_inst, gles_lib.as_ref(), "glGenFramebuffers")
                .or_else(|_| load_proc(&egl_inst, gles_lib.as_ref(), "glGenFramebuffersOES"))?;
        let bind_framebuffer = load_proc(&egl_inst, gles_lib.as_ref(), "glBindFramebuffer")
            .or_else(|_| load_proc(&egl_inst, gles_lib.as_ref(), "glBindFramebufferOES"))?;
        let framebuffer_texture_2d =
            load_proc(&egl_inst, gles_lib.as_ref(), "glFramebufferTexture2D").or_else(|_| {
                load_proc(&egl_inst, gles_lib.as_ref(), "glFramebufferTexture2DOES")
            })?;
        let check_framebuffer_status =
            load_proc(&egl_inst, gles_lib.as_ref(), "glCheckFramebufferStatus").or_else(|_| {
                load_proc(&egl_inst, gles_lib.as_ref(), "glCheckFramebufferStatusOES")
            })?;
        let delete_framebuffers =
            load_proc(&egl_inst, gles_lib.as_ref(), "glDeleteFramebuffers")
                .or_else(|_| load_proc(&egl_inst, gles_lib.as_ref(), "glDeleteFramebuffersOES"))?;
        let read_pixels = load_proc(&egl_inst, gles_lib.as_ref(), "glReadPixels")?;
        let get_error = load_proc(&egl_inst, gles_lib.as_ref(), "glGetError")?;

        // Allocate the reusable texture + FBO while the context is
        // current.
        let (texture, fbo) = unsafe {
            let mut t = 0u32;
            (gen_textures)(1, &mut t);
            let mut f = 0u32;
            (gen_framebuffers)(1, &mut f);
            (t, f)
        };

        // read_dma_buf binds the context for each frame, so no binding
        // made in between on this thread breaks the readback.
        let _ = egl_inst.make_current(display, None, None, None);

        Ok(Self {
            egl: egl_inst,
            display,
            context,
            surface,
            _gles_lib: gles_lib,
            create_image,
            destroy_image,
            image_target_texture_2d,
            bind_texture,
            delete_textures,
            bind_framebuffer,
            framebuffer_texture_2d,
            check_framebuffer_status,
            delete_framebuffers,
            read_pixels,
            get_error,
            texture,
            fbo,
        })
    }
}

impl Drop for GlContext {
    fn drop(&mut self) {
        // The stream is dead by now, so the context can be bound here
        // to delete the cached GL objects before it is destroyed.
        let _ = self
            .egl
            .make_current(self.display, self.surface, self.surface, Some(self.context));
        unsafe {
            if self.fbo != 0 {
                (self.delete_framebuffers)(1, &self.fbo);
            }
            if self.texture != 0 {
                (self.delete_textures)(1, &self.texture);
            }
        }
        let _ = self.egl.make_current(self.display, None, None, None);
        if let Some(surface) = self.surface {
            let _ = self.egl.destroy_surface(self.display, surface);
        }
        let _ = self.egl.destroy_context(self.display, self.context);
        let _ = self.egl.terminate(self.display);
    }
}
