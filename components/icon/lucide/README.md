# waterui-icons-lucide

Lucide icons for WaterUI.

## License

Icons are licensed under **ISC** by Lucide Contributors.
See [LICENSE](https://github.com/lucide-icons/lucide/blob/main/LICENSE) for full license text.

## Features

- `svg` (default) - Icons backed by SVG paths
- `webfont` - Icons backed by Lucide font (requires font installation)

## Usage

```rust
use waterui_icons_lucide as lucide;

// Every icon constructor returns `waterui_icon::Icon`
lucide::house()
lucide::settings()
lucide::user()

// Or access the raw SVG path data
lucide::HOUSE_PATH
lucide::SETTINGS_PATH
```
