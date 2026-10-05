//! How leaves answer layout proposals — the `MeasureSpec` counterpart of the
//! Apple backend's `native_layout` bridge.
//!
//! Every leaf wraps its platform view in [`ViewSubView`]: proposals become
//! `MeasureSpec`s, `view.measure` runs the platform's own intrinsic measure,
//! and the measured pixels come back in dp. The modes map one-to-one onto
//! `ProposalSize`: `Some(v)` offers `AT_MOST(v)` — an upper bound, the same
//! meaning `sizeThatFits` reads — and `None` is `UNSPECIFIED`. Containers do
//! not probe through this file; their group reports the specs Kotlin hands
//! them directly.

use alloc::rc::Rc;

use jni::Env;
#[cfg(feature = "container")]
use jni::objects::JObject;
use jni::sys::jint;
#[cfg(feature = "container")]
use waterui_core::layout::{Point, Rect};
use waterui_core::layout::{ProposalSize, Size, StretchAxis, SubView, ViewDimensions};
#[cfg(feature = "container")]
use waterui_layout::IgnoreSafeArea;

use crate::contract::PlatformView;
use crate::jvm::{self, Bindings, Platform};

/// The mode a proposal axis carries: `AT_MOST` — a bound — when the
/// parent offered one, `UNSPECIFIED` when it did not. The values are the
/// platform's own `View.MeasureSpec` constants, resolved in `Bindings`.
const fn mode_of(bindings: &Bindings, axis: Option<f32>) -> jint {
    if axis.is_some() {
        bindings.measure_spec_at_most()
    } else {
        bindings.measure_spec_unspecified()
    }
}

/// A `MeasureSpec` for `proposal`: `Some(dp)` → `AT_MOST(dp as px)`,
/// `None` → `UNSPECIFIED`.
fn spec_of(
    env: &mut Env,
    platform: &Platform,
    proposal: ProposalSize,
) -> jni::errors::Result<(jint, jint)> {
    let width = make_spec(env, platform, proposal.width)?;
    let height = make_spec(env, platform, proposal.height)?;
    Ok((width, height))
}

fn make_spec(env: &mut Env, platform: &Platform, axis: Option<f32>) -> jni::errors::Result<jint> {
    let px = axis.map_or(0, |dp| platform.dp_to_px(dp));
    let bindings = platform.bindings();
    bindings.make_measure_spec(env, px, mode_of(bindings, axis))
}

/// Packs a measured pixel size the way `nativeMeasure` returns it to Kotlin.
pub fn pack_measured(width: i32, height: i32) -> i64 {
    (i64::from(width) << 32) | (i64::from(height) & 0xffff_ffff)
}

/// The two axes of an `onMeasure` pair as one `ProposalSize`.
pub fn proposal_from_specs(
    env: &mut Env,
    platform: &Platform,
    width_spec: jint,
    height_spec: jint,
) -> jni::errors::Result<ProposalSize> {
    Ok(ProposalSize::new(
        spec_axis_to_proposal(env, platform, width_spec)?,
        spec_axis_to_proposal(env, platform, height_spec)?,
    ))
}

/// Reads a `MeasureSpec` into the `ProposalSize` axis it offers: `EXACTLY`
/// and `AT_MOST` both bound the child to the spec size; `UNSPECIFIED` leaves
/// it free.
pub fn spec_axis_to_proposal(
    env: &mut Env,
    platform: &Platform,
    spec: jint,
) -> jni::errors::Result<Option<f32>> {
    let bindings = platform.bindings();
    let mode = bindings.measure_spec_mode(env, spec)?;
    let size = bindings.measure_spec_size(env, spec)?;
    // `EXACTLY` and `AT_MOST` both bound the child to the spec size;
    // `UNSPECIFIED` leaves it free.
    Ok(if mode == bindings.measure_spec_unspecified() {
        None
    } else {
        Some(platform.px_to_dp(size))
    })
}

/// A leaf's layout face: measure the platform view, report its dp size.
///
/// Holds the runtime's [`Platform`] by `Rc`: `measure` reads its identifier
/// table and density cache without reaching for a static.
pub struct ViewSubView {
    view: PlatformView,
    platform: Rc<Platform>,
}

impl core::fmt::Debug for ViewSubView {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ViewSubView")
            .field("view", &self.view)
            .finish_non_exhaustive()
    }
}

impl ViewSubView {
    /// Wraps the leaf's platform view; `platform` is the render context's,
    /// shared by `Rc` (`ctx.platform()`).
    pub fn new(view: PlatformView, platform: &Rc<Platform>) -> Self {
        Self {
            view,
            platform: platform.clone(),
        }
    }
}

impl SubView for ViewSubView {
    fn measure(&self, proposal: ProposalSize) -> ViewDimensions {
        let platform = &*self.platform;
        let (width, height) = jvm::with_env(|env| {
            let (width_spec, height_spec) =
                spec_of(env, platform, proposal).expect("MeasureSpec packing is infallible");
            let bindings = platform.bindings();
            bindings
                .measure(env, self.view.as_ref(), width_spec, height_spec)
                .expect("a platform measure must not throw");
            bindings
                .measured_size(env, self.view.as_ref())
                .expect("measured getters are infallible")
        });
        ViewDimensions::new(Size::new(
            platform.px_to_dp(width),
            platform.px_to_dp(height),
        ))
    }

    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::None
    }

    fn priority(&self) -> i32 {
        0
    }
}

/// The empty leaf's layout face: no proposal produces a nonzero size.
#[derive(Debug)]
pub struct EmptySubView;

impl SubView for EmptySubView {
    fn measure(&self, _proposal: ProposalSize) -> ViewDimensions {
        ViewDimensions::new(Size::zero())
    }

    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::None
    }

    fn priority(&self) -> i32 {
        0
    }

    fn is_empty(&self) -> bool {
        true
    }
}

// ============================================================================
// Safe area — the two-region model of `docs/layout-spec.md` §7.1
// ============================================================================
//
// The activity's inset forwarder pushes the window's insets split by
// region — `Platform::safe_area` holds both, in pixels — and every
// `RustViewGroup` turns them into depths inside its own frame at layout
// time. A group's content rect is its local rect shrunk by the deepest
// unmarked region on each edge; an `ignore_safe_area` wrapper carries its
// regions' edge mask in the view's `ignoredSafeAreaMask` field, and every
// descendant accumulates those marks up the `ViewParent` chain.
//
// The mask's bit order mirrors `Edges::mask()` in `cocoa-ui`: bits 0–3
// the container region's edges (top, leading, bottom, trailing), bits 4–7
// the keyboard region's, bit 8 the ignorer mark.

#[cfg(feature = "container")]
/// Bit 0 — the top edge, container region.
const TOP: jint = 1;
#[cfg(feature = "container")]
/// Bit 1 — the leading edge, container region.
const LEADING: jint = 2;
#[cfg(feature = "container")]
/// Bit 2 — the bottom edge, container region.
const BOTTOM: jint = 4;
#[cfg(feature = "container")]
/// Bit 3 — the trailing edge, container region.
const TRAILING: jint = 8;
#[cfg(feature = "container")]
/// The keyboard region's edge mask sits four bits above the container's.
const KEYBOARD_SHIFT: jint = 4;
#[cfg(feature = "container")]
/// Bit 8 — marks a `RustViewGroup` an ignore-safe-area wrapper: it manages
/// its own safe-area contract and extends into the bands it crosses.
pub const IGNORER_MARK: jint = 0x100;

#[cfg(feature = "container")]
/// The mask an `IgnoreSafeArea` declares on its wrapper group.
pub fn declared_mask(ignore: IgnoreSafeArea) -> jint {
    let edges = |enabled: bool| -> jint {
        if !enabled {
            return 0;
        }
        let e = &ignore.edges;
        jint::from(e.top)
            | (jint::from(e.leading) << 1)
            | (jint::from(e.bottom) << 2)
            | (jint::from(e.trailing) << 3)
    };
    edges(ignore.regions.container)
        | (edges(ignore.regions.keyboard) << KEYBOARD_SHIFT)
        | IGNORER_MARK
}

#[cfg(feature = "container")]
/// The accumulated mask a view lives under — its own declared bits OR'd
/// with every ancestor's, collected up the `ViewParent` chain. An
/// `ignore_safe_area` wrapper's marks reach every descendant group.
pub fn accumulated_mask(
    env: &mut Env,
    platform: &Platform,
    view: &JObject,
) -> jni::errors::Result<jint> {
    let bindings = platform.bindings();
    let mut mask = bindings.declared_safe_area_mask(env, view)?;
    let mut ancestor = bindings.parent(env, view)?;
    while let Some(current) = ancestor {
        mask |= bindings.declared_safe_area_mask(env, &current)?;
        ancestor = bindings.parent(env, &current)?;
    }
    Ok(mask)
}

#[cfg(feature = "container")]
/// One safe-area region's depths inside a group, in dp.
#[derive(Clone, Copy, Debug, Default)]
pub struct RegionDepths {
    /// Depth intruding from the left edge.
    pub left: f32,
    /// Depth intruding from the top edge.
    pub top: f32,
    /// Depth intruding from the right edge.
    pub right: f32,
    /// Depth intruding from the bottom edge.
    pub bottom: f32,
}

#[cfg(feature = "container")]
/// The safe-area region depths `group` actually spans, in dp — the
/// window-space depths `Platform::safe_area` carries, clipped to the
/// group's own window-space frame so a mid-window group sees zero.
pub fn region_depths(
    env: &mut Env,
    platform: &Platform,
    group: &JObject,
) -> jni::errors::Result<(RegionDepths, RegionDepths)> {
    let bindings = platform.bindings();
    let [origin_x, origin_y] = bindings.location_in_window(env, group)?;
    let Some(root) = bindings.root_view(env, group)? else {
        return Ok((RegionDepths::default(), RegionDepths::default()));
    };
    let window_width = bindings.width(env, &root)?;
    let window_height = bindings.height(env, &root)?;
    let view_width = bindings.width(env, group)?;
    let view_height = bindings.height(env, group)?;
    let local = |depths: [i32; 4]| -> RegionDepths {
        let [left, top, right, bottom] = depths;
        RegionDepths {
            left: platform.px_to_dp((left - origin_x).clamp(0, view_width)),
            top: platform.px_to_dp((top - origin_y).clamp(0, view_height)),
            right: platform
                .px_to_dp((right - (window_width - origin_x - view_width)).clamp(0, view_width)),
            bottom: platform.px_to_dp(
                (bottom - (window_height - origin_y - view_height)).clamp(0, view_height),
            ),
        }
    };
    let safe_area = platform.safe_area();
    Ok((
        local(safe_area.container.get()),
        local(safe_area.keyboard.get()),
    ))
}

#[cfg(feature = "container")]
/// The depth a group's content must clear on one edge: the deepest region
/// whose edge it does not ignore.
const fn edge_avoided(mask: jint, edge: jint, container: f32, keyboard: f32) -> f32 {
    let mut depth: f32 = 0.0;
    if mask & edge == 0 {
        depth = depth.max(container);
    }
    if mask & (edge << KEYBOARD_SHIFT) == 0 {
        depth = depth.max(keyboard);
    }
    depth
}

#[cfg(feature = "container")]
/// The per-edge depths `mask` leaves to avoid — the max over unmarked
/// regions on each edge.
pub const fn avoided(mask: jint, container: RegionDepths, keyboard: RegionDepths) -> RegionDepths {
    RegionDepths {
        left: edge_avoided(mask, LEADING, container.left, keyboard.left),
        top: edge_avoided(mask, TOP, container.top, keyboard.top),
        right: edge_avoided(mask, TRAILING, container.right, keyboard.right),
        bottom: edge_avoided(mask, BOTTOM, container.bottom, keyboard.bottom),
    }
}

#[cfg(feature = "container")]
/// `local` shrunk by `depths` on each edge — the shape both the safe rect
/// and the band target take.
fn inset_rect(local: Rect, depths: RegionDepths) -> Rect {
    Rect::new(
        Point::new(local.min_x() + depths.left, local.min_y() + depths.top),
        Size::new(
            (local.width() - depths.left - depths.right).max(0.0),
            (local.height() - depths.top - depths.bottom).max(0.0),
        ),
    )
}

#[cfg(feature = "container")]
/// The rect a group's content lays out inside — `local` minus every region
/// `mask` does not mark.
pub fn safe_rect(local: Rect, mask: jint, container: RegionDepths, keyboard: RegionDepths) -> Rect {
    inset_rect(local, avoided(mask, container, keyboard))
}

#[cfg(feature = "container")]
/// The depth a marked child may reach past on one edge — mirroring the
/// Apple backend's `band_target`: the avoided depth when no region on the
/// edge is marked (no extension), else the shallowest unmarked depth — an
/// extension never paints inside a region it does not ignore — or zero
/// when every region is marked.
const fn edge_target(mask: jint, edge: jint, c: f32, k: f32, avoid: f32) -> f32 {
    let marked = mask & edge != 0 || mask & (edge << KEYBOARD_SHIFT) != 0;
    if !marked {
        return avoid;
    }
    let mut gap = f32::INFINITY;
    if mask & edge == 0 {
        gap = gap.min(c);
    }
    if mask & (edge << KEYBOARD_SHIFT) == 0 {
        gap = gap.min(k);
    }
    if gap.is_infinite() { 0.0 } else { gap }
}

#[cfg(feature = "container")]
/// The rect a band-reaching child of a group may extend to, in the group's
/// coordinates. A mask of zero — the default a fill or chrome container
/// carries — reaches the bounds edge everywhere; a marked edge stops at the
/// nearest unmarked zone.
pub fn band_target(
    local: Rect,
    mask: jint,
    container: RegionDepths,
    keyboard: RegionDepths,
    avoided: RegionDepths,
) -> Rect {
    if mask == 0 {
        return local;
    }
    inset_rect(
        local,
        RegionDepths {
            left: edge_target(mask, LEADING, container.left, keyboard.left, avoided.left),
            top: edge_target(mask, TOP, container.top, keyboard.top, avoided.top),
            right: edge_target(
                mask,
                TRAILING,
                container.right,
                keyboard.right,
                avoided.right,
            ),
            bottom: edge_target(
                mask,
                BOTTOM,
                container.bottom,
                keyboard.bottom,
                avoided.bottom,
            ),
        },
    )
}

#[cfg(feature = "container")]
/// The fill-extension tolerance — sub-dp rounding in placed frames.
const EDGE_TOLERANCE: f32 = 0.5;

#[cfg(feature = "container")]
/// Grows `frame` to `target` on every edge where it already touches `safe`
/// — the §7.1 extension rule: a band-reaching child flush against the
/// safe rect's edge paints through to the band target's edge on that side.
pub fn extended_through(frame: Rect, safe: Rect, target: Rect) -> Rect {
    let (mut min_x, mut min_y, mut max_x, mut max_y) =
        (frame.min_x(), frame.min_y(), frame.max_x(), frame.max_y());
    if (frame.min_x() - safe.min_x()).abs() <= EDGE_TOLERANCE {
        min_x = target.min_x();
    }
    if (frame.min_y() - safe.min_y()).abs() <= EDGE_TOLERANCE {
        min_y = target.min_y();
    }
    if (frame.max_x() - safe.max_x()).abs() <= EDGE_TOLERANCE {
        max_x = target.max_x();
    }
    if (frame.max_y() - safe.max_y()).abs() <= EDGE_TOLERANCE {
        max_y = target.max_y();
    }
    Rect::new(
        Point::new(min_x, min_y),
        Size::new((max_x - min_x).max(0.0), (max_y - min_y).max(0.0)),
    )
}
