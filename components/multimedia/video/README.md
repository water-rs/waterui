# waterui-video

Semantic video components for WaterUI. This crate defines playback state,
sources, playlists, tracks, events, policies, and accessibility without
embedding a decoder or GPU renderer.

This crate provides:

- `Video`: playback presentation with no controls.
- `VideoPlayer`: interactive playback with accessible controls.
- `PlaybackSession` and `PlayerController`: shared playlist, transport,
  track-selection, live-window, frame-step, repeat, and shuffle state.
- Typed source, DRM, subtitle, metadata, metrics, output-path, and policy APIs.

Apple backends realize these semantics through AVPlayer and AVKit. Portable
self-drawn playback is implemented separately by `waterui-video-gpu`, which
connects this crate to WaterKit and `GpuSurface`. Keeping the semantic crate
independent preserves tree shaking for applications that use only the native
Apple bridge.
