# wgpu 30 upgrade (#63)

The engine's public GPU interop types now use wgpu 30.0.1. Applications sharing
`SharedDevice`, textures or custom-content encoders with the engine must use
the same wgpu major version. The core recorded-content API is unchanged.

The upgrade preserves rendering policy: surfaces use `SurfaceColorSpace::Auto`
(the wgpu 29 default policy), adapter requests do not bucket limits, and fixed
shader passthrough modules explicitly declare `vs_main` and `fs_main`.
Presentation is queued with `Queue::present`. Successful buffer mapping is
still required before readback; the newly fallible mapped-range access keeps
the previous invariant checks. Shader generation keeps naga's integer-division
checks enabled, matching the previous compiler's behavior.

The bench's Vello baselines are not upgraded: the pinned lexoliu/vello forks
require `wgpu ^29`, and Vello is not this project's code to port. The bench
links both majors — `wgpu` 30 for the cherenkov adapter, `wgpu29` (a renamed
`wgpu` 29 dependency) for `vello-classic`, `vello-hybrid` and their shared
`wgpu_ctx` — and each engine keeps its own device. The transitional
in-repo Vello backend is removed: `cherenkov-gpu` has superseded it, and
it could not survive filtrate's move to wgpu 30.

`TRANSIENT_ATTACHMENT` is not enabled by this change. No existing attachment
uses the old `TRANSIENT` spelling, and converting persistent attachments to
memoryless storage would be a separate allocation and lifetime decision.
Likewise, existing compute calls already use `dispatch_workgroups`.

Native external-frame imports must pass their actual initial texture state
to `Device::create_texture_from_hal`; `UNINITIALIZED` is not a valid substitute
for producer-written content. Producer synchronization and imported allocation
ownership remain explicit contracts of the external-frame API. Merely exposing
a queue-wide event hook does not establish safe synchronization for a shared
queue: staging, execution ordering and retained allocation release must each
be proved for the selected backend.
