# Timestamp completion and ownership

The renderer requests `TIMESTAMP_QUERY` and records each render pass's
beginning/end queries. It encodes a frame's resolve **after**
`Queue::on_submitted_work_done` reports the draws complete. A callback only
sets an atomic flag; subsequent rendering calls poll and advance the
readback without waiting for GPU idle. `finish_timings`, a tooling API,
can wait for both draws and the later resolve copies.

A frame owns a disjoint range of query indices through readback. Its
frame ID and pass metadata travel with that range. Ranges are recycled
only after their readbacks complete; many ranges share one query set to
avoid exhausting Metal's counter sample buffers. Each set accommodates
up to 64 frame ranges, capped at wgpu's 4096-query limit. Growth happens
while lowering, before any surface of the frame is encoded.

Sampling points, tick conversion, and the reported frame span remain
unchanged on every backend, including M1. There is no alternate timing
source, estimate, native Metal escape, or CPU wait in the frame path.

## Evidence for issue #39

Investigation used wgpu/wgpu-hal 29.0.4 from `Cargo.lock`, and an operator's
physical iPad Pro 13-inch M4 running iPadOS 26.5. The original log contained
65 ordered frames with reversed pairs, including:

| Frame | Start tick | End tick |
| --- | ---: | ---: |
| 0 | 585114297833 | 0 |
| 1 | 585115220000 | 585115216750 |
| 2 | 585115979208 | 585115975750 |
| 41 | 585144751000 | 585144747916 |

The timestamp period was 1 ns. Later inversions reached hundreds of
microseconds, so this was not a constant clock offset. Changing arithmetic
or exchanging endpoints would turn incomplete samples into false timings.

The pinned Metal backend:

- Detects stage/draw/dispatch/blit sampling with `supportsCounterSampling`
  in `src/metal/adapter.rs`. Stage support enables `TIMESTAMP_QUERY` and
  encoder timestamp queries; it does not establish reliable arbitrary
  encoder sampling on Apple GPUs.
- Allocates a shared `MTLCounterSampleBuffer` in `src/metal/device.rs`.
- Maps render pass boundaries to `startOfVertexSampleIndex` and
  `endOfFragmentSampleIndex` in `src/metal/command.rs`.
- Implements query resolve with `resolveCounters:inRange:destinationBuffer:
  destinationOffset:` in a blit encoder. The ordinary buffer copy then
  copies that destination to staging.

Controlled device experiments isolated **incomplete counter data at early
resolve**, rather than incorrect tick interpretation or a bad staging copy:

| Experiment | Observation |
| --- | --- |
| Fresh retained query sets, resolve again after completion | Frame 1 changed from `[7419657812625, 0]` to `[7419657812625, 7419659424375]`. All eight starts were identical; all late ends were valid. |
| Positive early duration | Frame 0's end changed from `7419658168458` to `7419658597666`. A positive duration alone was insufficient validation. |
| Fragment fence before resolve | All four fenced frames still differed from their late resolves. |
| Separate resolve/copy blit encoders; map resolve destination directly | All copied pairs matched their original resolve destinations, including zeros. Splitting encoders did not repair them. |
| Encode before submitting render; explicit GPU event before resolve | Starts were nonzero, disproving a simple CPU encoding-time snapshot. Event-protected ends were still incomplete. |
| Completion callback, one query set per frame | Paced map timings worked, but an unpaced burst failed to allocate its 33rd counter sample buffer. |
| Completion callback, independent ranges in shared query sets | All 96 frames across map/effects/paced-map runs matched late re-resolves bit-for-bit and were reported once in order. The 65-frame burst succeeded; 14 paced frames were reported before tooling waited. |

The experiments establish the completion condition needed by this driver;
they do not identify its private counter-publication mechanism. GPU-side
fences/events alone did not supply that condition. The public wgpu
completion callback supplies it without blocking rendering, so the fix
can live in cherenkov-gpu using the unchanged pinned dependency.

The effects scene also includes a clear-only pass (zero draws and zero
instances). Its end sample remains zero even on a late resolve on this
M4. Its individual `PassTiming::gpu_seconds` remains `None`; the whole
frame and drawn passes have valid timings. Missing samples are never
fabricated.

Apple's documentation describes [stage-boundary sampling](https://developer.apple.com/documentation/metal/sampling-gpu-data-into-counter-sample-buffers)
and [resolving counter buffers after GPU completion](https://developer.apple.com/documentation/metal/converting-a-gpus-counter-data-into-a-readable-format).

## Final verification

Committed code `2e28166`, with range recycling enabled and no diagnostic
patches, passed the operator's physical-device runs on September 26, 2026:

| Device | Runs | Result, including warmup |
| --- | --- | --- |
| iPad Pro M4, iPadOS 26.5 | Map, effects, map at 60 Hz | 195/195 frames reported once, in order, with positive whole-frame timings |
| Apple M1, macOS | Map, effects, map at 60 Hz | 195/195 frames reported once, in order, with positive whole-frame timings |
| iPhone 17 simulator, iOS 26.5 | Three-frame map smoke | Done screen, exit 0; no timestamp support |
| iPhone 16 Pro, A18 Pro | No final device available | Not retested |

Each physical run included 60 measured frames and five warmup frames.
Both paced runs delivered 63 timings before the tooling wait, with no
additional query-set allocations. All recorded render-phase wait times
were zero. The clear-only effects pass had no end sample on either M4 or
M1; drawn passes and whole-frame timings remained valid.

On the macOS VM, formatting, workspace Clippy with warnings denied, all
69 GPU crate tests, and one doctest passed. The VM has no timestamp
queries; physical M1 coverage came from the benchmark runs, not the VM's
Metal test. Raw device logs and JSON, rather than the preliminary handoff
summary, established device provenance and these frame counts.
