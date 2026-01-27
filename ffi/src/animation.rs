use waterui::animation::Animation;

use crate::reactive::WuiWatcherMetadata;

use super::IntoFFI;

/// FFI-safe representation of an animation.
///
/// cbindgen generates a tagged union with:
/// - `WuiAnimation_Tag` enum for variant discrimination
/// - Body structs for each variant with data
/// - `WuiAnimation` struct with tag field and anonymous union
#[repr(C)]
pub enum WuiAnimation {
    /// No animation - changes apply immediately
    None,
    /// Default animation (0.25s ease-in-out)
    Default,
    /// Linear animation with constant velocity
    Linear {
        /// Duration in milliseconds
        duration_ms: u64,
    },
    /// Ease-in animation that starts slow and accelerates
    EaseIn {
        /// Duration in milliseconds
        duration_ms: u64,
    },
    /// Ease-out animation that starts fast and decelerates
    EaseOut {
        /// Duration in milliseconds
        duration_ms: u64,
    },
    /// Ease-in-out animation that starts and ends slowly
    EaseInOut {
        /// Duration in milliseconds
        duration_ms: u64,
    },
    /// Custom cubic bezier animation with control points
    ///
    /// Native backends can use these control points with:
    /// - Apple: `CAMediaTimingFunction(controlPoints:)`
    /// - Android: `PathInterpolator(x1, y1, x2, y2)`
    CubicBezier {
        /// Duration in milliseconds
        duration_ms: u64,
        /// First control point X (0.0 to 1.0)
        x1: f32,
        /// First control point Y
        y1: f32,
        /// Second control point X (0.0 to 1.0)
        x2: f32,
        /// Second control point Y
        y2: f32,
    },
    /// Spring animation with physics-based movement
    Spring {
        /// Stiffness of the spring (higher = faster)
        stiffness: f32,
        /// Damping factor (higher = less bounce)
        damping: f32,
    },
}

impl IntoFFI for Animation {
    type FFI = WuiAnimation;

    fn into_ffi(self) -> Self::FFI {
        match self {
            Animation::Default => WuiAnimation::Default,
            Animation::Linear(d) => WuiAnimation::Linear {
                duration_ms: d.as_millis() as u64,
            },
            Animation::EaseIn(d) => WuiAnimation::EaseIn {
                duration_ms: d.as_millis() as u64,
            },
            Animation::EaseOut(d) => WuiAnimation::EaseOut {
                duration_ms: d.as_millis() as u64,
            },
            Animation::EaseInOut(d) => WuiAnimation::EaseInOut {
                duration_ms: d.as_millis() as u64,
            },
            Animation::CubicBezier {
                duration,
                x1,
                y1,
                x2,
                y2,
            } => WuiAnimation::CubicBezier {
                duration_ms: duration.as_millis() as u64,
                x1,
                y1,
                x2,
                y2,
            },
            Animation::Spring { stiffness, damping } => WuiAnimation::Spring { stiffness, damping },
        }
    }
}

/// Extracts animation metadata from a watcher context.
///
/// # Safety
/// The metadata pointer must be valid and point to a properly initialized metadata object.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_get_animation(
    metadata: *const WuiWatcherMetadata,
) -> WuiAnimation {
    unsafe {
        (*metadata)
            .try_get::<Animation>()
            .map(IntoFFI::into_ffi)
            .unwrap_or(WuiAnimation::None)
    }
}
