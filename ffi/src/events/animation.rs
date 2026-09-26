use waterui::animation::Animation;

use crate::reactive::WuiWatcherMetadata;

use crate::IntoFFI;

/// FFI-safe representation of a Cherenkov animation.
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
        /// The oscillation period in seconds
        response: f32,
        /// The damping ratio; 1 is critically damped
        damping: f32,
    },
    /// Momentum decay from an initial velocity
    Decay {
        /// Initial velocity along X, logical pixels per second
        velocity_x: f32,
        /// Initial velocity along Y, logical pixels per second
        velocity_y: f32,
        /// Exponential deceleration constant, per second
        deceleration: f32,
    },
}

#[allow(clippy::cast_possible_truncation)]
impl IntoFFI for Animation {
    type FFI = WuiAnimation;

    fn into_ffi(self) -> Self::FFI {
        match self {
            Self::Curve(curve) => WuiAnimation::Curve {
                duration_ms: u64::try_from(curve.duration.as_millis())
                    .expect("Animation duration exceeds u64::MAX milliseconds"),
                x1: curve.p1.x as f32,
                y1: curve.p1.y as f32,
                x2: curve.p2.x as f32,
                y2: curve.p2.y as f32,
            },
            Self::Spring(spring) => WuiAnimation::Spring {
                response: spring.response as f32,
                damping: spring.damping as f32,
            },
            Self::Decay(decay) => WuiAnimation::Decay {
                velocity_x: decay.velocity.x as f32,
                velocity_y: decay.velocity.y as f32,
                deceleration: decay.deceleration as f32,
            },
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
            .map_or(WuiAnimation::None, IntoFFI::into_ffi)
    }
}
