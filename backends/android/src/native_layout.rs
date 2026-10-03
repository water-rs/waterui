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
use jni::sys::jint;
use waterui_core::layout::{ProposalSize, Size, StretchAxis, SubView, ViewDimensions};

use crate::contract::PlatformView;
use crate::jvm::{self, Platform};

/// `View.MeasureSpec` modes, as `MeasureSpec` encodes them.
const MODE_UNSPECIFIED: jint = 0;
const MODE_AT_MOST: jint = 0x8000_0000u32.cast_signed();

/// The mode a proposal axis carries: a bound when the parent offered one,
/// free when it did not.
const fn mode_of(axis: Option<f32>) -> jint {
    if axis.is_some() {
        MODE_AT_MOST
    } else {
        MODE_UNSPECIFIED
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
    platform
        .bindings()
        .make_measure_spec(env, px, mode_of(axis))
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
    let mode = platform.bindings().measure_spec_mode(env, spec)?;
    let size = platform.bindings().measure_spec_size(env, spec)?;
    Ok(match mode {
        MODE_UNSPECIFIED => None,
        _ => Some(platform.px_to_dp(size)),
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
