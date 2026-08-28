//! Linux DMA-BUF import.
//!
//! A DMA-BUF is a kernel handle to buffer memory another process rendered
//! into. Importing one into `wgpu` needs the platform's own external-memory
//! path, and which one that is depends on the backend the `wgpu` device runs
//! on: Vulkan's `VK_EXT_external_memory_dma_buf` or EGL's
//! `EGL_LINUX_DMA_BUF_EXT`. [`DmaBufImporter`] picks between them once, from
//! the adapter, and hides the difference behind two operations:
//!
//! - [`DmaBufImporter::copy_to_texture`] for producers whose buffer is only
//!   valid for the duration of a callback: the copy is complete on the GPU
//!   before it returns.
//! - [`DmaBufImporter::copy_into`] for a render loop that already has a
//!   destination texture and can defer the buffer's release until its own
//!   submission completes.
//!
//! Both copy the frame's [visible extent](DmaBufFrame::visible_size) rather
//! than the whole allocation, because a producer may pad its buffer and
//! presenting the padding stretches the picture and draws the gutter.

mod frame;
mod gles;
mod vulkan;

pub use frame::{DRM_FORMAT_MOD_INVALID, DmaBufFormat, DmaBufFrame, DmaBufLease, DmaBufPlane};

use gles::GlesInterop;
use vulkan::ImportedVulkanImage;

#[derive(Debug)]
enum Backend {
    Vulkan,
    Gles(Box<GlesInterop>),
}

fn create_backend(backend: wgpu::Backend) -> Backend {
    match backend {
        wgpu::Backend::Vulkan => Backend::Vulkan,
        wgpu::Backend::Gl => Backend::Gles(Box::new(GlesInterop::new())),
        backend => {
            panic!("DMA-BUF import requires a Vulkan or EGL/GLES wgpu device, received {backend:?}")
        }
    }
}

/// Whatever a deferred import allocated, alive until the GPU is done with it.
///
/// Drop this only once the submission that reads the imported frame has
/// completed — from `wgpu::Queue::on_submitted_work_done`, or after an explicit
/// `wgpu::Device::poll` on that submission.
#[derive(Debug)]
pub struct DmaBufImportGuard {
    #[expect(
        dead_code,
        reason = "the import is held only so that dropping the guard destroys it, which dead-code analysis does not count as a read"
    )]
    imported: Option<ImportedVulkanImage>,
}

/// The result of [`DmaBufImporter::copy_into`].
#[derive(Debug)]
pub struct DmaBufImport {
    /// The encoder holding the copy, for the caller to record into and submit.
    ///
    /// The EGL/GLES path performs its copy immediately and submits the encoder
    /// it was working with, so this is a fresh encoder that merely follows the
    /// copy; the Vulkan path records the copy into it. Either way the copy is
    /// ordered before anything the caller records next.
    pub encoder: wgpu::CommandEncoder,
    /// Must outlive the submission of [`Self::encoder`].
    pub guard: DmaBufImportGuard,
}

/// Imports Linux DMA-BUF frames into textures on one `wgpu` device.
#[derive(Debug)]
pub struct DmaBufImporter {
    device: wgpu::Device,
    queue: wgpu::Queue,
    backend: Backend,
}

impl DmaBufImporter {
    /// Creates an importer for `device`, resolving the platform import path
    /// from the adapter that device came from.
    ///
    /// # Panics
    ///
    /// Panics unless the adapter's backend is Vulkan or EGL/GLES; no other
    /// backend can import a DMA-BUF.
    #[must_use]
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, adapter: &wgpu::Adapter) -> Self {
        Self {
            device: device.clone(),
            queue: queue.clone(),
            backend: create_backend(adapter.get_info().backend),
        }
    }

    /// Copies `frame` into a newly created texture owned exclusively by the
    /// caller, completing the copy on the GPU before returning.
    ///
    /// This is for producers whose buffer becomes invalid the moment their
    /// callback returns: the frame is presented and released here, and the
    /// returned texture no longer depends on it. Pixels are never read back to
    /// the CPU. The texture carries the frame's visible extent and is usable as
    /// `COPY_DST | TEXTURE_BINDING`.
    ///
    /// # Panics
    ///
    /// Panics when the frame's rendering fence has not signalled, or when GPU
    /// import, copying, or synchronization fails.
    #[must_use]
    pub fn copy_to_texture(&self, mut frame: DmaBufFrame) -> wgpu::Texture {
        assert!(
            frame.is_render_ready(),
            "a DMA-BUF must be ready before a synchronous GPU copy"
        );
        // The destination is the *visible* extent: a producer's buffer may be
        // allocated with alignment padding beyond it, and copying the padded
        // buffer then presenting it edge to edge stretches the picture and
        // draws the gutter. The source import still uses the buffer's own
        // dimensions and stride, so this only narrows what is taken from it.
        let (visible_width, visible_height) = frame.visible_size();
        let size = wgpu::Extent3d {
            width: visible_width,
            height: visible_height,
            depth_or_array_layers: 1,
        };
        let destination = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("wgpu_external_frame_owned_dma_buf"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: frame.format.texture_format(),
            usage: wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        match &self.backend {
            Backend::Vulkan => {
                let mut encoder =
                    self.device
                        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                            label: Some("wgpu_external_frame_owned_dma_buf_copy"),
                        });
                encoder.clear_texture(&destination, &wgpu::ImageSubresourceRange::default());
                let imported = vulkan::import_dma_buf(&self.device, &mut frame);
                imported.record_copy(&mut encoder, &destination, visible_width, visible_height);
                frame.presented();
                let submission = self.queue.submit([encoder.finish()]);
                self.device
                    .poll(wgpu::PollType::Wait {
                        submission_index: Some(submission),
                        timeout: None,
                    })
                    .expect("the Vulkan DMA-BUF copy failed");
                drop(imported);
            }
            Backend::Gles(gles) => {
                gles.copy_dma_buf(&frame, &destination);
                gles.finish();
                frame.presented();
            }
        }
        frame.release(None);
        destination
    }

    /// Copies `frame`'s visible extent into `destination`, deferring the wait.
    ///
    /// `destination` must be a `COPY_DST` texture of the frame's
    /// [`DmaBufFormat::texture_format`] and at least its visible extent; it is
    /// cleared before the copy. The caller owns the rest of the protocol:
    /// call [`DmaBufFrame::presented`] once this returns, record and submit
    /// [`DmaBufImport::encoder`], and only after that submission completes drop
    /// [`DmaBufImport::guard`] and call [`DmaBufFrame::release`].
    ///
    /// # Panics
    ///
    /// Panics when GPU import or copying fails.
    #[must_use]
    pub fn copy_into(&self, frame: &mut DmaBufFrame, destination: &wgpu::Texture) -> DmaBufImport {
        let (visible_width, visible_height) = frame.visible_size();
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("wgpu_external_frame_dma_buf_copy"),
            });
        encoder.clear_texture(destination, &wgpu::ImageSubresourceRange::default());
        let imported = match &self.backend {
            Backend::Vulkan => {
                let imported = vulkan::import_dma_buf(&self.device, frame);
                imported.record_copy(&mut encoder, destination, visible_width, visible_height);
                Some(imported)
            }
            Backend::Gles(gles) => {
                // The clear has to reach the driver before the GL blit that
                // overwrites the same texture outside wgpu's command stream.
                self.queue.submit([encoder.finish()]);
                gles.copy_dma_buf(frame, destination);
                encoder = self
                    .device
                    .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                        label: Some("wgpu_external_frame_dma_buf_follow_up"),
                    });
                None
            }
        };
        DmaBufImport {
            encoder,
            guard: DmaBufImportGuard { imported },
        }
    }
}
