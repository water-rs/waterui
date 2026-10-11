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

use waterui_graphics::gpu::GpuRuntime;

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
    let fresh = pollster::block_on(waiter);
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
    let fresh = pollster::block_on(runtime.context_after(stale.generation()));

    let current = pollster::block_on(runtime.context_after(stale.generation()));
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
    let fresh = pollster::block_on(runtime.context_after(stale.generation()));
    assert!(fresh.generation() > stale.generation());
}
