# Multi-Window Gallery Example

A comprehensive demonstration of WaterUI's multi-window capabilities, showcasing different window styles and background effects.

## Features Demonstrated

### Window Styles

1. **Titled** - Standard window with title bar and native controls
2. **Borderless** - Frameless window without title bar decorations
3. **FullSizeContentView** - Content extends into the title bar area (macOS)

### Window Backgrounds

1. **Opaque** - Standard opaque system background
2. **Transparent** - Fully transparent window (no background)
3. **Color** - Custom solid or semi-transparent colored background
4. **Material** - Blur effects with various thickness levels:
   - **UltraThin** - Most transparent, subtle frosted effect
   - **Thin** - Light blur, slightly more opaque
   - **Regular** - Balanced transparency and blur
   - **Thick** - More opaque with stronger blur
   - **UltraThick** - Most opaque, heavy frosted effect

## Window Examples

### 1. Standard Titled Window
Classic window with:
- Titled style
- Opaque background
- Traditional title bar with controls

### 2. Borderless Window
Modern frameless window with:
- Borderless style
- Semi-transparent blue colored background (85% opacity)
- No title bar decorations

### 3. Frosted Glass Window
Beautiful frosted effect with:
- Titled style
- Regular material blur background
- See-through with blur effect

### 4. Transparent Overlay
Fully transparent window with:
- FullSizeContentView style
- Transparent background
- Content extends into title bar area (macOS)

### 5. Ultra-Thin Material Window
Subtle frosted window with:
- Borderless style
- UltraThin material blur
- Most transparent material option

## Window Management

Each window section includes:
- **Open Button** - Creates and shows the window
- **Close Button** - Closes the window using its handle
- Window handles stored in reactive bindings for state management

## Running

```bash
# iOS Simulator
water run --platform ios

# Android Emulator
water run --platform android

# macOS (native)
water run --platform macos
```

## Platform Support

### macOS
- Full support for all window styles and backgrounds
- Material backgrounds use `NSVisualEffectView` for native vibrancy
- FullSizeContentView extends content into title bar area

### iOS
- Transparent and color backgrounds supported
- Material backgrounds can be implemented via `UIVisualEffectView`
- Window management follows iOS conventions

### Android
- Transparent windows supported via `Window.setBackgroundDrawable()`
- Window styling follows Android Material Design guidelines
- Some features may have platform-specific behavior

## Code Highlights

### Creating a Window

```rust
let (windows, channel) = WindowManager::new();
// Register `channel` with `App::window(channel)` or `App::new_with_windows(channel, env)`.
Window::new("Window Title", binding(WindowState::Normal), content)
    .style(WindowStyle::Titled)
    .resizable(true)
    .show(&windows);
```

### Window with Material Blur

```rust
Window::new("Frosted Glass", binding(WindowState::Normal), content)
    .style(WindowStyle::Titled)
    .background(Material::Regular)
    .show(&windows);
```

### Window with Custom Color

```rust
let color = Color::srgb_f32(0.2, 0.4, 0.8).with_opacity(0.85);
Window::new("Colored Window", binding(WindowState::Normal), content)
    .background(color)
    .show(&windows);
```

### Managing Window State

```rust
let state = binding(WindowState::Closed);
let presentation = WindowPresentation::new(&state);

// Declarative window in the view tree
conditional_window(&presentation, |state| {
    Window::new("My Window", state, content)
})

// Open or close reactively
state.set(WindowState::Normal);
state.set(WindowState::Closed);
```

## Architecture Notes

- Windows declared in the view tree (`Window`, `conditional_window`) reach their backend through static `View::dispatch`; imperative windows use a typed `(WindowManager<C>, WindowChannel<C>)` pair registered on `App`
- Each window tracks its lifecycle reactively through its `Binding<WindowState>`
- Window backgrounds are composited by the native platform
- Material effects leverage platform-native blur APIs for best performance

## Learning Resources

- Window API: `src/window.rs`
- Material types: `src/background.rs`

## Related Examples

- `examples/navigation` - Navigation patterns
- `examples/form` - Form components
- `examples/hover` - Hover, cursor styles, and reactive backgrounds
