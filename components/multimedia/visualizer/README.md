# WaterUI Visualizer

Real-time audio visualization components for WaterUI.

## Views

- **`Waveform`** - Time-domain oscilloscope display

## Example

```rust
use waterui_visualizer::Waveform;

Waveform::new()
    .sensitivity(1.5)
    .glow(true)
```
