# Icons Example

This example demonstrates WaterUI's icon system with three icon packs.

## Icon Packs

| Pack | Crate | Rendering | License |
|------|-------|-----------|---------|
| SF Symbols | `waterui-icons-sf-symbol` | Native (Apple platforms only) | Apple (system) |
| Material Design Icons | `waterui-icons-material-icon` | SVG | Apache 2.0 |
| Lucide | `waterui-icons-lucide` | SVG | ISC |

## Features

- **Multiple icon sources** - Choose the best icons for your app
- **Native rendering** - SF Symbols on Apple, SVG elsewhere
- **Tree-shaking** - Only used icons are included in final binary
- **Customization** - Tint colors and sizing

## Usage

```rust
use waterui_icons_lucide as lucide;
use waterui_icons_material_icon as mdi;
#[cfg(target_vendor = "apple")]
use waterui_icons_sf_symbol as sf;

// SF Symbols (Apple platforms)
sf::house_fill()
sf::gearshape()

// Material Design Icons
mdi::home()
mdi::cog()

// Lucide
lucide::house()
lucide::settings()

// Tint and size
mdi::heart().tint(Color::srgb_hex("#EF4444")).size(32.0, 32.0)
```

## Running

```bash
water run --platform ios
# or
water run --platform android
```

## Attribution

- Material Design Icons by [Pictogrammers](https://pictogrammers.com/) - Apache 2.0
- Lucide by [Lucide Contributors](https://lucide.dev/) - ISC
