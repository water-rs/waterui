# waterui-svg

SVG rendering for WaterUI applications.

`waterui-svg` provides the `Svg` view, which renders SVG markup or SVG path data
through the WaterUI graphics scene pipeline. It is useful for vector icons,
illustrations, and scalable UI artwork that should remain crisp across display
scales.

## Usage

```rust
use waterui_svg::Svg;

let icon = Svg::from_path("M10 20v-6h4v6h5v-8h3L12 3 2 12h3v8z", 24.0, 24.0);
```

Stroke-based icon sets can use `Svg::from_stroke_path`, and monochrome assets
can be tinted with `Svg::tint`.
