use waterui::animation::Animation;

use crate::reactive::WuiWatcherMetadata;

use crate::IntoFFI;

/// FFI-safe representation of an animation.
///
/// cbindgen generates a tagged union with:
/// - `WuiAnimation_Tag` enum for variant discrimination
/// - Body structs for each variant with data
/// - `WuiAnimation` struct with tag field and anonymous union
#[repr(C)]
#[derive(Debug)]
pub enum WuiAnimation {
    /// No animation - changes apply immediately
    None,
    /// Timed cubic bezier animation with control points
    ///
    /// Native backends can use these control points with:
    /// - Apple: `CAMediaTimingFunction(controlPoints:)`
    /// - Android: `PathInterpolator(x1, y1, x2, y2)`
    Bezier {
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
            Self::Default => WuiAnimation::Bezier {
                duration_ms: 250,
                x1: 0.42,
                y1: 0.0,
                x2: 0.58,
                y2: 1.0,
            },
            Self::Bezier {
                duration,
                x1,
                y1,
                x2,
                y2,
            } => WuiAnimation::Bezier {
                duration_ms: u64::try_from(duration.as_millis())
                    .expect("Animation duration exceeds u64::MAX milliseconds"),
                x1,
                y1,
                x2,
                y2,
            },
            Self::Spring { stiffness, damping } => WuiAnimation::Spring { stiffness, damping },
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
            .map_or(WuiAnimation::None, IntoFFI::into_ffi)
    }
}
