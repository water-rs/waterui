# WaterUI Components

## Layout
- `hstack`, `vstack`, `zstack` - stack layouts
- `scroll` - scrollable container
- `spacer`, `spacer_min` - flexible spacing
- `overlay`, `background` - layering
- `grid` - grid layout

## Controls
- `button` - clickable button
- `toggle` - boolean switch
- `Slider` - range input
- `Stepper` - increment/decrement
- `field` / `TextField` - text input
- `text_editor` - multiline text
- `Menu`, `MenuItem` - menus

## Text
- `text()` - static text
- `text!()` - localized/interpolated text
- `.title()`, `.headline()`, `.body()`, `.caption()` - semantic styling

## Form
- `#[derive(FormBuilder)]` - auto form generation
- `form()` - render form from struct
- `ColorPicker`, `FilePicker`, `DatePicker`

## Navigation
- `NavigationStack`, `NavigationView`
- `NavigationLink` - push destinations
- `TabView` - tab navigation

## Media
- `Photo` - images with placeholders
- `VideoPlayer` - video playback
- `MediaPicker` - media selection

## Graphics
- `Color`, `Srgb` - colors
- `AnimatedMeshGradient` - animated gradients
- `GpuSurface` - custom GPU rendering

## Shape
- `Circle`, `Ellipse`, `Capsule`, `Rectangle`, `RoundedRectangle`
- `.fill()`, `.clip()` - fill and clip shapes

## Other
- `Canvas` - 2D drawing API
- `Map` - native maps
- `WebView` - embedded web content
- `Chart` - data visualization (Bar, Line, Pie, etc.)
- `Barcode::qr()` - QR codes
- `SystemIcon` - platform icons
- `Svg` - SVG rendering
- `ParticleSystem` - GPU particles
