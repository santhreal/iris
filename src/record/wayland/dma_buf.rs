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

impl GlContext {
    /// Read one DMA-buf frame into `scratch`. Returns Ok(false) when
    /// the PBO pipeline primed but produced no pixels yet (the first
    /// frame only): the caller skips the encode for that call.
    pub fn read_dma_buf(
        &mut self,
        width: u32,
        height: u32,
        format: VideoFormat,
        modifier: u64,
        planes: &[PlaneInfo],
        scratch: &mut Vec<u8>,
    ) -> Result<bool, String> {
        if planes.is_empty() {
            return Err("empty DMA-buf planes".to_string());
        }
        // GL calls run on PipeWire's RT thread; the context was
        // released at setup so it can be bound here.
        self.egl
            .make_current(self.display, self.surface, self.surface, Some(self.context))
            .map_err(|e| format!("eglMakeCurrent on stream thread: {e}"))?;
        let fourcc = drm_fourcc_for_format(format)?;
        // Stack array, not a Vec: this runs per frame on PipeWire's
        // RT thread, and a heap alloc per frame is a needless syscall
        // in the capture path. Max: 6 header + 4 planes * 8 + NONE.
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

            (self.viewport)(0, 0, width as i32, height as i32);
            let total_bytes = (width as usize) * (height as usize) * 4;
            scratch.resize(total_bytes, 0);

            // Double-buffered PBO readback when ES3 procs resolved:
            // readPixels targets a pixel pack buffer (returns
            // immediately), and the PREVIOUS frame's PBO is mapped and
            // copied out. The GPU's transfer overlaps the next frame's
            // capture instead of stalling the RT thread. Without PBOs
            // the readPixels is synchronous into scratch.
            const GL_PIXEL_PACK_BUFFER: u32 = 0x88EB;
            const GL_STREAM_READ: u32 = 0x88E1;
            const GL_MAP_READ_BIT: u32 = 0x0001;
            let use_pbo = self
                .gen_buffers
                .zip(self.bind_buffer)
                .zip(self.buffer_data)
                .zip(self.map_buffer_range)
                .zip(self.unmap_buffer)
                .is_some()
                && self.pbos[0] != 0;
            if use_pbo {
                let bind_buffer = self.bind_buffer.unwrap();
                let buffer_data = self.buffer_data.unwrap();
                let map_buffer_range = self.map_buffer_range.unwrap();
                let unmap_buffer = self.unmap_buffer.unwrap();
                if self.pbo_bytes != total_bytes {
                    for pbo in self.pbos {
                        (bind_buffer)(GL_PIXEL_PACK_BUFFER, pbo);
                        (buffer_data)(
                            GL_PIXEL_PACK_BUFFER,
                            total_bytes as isize,
                            std::ptr::null(),
                            GL_STREAM_READ,
                        );
                    }
                    self.pbo_bytes = total_bytes;
                    self.pbo_pending = None;
                }
                let cur = self.pbo_pending.map(|i| 1 - i).unwrap_or(0);
                (bind_buffer)(GL_PIXEL_PACK_BUFFER, self.pbos[cur]);
                (self.read_pixels)(
                    0,
                    0,
                    width as i32,
                    height as i32,
                    GL_RGBA,
                    GL_UNSIGNED_BYTE,
                    std::ptr::null_mut(),
                );
                // produced = whether a PREVIOUS readback exists to map:
                // the first call primes the pipeline and yields nothing.
                let produced = self.pbo_pending.is_some();
                if let Some(prev) = self.pbo_pending {
                    (bind_buffer)(GL_PIXEL_PACK_BUFFER, self.pbos[prev]);
                    let ptr = (map_buffer_range)(
                        GL_PIXEL_PACK_BUFFER,
                        0,
                        total_bytes as isize,
                        GL_MAP_READ_BIT,
                    );
                    if ptr.is_null() {
                        (bind_buffer)(GL_PIXEL_PACK_BUFFER, 0);
                        (self.bind_framebuffer)(GL_FRAMEBUFFER, 0);
                        return Err("glMapBufferRange failed".to_string());
                    }
                    // Banded memcpy: ffmpeg is fed rgba and drops
                    // alpha in its own conversion, so no stamping
                    // pass; a serial copy of a 33MB frame is a
                    // visible slice of the frame budget.
                    let src = std::slice::from_raw_parts(ptr as *const u8, total_bytes);
                    crate::par::par_bands_mut(scratch.as_mut_slice(), 4096, |dst, start| {
                        dst.copy_from_slice(&src[start..start + dst.len()]);
                    });
                    (unmap_buffer)(GL_PIXEL_PACK_BUFFER);
                }
                (bind_buffer)(GL_PIXEL_PACK_BUFFER, 0);
                self.pbo_pending = Some(cur);
                (self.bind_framebuffer)(GL_FRAMEBUFFER, 0);
                return Ok(produced);
            }
            // Sync readPixels path.
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
            Ok(true)
        })();

        unsafe {
            (self.destroy_image)(self.display.as_ptr(), image);
        }

        let produced = result?;
        Ok(produced)
    }
}
