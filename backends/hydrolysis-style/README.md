# hydrolysis-style

`hydrolysis-style` defines the renderer-independent style and widget-theme contract (`Style`, `WidgetTheme`, and the `theme` look and chrome vocabulary over `cherenkov-record`) for WaterUI's self-drawn Hydrolysis backend. Style packages depend on `hydrolysis-style` without pulling in the Hydrolysis renderer, Cherenkov GPU/CPU painters, `winit`, or Android JNI dependencies.
