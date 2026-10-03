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

use jni::Env;
use jni::sys::jint;
use waterui_core::layout::{
    ProposalSize, Size, StretchAxis, SubView, ViewDimensions,
};

use crate::contract::PlatformView;
use crate::jvm::{self, globals};

/// `View.MeasureSpec` modes, as `MeasureSpec` encodes them.
const MODE_UNSPECIFIED: jint = 0;
const MODE_EXACTLY: jint = 0x4000_0000u32 as jint;
const MODE_AT_MOST: jint = -0x8000_0000u32 as jint;

/// The mode a proposal axis carries: a bound when the parent offered one,
/// free when it did not.
fn mode_of(axis: Option<f32>) -> jint {
    if axis.is_some() {
        MODE_AT_MOST
    } else {
        MODE_UNSPECIFIED
    }
}

/// A `MeasureSpec` for `proposal`: `Some(dp)` → `AT_MOST(dp as px)`,
/// `None` → `UNSPECIFIED`.
fn spec_of(env: &mut Env, proposal: ProposalSize) -> jni::errors::Result<(jint, jint)> {
    let width = make_spec(env, proposal.width)?;
    let height = make_spec(env, proposal.height)?;
    Ok((width, height))
}

fn make_spec(env: &mut Env, axis: Option<f32>) -> jni::errors::Result<jint> {
    let px = axis.map_or(0, jvm::dp_to_px);
    globals()
        .bindings()
        .make_measure_spec(env, px, mode_of(axis))
}

/// Reads a spec pair Kotlin packed into a `jlong`: width in the high word,
/// height in the low word.
pub(crate) fn unpack_specs(packed: i64) -> (jint, jint) {
    let width = (packed >> 32) as jint;
    let height = (packed & 0xffff_ffff) as jint;
    (width, height)
}

/// Packs a measured pixel size the way `nativeMeasure` returns it to Kotlin.
pub(crate) fn pack_measured(width: i32, height: i32) -> i64 {
    (i64::from(width) << 32) | (i64::from(height) & 0xffff_ffff)
}

/// Reads a `MeasureSpec` into the `ProposalSize` axis it offers: `EXACTLY`
/// and `AT_MOST` both bound the child to the spec size; `UNSPECIFIED` leaves
/// it free.
pub(crate) fn spec_axis_to_proposal(env: &mut Env, spec: jint) -> jni::errors::Result<Option<f32>> {
    let mode = globals().bindings().measure_spec_mode(env, spec)?;
    let size = globals().bindings().measure_spec_size(env, spec)?;
    Ok(match mode {
        MODE_UNSPECIFIED => None,
        _ => Some(jvm::px_to_dp(size)),
    })
}

/// A leaf's layout face: measure the platform view, report its dp size.
#[derive(Debug)]
pub(crate) struct ViewSubView {
    view: PlatformView,
}

impl ViewSubView {
    /// Wraps the leaf's platform view.
    pub(crate) fn new(view: PlatformView) -> Self {
        Self { view }
    }
}

impl SubView for ViewSubView {
    fn measure(&self, proposal: ProposalSize) -> ViewDimensions {
        let (width, height) = jvm::with_env(|env| {
            let (width_spec, height_spec) =
                spec_of(env, proposal).expect("MeasureSpec packing is infallible");
            let bindings = globals().bindings();
            bindings
                .measure(env, self.view.as_ref(), width_spec, height_spec)
                .expect("a platform measure must not throw");
            bindings
                .measured_size(env, self.view.as_ref())
                .expect("measured getters are infallible")
        });
        ViewDimensions::new(Size::new(
            jvm::px_to_dp(width),
            jvm::px_to_dp(height),
        ))
    }

    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::None
    }
}

/// The empty leaf's layout face: no proposal produces a nonzero size.
#[derive(Debug)]
pub(crate) struct EmptySubView;

impl SubView for EmptySubView {
    fn measure(&self, _proposal: ProposalSize) -> ViewDimensions {
        ViewDimensions::new(Size::zero())
    }

    fn is_empty(&self) -> bool {
        true
    }
}
