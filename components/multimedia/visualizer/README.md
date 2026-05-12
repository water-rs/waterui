# WaterUI Visualizer

Real-time audio visualization components for WaterUI.

## Views

- **`Waveform`** - Time-domain oscilloscope display
- **`Spectrum`** - Frequency spectrum bars (FFT)
- **`Spectrogram`** - Frequency heatmap over time
- **`PhaseScope`** - Stereo correlation (Lissajous)

## Example

```rust
use waterui_visualizer::Waveform;

Waveform::new()
    .sensitivity(1.5)
    .glow(true)
```
