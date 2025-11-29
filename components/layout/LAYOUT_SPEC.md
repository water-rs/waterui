# WaterUI Layout Specification

This document defines the standardized layout behavior for WaterUI across all backends (Android, Apple, Web, etc.). The layout system is inspired by SwiftUI's concepts but **all backends perform layout calculations manually** using the shared Rust layout engine.

---

## Architecture Overview

WaterUI uses a **centralized layout engine written in Rust**. All backends (Android, Apple, Web) call into this shared engine via FFI to:

1. Propose sizes to children
2. Calculate container sizes
3. Determine child placements

**Important**: No backend should rely on platform-native layout systems (Android's `ViewGroup` layout, SwiftUI's layout, CSS flexbox) for WaterUI containers. The Rust layout engine is the single source of truth for layout calculations.

### Backend Responsibilities

| Component | Rust Layout Engine | Native Backend |
|-----------|-------------------|----------------|
| Container layout (HStack, VStack, etc.) | ✅ Calculates | Applies positions |
| Raw view measurement (Text, Button) | Receives proposals | ✅ Measures intrinsic size |
| Child placement | ✅ Calculates rects | Applies positions |

---

## Core Concepts

### Proposal Values

A `ProposalSize` contains optional width and height constraints. Each dimension can be one of:

| Value | Rust Representation | FFI Representation | Meaning |
|-------|--------------------|--------------------|---------|
| **None** | `None` | `NaN` | "I don't care" - child decides its own size, capped by available space |
| **Zero** | `Some(0.0)` | `0.0` | Minimum possible size |
| **Exact** | `Some(value)` | `value` | Specific size in pixels |
| **Infinity** | `Some(f32::INFINITY)` | `f32::INFINITY` | Maximum possible size - expand as much as you can |

### Size Constraints

Views can specify constraints using these properties:

| Property | Also Known As | Description |
|----------|---------------|-------------|
| `width` | `idealWidth` | The preferred width when parent proposes `None` |
| `minWidth` | - | Minimum acceptable width |
| `maxWidth` | - | Maximum acceptable width |
| `height` | `idealHeight` | The preferred height when parent proposes `None` |
| `minHeight` | - | Minimum acceptable height |
| `maxHeight` | - | Maximum acceptable height |

---

## Layout Protocol

### Three-Pass Algorithm

1. **Propose** (top → bottom): Parent sends `ProposalSize` to children
2. **Size** (bottom → top): Children report their measured size via `ChildMetadata`
3. **Place** (top → bottom): Parent assigns final `Rect` positions to children

### The `Layout` Trait

```rust
pub trait Layout: Debug {
    /// Proposes sizes for each child based on parent's proposal
    fn propose(&mut self, parent: ProposalSize, children: &[ChildMetadata]) -> Vec<ProposalSize>;
    
    /// Computes layout's own size after children respond
    fn size(&mut self, parent: ProposalSize, children: &[ChildMetadata]) -> Size;
    
    /// Places children within final bounds
    fn place(&mut self, bound: Rect, proposal: ProposalSize, children: &[ChildMetadata]) -> Vec<Rect>;
}
```

Users can implement `Layout` to create custom container layouts. However, **raw view layout behavior** (Text, Button, Image, etc.) is defined by the framework and cannot be customized directly by users.

---

## Raw View Layout Behaviors

Raw views have predefined layout behaviors that **must be consistent across all backends**. Backend maintainers must implement these behaviors exactly as specified.

### Text

**Expansion Priority**: Width → Height → Overflow

1. **Width Expansion**: Text expands horizontally to fit content
2. **Height Expansion**: When width is constrained, text wraps and expands vertically
3. **Overflow**: When both dimensions are constrained, text truncates with ellipsis

**Measurement Behavior**:

| Proposal | Behavior |
|----------|----------|
| `width: None` | Measure to intrinsic width, capped by parent's available space; wrap if needed |
| `width: Some(w)` | Constrain to width `w`, wrap text as needed |
| `width: Infinity` | Measure to intrinsic single-line width (no wrapping) |
| `height: None` | Expand to fit wrapped content |
| `height: Some(h)` | Constrain to height `h`, truncate if needed |

**Default Properties**:
- `minWidth`: 0
- `maxWidth`: Infinity
- `minHeight`: Single line height
- `maxHeight`: Infinity

### Button

**Behavior**: Hugs content with minimum touch target size

| Proposal | Behavior |
|----------|----------|
| `width: None` | Fit content width, minimum 44pt touch target |
| `width: Some(w)` | Use width `w` |
| `height: None` | Fit content height, minimum 44pt touch target |
| `height: Some(h)` | Use height `h` |

### Image

**Behavior**: Maintains aspect ratio by default

| Proposal | Behavior |
|----------|----------|
| `width: None, height: None` | Use intrinsic image size |
| `width: Some(w), height: None` | Scale to width, maintain aspect ratio |
| `width: None, height: Some(h)` | Scale to height, maintain aspect ratio |
| `width: Some(w), height: Some(h)` | Fit within bounds, maintain aspect ratio |

### Spacer

**Behavior**: Zero intrinsic size, expands to fill available space

| Proposal | Behavior |
|----------|----------|
| Any | Reports zero size but marks `stretch: true` |

Spacers are detected by parent layouts and receive proportional share of remaining space.

### Color

**Behavior**: Filled rectangle that expands to fill available space (like SwiftUI's `Color`)

| Proposal | Behavior |
|----------|----------|
| `width: None` | Expand to fill available width |
| `width: Some(w)` | Use width `w` |
| `width: Infinity` | Expand to maximum available width |
| `height: None` | Expand to fill available height |
| `height: Some(h)` | Use height `h` |
| `height: Infinity` | Expand to maximum available height |

**Default Properties**:
- `minWidth`: 0
- `maxWidth`: Infinity
- `minHeight`: 0
- `maxHeight`: Infinity

**Note**: Color is a "greedy" view - it will expand to fill all available space unless constrained by frame modifiers. This is consistent with SwiftUI's `Color` behavior.

---

## Container Layout Behaviors

All container layouts are implemented in Rust and shared across backends.

### HStack (Horizontal Stack)

**Algorithm**:
1. Propose `None` width to all children (measure intrinsic)
2. Sum non-stretchy children widths + spacing
3. Distribute remaining width among stretchy children
4. Place children left-to-right with spacing

**Child Proposal**:
- Width: `None` (child decides)
- Height: Parent's height proposal

### VStack (Vertical Stack)

**Algorithm**:
1. Propose `None` height to all children (measure intrinsic)
2. Sum non-stretchy children heights + spacing
3. Distribute remaining height among stretchy children
4. Place children top-to-bottom with spacing

**Child Proposal**:
- Width: Parent's width proposal
- Height: `None` (child decides)

### ZStack (Overlay Stack)

**Algorithm**:
1. Propose parent's full size to all children
2. Size is maximum of all children
3. Place all children at origin with alignment

### Overlay

**Algorithm**:
1. Measure base child with parent proposal
2. Measure overlay child with base child's size
3. Size is base child's size (overlay doesn't affect size)
4. Place overlay aligned to base

---

## Backend Implementation Guide

### General Requirements

1. **Use Rust Layout Engine**: All container layouts must call `waterui_layout_propose`, `waterui_layout_size`, and `waterui_layout_place` via FFI
2. **Manual Placement**: Backends must manually position child views using the `Rect` values returned by `waterui_layout_place`
3. **Consistent Measurement**: Raw views must measure consistently with this spec

### Android Implementation

**Proposal to MeasureSpec Conversion**:

| Proposal | MeasureSpec Mode | Size |
|----------|------------------|------|
| `None` (NaN) | `AT_MOST` | Parent's available space |
| `Zero` (0.0) | `EXACTLY` | 0 |
| `Exact` (value) | `EXACTLY` | value |
| `Infinity` | `UNSPECIFIED` | 0 |

**Key Points**:
- Use `RustLayoutViewGroup` which delegates to Rust layout engine
- Do NOT use Android's built-in `LinearLayout`, `FrameLayout`, etc. for WaterUI containers
- Raw views (TextView, Button) are measured natively, then results sent to Rust

### Apple Implementation (iOS/macOS)

**Important**: WaterUI on Apple platforms does **NOT** use SwiftUI's layout system. Instead:

1. Use `UIView`/`NSView` subclasses that call Rust layout engine
2. Override `layoutSubviews()` / `layout()` to:
   - Call `waterui_layout_propose` and measure children
   - Call `waterui_layout_size` to determine container size
   - Call `waterui_layout_place` to get child rects
   - Manually set each child's `frame`

**Proposal Conversion**:
- `None` → Measure with `sizeThatFits(.init(width: availableWidth, height: .greatestFiniteMagnitude))`
- `Exact` → Use exact value
- `Infinity` → Measure with `sizeThatFits(.init(width: .greatestFiniteMagnitude, height: ...))`

### Web Implementation

**Proposal to CSS Conversion**:

| Proposal | CSS |
|----------|-----|
| `None` | `max-width: {parent}px` (measure intrinsic, cap at parent) |
| `Zero` | `width: 0` |
| `Exact` | `width: {value}px` |
| `Infinity` | `width: max-content` |

**Key Points**:
- Use absolute positioning for child placement
- Do NOT rely on CSS flexbox/grid for WaterUI container layouts
- Container divs should have `position: relative`
- Child divs positioned with `position: absolute; left: {x}px; top: {y}px`

---

## Frame Modifier

The frame modifier allows users to override default layout behavior:

```rust
view
    .width(100.0)           // idealWidth
    .min_width(50.0)        // minWidth
    .max_width(200.0)       // maxWidth
    .height(50.0)           // idealHeight
    .min_height(30.0)       // minHeight
    .max_height(100.0)      // maxHeight
```

**Resolution Algorithm**:
1. If parent proposes `None`: use `idealWidth/idealHeight`
2. Clamp result to `[minWidth, maxWidth]` and `[minHeight, maxHeight]`
3. If parent proposes specific value: use that value, clamped to min/max

---

## Design Principles

1. **Single Source of Truth**: Rust layout engine calculates all container layouts
2. **Consistency**: Same code produces same layout on all platforms
3. **Predictability**: Layout behavior is deterministic and documented
4. **Backend Simplicity**: Backends only measure raw views and apply positions
5. **Flexibility**: Users can customize container layouts via `Layout` trait

---

## For Backend Maintainers

### Adding a New Backend

1. Implement FFI bindings for `waterui_layout_*` functions
2. Create a container view class that:
   - Holds a `layoutPtr` to the Rust layout object
   - Calls Rust layout engine in measure/layout passes
   - Manually positions children using returned `Rect` values
3. Implement raw view renderers that measure correctly per this spec
4. Test against the reference implementation (use `water-demo` app)

### Debugging Layout Issues

1. Add logging to see proposal values and measured sizes
2. Compare with another backend's output
3. Verify Rust layout engine is receiving correct `ChildMetadata`
4. Check that child positions match returned `Rect` values exactly

---

## Platform Component Differences & Normalization

Different platforms have different native components with varying default behaviors. WaterUI normalizes these to provide consistent behavior across all platforms.

### Text / Label

| Aspect | Android (TextView) | UIKit (UILabel) | AppKit (NSTextField) | WaterUI Normalized |
|--------|-------------------|-----------------|---------------------|-------------------|
| Default lines | Unlimited | 1 line | 1 line | **Unlimited** (wrap by default) |
| Line break mode | Word wrap | Truncate tail | Truncate tail | **Word wrap** |
| Intrinsic size | Content size | Content size (single line) | Content size (single line) | **Content size with wrapping** |
| Text alignment | Start | Natural | Natural | **Start** (LTR) / **End** (RTL) |

**Normalization**: All platforms should measure text with word wrapping enabled by default. When width is constrained, text wraps to multiple lines.

### Button

| Aspect | Android (Button) | UIKit (UIButton) | AppKit (NSButton) | WaterUI Normalized |
|--------|-----------------|------------------|-------------------|-------------------|
| Min touch target | 48dp | 44pt | None | **44pt** (accessibility) |
| Content padding | 16dp horizontal | Varies by style | Varies by style | **12pt horizontal, 8pt vertical** |
| Default style | Material filled | System | Push | **Platform-adaptive** |

**Normalization**: Buttons have minimum 44pt touch target on all platforms. Visual style adapts to platform conventions.

### TextField / TextInput

| Aspect | Android (EditText) | UIKit (UITextField) | AppKit (NSTextField) | WaterUI Normalized |
|--------|-------------------|--------------------|--------------------|-------------------|
| Border style | Underline (Material) | Rounded rect | Bezel | **Platform-adaptive** |
| Clear button | None by default | Optional | None | **None by default** |
| Return key action | Next/Done | Return | Return | **Configurable** |
| Keyboard type | Default | Default | N/A | **Configurable** |

**Normalization**: Text fields adapt visual style to platform but expose consistent API for keyboard type, return action, etc.

### Toggle / Switch

| Aspect | Android (Switch) | UIKit (UISwitch) | AppKit (NSSwitch) | WaterUI Normalized |
|--------|-----------------|------------------|-------------------|-------------------|
| Size | 52x32dp | 51x31pt | 38x22pt | **Platform default** |
| Thumb style | Circular | Circular | Circular | **Platform default** |
| Track color | Material colors | Green/Gray | Blue/Gray | **Platform-adaptive** |

**Normalization**: Toggles use platform-native appearance. Size is not customizable to maintain platform consistency.

### Slider

| Aspect | Android (Slider) | UIKit (UISlider) | AppKit (NSSlider) | WaterUI Normalized |
|--------|-----------------|------------------|-------------------|-------------------|
| Track height | 4dp | 2pt | 4pt | **Platform default** |
| Thumb size | 20dp | 28pt | Varies | **Platform default** |
| Continuous updates | Yes | Yes | Yes | **Yes** |

**Normalization**: Sliders provide continuous value updates on all platforms. Visual appearance is platform-native.

### ScrollView

| Aspect | Android (ScrollView) | UIKit (UIScrollView) | AppKit (NSScrollView) | WaterUI Normalized |
|--------|---------------------|---------------------|-----------------------|-------------------|
| Bounce | No (by default) | Yes | Yes | **Platform default** |
| Scroll indicators | Fading | Fading | Overlay/Legacy | **Platform default** |
| Content insets | Manual | Safe area aware | Manual | **Safe area aware** |

**Normalization**: Scroll views respect safe area insets on all platforms. Bounce and indicator behavior follows platform conventions.

### List / Table

| Aspect | Android (RecyclerView) | UIKit (UITableView) | AppKit (NSTableView) | WaterUI Normalized |
|--------|----------------------|--------------------|--------------------|-------------------|
| Cell recycling | Yes | Yes | Yes | **Yes** |
| Separator style | None by default | Line | Line | **Platform default** |
| Selection style | Ripple | Highlight | Highlight | **Platform-adaptive** |
| Swipe actions | Manual | Built-in | Manual | **Unified API** |

**Normalization**: Lists provide unified API for swipe actions, selection, and cell configuration. Visual feedback follows platform conventions.

---

## Measurement Normalization

### Density-Independent Units

WaterUI uses **points (pt)** as the standard unit across all platforms:

| Platform | Native Unit | Conversion |
|----------|-------------|------------|
| Android | dp (density-independent pixels) | 1pt = 1dp |
| iOS | pt (points) | 1pt = 1pt |
| macOS | pt (points) | 1pt = 1pt |
| Web | px (CSS pixels) | 1pt = 1px (at 1x scale) |

### Font Sizes

| WaterUI Size | Android (sp) | iOS/macOS (pt) | Web (px) |
|--------------|--------------|----------------|----------|
| `caption` | 12sp | 12pt | 12px |
| `footnote` | 13sp | 13pt | 13px |
| `body` | 16sp | 17pt | 16px |
| `headline` | 17sp | 17pt | 17px |
| `title` | 20sp | 20pt | 20px |
| `largeTitle` | 34sp | 34pt | 34px |

**Note**: Font sizes may vary slightly between platforms to match native typography conventions while maintaining visual harmony.

---

## Safe Area System

Safe area is a critical concept for modern UI that handles device-specific regions (notches, home indicators, status bars) and dynamic regions (keyboard). WaterUI's safe area system is inspired by SwiftUI but adapted for our cross-platform, Rust-based architecture.

### Platform Safe Area Sources

| Area | Android | iOS | macOS | Web |
|------|---------|-----|-------|-----|
| Status bar | `WindowInsetsCompat.Type.statusBars()` | `safeAreaInsets.top` | N/A | N/A |
| Navigation bar | `WindowInsetsCompat.Type.navigationBars()` | `safeAreaInsets.bottom` | N/A | N/A |
| Notch/Dynamic Island | `WindowInsetsCompat.Type.displayCutout()` | `safeAreaInsets` | N/A | N/A |
| Keyboard | `WindowInsetsCompat.Type.ime()` | Keyboard notifications | N/A | `visualViewport` |
| Home indicator | N/A | `safeAreaInsets.bottom` | N/A | N/A |

---

### Core Data Structures

```rust
/// Safe area insets in points, relative to the container bounds
#[derive(Debug, Clone, Default)]
pub struct SafeAreaInsets {
    pub top: f32,
    pub bottom: f32,
    pub leading: f32,
    pub trailing: f32,
}

impl SafeAreaInsets {
    pub const ZERO: Self = Self { top: 0.0, bottom: 0.0, leading: 0.0, trailing: 0.0 };
    
    /// Inset a rect by the safe area
    pub fn inset(&self, rect: Rect) -> Rect { /* ... */ }
    
    /// Combine with another safe area (takes max of each edge)
    pub fn union(&self, other: &Self) -> Self { /* ... */ }
    
    /// Subtract padding that has already been applied
    pub fn subtract(&self, insets: EdgeInsets) -> Self { /* ... */ }
}

/// Regions of safe area that can be ignored
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SafeAreaRegion {
    /// Device safe areas (notch, home indicator, status bar)
    Container,
    /// Keyboard safe area
    Keyboard,
    /// All safe areas
    All,
}

/// Edges that can be selectively ignored (bitflags)
bitflags! {
    pub struct SafeAreaEdges: u8 {
        const TOP = 0b0001;
        const BOTTOM = 0b0010;
        const LEADING = 0b0100;
        const TRAILING = 0b1000;
        const HORIZONTAL = Self::LEADING.bits() | Self::TRAILING.bits();
        const VERTICAL = Self::TOP.bits() | Self::BOTTOM.bits();
        const ALL = Self::HORIZONTAL.bits() | Self::VERTICAL.bits();
    }
}
```

---

### Safe Area Propagation

Safe area **propagates down** the view hierarchy and can be **consumed** or **ignored** at any level.

#### Propagation Rules

1. **Root receives full safe area** from the backend
2. **Containers pass safe area to children** via `LayoutContext`
3. **Padding/insets consume safe area** - reduces it for children
4. **`ignores_safe_area` expands bounds** - view renders beyond safe area

```
┌─────────────────────────────────────┐
│ Screen (with notch at top)          │
│ ┌─────────────────────────────────┐ │
│ │ Safe Area (inset from notch)    │ │
│ │ ┌─────────────────────────────┐ │ │
│ │ │ VStack with padding         │ │ │
│ │ │ (consumes more safe area)   │ │ │
│ │ │ ┌─────────────────────────┐ │ │ │
│ │ │ │ Child (remaining safe)  │ │ │ │
│ │ │ └─────────────────────────┘ │ │ │
│ │ └─────────────────────────────┘ │ │
│ └─────────────────────────────────┘ │
└─────────────────────────────────────┘
```

---

### Layout Trait Integration

The `Layout` trait receives safe area context to make informed decisions:

```rust
/// Context passed to layout operations
#[derive(Debug, Clone)]
pub struct LayoutContext {
    /// Safe area insets relative to this container's bounds
    pub safe_area: SafeAreaInsets,
    
    /// Which safe area edges this container ignores (for children to inherit)
    pub ignores_safe_area: SafeAreaEdges,
}

pub trait Layout: Debug {
    /// Proposes sizes for each child
    /// `context` contains safe area info for this container
    fn propose(
        &mut self,
        parent: ProposalSize,
        children: &[ChildMetadata],
        context: &LayoutContext,
    ) -> Vec<ProposalSize>;

    /// Computes layout's own size
    fn size(
        &mut self,
        parent: ProposalSize,
        children: &[ChildMetadata],
        context: &LayoutContext,
    ) -> Size;

    /// Places children within bounds
    /// Returns (rects, child_contexts) - each child gets its own safe area context
    fn place(
        &mut self,
        bound: Rect,
        proposal: ProposalSize,
        children: &[ChildMetadata],
        context: &LayoutContext,
    ) -> Vec<(Rect, LayoutContext)>;
}
```

#### Example: VStack respecting safe area

```rust
impl Layout for VStackLayout {
    fn place(
        &mut self,
        bound: Rect,
        proposal: ProposalSize,
        children: &[ChildMetadata],
        context: &LayoutContext,
    ) -> Vec<(Rect, LayoutContext)> {
        let mut results = Vec::new();
        let mut current_y = bound.y();
        
        // First child may need top safe area
        let mut remaining_safe_area = context.safe_area.clone();
        
        for (i, child) in children.iter().enumerate() {
            let child_height = child.proposal_height().unwrap_or(0.0);
            let child_rect = Rect::new(
                Point::new(bound.x(), current_y),
                Size::new(bound.width(), child_height),
            );
            
            // Calculate safe area for this child
            let child_safe_area = SafeAreaInsets {
                top: if i == 0 { remaining_safe_area.top } else { 0.0 },
                bottom: if i == children.len() - 1 { remaining_safe_area.bottom } else { 0.0 },
                leading: remaining_safe_area.leading,
                trailing: remaining_safe_area.trailing,
            };
            
            let child_context = LayoutContext {
                safe_area: child_safe_area,
                ignores_safe_area: context.ignores_safe_area,
            };
            
            results.push((child_rect, child_context));
            current_y += child_height + self.spacing;
            
            // Consume top safe area after first child
            remaining_safe_area.top = 0.0;
        }
        
        results
    }
}
```

---

### User API: Ignoring Safe Area

```rust
use waterui::prelude::*;

// Ignore all safe areas (background extends edge-to-edge)
Color::blue.ignores_safe_area(SafeAreaRegion::All)

// Ignore only container safe area (still respect keyboard)
image("hero").ignores_safe_area(SafeAreaRegion::Container)

// Ignore specific edges
header.ignores_safe_area_edges(SafeAreaEdges::TOP)

// Ignore multiple edges
content.ignores_safe_area_edges(SafeAreaEdges::TOP | SafeAreaEdges::BOTTOM)
```

---

### Implementing `IgnoreSafeArea` with the `Layout` Trait

`IgnoreSafeArea` is **not a special case** - it's implemented as a normal container view with a custom `Layout`. This keeps the system simple and composable.

```rust
/// A container that ignores safe area for its child
#[derive(Debug)]
pub struct IgnoreSafeArea<V> {
    child: V,
    edges: SafeAreaEdges,
}

/// The layout implementation for IgnoreSafeArea
#[derive(Debug, Clone)]
pub struct IgnoreSafeAreaLayout {
    edges: SafeAreaEdges,
}

impl Layout for IgnoreSafeAreaLayout {
    fn propose(
        &mut self,
        parent: ProposalSize,
        children: &[ChildMetadata],
        context: &LayoutContext,
    ) -> Vec<ProposalSize> {
        // Expand the proposal to include safe area on ignored edges
        let expanded_width = parent.width.map(|w| {
            let mut extra = 0.0;
            if self.edges.contains(SafeAreaEdges::LEADING) {
                extra += context.safe_area.leading;
            }
            if self.edges.contains(SafeAreaEdges::TRAILING) {
                extra += context.safe_area.trailing;
            }
            w + extra
        });
        
        let expanded_height = parent.height.map(|h| {
            let mut extra = 0.0;
            if self.edges.contains(SafeAreaEdges::TOP) {
                extra += context.safe_area.top;
            }
            if self.edges.contains(SafeAreaEdges::BOTTOM) {
                extra += context.safe_area.bottom;
            }
            h + extra
        });
        
        vec![ProposalSize::new(expanded_width, expanded_height)]
    }

    fn size(
        &mut self,
        parent: ProposalSize,
        children: &[ChildMetadata],
        context: &LayoutContext,
    ) -> Size {
        // Return the child's size (which was measured with expanded proposal)
        children.first()
            .map(|c| Size::new(
                c.proposal_width().unwrap_or(0.0),
                c.proposal_height().unwrap_or(0.0),
            ))
            .unwrap_or(Size::zero())
    }

    fn place(
        &mut self,
        bound: Rect,
        proposal: ProposalSize,
        children: &[ChildMetadata],
        context: &LayoutContext,
    ) -> Vec<(Rect, LayoutContext)> {
        // Expand the bounds to include safe area
        let mut expanded_bound = bound;
        
        if self.edges.contains(SafeAreaEdges::TOP) {
            expanded_bound = Rect::new(
                Point::new(expanded_bound.x(), expanded_bound.y() - context.safe_area.top),
                Size::new(expanded_bound.width(), expanded_bound.height() + context.safe_area.top),
            );
        }
        if self.edges.contains(SafeAreaEdges::BOTTOM) {
            expanded_bound = Rect::new(
                expanded_bound.origin(),
                Size::new(expanded_bound.width(), expanded_bound.height() + context.safe_area.bottom),
            );
        }
        if self.edges.contains(SafeAreaEdges::LEADING) {
            expanded_bound = Rect::new(
                Point::new(expanded_bound.x() - context.safe_area.leading, expanded_bound.y()),
                Size::new(expanded_bound.width() + context.safe_area.leading, expanded_bound.height()),
            );
        }
        if self.edges.contains(SafeAreaEdges::TRAILING) {
            expanded_bound = Rect::new(
                expanded_bound.origin(),
                Size::new(expanded_bound.width() + context.safe_area.trailing, expanded_bound.height()),
            );
        }
        
        // Child gets expanded bounds and zeroed safe area for ignored edges
        let child_context = LayoutContext {
            safe_area: SafeAreaInsets {
                top: if self.edges.contains(SafeAreaEdges::TOP) { 0.0 } else { context.safe_area.top },
                bottom: if self.edges.contains(SafeAreaEdges::BOTTOM) { 0.0 } else { context.safe_area.bottom },
                leading: if self.edges.contains(SafeAreaEdges::LEADING) { 0.0 } else { context.safe_area.leading },
                trailing: if self.edges.contains(SafeAreaEdges::TRAILING) { 0.0 } else { context.safe_area.trailing },
            },
            ignores_safe_area: context.ignores_safe_area | self.edges,
        };
        
        vec![(expanded_bound, child_context)]
    }
}

impl<V: View> View for IgnoreSafeArea<V> {
    fn body(self, _env: &Environment) -> impl View {
        FixedContainer::new(
            IgnoreSafeAreaLayout { edges: self.edges },
            (self.child,),
        )
    }
}

// Convenience extension trait
pub trait ViewExt: View + Sized {
    fn ignores_safe_area(self, region: SafeAreaRegion) -> IgnoreSafeArea<Self> {
        let edges = match region {
            SafeAreaRegion::All => SafeAreaEdges::ALL,
            SafeAreaRegion::Container => SafeAreaEdges::ALL, // TODO: separate keyboard
            SafeAreaRegion::Keyboard => SafeAreaEdges::BOTTOM,
        };
        IgnoreSafeArea { child: self, edges }
    }
    
    fn ignores_safe_area_edges(self, edges: SafeAreaEdges) -> IgnoreSafeArea<Self> {
        IgnoreSafeArea { child: self, edges }
    }
}
```

#### Why This Design is Good

1. **No special cases** - `IgnoreSafeArea` is just another layout container
2. **Composable** - Can be combined with other modifiers naturally
3. **Testable** - Layout logic can be unit tested like any other layout
4. **Backend-agnostic** - Backends don't need special handling for safe area ignoring
5. **Extensible** - Easy to add new safe area behaviors (e.g., `SafeAreaInset`)

#### The Key Insight

The `Layout` trait's `place` method returns `Vec<(Rect, LayoutContext)>` - this allows any layout to:
- **Expand bounds** beyond what it received
- **Modify the safe area** passed to children
- **Consume safe area** (like padding does)

This makes safe area handling a **first-class layout concern**, not a special backend feature.

---

### Backend Implementation Requirements

#### 1. Provide Root Safe Area

Backend must populate safe area at the root level:

```kotlin
// Android (in RustLayoutViewGroup or root view)
fun getRootSafeArea(): SafeAreaInsets {
    val insets = ViewCompat.getRootWindowInsets(this)
    val systemBars = insets?.getInsets(WindowInsetsCompat.Type.systemBars())
    val ime = insets?.getInsets(WindowInsetsCompat.Type.ime())
    
    return SafeAreaInsets(
        top = systemBars?.top?.toFloat() ?: 0f,
        bottom = max(systemBars?.bottom ?: 0, ime?.bottom ?: 0).toFloat(),
        leading = systemBars?.left?.toFloat() ?: 0f,
        trailing = systemBars?.right?.toFloat() ?: 0f,
    )
}
```

#### 2. Pass Context Through FFI

```c
// FFI functions need context parameter
WuiArray_WuiRect waterui_layout_place(
    WuiLayout* layout,
    WuiRect bound,
    WuiProposalSize proposal,
    WuiArray_WuiChildMetadata children,
    WuiLayoutContext context  // NEW: includes safe area
);
```

#### 3. Handle `ignores_safe_area` in View Tree

Backend must track which views ignore safe area and adjust their bounds accordingly during the render pass.

---

### Example: Full-Screen Background with Safe Content

```rust
fn app_screen() -> impl View {
    zstack((
        // Background extends edge-to-edge (ignores all safe areas)
        Color::blue.ignores_safe_area(SafeAreaRegion::All),
        
        // Content respects safe area
        vstack((
            text("Welcome")
                .font_size(34.0)
                .bold(),
            
            spacer(),
            
            text_field(&email, "Email"),
            text_field(&password, "Password"),
            
            button("Sign In", || { /* ... */ }),
        ))
        .padding(EdgeInsets::all(16.0)),
    ))
}
```

In this example:
- `Color::blue` renders from screen edge to screen edge (under notch, over home indicator)
- `vstack` content is inset by safe area, so text/buttons don't overlap with notch
- When keyboard appears, the `vstack` may scroll/resize (if `SafeAreaRegion::Keyboard` is respected)

---

### Open Questions / Future Work

- [ ] Should `LayoutContext` be part of `ChildMetadata` or separate?
- [ ] How to handle animated safe area changes (keyboard appear/dismiss)?
- [ ] Should we support `safeAreaInset(edge:content:)` like SwiftUI for custom safe areas?
- [ ] How does `ignores_safe_area` interact with `scroll` views?

---

## Animation Normalization

| Animation Type | Android | iOS/macOS | WaterUI Normalized |
|---------------|---------|-----------|-------------------|
| Default duration | 300ms | 250ms | **250ms** |
| Default curve | FastOutSlowIn | EaseInOut | **EaseInOut** |
| Spring damping | N/A | 0.7 | **0.7** |
| Spring response | N/A | 0.5s | **0.5s** |

**Normalization**: Animation timing and curves are specified in Rust and applied consistently. Backends translate to native animation APIs.

---

## Future Work

- [ ] Implement `Frame` view with full min/ideal/max support
- [ ] Add `fixedSize()` modifier to prevent expansion
- [ ] Add `layoutPriority()` for controlling measurement order
- [ ] Add `alignmentGuide()` for custom alignment
- [ ] Document animation behavior during layout changes
- [ ] Add layout debugging tools (visual bounds, proposal visualization)
- [ ] Add platform-specific style overrides for advanced customization
- [ ] Document accessibility normalization (VoiceOver, TalkBack, etc.)
