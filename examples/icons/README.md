# Icons Example

This example demonstrates WaterUI's comprehensive icon system with multiple icon libraries.

## Icon Libraries

| Library | Icons | License |
|---------|-------|---------|
| SF Symbols | 65+ | Apple (system) |
| Material Design | 7,447 | Apache 2.0 |
| Font Awesome 7 | 2,806 | CC BY 4.0 |
| Native | 50+ | Cross-platform |

## Features

- **Multiple icon sources** - Choose the best icons for your app
- **Native rendering** - SF Symbols on Apple, SVG elsewhere
- **Tree-shaking** - Only used icons are included in final binary
- **Customization** - Tint colors and sizing

## Usage

```rust
use waterui_icons_sf_symbol as sf;
use waterui_icons_material_icon as mdi;
use waterui_icons_fontawesome7 as fa;
use waterui_icons_native as icons;

// SF Symbols (Apple platforms)
sf::HOUSE
sf::GEAR

// Material Design Icons
mdi::home()
mdi::settings()

// Font Awesome 7
fa::solid::house()
fa::brands::github()

// Cross-platform native icons
icons::HOME
icons::SETTINGS
```

## Running

```bash
water run --platform ios
# or
water run --platform android
```

## Platform Support

| Icon Library | iOS/macOS | Android |
|--------------|-----------|---------|
| SF Symbols | Native | Placeholder |
| Material Design | SVG | SVG |
| Font Awesome 7 | SVG | SVG |
| Native | SF Symbol | Placeholder |

## Attribution

- Material Design Icons by [Pictogrammers](https://pictogrammers.com/) - Apache 2.0
- Font Awesome Free by [Fonticons, Inc.](https://fontawesome.com/) - CC BY 4.0
