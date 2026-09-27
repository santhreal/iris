use khronos_egl as egl;
use pipewire::spa::param::video::VideoFormat;

use super::gl::{GlContext, DRM_FORMAT_MOD_INVALID};
use super::stream::{drm_fourcc_for_format, PlaneInfo};

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

const GL_TEXTURE_2D: u32 = 0x0DE1;
const GL_FRAMEBUFFER: u32 = 0x8D40;
const GL_COLOR_ATTACHMENT0: u32 = 0x8CE0;
const GL_FRAMEBUFFER_COMPLETE: u32 = 0x8CD5;
const GL_RGBA: u32 = 0x1908;
const GL_UNSIGNED_BYTE: u32 = 0x1401;
const GL_NO_ERROR: u32 = 0;

impl GlContext {
    /// Read one DMA-buf frame into `out` as packed R,G,B,A rows. The
    /// read is synchronous: the pixels are this buffer's, and the GPU
    /// is done with the buffer before PipeWire hands it back to the
    /// compositor.
    pub fn read_dma_buf(
        &mut self,
        width: u32,
        height: u32,
        format: VideoFormat,
        modifier: u64,
        planes: &[PlaneInfo],
        out: &mut Vec<u8>,
    ) -> Result<(), String> {
        if planes.is_empty() {
            return Err("empty DMA-buf planes".to_string());
        }
        // The context was released at setup; bind it on this thread.
        self.egl
            .make_current(self.display, self.surface, self.surface, Some(self.context))
            .map_err(|e| format!("eglMakeCurrent on stream thread: {e}"))?;
        let fourcc = drm_fourcc_for_format(format)?;
        // Stack array, not a Vec: this runs per frame. Max: 6 header +
        // 4 planes * 8 + NONE.
        let mut attribs = [0 as egl::Int; 40];
        let mut n = 0usize;
        let mut push = |v: egl::Int| {
            attribs[n] = v;
            n += 1;
        };
        push(egl::WIDTH);
        push(width as egl::Int);
        push(egl::HEIGHT);
        push(height as egl::Int);
        push(EGL_LINUX_DRM_FOURCC_EXT);
        push(fourcc as egl::Int);

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
            push(PLANE_FD_ATTRS[i]);
            push(plane.fd as egl::Int);
            push(PLANE_OFFSET_ATTRS[i]);
            push(plane.offset as egl::Int);
            push(PLANE_PITCH_ATTRS[i]);
            push(plane.stride as egl::Int);
            if modifier != DRM_FORMAT_MOD_INVALID {
                push(PLANE_MOD_LO_ATTRS[i]);
                push((modifier & 0xFFFFFFFF) as egl::Int);
                push(PLANE_MOD_HI_ATTRS[i]);
                push(((modifier >> 32) & 0xFFFFFFFF) as egl::Int);
            }
        }
        push(egl::NONE);

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
            if self.texture == 0 || self.fbo == 0 {
                return Err("GL texture/FBO were not allocated at setup".to_string());
            }
            // Re-point the cached texture at this frame's EGLImage and
            // re-attach it: the objects persist, only the binding
            // changes per frame.
            (self.bind_texture)(GL_TEXTURE_2D, self.texture);
            (self.image_target_texture_2d)(GL_TEXTURE_2D, image);
            (self.bind_framebuffer)(GL_FRAMEBUFFER, self.fbo);
            (self.framebuffer_texture_2d)(
                GL_FRAMEBUFFER,
                GL_COLOR_ATTACHMENT0,
                GL_TEXTURE_2D,
                self.texture,
                0,
            );
            let status = (self.check_framebuffer_status)(GL_FRAMEBUFFER);
            if status != GL_FRAMEBUFFER_COMPLETE {
                (self.bind_framebuffer)(GL_FRAMEBUFFER, 0);
                return Err(format!("glCheckFramebufferStatus returned {status:#x}"));
            }
            // Rows of width*4 bytes meet the default pack alignment of
            // 4, so the read fills exactly `len` bytes of the spare
            // capacity: no zero fill first.
            let len = width as usize * height as usize * 4;
            out.clear();
            out.reserve(len);
            (self.read_pixels)(
                0,
                0,
                width as i32,
                height as i32,
                GL_RGBA,
                GL_UNSIGNED_BYTE,
                out.as_mut_ptr().cast(),
            );
            let err = (self.get_error)();
            (self.bind_framebuffer)(GL_FRAMEBUFFER, 0);
            if err != GL_NO_ERROR {
                return Err(format!("glReadPixels failed: GL error {err:#x}"));
            }
            // SAFETY: the read succeeded, so all `len` bytes are written.
            out.set_len(len);
            Ok(())
        })();

        unsafe {
            (self.destroy_image)(self.display.as_ptr(), image);
        }
        result
    }
}
