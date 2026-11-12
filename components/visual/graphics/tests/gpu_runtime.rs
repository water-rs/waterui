//! The shared runtime's device contract: a device created by
//! [`GpuRuntime::new`] must be accepted by `cherenkov::Engine<Gpu>` — which
//! requires `Features::PASSTHROUGH_SHADERS` on Vulkan and Metal for its
//! precompiled fixed shaders (cherenkov issue #57) — and by shaderloom's
//! `CompiledShader` consumers, which need it on Direct3D 12 as well.
//!
//! `context_after` coverage exercises the rebuild the same device-loss
//! observer drives: a recorded loss starts the rebuild thread, and the
//! publication it lands must reach waiters that parked before and after it.
#![cfg(all(feature = "gpu", not(target_arch = "wasm32")))]

use std::sync::Arc;

use cherenkov_gpu::interop::SharedDevice;
use waterui_graphics::OffscreenSize;
use waterui_graphics::gpu::{Context, Frame, GpuContent, GpuRuntime};

fn runtime() -> GpuRuntime {
    pollster::block_on(GpuRuntime::new()).expect("a GPU adapter is required on test hardware")
}

#[test]
fn shared_runtime_device_is_accepted_by_cherenkov_engine() {
    let runtime = runtime();
    runtime
        .engine()
        .expect("cherenkov engine must accept the shared runtime's device");
}

/// A waiter parked before the loss is even recorded is woken by the
/// publication its rebuild lands — nothing but the event drives the wake.
#[test]
fn context_after_resolves_when_the_publication_follows_the_listen() {
    let runtime = runtime();
    let stale = runtime.context();
    let mut waiter = Box::pin(runtime.context_after(stale.generation()));
    let waker = std::task::Waker::noop();
    let mut cx = std::task::Context::from_waker(waker);
    assert!(
        std::future::Future::poll(waiter.as_mut(), &mut cx).is_pending(),
        "no newer context exists yet, so the waiter must park"
    );

    stale.mark_device_lost_for_testing("test device loss");
    let fresh =
        pollster::block_on(waiter).expect("a lost context within the rebuild budget recovers");
    assert!(fresh.generation() > stale.generation());
    assert!(
        fresh.device_lost_reason().is_none(),
        "the published context is live"
    );
}

/// A publication that already landed resolves without a second rebuild: the
/// waiter observes the current context is newer than the generation it was
/// given and returns it directly.
#[test]
fn context_after_resolves_immediately_when_publication_precedes_the_listen() {
    let runtime = runtime();
    let stale = runtime.context();
    stale.mark_device_lost_for_testing("test device loss");
    let fresh = pollster::block_on(runtime.context_after(stale.generation()))
        .expect("a lost context within the rebuild budget recovers");

    let current = pollster::block_on(runtime.context_after(stale.generation()))
        .expect("the published context answers again");
    assert!(
        Arc::ptr_eq(&current, &fresh),
        "the same published context answers, not a second rebuild"
    );
}

/// Dropping a parked waiter cancels its listen: publication still lands for
/// later waiters, and nothing the dropped future registered is retained.
#[test]
fn a_dropped_context_after_waiter_does_not_block_publication() {
    let runtime = runtime();
    let stale = runtime.context();
    {
        let mut waiter = Box::pin(runtime.context_after(stale.generation()));
        let waker = std::task::Waker::noop();
        let mut cx = std::task::Context::from_waker(waker);
        assert!(std::future::Future::poll(waiter.as_mut(), &mut cx).is_pending());
    }

    stale.mark_device_lost_for_testing("test device loss");
    let fresh = pollster::block_on(runtime.context_after(stale.generation()))
        .expect("a lost context within the rebuild budget recovers");
    assert!(fresh.generation() > stale.generation());
}

/// Loses `runtime`'s current device and every rebuilt one before any of them
/// presents a frame, until the rebuild budget is spent. Returns the last,
/// unrebuildable lost generation.
fn lose_devices_until_the_budget_is_spent(runtime: &GpuRuntime) -> u64 {
    // `MAX_CONSECUTIVE_UNPRODUCTIVE_REBUILDS` (3) rebuilds are paid for; the
    // loss after them is the one that exhausts the budget.
    let mut context = runtime.context();
    for _ in 0..3 {
        context.mark_device_lost_for_testing("test device loss");
        context = pollster::block_on(runtime.context_after(context.generation()))
            .expect("a loss within the rebuild budget recovers");
    }
    context.mark_device_lost_for_testing("test device loss");
    context.generation()
}

/// A run of devices that each died before presenting a frame spends the
/// rebuild budget: `context_after` then resolves `Err` instead of parking
/// forever, for a later waiter as well as a repeat one.
#[test]
fn context_after_resolves_err_once_the_rebuild_budget_is_spent() {
    let runtime = runtime();
    let last = lose_devices_until_the_budget_is_spent(&runtime);

    let error = pollster::block_on(runtime.context_after(last))
        .expect_err("the budget is spent, so no context publishes");
    assert_eq!(
        error.losses().len(),
        4,
        "every stillborn device is recorded"
    );
    let again = pollster::block_on(runtime.context_after(0));
    assert!(again.is_err(), "the verdict is stable for any generation");
}

/// A waiter already parked when the budget is exhausted wakes with `Err`:
/// the exhaustion notifies the listener.
#[test]
fn a_parked_context_after_waiter_wakes_err_when_the_budget_is_spent() {
    let runtime = runtime();
    let mut context = runtime.context();
    for _ in 0..3 {
        context.mark_device_lost_for_testing("test device loss");
        context = pollster::block_on(runtime.context_after(context.generation()))
            .expect("a loss within the rebuild budget recovers");
    }
    let mut waiter = Box::pin(runtime.context_after(context.generation()));
    let mut cx = std::task::Context::from_waker(std::task::Waker::noop());
    assert!(
        std::future::Future::poll(waiter.as_mut(), &mut cx).is_pending(),
        "the device is live, so the waiter parks"
    );

    context.mark_device_lost_for_testing("test device loss");
    assert!(pollster::block_on(waiter).is_err());
}

/// Builds a `SharedDevice` from nothing but its setup context.
struct BuildsSharedDevice {
    adapter_info: Arc<std::sync::Mutex<Option<wgpu::AdapterInfo>>>,
}

impl GpuContent for BuildsSharedDevice {
    fn setup(&mut self, gpu: &Context<'_>) {
        let shared = SharedDevice {
            instance: gpu.instance().clone(),
            adapter: gpu.adapter().clone(),
            device: gpu.device().clone(),
            queue: gpu.queue().clone(),
        };
        // The device is usable: it allocates and its queue submits.
        let buffer = shared.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("shared device probe"),
            size: 16,
            usage: wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        shared.queue.write_buffer(&buffer, 0, &[0_u8; 16]);
        shared.queue.submit([]);
        *self.adapter_info.lock().unwrap() = Some(shared.adapter.get_info());
    }

    fn render(&mut self, _frame: &mut Frame<'_>) {}
}

#[test]
fn setup_context_builds_a_shared_device_of_the_runtime() {
    let runtime = runtime();
    let adapter_info = Arc::new(std::sync::Mutex::new(None));
    runtime
        .render_content(
            BuildsSharedDevice {
                adapter_info: adapter_info.clone(),
            },
            OffscreenSize::try_from_pixels(8, 8).unwrap(),
            1.0,
        )
        .expect("the offscreen frame renders");
    let seen = adapter_info.lock().unwrap().take().expect("setup ran");
    assert_eq!(seen, runtime.context().adapter().get_info());
}
