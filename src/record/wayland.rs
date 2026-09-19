//! Wayland window recording: xdg-desktop-portal ScreenCast for source
//! selection (the compositor's own window picker), PipeWire for frame
//! delivery, and the shared ffmpeg encoder for the mp4.
//! Frames arrive as MemPtr, MemFd, or DMA-buf buffers; MAP_BUFFERS asks
//! PipeWire to mmap MemFd for us. DMA-buf buffers are imported via EGL
//! and read back via glReadPixels, or via CPU mmap when permitted.

use std::cell::RefCell;
use std::os::fd::OwnedFd;
use std::path::PathBuf;
use std::rc::Rc;

use ashpd::desktop::screencast::{
    CursorMode, SelectSourcesOptions, Screencast, SourceType,
};
use ashpd::desktop::PersistMode;
use enumflags2::BitFlags;
use khronos_egl as egl;
use pipewire::properties::properties;
use pipewire::spa::buffer::DataType;
use pipewire::spa::param::format::{FormatProperties, MediaType, MediaSubtype};
use pipewire::spa::param::video::{VideoFormat, VideoInfoRaw};
use pipewire::spa::param::ParamType;
use pipewire::spa::pod;
use pipewire::spa::utils::{Choice, ChoiceEnum, ChoiceFlags, SpaTypes};
use pipewire::stream::{StreamFlags, StreamRef};

use super::encoder::{Encoder, EncoderConfig};
use super::{RecordingSpec, CANCELLED_PREFIX};

const EGL_PLATFORM_SURFACELESS_MESA: egl::Enum = 0x31DD;
const EGL_LINUX_DMA_BUF_EXT: egl::Enum = 0x3270;
const EGL_LINUX_DRM_FOURCC_EXT: egl::Int = 0x3271;
const EGL_DMA_BUF_PLANE0_FD_EXT: egl::Int = 0x3272;
const EGL_DMA_BUF_PLANE0_OFFSET_EXT: egl::Int = 0x3273;
const EGL_DMA_BUF_PLANE0_PITCH_EXT: egl::Int = 0x3274;
const EGL_DMA_BUF_PLANE0_MODIFIER_LO_EXT: egl::Int = 0x3443;
const EGL_DMA_BUF_PLANE0_MODIFIER_HI_EXT: egl::Int = 0x3444;

const EGL_DMA_BUF_PLANE1_FD_EXT: egl::Int = 0x3275;
const EGL_DMA_BUF_PLANE1_OFFSET_EXT: egl::Int = 0x3276;
const EGL_DMA_BUF_PLANE1_PITCH_EXT: egl::Int = 0x3277;
const EGL_DMA_BUF_PLANE1_MODIFIER_LO_EXT: egl::Int = 0x3445;
const EGL_DMA_BUF_PLANE1_MODIFIER_HI_EXT: egl::Int = 0x3446;

const EGL_DMA_BUF_PLANE2_FD_EXT: egl::Int = 0x3278;
const EGL_DMA_BUF_PLANE2_OFFSET_EXT: egl::Int = 0x3279;
const EGL_DMA_BUF_PLANE2_PITCH_EXT: egl::Int = 0x327A;
const EGL_DMA_BUF_PLANE2_MODIFIER_LO_EXT: egl::Int = 0x3447;
const EGL_DMA_BUF_PLANE2_MODIFIER_HI_EXT: egl::Int = 0x3448;

const EGL_DMA_BUF_PLANE3_FD_EXT: egl::Int = 0x3440;
const EGL_DMA_BUF_PLANE3_OFFSET_EXT: egl::Int = 0x3441;
const EGL_DMA_BUF_PLANE3_PITCH_EXT: egl::Int = 0x3442;
const EGL_DMA_BUF_PLANE3_MODIFIER_LO_EXT: egl::Int = 0x3449;
const EGL_DMA_BUF_PLANE3_MODIFIER_HI_EXT: egl::Int = 0x344A;

const DRM_FORMAT_MOD_INVALID: u64 = 0x00ffffffffffffff;
const DRM_FORMAT_MOD_LINEAR: u64 = 0;

const GL_TEXTURE_2D: u32 = 0x0DE1;
const GL_FRAMEBUFFER: u32 = 0x8D40;
const GL_COLOR_ATTACHMENT0: u32 = 0x8CE0;
const GL_FRAMEBUFFER_COMPLETE: u32 = 0x8CD5;
const GL_RGBA: u32 = 0x1908;
const GL_UNSIGNED_BYTE: u32 = 0x1401;

const fn fourcc_code(a: u8, b: u8, c: u8, d: u8) -> u32 {
    (a as u32) | ((b as u32) << 8) | ((c as u32) << 16) | ((d as u32) << 24)
}

fn drm_fourcc_for_format(format: VideoFormat) -> Result<u32, String> {
    match format {
        VideoFormat::BGRx => Ok(fourcc_code(b'X', b'R', b'2', b'4')),
        VideoFormat::BGRA => Ok(fourcc_code(b'A', b'R', b'2', b'4')),
        VideoFormat::RGBx => Ok(fourcc_code(b'X', b'B', b'2', b'4')),
        VideoFormat::RGBA => Ok(fourcc_code(b'A', b'B', b'2', b'4')),
        other => Err(format!("no DRM fourcc mapping for format {other:?}")),
    }
}

#[derive(Clone, Copy, Debug)]
struct PlaneInfo {
    fd: i32,
    offset: u32,
    stride: u32,
}

struct GlContext {
    egl: egl::DynamicInstance<egl::EGL1_4>,
    display: egl::Display,
    context: egl::Context,
    surface: Option<egl::Surface>,
    _gles_lib: Option<libloading::Library>,
    create_image: unsafe extern "C" fn(
        *mut std::ffi::c_void,
        *mut std::ffi::c_void,
        egl::Enum,
        *mut std::ffi::c_void,
        *const egl::Int,
    ) -> *mut std::ffi::c_void,
    destroy_image: unsafe extern "C" fn(*mut std::ffi::c_void, *mut std::ffi::c_void) -> egl::Boolean,
    image_target_texture_2d: unsafe extern "C" fn(u32, *mut std::ffi::c_void),
    gen_textures: unsafe extern "C" fn(i32, *mut u32),
    bind_texture: unsafe extern "C" fn(u32, u32),
    delete_textures: unsafe extern "C" fn(i32, *const u32),
    gen_framebuffers: unsafe extern "C" fn(i32, *mut u32),
    bind_framebuffer: unsafe extern "C" fn(u32, u32),
    framebuffer_texture_2d: unsafe extern "C" fn(u32, u32, u32, u32, i32),
    check_framebuffer_status: unsafe extern "C" fn(u32) -> u32,
    delete_framebuffers: unsafe extern "C" fn(i32, *const u32),
    viewport: unsafe extern "C" fn(i32, i32, i32, i32),
    read_pixels: unsafe extern "C" fn(i32, i32, i32, i32, u32, u32, *mut std::ffi::c_void),
}

fn load_proc<T: Copy>(
    egl: &egl::DynamicInstance<egl::EGL1_4>,
    lib: Option<&libloading::Library>,
    name: &str,
) -> Result<T, String> {
    if let Some(proc) = egl.get_proc_address(name) {
        return Ok(unsafe { std::mem::transmute_copy(&proc) });
    }
    if let Some(lib) = lib {
        let cname = std::ffi::CString::new(name).map_err(|e| e.to_string())?;
        if let Ok(sym) = unsafe { lib.get::<T>(cname.as_bytes_with_nul()) } {
            return Ok(unsafe { std::mem::transmute_copy(&*sym) });
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
                unsafe { std::mem::transmute(raw_display) }
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
                    ) -> *mut std::ffi::c_void;
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
                unsafe { std::mem::transmute(raw_ctx) }
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
        let image_target_texture_2d = load_proc(
            &egl_inst,
            gles_lib.as_ref(),
            "glEGLImageTargetTexture2DOES",
        )
        .or_else(|_| load_proc(&egl_inst, gles_lib.as_ref(), "glEGLImageTargetTexture2D"))?;

        let gen_textures = load_proc(&egl_inst, gles_lib.as_ref(), "glGenTextures")?;
        let bind_texture = load_proc(&egl_inst, gles_lib.as_ref(), "glBindTexture")?;
        let delete_textures = load_proc(&egl_inst, gles_lib.as_ref(), "glDeleteTextures")?;
        let gen_framebuffers = load_proc(&egl_inst, gles_lib.as_ref(), "glGenFramebuffers")
            .or_else(|_| load_proc(&egl_inst, gles_lib.as_ref(), "glGenFramebuffersOES"))?;
        let bind_framebuffer = load_proc(&egl_inst, gles_lib.as_ref(), "glBindFramebuffer")
            .or_else(|_| load_proc(&egl_inst, gles_lib.as_ref(), "glBindFramebufferOES"))?;
        let framebuffer_texture_2d = load_proc(&egl_inst, gles_lib.as_ref(), "glFramebufferTexture2D")
            .or_else(|_| load_proc(&egl_inst, gles_lib.as_ref(), "glFramebufferTexture2DOES"))?;
        let check_framebuffer_status = load_proc(
            &egl_inst,
            gles_lib.as_ref(),
            "glCheckFramebufferStatus",
        )
        .or_else(|_| load_proc(&egl_inst, gles_lib.as_ref(), "glCheckFramebufferStatusOES"))?;
        let delete_framebuffers = load_proc(&egl_inst, gles_lib.as_ref(), "glDeleteFramebuffers")
            .or_else(|_| load_proc(&egl_inst, gles_lib.as_ref(), "glDeleteFramebuffersOES"))?;
        let viewport = load_proc(&egl_inst, gles_lib.as_ref(), "glViewport")?;
        let read_pixels = load_proc(&egl_inst, gles_lib.as_ref(), "glReadPixels")?;

        Ok(Self {
            egl: egl_inst,
            display,
            context,
            surface,
            _gles_lib: gles_lib,
            create_image,
            destroy_image,
            image_target_texture_2d,
            gen_textures,
            bind_texture,
            delete_textures,
            gen_framebuffers,
            bind_framebuffer,
            framebuffer_texture_2d,
            check_framebuffer_status,
            delete_framebuffers,
            viewport,
            read_pixels,
        })
    }

    pub fn read_dma_buf(
        &mut self,
        width: u32,
        height: u32,
        format: VideoFormat,
        modifier: u64,
        planes: &[PlaneInfo],
        scratch: &mut Vec<u8>,
    ) -> Result<(), String> {
        if planes.is_empty() {
            return Err("empty DMA-buf planes".to_string());
        }
        let fourcc = drm_fourcc_for_format(format)?;
        let mut attribs: Vec<egl::Int> = Vec::with_capacity(32);
        attribs.push(egl::WIDTH);
        attribs.push(width as egl::Int);
        attribs.push(egl::HEIGHT);
        attribs.push(height as egl::Int);
        attribs.push(EGL_LINUX_DRM_FOURCC_EXT);
        attribs.push(fourcc as egl::Int);

        const PLANE_FD_ATTRS: [egl::Int; 4] = [
            EGL_DMA_BUF_PLANE0_FD_EXT,
            EGL_DMA_BUF_PLANE1_FD_EXT,
            EGL_DMA_BUF_PLANE2_FD_EXT,
            EGL_DMA_BUF_PLANE3_FD_EXT,
        ];
        const PLANE_OFFSET_ATTRS: [egl::Int; 4] = [
            EGL_DMA_BUF_PLANE0_OFFSET_EXT,
            EGL_DMA_BUF_PLANE1_OFFSET_EXT,
            EGL_DMA_BUF_PLANE2_OFFSET_EXT,
            EGL_DMA_BUF_PLANE3_OFFSET_EXT,
        ];
        const PLANE_PITCH_ATTRS: [egl::Int; 4] = [
            EGL_DMA_BUF_PLANE0_PITCH_EXT,
            EGL_DMA_BUF_PLANE1_PITCH_EXT,
            EGL_DMA_BUF_PLANE2_PITCH_EXT,
            EGL_DMA_BUF_PLANE3_PITCH_EXT,
        ];
        const PLANE_MOD_LO_ATTRS: [egl::Int; 4] = [
            EGL_DMA_BUF_PLANE0_MODIFIER_LO_EXT,
            EGL_DMA_BUF_PLANE1_MODIFIER_LO_EXT,
            EGL_DMA_BUF_PLANE2_MODIFIER_LO_EXT,
            EGL_DMA_BUF_PLANE3_MODIFIER_LO_EXT,
        ];
        const PLANE_MOD_HI_ATTRS: [egl::Int; 4] = [
            EGL_DMA_BUF_PLANE0_MODIFIER_HI_EXT,
            EGL_DMA_BUF_PLANE1_MODIFIER_HI_EXT,
            EGL_DMA_BUF_PLANE2_MODIFIER_HI_EXT,
            EGL_DMA_BUF_PLANE3_MODIFIER_HI_EXT,
        ];

        for (i, plane) in planes.iter().enumerate().take(4) {
            attribs.push(PLANE_FD_ATTRS[i]);
            attribs.push(plane.fd as egl::Int);
            attribs.push(PLANE_OFFSET_ATTRS[i]);
            attribs.push(plane.offset as egl::Int);
            attribs.push(PLANE_PITCH_ATTRS[i]);
            attribs.push(plane.stride as egl::Int);
            if modifier != DRM_FORMAT_MOD_INVALID {
                attribs.push(PLANE_MOD_LO_ATTRS[i]);
                attribs.push((modifier & 0xFFFFFFFF) as egl::Int);
                attribs.push(PLANE_MOD_HI_ATTRS[i]);
                attribs.push(((modifier >> 32) & 0xFFFFFFFF) as egl::Int);
            }
        }
        attribs.push(egl::NONE);

        let image = unsafe {
            (self.create_image)(
                self.display.as_ptr(),
                std::ptr::null_mut(),
                EGL_LINUX_DMA_BUF_EXT,
                std::ptr::null_mut(),
                attribs.as_ptr(),
            )
        };
        if image.is_null() {
            let err = self.egl.get_error();
            return Err(format!("eglCreateImageKHR failed: error {err:?}"));
        }

        let result = (|| unsafe {
            let mut texture = 0u32;
            (self.gen_textures)(1, &mut texture);
            if texture == 0 {
                return Err("glGenTextures failed".to_string());
            }
            (self.bind_texture)(GL_TEXTURE_2D, texture);
            (self.image_target_texture_2d)(GL_TEXTURE_2D, image);

            let mut fbo = 0u32;
            (self.gen_framebuffers)(1, &mut fbo);
            if fbo == 0 {
                (self.bind_texture)(GL_TEXTURE_2D, 0);
                (self.delete_textures)(1, &texture);
                return Err("glGenFramebuffers failed".to_string());
            }
            (self.bind_framebuffer)(GL_FRAMEBUFFER, fbo);
            (self.framebuffer_texture_2d)(
                GL_FRAMEBUFFER,
                GL_COLOR_ATTACHMENT0,
                GL_TEXTURE_2D,
                texture,
                0,
            );

            let status = (self.check_framebuffer_status)(GL_FRAMEBUFFER);
            if status != GL_FRAMEBUFFER_COMPLETE {
                (self.bind_framebuffer)(GL_FRAMEBUFFER, 0);
                (self.delete_framebuffers)(1, &fbo);
                (self.bind_texture)(GL_TEXTURE_2D, 0);
                (self.delete_textures)(1, &texture);
                return Err(format!("glCheckFramebufferStatus returned {status:#x}"));
            }

            (self.viewport)(0, 0, width as i32, height as i32);
            let total_bytes = (width as usize) * (height as usize) * 4;
            scratch.resize(total_bytes, 0);
            (self.read_pixels)(
                0,
                0,
                width as i32,
                height as i32,
                GL_RGBA,
                GL_UNSIGNED_BYTE,
                scratch.as_mut_ptr() as *mut std::ffi::c_void,
            );

            (self.bind_framebuffer)(GL_FRAMEBUFFER, 0);
            (self.delete_framebuffers)(1, &fbo);
            (self.bind_texture)(GL_TEXTURE_2D, 0);
            (self.delete_textures)(1, &texture);
            Ok(())
        })();

        unsafe {
            (self.destroy_image)(self.display.as_ptr(), image);
        }

        result?;

        for px in scratch.chunks_exact_mut(4) {
            px[3] = 0xFF;
        }

        Ok(())
    }
}

impl Drop for GlContext {
    fn drop(&mut self) {
        let _ = self.egl.make_current(self.display, None, None, None);
        if let Some(surface) = self.surface {
            let _ = self.egl.destroy_surface(self.display, surface);
        }
        let _ = self.egl.destroy_context(self.display, self.context);
        let _ = self.egl.terminate(self.display);
    }
}
/// Negotiate the portal side: session, source selection (window picker
/// shown by the compositor), start, PipeWire remote fd.
async fn portal_negotiate() -> Result<(OwnedFd, u32), String> {
    let screencast = Screencast::new()
        .await
        .map_err(|e| format!("portal ScreenCast unavailable (is xdg-desktop-portal running?): {e}"))?;
    let session = screencast
        .create_session(Default::default())
        .await
        .map_err(|e| format!("portal session create failed: {e}"))?;

    let options = SelectSourcesOptions::default()
        .set_sources(BitFlags::from(SourceType::Window))
        .set_cursor_mode(CursorMode::Hidden)
        .set_multiple(false)
        .set_persist_mode(PersistMode::DoNot);
    screencast
        .select_sources(&session, options)
        .await
        .map_err(|e| format!("portal select_sources failed: {e}"))?;

    let streams = screencast
        .start(&session, None, Default::default())
        .await
        .map_err(|e| format!("portal start failed: {e}"))?
        .response()
        .map_err(|e| {
            if matches!(e, ashpd::Error::Response(ashpd::desktop::ResponseError::Cancelled)) {
                format!("{CANCELLED_PREFIX} portal dialog dismissed")
            } else {
                format!("portal start denied or failed: {e}")
            }
        })?;

    let stream = streams
        .streams()
        .first()
        .ok_or_else(|| "portal returned no streams".to_string())?;
    let node_id = stream.pipe_wire_node_id();

    let fd = screencast
        .open_pipe_wire_remote(&session, Default::default())
        .await
        .map_err(|e| format!("portal PipeWire remote failed: {e}"))?;
    Ok((fd, node_id))
}

struct Shared {
    encoder: Option<Encoder>,
    error: Option<String>,
    quit: Option<pipewire::main_loop::WeakMainLoop>,
    /// Set once the encoder is taken for finish(): on_process can fire
    /// once more as the loop drains, and must not respawn ffmpeg.
    done: bool,
}

struct StreamData {
    output: PathBuf,
    fps: u32,
    mic: bool,
    rec_format: crate::config::RecordingFormat,
    rec_encoder: crate::config::RecordingEncoder,
    format: Option<(u32, u32, VideoFormat, u64)>,
    shared: Rc<RefCell<Shared>>,
    unsupported_reported: bool,
    frames: u64,
    gl_context: Option<GlContext>,
    scratch: Vec<u8>,
}

impl StreamData {
    fn quit(&self) {
        let mainloop = self.shared.borrow().quit.as_ref().and_then(|w| w.upgrade());
        if let Some(mainloop) = mainloop {
            mainloop.quit();
        }
    }

    fn fail(&mut self, error: String) {
        self.shared.borrow_mut().error = Some(error);
        self.quit();
    }
}

fn on_process(stream: &StreamRef, data: &mut StreamData) {
    let Some(mut buffer) = stream.dequeue_buffer() else {
        return;
    };
    let datas = buffer.datas_mut();
    let Some(first) = datas.first_mut() else {
        return;
    };

    match first.type_() {
        DataType::MemPtr | DataType::MemFd => {
            let Some((width, height, format, _modifier)) = data.format else {
                return;
            };
            let (offset, stride, size) = {
                let chunk = first.chunk();
                (chunk.offset() as usize, chunk.stride() as usize, chunk.size() as usize)
            };
            let Some(buf) = first.data() else {
                return;
            };
            let end = offset.saturating_add(size).min(buf.len());
            let src = &buf[offset.min(end)..end];

            if let Err(e) = convert_frame(src, stride, width, height, format, &mut data.scratch) {
                data.fail(e);
                return;
            }
        }
        DataType::DmaBuf => {
            let Some((width, height, format, modifier)) = data.format else {
                return;
            };
            if let Some(gl) = &mut data.gl_context {
                let mut planes = Vec::with_capacity(datas.len());
                for d in datas.iter() {
                    let raw = d.as_raw();
                    let chunk = d.chunk();
                    planes.push(PlaneInfo {
                        fd: raw.fd as i32,
                        offset: (chunk.offset() + raw.mapoffset) as u32,
                        stride: chunk.stride() as u32,
                    });
                }
                if let Err(e) = gl.read_dma_buf(
                    width,
                    height,
                    format,
                    modifier,
                    &planes,
                    &mut data.scratch,
                ) {
                    data.fail(format!("DMA-buf EGL import failed: {e}"));
                    return;
                }
            } else {
                let (offset, stride, size) = {
                    let chunk = first.chunk();
                    (chunk.offset() as usize, chunk.stride() as usize, chunk.size() as usize)
                };
                if let Some(buf) = first.data() {
                    let end = offset.saturating_add(size).min(buf.len());
                    let src = &buf[offset.min(end)..end];

                    if let Err(e) = convert_frame(src, stride, width, height, format, &mut data.scratch) {
                        data.fail(e);
                        return;
                    }
                } else {
                    if !data.unsupported_reported {
                        data.unsupported_reported = true;
                        data.fail(format!(
                            "compositor offers only DMA-buf buffers (modifier {modifier:#x}); EGL is unavailable and buffer is not CPU-mappable"
                        ));
                    }
                    return;
                }
            }
        }
        other => {
            if !data.unsupported_reported {
                data.unsupported_reported = true;
                data.fail(format!(
                    "compositor offers only {other:?} buffers; iris's Wayland path needs MemPtr, MemFd, or DMA-buf"
                ));
            }
            return;
        }
    }

    let Some((width, height, _format, _modifier)) = data.format else {
        return;
    };

    let mut shared = data.shared.borrow_mut();
    if shared.done {
        return;
    }
    if shared.encoder.is_none() {
        match Encoder::start(&EncoderConfig {
            output: data.output.clone(),
            width,
            height,
            fps: data.fps,
            mic: data.mic,
            format: data.rec_format,
            encoder: data.rec_encoder,
        }) {
            Ok(encoder) => shared.encoder = Some(encoder),
            Err(e) => {
                drop(shared);
                data.fail(e);
                return;
            }
        }
    }
    let frame = std::mem::replace(&mut data.scratch, shared.encoder.as_mut().unwrap().take_buf());
    let write_result = shared.encoder.as_mut().unwrap().write_frame(frame);
    drop(shared);
    data.frames += 1;
    if data.frames == 1 {
        crate::ilog!("iris: record: first frame written");
    }
    if let Err(e) = write_result {
        crate::ilog!("iris: record: write_frame failed: {e}");
        data.fail(e);
    }
}

/// Swizzle one PipeWire frame into `out` (resized to w*h*4), forcing
/// alpha to opaque. `out` is reused across frames so recording does not
/// allocate a multi-MB buffer per frame.
fn convert_frame(
    src: &[u8],
    stride: usize,
    width: u32,
    height: u32,
    format: VideoFormat,
    out: &mut Vec<u8>,
) -> Result<(), String> {
    let (w, h) = (width as usize, height as usize);
    let stride = if stride == 0 { w * 4 } else { stride };
    if src.len() < stride * h {
        return Err(format!(
            "short frame buffer: {} bytes for {w}x{h} at stride {stride}",
            src.len()
        ));
    }
    out.resize(w * h * 4, 0);
    let pixels = w * h;
    // Parallel row-band swizzle: a single-threaded per-pixel loop at
    // 1080p+ is a visible slice of the frame budget and drops frames.
    const PARALLEL_MIN: usize = 1 << 20; // ~1 MP
    let bgrx = matches!(format, VideoFormat::BGRx | VideoFormat::BGRA);
    let rgbx = matches!(format, VideoFormat::RGBx | VideoFormat::RGBA);
    if !bgrx && !rgbx {
        return Err(format!(
            "unsupported negotiated pixel format {format:?}; expected BGRx/BGRA/RGBx/RGBA"
        ));
    }
    let swizzle_row = |row_out: &mut [u8], row_in: &[u8]| {
        for (o, px) in row_out.chunks_exact_mut(4).zip(row_in[..w * 4].chunks_exact(4)) {
            let v = u32::from_le_bytes([px[0], px[1], px[2], px[3]]);
            let rgb = if bgrx {
                (v & 0xFF00) | ((v & 0xFF) << 16) | ((v >> 16) & 0xFF)
            } else {
                v & 0xFF_FFFF
            };
            o.copy_from_slice(&(rgb | 0xFF00_0000).to_le_bytes());
        }
    };
    if pixels < PARALLEL_MIN {
        for (row_out, row_in) in out.chunks_exact_mut(w * 4).zip(src.chunks(stride).take(h)) {
            swizzle_row(row_out, row_in);
        }
    } else {
        let threads = std::thread::available_parallelism()
            .map(|n| n.get().min(8))
            .unwrap_or(4)
            .min(h)
            .max(1);
        let band = h.div_ceil(threads);
        std::thread::scope(|scope| {
            let mut out_rest = out.as_mut_slice();
            let mut in_rest = src;
            for _ in 0..threads {
                let rows = band.min(in_rest.len() / stride);
                if rows == 0 {
                    break;
                }
                let (o_chunk, o_rest) = out_rest.split_at_mut(rows * w * 4);
                let (i_chunk, i_rest) = in_rest.split_at(rows * stride);
                out_rest = o_rest;
                in_rest = i_rest;
                scope.spawn(move || {
                    for (row_out, row_in) in
                        o_chunk.chunks_exact_mut(w * 4).zip(i_chunk.chunks(stride).take(rows))
                    {
                        swizzle_row(row_out, row_in);
                    }
                });
            }
        });
    }
    Ok(())
}

fn on_param_changed(
    _stream: &StreamRef,
    data: &mut StreamData,
    id: u32,
    param: Option<&pod::Pod>,
) {
    if id != ParamType::Format.as_raw() {
        return;
    }
    let Some(param) = param else { return };
    let mut info = VideoInfoRaw::new();
    if info.parse(param).is_err() {
        return;
    }
    let size = info.size();
    let format = info.format();
    let modifier = info.modifier();
    data.format = Some((size.width, size.height, format, modifier));
    crate::ilog!(
        "iris: record: negotiated format {}x{} {:?} modifier {:#x}",
        size.width,
        size.height,
        format,
        modifier
    );
}

fn build_param_pods() -> Result<Vec<Vec<u8>>, String> {
    let obj_raw = pod::object!(
        SpaTypes::ObjectParamFormat,
        ParamType::EnumFormat,
        pod::property!(FormatProperties::MediaType, Id, MediaType::Video),
        pod::property!(FormatProperties::MediaSubtype, Id, MediaSubtype::Raw),
        pod::property!(
            FormatProperties::VideoFormat,
            Choice, Enum, Id,
            VideoFormat::BGRx,
            VideoFormat::BGRx,
            VideoFormat::BGRA,
            VideoFormat::RGBx,
            VideoFormat::RGBA
        ),
    );

    let obj_dma = pod::object!(
        SpaTypes::ObjectParamFormat,
        ParamType::EnumFormat,
        pod::property!(FormatProperties::MediaType, Id, MediaType::Video),
        pod::property!(FormatProperties::MediaSubtype, Id, MediaSubtype::Raw),
        pod::property!(
            FormatProperties::VideoFormat,
            Choice, Enum, Id,
            VideoFormat::BGRx,
            VideoFormat::BGRx,
            VideoFormat::BGRA,
            VideoFormat::RGBx,
            VideoFormat::RGBA
        ),
        pod::Property::new(
            FormatProperties::VideoModifier.as_raw(),
            pod::Value::Choice(pod::ChoiceValue::Long(Choice(
                ChoiceFlags::empty(),
                ChoiceEnum::Enum {
                    default: DRM_FORMAT_MOD_INVALID as i64,
                    alternatives: vec![
                        DRM_FORMAT_MOD_INVALID as i64,
                        DRM_FORMAT_MOD_LINEAR as i64,
                    ],
                },
            ))),
        )
    );

    let buffers_obj = pod::object!(
        SpaTypes::ObjectParamBuffers,
        ParamType::Buffers,
        pod::Property::new(
            pipewire::spa::sys::SPA_PARAM_BUFFERS_dataType,
            pod::Value::Int(
                (1 << DataType::MemPtr.as_raw())
                    | (1 << DataType::MemFd.as_raw())
                    | (1 << DataType::DmaBuf.as_raw())
            ),
        ),
    );

    let mut params = Vec::new();
    for obj in [obj_raw, obj_dma, buffers_obj] {
        let values: Vec<u8> = pod::serialize::PodSerializer::serialize(
            std::io::Cursor::new(Vec::new()),
            &pod::Value::Object(obj),
        )
        .map_err(|e| format!("serialize format params: {e}"))?
        .0
        .into_inner();
        params.push(values);
    }
    Ok(params)
}

/// Record a portal-selected window until `spec.stop` fires.
pub fn record_window(spec: RecordingSpec) -> Result<(), String> {
    crate::ilog!("iris: record: record_window start -> {}", spec.output.display());
    if std::env::var_os("WAYLAND_DISPLAY").is_none() {
        return Err(
            "Wayland recording needs WAYLAND_DISPLAY; this is not a Wayland session".to_string(),
        );
    }
    let (fd, node_id) = futures::executor::block_on(portal_negotiate())?;

    pipewire::init();
    let mainloop = pipewire::main_loop::MainLoop::new(None)
        .map_err(|e| format!("PipeWire main loop: {e}"))?;
    let context = pipewire::context::Context::new(&mainloop)
        .map_err(|e| format!("PipeWire context: {e}"))?;
    let core = context
        .connect_fd(fd, None)
        .map_err(|e| format!("PipeWire connect (is pipewire running?): {e}"))?;

    let shared = Rc::new(RefCell::new(Shared {
        encoder: None,
        error: None,
        quit: Some(mainloop.downgrade()),
        done: false,
    }));

    let stream = pipewire::stream::Stream::new(
        &core,
        "iris-rec",
        properties! {
            *pipewire::keys::MEDIA_TYPE => "Video",
            *pipewire::keys::MEDIA_CLASS => "Stream/Input/Video",
            *pipewire::keys::MEDIA_CATEGORY => "Capture",
            *pipewire::keys::MEDIA_ROLE => "Screen",
        },
    )
    .map_err(|e| format!("PipeWire stream: {e}"))?;

    let gl_context = match GlContext::new() {
        Ok(ctx) => {
            crate::ilog!("iris: record: EGL context initialized for DMA-buf");
            Some(ctx)
        }
        Err(e) => {
            crate::ilog!("iris: record: EGL context unavailable: {e}");
            None
        }
    };

    let data = StreamData {
        output: spec.output.clone(),
        fps: spec.fps,
        mic: spec.mic,
        rec_format: spec.format,
        rec_encoder: spec.encoder,
        format: None,
        shared: shared.clone(),
        unsupported_reported: false,
        frames: 0,
        gl_context,
        scratch: Vec::new(),
    };

    let stop = spec.stop;
    let weak = mainloop.downgrade();
    let _stop_timer = mainloop.loop_().add_timer(move |_| {
        if stop.try_recv().is_ok() {
            if let Some(mainloop) = weak.upgrade() {
                mainloop.quit();
            }
        }
    });
    _stop_timer
        .update_timer(
            Some(std::time::Duration::from_millis(50)),
            Some(std::time::Duration::from_millis(50)),
        )
        .into_result()
        .map_err(|e| format!("PipeWire stop timer: {e}"))?;

    let _listener = stream
        .add_local_listener_with_user_data(data)
        .state_changed(|_stream, _data, _old, new| {
            crate::ilog!("iris: record: stream {new:?}");
        })
        .param_changed(on_param_changed)
        .process(on_process)
        .register()
        .map_err(|e| format!("PipeWire listener: {e}"))?;

    let params = build_param_pods()?;
    let mut pods: Vec<&pod::Pod> = params
        .iter()
        .map(|v| pod::Pod::from_bytes(v).ok_or("invalid format param pod"))
        .collect::<Result<_, _>>()?;

    stream
        .connect(
            pipewire::spa::utils::Direction::Input,
            Some(node_id),
            StreamFlags::AUTOCONNECT | StreamFlags::MAP_BUFFERS,
            &mut pods,
        )
        .map_err(|e| format!("PipeWire stream connect to node {node_id}: {e}"))?;

    mainloop.run();

    let mut shared = shared.borrow_mut();
    shared.done = true;
    if let Some(error) = shared.error.take() {
        return Err(error);
    }
    let encoder = shared
        .encoder
        .take()
        .ok_or_else(|| "stream ended before any frame was captured".to_string())?;
    encoder.finish()?;
    Ok(())
}

// WHY: DMA-buf screencasting requires parameter negotiation and pixel conversion:
// 1) format pods must advertise DmaBuf in ParamBuffers alongside MemPtr and MemFd.
// 2) format pods must serialize modifier choice pods for compositor negotiation.
// 3) DRM FOURCC codes must match little-endian SPA VideoFormat channel layouts.
// 4) convert_frame swizzles 32-bit channel words correctly into opaque RGBA.
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drm_fourcc_mapping_matches_spa_formats() {
        assert_eq!(drm_fourcc_for_format(VideoFormat::BGRx).unwrap(), 0x34325258);
        assert_eq!(drm_fourcc_for_format(VideoFormat::BGRA).unwrap(), 0x34325241);
        assert_eq!(drm_fourcc_for_format(VideoFormat::RGBx).unwrap(), 0x34324258);
        assert_eq!(drm_fourcc_for_format(VideoFormat::RGBA).unwrap(), 0x34324241);
        assert!(drm_fourcc_for_format(VideoFormat::YUY2).is_err());
    }

    #[test]
    fn param_pods_serialize_dmabuf_and_memfd() {
        let pods = build_param_pods().expect("serialize param pods");
        assert_eq!(pods.len(), 3);
        for pod_bytes in &pods {
            assert!(pod::Pod::from_bytes(pod_bytes).is_some());
        }
    }

    #[test]
    fn convert_frame_bgrx_swizzle() {
        let src = [
            0x10, 0x20, 0x30, 0x00,
            0x40, 0x50, 0x60, 0x00,
        ];
        let mut out = Vec::new();
        convert_frame(&src, 8, 2, 1, VideoFormat::BGRx, &mut out).unwrap();
        assert_eq!(out, vec![0x30, 0x20, 0x10, 0xFF, 0x60, 0x50, 0x40, 0xFF]);
    }

    #[test]
    fn convert_frame_rgbx_swizzle() {
        let src = [
            0x30, 0x20, 0x10, 0x00,
            0x60, 0x50, 0x40, 0x00,
        ];
        let mut out = Vec::new();
        convert_frame(&src, 8, 2, 1, VideoFormat::RGBx, &mut out).unwrap();
        assert_eq!(out, vec![0x30, 0x20, 0x10, 0xFF, 0x60, 0x50, 0x40, 0xFF]);
    }

    #[test]
    fn gl_context_headless_initialization_does_not_panic() {
        let result = GlContext::new();
        match result {
            Ok(ctx) => {
                assert!(!ctx.display.as_ptr().is_null());
            }
            Err(e) => {
                assert!(!e.is_empty());
            }
        }
    }
}
