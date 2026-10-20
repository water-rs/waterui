//! `memory-keepalive`: a steady trickle of DRAM reads beside a measurement.
//!
//! A one-worker CPU frame misses its caches only sparsely, and on the
//! Pixel 9 Pro the memory controller drops into a low-power state in the
//! gaps between those misses, so each one pays the state's exit latency.
//! Whether a gap is long enough depends on what everything else on the chip
//! sends to DRAM at that moment: identical one-worker runs moved by a
//! percent and more, within a process over seconds, and outside activity
//! made them faster (#2988). Holding the interconnect clocks does not reach
//! that state. Reads at a fixed rate, each from a line no cache level
//! holds, keep the controller active, so the measured process sees one
//! memory state whatever else runs.

use crate::BenchError;

/// The buffer the reads walk: larger than the CPU cluster's L3 and the
/// system-level cache together, so every read reaches DRAM.
const BUFFER_BYTES: usize = 64 << 20;

/// Distance between consecutive reads: a page and a cache line, so each
/// read lands on a new line of a new page and no prefetcher runs ahead.
const STRIDE: usize = 4096 + 64;

/// Reads one line of [`BUFFER_BYTES`] every `1 / rate` seconds on `cpu`.
///
/// It runs until the process is killed, and prints `ready` on stdout once
/// the buffer is resident and the reads begin, so a supervisor can start
/// its measurement on that signal.
///
/// # Errors
/// Returns [`BenchError::Engine`] for a zero rate or when the thread
/// cannot be pinned, and [`BenchError::Io`] when the signal cannot be
/// written.
pub fn run(cpu: u32, rate: u32) -> Result<std::convert::Infallible, BenchError> {
    use std::io::Write as _;
    use std::time::{Duration, Instant};

    if rate == 0 {
        return Err(BenchError::Engine(
            "memory-keepalive --rate must be positive".into(),
        ));
    }
    crate::affinity::pin_current_thread(&[cpu])?;
    // Non-zero contents fault in every page instead of mapping the shared
    // zero page.
    let buffer = vec![1_u8; BUFFER_BYTES];
    let period = Duration::from_secs(1) / rate;
    let mut stdout = std::io::stdout().lock();
    writeln!(stdout, "ready")?;
    stdout.flush()?;
    drop(stdout);
    let mut at = 0;
    let mut next = Instant::now();
    loop {
        // SAFETY: `at` stays below `buffer.len()`; a volatile read keeps
        // the load the program exists for.
        std::hint::black_box(unsafe { std::ptr::read_volatile(buffer.as_ptr().add(at)) });
        at = (at + STRIDE) % BUFFER_BYTES;
        next += period;
        while Instant::now() < next {
            std::hint::spin_loop();
        }
    }
}
