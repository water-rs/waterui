# WaterUI Layout Issue Report: Axis-Expanding Views on Apple Backend

**Date:** November 29, 2025  
**Platform:** macOS (SwiftUI rendering path)  
**Severity:** Medium - UI elements render incorrectly but app is functional

---

## Executive Summary

The Apple backend (macOS/iOS) has a layout discrepancy compared to Android: **axis-expanding views** (TextField, Slider, ProgressView, Divider) do not expand to fill available width in VStack containers. These views appear narrow/collapsed while the Android backend correctly expands them.

An attempted fix (proposing parent's width to children) caused **text content to overflow** beyond container bounds, indicating a deeper architectural tension in the layout system.

---

## Problem Description

### Observed Behavior (macOS)
- TextField, Slider, ProgressView render at minimal width instead of filling container
- Divider appears as a tiny dash instead of full-width line
- Text labels are correctly sized and centered

### Expected Behavior (Android - Working)
- TextField, Slider, ProgressView expand to fill container width
- Divider spans full container width
- Text labels are correctly sized and centered

### Screenshots Reference
- **Android:** All controls properly expanded
- **macOS (before fix):** Controls collapsed to minimum width
- **macOS (after attempted fix):** Text overflows container bounds

---

## Technical Analysis

### The Three-Pass Layout Algorithm

```
1. PROPOSE (top → bottom): Parent sends ProposalSize to children
2. SIZE (bottom → top): Children report measured size via ChildMetadata
3. PLACE (top → bottom): Parent assigns final Rect positions
```

### Current VStack Implementation (Rust)

```rust
// components/layout/src/stack/vstack.rs

fn propose(&mut self, _parent: ProposalSize, children: &[ChildMetadata], _context: &LayoutContext) -> Vec<ProposalSize> {
    // Current: Proposes None for width to ALL children
    vec![ProposalSize::new(None, None); children.len()]
}

fn place(&mut self, bound: Rect, ...) -> Vec<ChildPlacement> {
    // Uses child's MEASURED width for placement
    let child_width = child_proposal.width.unwrap_or(0.0);
    
    // Center alignment calculation
    let x = bound.origin().x + (bound.width() - child_width) / 2.0;
    
    // Child placed at measured size, not container size
    let size = Size::new(child_width, child_height);
}
```

### The Core Tension

| Requirement | Needs | Current Behavior |
|------------|-------|------------------|
| **Content-sized views** (Text, Button) | `None` proposal → measure intrinsic | ✅ Works |
| **Axis-expanding views** (TextField, Slider) | Know available width → fill it | ❌ Gets `None`, measures minimum |
| **VStack sizing** | Size to content width for alignment | ✅ Works |
| **Center alignment** | Children centered based on measured width | ✅ Works |

The VStack proposes `None` so it can determine its own width from children's intrinsic sizes. But axis-expanding views need to know the available width to expand into it.

---

## Why Android Works

Android's axis-expanding components override `onMeasure`:

```kotlin
// TextFieldComponent.kt, SliderComponent.kt, ProgressComponent.kt
override fun onMeasure(widthMeasureSpec: Int, heightMeasureSpec: Int) {
    val widthMode = MeasureSpec.getMode(widthMeasureSpec)
    val widthSize = MeasureSpec.getSize(widthMeasureSpec)
    
    // If AT_MOST or EXACTLY, expand to fill
    val expandedWidthSpec = if (widthMode == MeasureSpec.AT_MOST || widthMode == MeasureSpec.EXACTLY) {
        MeasureSpec.makeMeasureSpec(widthSize, MeasureSpec.EXACTLY)
    } else {
        widthMeasureSpec
    }
    super.onMeasure(expandedWidthSpec, heightMeasureSpec)
}
```

**Key insight:** Android's `AT_MOST` mode provides the available space even when asking for intrinsic size. The view can choose to fill it.

### Apple's Problem

When VStack proposes `None`, Swift converts this to `ProposedViewSize(width: nil, height: nil)`. SwiftUI views receiving `nil` measure to their **intrinsic compressed size**, not the available space.

There's no equivalent to Android's `AT_MOST` - you either get a specific size or `nil` (intrinsic).

---

## Attempted Fix & Why It Failed

### The Fix
Changed VStack to propose parent's width:
```rust
fn propose(&mut self, parent: ProposalSize, ...) -> Vec<ProposalSize> {
    vec![ProposalSize::new(parent.width, None); children.len()]  // Pass parent width
}
```

### Why It Failed
1. **Text views now receive the full container width**
2. Text measured itself at container width (correct for wrapping)
3. VStack calculated `max_width` = container width (from Text)
4. VStack sized itself to container width
5. Text was placed at full width, **overflowing its actual content**

The issue: Text views should receive width for **wrapping calculation** but report their **intrinsic content width**, not the proposed width.

---

## View Categories (from LAYOUT_SPEC.md)

### Category A: Content-Sized (Hugging)
- Text, Button, Toggle, Stepper, Picker
- Should measure intrinsic size regardless of proposal
- **Current behavior:** ✅ Correct

### Category B: Axis-Expanding
- TextField, Slider, ProgressView (linear), Divider
- Should expand to fill proposed width, intrinsic height
- **Current behavior:** ❌ Broken on Apple

### Category C: Greedy
- Color, Spacer
- Should expand both dimensions
- **Current behavior:** Partially working via `stretch` flag

---

## Relevant Code Locations

### Rust Layout Engine
```
components/layout/src/stack/vstack.rs    - VStack layout algorithm
components/layout/src/stack/hstack.rs    - HStack layout algorithm
components/layout/src/frame.rs           - Frame modifier layout
components/layout/src/core.rs            - ChildMetadata struct
```

### Apple Backend (SwiftUI path)
```
backends/apple/Sources/WaterUI/Layout/Layout.swift  
  - RustLayout: SwiftUI Layout protocol implementation
  - Lines 462-534: sizeThatFits (measurement)
  - Lines 537-580: placeSubviews (placement)
  
backends/apple/Sources/WaterUI/PlatformRuntime/NativeLayoutBridge.swift
  - Bridge between Swift and Rust FFI
```

### Android Backend (Working Reference)
```
backends/android/runtime/.../TextFieldComponent.kt   - onMeasure override
backends/android/runtime/.../SliderComponent.kt      - onMeasure override
backends/android/runtime/.../layout/RustLayoutViewGroup.kt - Layout orchestration
```

---

## Possible Solutions

### Option 1: Two-Pass Measurement
VStack does two measurement passes:
1. First pass: Propose `None` to get intrinsic sizes (for sizing/alignment)
2. Second pass: Propose container width to axis-expanding views only

**Challenge:** Need to identify which views are axis-expanding.

### Option 2: Add `axis_expanding` Flag to ChildMetadata
```rust
pub struct ChildMetadata {
    proposal: ProposalSize,
    priority: u8,
    stretch: bool,
    axis_expanding_width: bool,   // NEW
    axis_expanding_height: bool,  // NEW
}
```

VStack's `place` method would give `bound.width()` to axis-expanding children.

**Challenge:** Requires FFI changes, Swift type updates.

### Option 3: Smart View Detection in Swift
In `RustLayout.sizeThatFits`, detect axis-expanding views by typeId and provide available width during measurement.

```swift
let isAxisExpanding = ["TextField", "Slider", "Progress"].contains(typeId)
let proposal = isAxisExpanding 
    ? ProposedViewSize(width: availableWidth, height: nil)
    : ProposedViewSize(width: nil, height: nil)
```

**Challenge:** Hardcoded list, doesn't propagate through nested layouts.

### Option 4: Separate Intrinsic vs. Rendered Size
Views report two sizes:
- `intrinsicSize`: For container sizing calculations
- `renderedSize`: Actual size to render at

Axis-expanding views would report small intrinsic but large rendered size.

**Challenge:** Significant architecture change.

### Option 5: Fix Content-Sized Views to Ignore Proposal Width
Text and other content-sized views should:
- Use proposal width only for wrapping calculation
- Report intrinsic content width, not proposal width

Then VStack can safely propose parent's width without overflow.

**Challenge:** Need to update all content-sized view measurements.

---

## Recommended Approach

**Option 5** seems most aligned with the spec and lowest risk:

1. Content-sized views (Text, Button, etc.) measure with proposed width for layout hints but report intrinsic width
2. Axis-expanding views (TextField, Slider, etc.) measure and report the proposed width
3. VStack proposes parent's width to all children

This matches Android's behavior where:
- `AT_MOST` width is available to all views
- Content-sized views ignore it and report intrinsic
- Axis-expanding views use it and report full width

---

## Testing Checklist

After any fix, verify:
- [ ] Text labels remain centered, don't overflow
- [ ] TextField expands to container width
- [ ] Slider expands to container width
- [ ] ProgressView (linear) expands to container width
- [ ] Divider expands to container width
- [ ] Nested VStack/HStack layouts work correctly
- [ ] Frame modifier constraints are respected
- [ ] Android behavior unchanged

---

## Files to Modify

Depending on chosen approach:

| File | Purpose |
|------|---------|
| `components/layout/src/stack/vstack.rs` | Proposal logic |
| `components/layout/src/core.rs` | ChildMetadata struct |
| `backends/apple/Sources/WaterUI/Layout/Layout.swift` | Swift measurement |
| `backends/apple/Sources/WaterUI/Text.swift` | Text measurement behavior |
| `ffi/src/layout.rs` | FFI bindings if ChildMetadata changes |

---

## Appendix: Divider Implementation

The Divider is implemented as:
```rust
// src/widget/divder.rs
impl View for Divider {
    fn body(self, _env: &Environment) -> impl View {
        Color::srgb_f32(0.8, 0.8, 0.8)
            .height(1.0)
            .width(f32::INFINITY)  // Signals "expand to fill"
    }
}
```

The `f32::INFINITY` width is handled by `FrameLayout.propose`:
```rust
let proposed_width = self.ideal_width.or(parent.width);  // Uses INFINITY
// ... then clamped, but INFINITY passes through
```

When parent proposes `None`, Frame's size calculation:
```rust
let target_width = self.ideal_width.unwrap_or(child_size.width);  // = INFINITY
Size::new(parent.width.unwrap_or(target_width), ...)  // = INFINITY if parent is None
```

This causes issues when parent width is `None` - the Frame reports `INFINITY` width.

---

*End of Report*

