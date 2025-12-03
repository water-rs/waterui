# Layout Architecture in WaterUI
by Lexo Liu 2025.12.1
*Please do not modify this file without opening a GitHub issue*

## 1. Philosophy & Units

The WaterUI Layout Engine enforces a strict separation between **Logical Calculation** (Rust) and **Physical Rendering** (Native).

### Coordinate System
*   **Logical Points (pt):** The exclusive unit of the Rust Engine. All layout math, positioning, and sizing occur in this resolution-independent space.
*   **Physical Pixels (px):** The unit of the Native Backend/Hardware.

**Constraint:** The Native backend is responsible for the correct conversion between Logical Points and Physical Pixels (DPI scaling).

```rust
struct Size { width: f32, height: f32 }
struct Point { x: f32, y: f32 }
struct Rect { origin: Point, size: Size }
```

## 2. Layout Containers and Views

In WaterUI, views are typically resolved as a leaf - `raw View` types via the recursive `.body()` call. For example, `WuiStr` resolves internally to `WuiLabel`. These views are backed by the native platform and handle their own content measuring.

However, **Layout Containers** (Stacks, Grids, etc.) must be strictly controlled by Rust to ensure cross-platform consistency. We avoid implementing layout logic in the native layer to prevent FFI overhead and behavior divergence.

To achieve this, we abstract a universal container logic using the `FixedContainer` struct:

```rust
pub struct FixedContainer {
    layout: Box<dyn Layout>,
    contents: Vec<AnyView>,
}
```

The core logic is encapsulated in the `Layout` trait, which describes the behavior of any layout container:

```rust
pub trait Layout: Debug {
    /// Determines the size of the container based on the parent's proposal and children's responses.
    fn size_that_fits(&self, proposal: ProposalSize, children: &mut [&mut dyn SubView]) -> Size;
    
    /// Calculates the position (Rect) for each child within the final bounds.
    fn place(&self, bounds: Rect, children: &mut [&mut dyn SubView]) -> Vec<Rect>;
}
```

Tip: Here is also a `Container` struct, it has same layout behavior with `FixedContainer`, but enable us to use lazy loading if user required.

## 3. The SubView Trait (Native Contract)

The `SubView` trait defines the interface that **Native backends must implement** to participate in the layout negotiation. Each native view (Text, Button, Image, etc.) must provide measurement capabilities through this trait.

```rust
pub trait SubView {
    /// Returns the size this view prefers given the parent's proposal.
    ///
    /// This is the core measurement function. The native backend must:
    /// 1. Interpret the proposal (None = intrinsic, Some(v) = constrained)
    /// 2. Calculate the appropriate size based on content
    /// 3. Return a concrete Size in logical points
    fn size_that_fits(&mut self, proposal: ProposalSize) -> Size;

    /// Returns the view's stretch axis preference.
    ///
    /// This tells the layout engine how this view behaves with surplus space:
    /// - `None`: Content-sized, does not expand
    /// - `Horizontal`/`Vertical`: Expands along one axis
    /// - `Both`: Greedy, fills all available space
    /// - `MainAxis`: Expands along parent stack's main axis (e.g., Spacer)
    /// - `CrossAxis`: Expands along parent stack's cross axis (e.g., Divider)
    fn stretch_axis(&self) -> StretchAxis;

    /// Returns the view's layout priority (default: 0).
    ///
    /// Higher priority views receive space allocation first during surplus,
    /// and compress last during overflow.
    fn layout_priority(&self) -> f32 { 0.0 }
}
```

### 3.1 Implementation Requirements

Native backends **must** ensure:

1.  **Consistent Measurement:** Calling `size_that_fits` with the same proposal must return the same size (deterministic).
2.  **Logical Units:** All returned sizes must be in logical points (pt), not physical pixels.
3.  **Respect Constraints:** When `proposal.width = Some(w)`, the returned width must be `<= w` (likewise for height).
4.  **Intrinsic Fallback:** When `proposal.width = None`, return the view's natural/ideal width.

## 4. The Propose-and-Response Model

WaterUI utilizes a **Propose-and-Response** negotiation model. This process allows the layout engine to "probe" children for their ideal size, minimum size, or constrained size.

```rust
struct ProposalSize {
    width: Option<f32>,  // None = Unspecified/Intrinsic, Some(v) = Hard Limit
    height: Option<f32>,
}
```

### 4.1 Negotiation Flow
1.  **Parent Proposes:** The container sends a `ProposalSize` to a child.
    *   `None`: "How big do you want to be ideally?"
    *   `Some(v)`: "You have at most `v` space. How big are you now?"
2.  **Child Responds:** The child calculates its size based on the proposal and returns a concrete `Size`.
3.  **Iteration:** The parent may propose multiple times (e.g., first to check ideal width, second to check wrapped height) before making a final decision.

### 4.2 StretchAxis

`StretchAxis` defines a component's static preference for consuming surplus space within a container.

```rust
enum StretchAxis {
    /// Content-Sized: The view prefers its intrinsic size (e.g., Text, Image, Toggle).
    None,

    /// Width-Expanding: The view fills horizontal space but keeps intrinsic height (e.g., Slider, TextField).
    Horizontal,

    /// Height-Expanding: The view fills vertical space but keeps intrinsic width.
    Vertical,

    /// Greedy: The view fills all available space in both directions (e.g. Shape like rectangle, Color).
    Both,

    /// Main-Axis: The view expands along the parent stack's main axis (e.g., Spacer).
    /// In VStack: expands vertically. In HStack: expands horizontally.
    MainAxis,

    /// Cross-Axis: The view expands along the parent stack's cross axis (e.g., Divider).
    /// In VStack: expands horizontally. In HStack: expands vertically.
    CrossAxis,
}
```

## 5. Safe Area Handling

Safe area insets represent regions of the screen obscured by system UI elements (notches, home indicators, status bars, etc.). **WaterUI handles safe areas entirely in the native backend** - Rust code only provides metadata hints.

### 5.1 Architecture

Safe area is a **native-only** concern:

- **Native Backend**: Queries platform safe area insets and applies them by default to all views
- **Rust Layer**: Provides `IgnoreSafeArea` metadata to signal which views should extend edge-to-edge

### 5.2 Ignoring Safe Area (`IgnoreSafeArea` Metadata)

Views can extend into unsafe regions using the `.ignore_safe_area()` modifier:

```rust
Color::blue()
    .ignore_safe_area(EdgeSet::ALL)  // Extend to all edges
```

**How it works:**

1. **Metadata Attachment**: The modifier wraps the view in `Metadata<IgnoreSafeArea>`
2. **Native Detection**: The renderer checks for this metadata
3. **Native Behavior**: Ignores safe area constraints on specified edges

**Edge control:**

```rust
EdgeSet::ALL        // All edges
EdgeSet::VERTICAL   // Top and bottom only
EdgeSet::HORIZONTAL // Leading and trailing only
EdgeSet::TOP        // Top edge only
EdgeSet::BOTTOM     // Bottom edge only
```

### 5.3 Native Backend Responsibilities

The native renderer must:

1. **Default behavior**: Apply platform safe area insets (e.g., `UIView.safeAreaInsets` on iOS) to all views
2. **When encountering `IgnoreSafeArea` metadata**:
   - Ignore safe area constraints on the specified edges
   - Allow the view to extend edge-to-edge for those edges
3. **Handle changes**: Re-layout when safe area changes (keyboard appearance, device rotation, etc.)

**Note:** Rust layout code is unaware of safe area - it only works with the bounds provided by native.

### 5.4 Example Usage

```rust
// Full-screen background
Color::blue()
    .ignore_safe_area(EdgeSet::ALL)  // Background fills entire screen

// Header that extends under status bar
header_view
    .ignore_safe_area(EdgeSet::TOP)
```

## 6. Component Layout Reference

This section provides a quick reference for how each WaterUI component behaves during layout. Components are categorized by their `StretchAxis` value.

### 6.1 Content-Sized Components (`StretchAxis::None`)

These components size themselves based on their content and platform styling. They never stretch to fill surplus space.

| Component | Measurement Behavior | Notes |
|-----------|---------------------|-------|
| **Text** | Multi-pass: (1) `proposal(nil, nil)` → single-line size, (2) `proposal(w, nil)` → wrapped height, (3) `proposal(w, h)` → truncate with ellipsis | Wraps to multiple lines when width-constrained |
| **Button** | `label_size + platform_padding` | Size determined by label content + platform button style |
| **Toggle** | `label_width + spacing + switch_width` | Switch size is platform-determined |
| **Stepper** | `label_width + spacing + stepper_buttons` | Button sizes are platform-determined |
| **Link** | Same as Text | Behaves like styled text with tap action |
| **Badge** | `content_size + overlay_size` | Overlay positioned at corner |
| **Picker** | Platform-determined based on style | Dropdown/wheel/segmented styles |
| **DatePicker** | Platform-determined | Compact, wheel, or graphical styles |
| **ColorPicker** | Platform-determined | Color well or expanded picker |
| **Progress (Circular)** | Fixed platform size | Spinning indicator |

### 6.2 Horizontally-Expanding Components (`StretchAxis::Horizontal`)

These components expand to fill available width but maintain intrinsic height. In `size_that_fits`, they report a minimum usable width; during `place`, they expand to fill the allocated bounds.

| Component | Measurement Behavior | Notes |
|-----------|---------------------|-------|
| **TextField** | Height: fixed intrinsic, Width: minimum usable (~100pt) | Expands horizontally to fill container width |
| **SecureField** | Same as TextField | Password input with masked characters |
| **Slider** | Height: fixed track height, Width: minimum usable | Expands horizontally; includes optional min/max labels |
| **Progress (Linear)** | Height: fixed track height, Width: minimum usable | Expands horizontally to show progress bar |

### 6.3 Greedy Components (`StretchAxis::Both`)

These components expand to fill all available space in both dimensions.

| Component | Measurement Behavior | Notes |
|-----------|---------------------|-------|
| **Color** | With proposal: returns full proposal size, Without: small fallback (10×10pt) | Background/shape fill |
| **ScrollView** | Expands to fill available bounds | Content can scroll beyond bounds |
| **NavigationView** | Fills container | Navigation controller wrapper |
| **NavigationStack** | Fills container | Stack-based navigation |

### 6.4 Axis-Relative Components

These components adapt their stretch behavior based on the parent container's axis.

| Component | StretchAxis | Behavior |
|-----------|-------------|----------|
| **Spacer** | `MainAxis` | In VStack: expands vertically. In HStack: expands horizontally. Reports `(minLength, minLength)` as intrinsic size, then fills remaining surplus during `place`. |
| **Divider** | `CrossAxis` (via Color) | In VStack: horizontal line (full width, 1-2pt height). In HStack: vertical line (full height, 1-2pt width). Uses `Color` internally with frame modifier. |

### 6.5 Container Components

Containers delegate to the Rust layout engine and inherit stretch behavior from their `Layout` implementation.

| Container | StretchAxis | Behavior |
|-----------|-------------|----------|
| **VStack** | `Horizontal` | Expands horizontally to fill available width; height is sum of children + spacing |
| **HStack** | `Vertical` (implicit) | Expands vertically to fill available height; width is sum of children + spacing |
| **ZStack** | `None` | Size is maximum of all children |
| **Grid** | `None` | Size determined by grid configuration |
| **Frame** | `None` | Fixed size wrapper |
| **Padding** | `None` | Adds insets around content |

### 6.6 Backend Implementation Notes

When implementing a native backend:

1. **Text Measurement Protocol:**
   ```
   Pass 1 - PROBE:    proposal(nil, nil)    → (single_line_width, line_height)
   Pass 2 - WRAP:     proposal(w, nil)      → (actual_width ≤ w, wrapped_height)
   Pass 3 - TRUNCATE: proposal(w, h)        → (w, h) with ellipsis if needed
   ```

2. **Horizontal-Stretch Components:**
   - Return minimum usable width in `size_that_fits`
   - Expand to full bounds width during `place` phase
   - Height remains intrinsic (platform-determined)

3. **Both-Stretch Components:**
   - Return proposal size when constrained
   - Return small fallback (e.g., 10×10pt) when unconstrained
   - Fill entire bounds during `place`

4. **MainAxis/CrossAxis Resolution:**
   - Check parent's `Axis` from environment
   - `MainAxis` + VStack → Vertical stretch
   - `MainAxis` + HStack → Horizontal stretch
   - `CrossAxis` + VStack → Horizontal stretch
   - `CrossAxis` + HStack → Vertical stretch
