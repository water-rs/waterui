use waterui::animation::Animation;

use crate::reactive::WuiWatcherMetadata;

use crate::IntoFFI;

/// FFI-safe representation of a `WaterUI` animation.
///
/// cbindgen generates a tagged union with:
/// - `WuiAnimation_Tag` enum for variant discrimination
/// - Body structs for each variant with data
/// - `WuiAnimation` struct with tag field and anonymous union
#[repr(C)]
#[derive(Debug)]
pub enum WuiAnimation {
    /// The platform's default animation for the change.
    SystemDefault,
    /// Timed cubic bezier curve with control points
    ///
    /// Native backends can use these control points with:
    /// - Apple: `CAMediaTimingFunction(controlPoints:)`
    /// - Android: `PathInterpolator(x1, y1, x2, y2)`
    Curve {
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
        /// The spring stiffness; higher values animate faster
        stiffness: f32,
        /// The damping coefficient; higher values reduce bouncing
        damping: f32,
    },
}

#[allow(clippy::cast_possible_truncation)]
impl IntoFFI for Animation {
    type FFI = WuiAnimation;

    fn into_ffi(self) -> Self::FFI {
        match self {
            Self::Default => WuiAnimation::SystemDefault,
            Self::Bezier {
                duration,
                x1,
                y1,
                x2,
                y2,
            } => WuiAnimation::Curve {
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
    // SAFETY: the caller contract requires `metadata` to be a valid handle alive for
    // this call; it is only borrowed.
    unsafe {
        (*metadata)
            .try_get::<Animation>()
            .map_or(WuiAnimation::SystemDefault, IntoFFI::into_ffi)
    }
}
