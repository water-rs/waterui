//! A dropped `GpuRuntime` leaves the process able to exit.
//!
//! wgpu unloads its driver libraries when the last `Instance` drops. Under
//! Mesa's software drivers — llvmpipe and lavapipe, which is every GPU-less
//! CI runner, container and VM — the driver and the LLVM inside it register
//! `atexit` destructors, so once the mapping is gone the process dies inside
//! `exit()` after everything has been torn down cleanly (water-rs/waterui#281,
//! found as a core dump of a smoke test whose teardown had already logged
//! completion). `SharedGpuContext` therefore keeps its instance for the life
//! of the process.
//!
//! nextest runs every test in a process of its own, so returning from this
//! test is exactly the exit that used to crash.

use waterui_graphics::GpuRuntime;

#[test]
fn a_dropped_runtime_leaves_the_process_able_to_exit() {
    let runtime = pollster::block_on(GpuRuntime::new())
        .expect("the teardown test needs a working GPU runtime");
    drop(runtime);
}
