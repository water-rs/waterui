# Map Example

This example demonstrates the Map component in WaterUI.

## Features

- Display a map centered on San Francisco
- Add annotation pins for landmarks
- Change map styles (Standard, Satellite, Hybrid)
- Zoom in/out controls
- Toggle user location display

## Running

```bash
cd examples/map
water ios run    # Run on iOS simulator
water macos run  # Run on macOS
```

## Usage

```rust
use waterui::reactive::binding;
use waterui_map::{Annotation, Coordinate, Map, Region};

// Create a map centered on a location with a selectable pin
let san_francisco = Coordinate::from_degrees(37.7749, -122.4194)?;
let camera = binding(Region::new(san_francisco, 0.1, 0.1));
let selected = binding(None);
let map = Map::new(&camera)
    .annotations(vec![Annotation::new("sf", san_francisco, "San Francisco")])
    .selection(&selected)
    .shows_user_location(true);
```
