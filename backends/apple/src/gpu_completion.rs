//! Completion service for GPU submissions.
//!
//! `Queue::on_submitted_work_done` is only serviced while wgpu's internal
//! `maintain` runs inside `Device::poll`; nothing else polls the device, so a
//! registered callback would never run on its own. The native Metal
//! completion event is the wake: when the submitted command buffer reports
//! completion, a single nonblocking `device.poll(PollType::Poll)` on the main
//! queue fires every callback whose submission finished.
//!
//! The same helper serves the normal frame path (a marker encoder carrying
//! only the completion ordering the previous `queue.submit([])` provided),
//! filtered rendering (the real frame encoder), and external capture (a
//! marker encoder after the render submissions).

use std::ptr::NonNull;
use std::sync::Arc;

use block2::RcBlock;
use cocoa_ui::Retained;
use objc2::MainThreadMarker;
use objc2::runtime::ProtocolObject;
use objc2_metal::{MTLCommandBuffer, MTLCommandBufferHandler};
use waterui_graphics::gpu::SharedGpuContext;
use waterui_graphics::wgpu;
use wgpu_hal::api::Metal as MetalApi;

/// Submits `encoder` through `context`'s queue exactly once, registers
/// `completion` via `Queue::on_submitted_work_done` for that submission, and
/// services the device poll that fires it when the submitted command buffer
/// completes.
///
/// `_mtm` proves the caller runs on the main thread, which is what makes the
/// ordering below sound: the Metal handler can only *enqueue* the service
/// poll onto the same main queue this call is registering `completion` on,
/// and a queued block cannot run until the synchronous part of this call —
/// submit and registration included — has returned. wgpu-hal's
/// `Fence::get_latest` then reads the raw command buffer's `Completed`/`Error`
/// status directly, so the single post-completion poll settles every callback
/// parked on this submission regardless of handler ordering.
///
/// `addCompletedHandler` must be attached before `commit`. The caller's
/// encoder may already carry wgpu-recorded passes, and wgpu forbids
/// `as_hal_mut` on an encoder that used the wgpu recording API — so the
/// native completion handler is armed on a trailing marker command buffer
/// (still `EncodingApi::Undecided`), and both buffers commit in one ordered
/// `queue.submit`. The marker completes only after the real work, so its
/// native completion implies the caller's command buffer completed.
/// `as_hal_mut` opens the marker, which runs `begin_encoding` and
/// materializes `raw_cmd_buf` even though no commands were recorded; the
/// retained handle names the same object the submission commits.
///
/// `context` is retained across the asynchronous completion so the poll runs
/// against the exact device generation that submitted the work.
pub fn submit_with_completion(
    _mtm: MainThreadMarker,
    encoder: wgpu::CommandEncoder,
    context: &Arc<SharedGpuContext>,
    completion: impl FnOnce() + Send + 'static,
) {
    let mut marker = context
        .device()
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("GPU completion marker"),
        });
    let raw_command_buffer: Retained<ProtocolObject<dyn MTLCommandBuffer>> = unsafe {
        // SAFETY: `as_hal_mut` is requested for the Metal backend only, on a
        // live marker encoder created by this same device that recorded no
        // wgpu commands. Opening the encoder begins native encoding, which
        // populates `raw_command_buffer`. The object is only retained and
        // read here; wgpu-hal owns and commits it.
        marker.as_hal_mut::<MetalApi, _, _>(|command_encoder| {
            command_encoder.and_then(move |hal| {
                hal.raw_command_buffer()
                    // `raw` is a valid object wgpu-hal owns for the
                    // encoder's lifetime; retaining it is safe.
                    .and_then(|raw| Retained::retain(std::ptr::from_ref(raw).cast_mut()))
            })
        })
    }
    .expect("a recording Metal marker encoder must expose its raw command buffer");

    let service_context = Arc::clone(context);
    let service = RcBlock::new(
        move |_cmd_buf: NonNull<ProtocolObject<dyn MTLCommandBuffer>>| {
            let context = service_context.clone();
            cocoa_ui::main_queue::enqueue(move |_mtm| {
                context
                    .device()
                    .poll(wgpu::PollType::Poll)
                    .expect("nonblocking GPU completion poll must not fail");
            });
        },
    );
    unsafe {
        // SAFETY: `service` is a valid completion block of the documented
        // signature, attached before `commit` as the API requires;
        // `addCompletedHandler` copies it.
        raw_command_buffer
            .addCompletedHandler(RcBlock::as_ptr(&service) as MTLCommandBufferHandler);
    }
    // One ordered batch: the caller's work commits before the marker, so the
    // marker's completion implies the work completed.
    context.queue().submit([encoder.finish(), marker.finish()]);
    context.queue().on_submitted_work_done(completion);
}
