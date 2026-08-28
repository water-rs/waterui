# `waterui-image`

GPU-backed image view and shared image decode helpers for WaterUI.

This crate centralizes:

- the `Image` GPU view previously hosted under `waterui-media`
- decode-route selection between platform and software paths
- HEIF/AVIF container bridging for software decoders

`waterui-media` re-exports this crate's public image APIs.
