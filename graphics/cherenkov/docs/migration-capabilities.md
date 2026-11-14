# Migration capability audit

Baseline: `ecf7c8e` on `dev`. Scope is the actual `feat/cherenkov-migration` callers in WaterUI, Hydrolysis, hydrolysis-m3, particle, barcode, image, svg and canvas. The old engine branch was reviewed as input, not merged or cherry-picked. Current retained recording, incremental lowering and asynchronous timestamps remain the implementation base.

## Required surface

| Callers | Capability | Implementation and coverage |
|---|---|---|
| Hydrolysis, hydrolysis-m3, WaterUI animation | Recorder lifecycle, owned ShapeData, exported animation samplers | Backend-neutral recording; subscriptions survive finish, explicit freezing snapshots latest values, owned paths keep their allocation |
| Hydrolysis window host | WindowTarget | Retained output, transparent compositor conventions, acquisition retry through Next |
| WaterUI native runtime | SharedDevice, TextureTarget, Presenter | Retained HDR texture export/resize; hardware/manual sRGB premultiplication comparison |
| WaterUI GPU bridge, particle | GPU producer setup/render, adapter, asynchronous wake, resize | Attached-use lifecycle, transform/clip/opacity composition, coalesced wakes, retained setup and engine frame timing; Vello implements the shared resize hook |
| WaterUI shader paint | Shader registration, uniforms, animation | Stable sampled-texture identities across live patches, complete geometry UVs, presentation time, invalid-resource errors |
| WaterUI effects | Filtrate filters/effects and wake callbacks | Exact captures, shared encoder, error propagation, callback-before-setup, attached scheduling, shared-effect animation delta |
| barcode, image, svg, canvas | Paths, strokes, gradients, images, glyphs, static/live recording | Existing dev APIs and backend behavior; downstream compilation plus original corpus and incremental behavior suites |

Recording, live operands, layer transactions, animation and Next remain shared APIs. GPU objects stay in GPU interop. This does not add CPU capabilities absent from both dev and the migration engine. MeshGradient is already public, but both GPU branches reject mesh paint: its appearance in downstream scene conversion is a pre-existing rendering limitation, not evidence that the migration branch implemented it.

## Old-branch disposition

| Commit(s) | Disposition |
|---|---|
| `0f67e8f..ab33360` | Superseded by dev's backend and front-end work. No wholesale port. |
| `f247bfd` | Reused sampler exports; algorithms remain dev's implementations. |
| `4d21bf4` | Adapted presentation pipeline boilerplate to current retained output and asynchronous timing. Re-derived retries and initialization. |
| `08a7f85` | Re-derived Recorder/Shape ownership and filter capture. Rejected the WIP's old front-end integration. |
| `4253f1c` | Retained interoperable producer concept; rewrote storage, composition, lifecycle and scheduling. Stored detached producers cannot drive frames. |
| `06a60e9` | Adapted shader pipeline boilerplate; rejected frame-relative sampled-texture identities. Retained emissions now own stable exact keys. |
| `dae97b0` | Retained exact extents and engine presentation timing; rewrote capture submission, setup errors and lifetime. |
| `6e60ee4` | Obsolete bounds API correction; no standalone port. |
| `a31b0e6` | Retained complete-geometry UV contract, re-derived in current prepared lowering. |
| `ce5c04f` | Rejected old lint and lockfile churn; add only required dependency edges. |
| `a82ecb7` | Retained owning-adapter access in producer setup. |
| `c9621f6` | Retained attached-use scheduling requirement; applied to producers and effects. |
| `9444e70` | Retained callback-before-setup ordering; rewrote wake coalescing, detach and destruction behavior. |
| `13d6b40` | Adapted shared-device/native-texture API. Corrected timestamp opt-in, resize binding lifetime and receiver-drop behavior. |
| `8e5ecda` | Retained working-space conversion intent; corrected signed decode and encoded-domain alpha for hardware-sRGB presentation. |
| `703a8c6` | Reused color tests and extended them with actual hardware-sRGB attachment comparison. |
| `bc688ad` | Rewrote documentation to match final contracts. |
| `7888f34` | Explicitly skipped; reflected-transform antialiasing is landing separately. |

No old Cargo.lock resolution changes, speculative fallback, global state, or broad lint allowance is part of this port. Capability-specific tests supplement, rather than replace, the original 83-scene byte/metric comparison and retained/incremental behavior suites.

## Downstream adaptation evidence

All ten Linux cargo checks pass against local engine and sibling migration checkouts: WaterUI graphics separately with GPU and CPU features, Hydrolysis, hydrolysis-m3, particle, barcode with and without GPU export, image, svg and canvas. These are compile checks, not proof that every downstream view has been behaviorally migrated.

Every repository must resolve the engine crates and filtrate crates to the same eventual dev revision. Framework/component pins must resolve to their migration versions rather than published pre-migration crates. Local validation used path patches; those absolute paths are not upstream changes.

| Repository | Source/API adaptation beyond repinning |
|---|---|
| WaterUI | None for the checked GPU/CPU graphics API. Keep the shared nami revision across the graph. Honor retained-content resize, Next and wake contracts. |
| Hydrolysis | Scene replay borrows its operations: pass `run.clone()` to `Draw::glyphs`. Resolve SVG to its migration version. |
| hydrolysis-m3 | No direct engine source change. Resolve Hydrolysis and SVG to their migration versions. |
| particle | Import `SignalExt` for computed-color mapping; align the stale nami pin to `6908aac8` (the kurbo-capable revision used by this dev baseline). |
| barcode | None; both ordinary drawing and GPU export checks pass. |
| image | None. |
| svg | Refresh the lock graph to select the migrated framework and its common nami source; the old lock retained registry WaterUI 0.5.0 and incompatible nami 0.11.2. No SVG source change. |
| canvas | Its replay owns each operation: move `run` into `Draw::glyphs` instead of borrowing it. This avoids the old implicit glyph-run copy. |

The generic Draw ownership contract is preserved. No blanket borrowed-operand conversion was added to conceal downstream copies. The future Hydrolysis signal-to-live-operand/layout redesign is outside this port; use retained live Content for that work, because into_picture deliberately freezes it.
