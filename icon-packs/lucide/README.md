# waterui-icons-lucide

Lucide icons for WaterUI.

## License

Icons are licensed under **ISC** by Lucide Contributors.
See [LICENSE](https://github.com/lucide-icons/lucide/blob/main/LICENSE) for full license text.

## Features

- `svg` (default) - Icons as `Svg` components for native rendering
- `webfont` - Icons as styled text using Lucide font

## Usage

```rust
use waterui_icons_lucide as lucide;

// Get an icon as Svg component (svg feature)
lucide::home()
lucide::settings()
lucide::user()

// Or access the raw SVG path data
lucide::HOME_PATH
lucide::SETTINGS_PATH
```
