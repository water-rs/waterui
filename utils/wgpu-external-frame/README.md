# wgpu-external-frame

Import an externally produced GPU frame into [`wgpu`](https://wgpu.rs) as a
texture, without copying it through the CPU.

Browser engines, media decoders, screen capture, and compositors all publish
their output as a platform handle to memory the GPU already holds. This crate
turns each of those handles into a `wgpu::Texture` on a device you already own:

| Platform | Handle | Import path |
| --- | --- | --- |
| Linux | DMA-BUF file descriptor | Vulkan `VK_EXT_external_memory_dma_buf`, or EGL `EGL_LINUX_DMA_BUF_EXT` + `glEGLImageTargetTexture2DOES` |
| macOS | `IOSurface` | `MTLDevice newTextureWithDescriptor:iosurface:plane:` |
| Windows | Shared texture `HANDLE` | `ID3D12Device::OpenSharedHandle` |

Each platform is its own module with its own frame type — `DmaBufFrame`,
`IoSurfaceFrame`, `SharedHandleFrame` — because the handles have nothing in
common beyond the goal. The Linux side additionally models the producer's
*lease* on the buffer (`DmaBufLease`), since a DMA-BUF usually comes from a pool
the producer needs back, guarded by an explicit rendering fence.

Every import reaches through `wgpu` to its `hal` layer, so a device must be on
the backend its platform's import requires. Each entry point says which, and
asserts it.

## Status

This exists because `wgpu` has no portable external-memory import API yet. When
the work in the lineage of [wgpu#2320](https://github.com/gfx-rs/wgpu/issues/2320)
lands, most of what is here becomes a thin adapter over it, and the `hal`
reach-through goes away.
