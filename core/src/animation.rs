//! Animation: Cherenkov's animation types, carried in nami change metadata.
//!
//! A change carries its animation as metadata on the signal: `value.with(
//! Animation::Spring(Spring::smooth()))` or `value.animated()`. Whoever
//! applies the change — a layer property, a filter parameter, a native
//! backend's property — reads the metadata and interpolates toward the new
//! value on its own clock.

pub use cherenkov::{Animation, Curve, Decay, Spring};
use nami::signal::WithMetadata;

/// Adds [`Animation`] metadata to a signal.
///
/// The one animation extension trait in the framework: `waterui` re-exports
/// it from its prelude, so `.animated()` means the same thing everywhere.
pub trait AnimationExt: nami::SignalExt {
    /// Animates changes with the framework default, a smooth spring.
    ///
    /// Equivalent to `self.with(Animation::Spring(Spring::smooth()))`; to pick
    /// another animation use [`nami::SignalExt::with`] directly:
    /// `value.with(Animation::Curve(Curve::ease_out(duration)))`.
    #[track_caller]
    fn animated(&self) -> WithMetadata<Self, Animation> {
        self.with(Animation::Spring(Spring::smooth()))
    }
}

impl<S: nami::SignalExt> AnimationExt for S {}

#[cfg(test)]
mod tests {
    use super::*;
    use nami::{Signal, binding};

    #[test]
    fn animated_carries_the_default_spring_in_metadata() {
        let value = binding(0.0_f32);
        let seen = std::rc::Rc::new(std::cell::Cell::new(None));
        let sink = seen.clone();
        let _guard = value.animated().watch(move |context| {
            sink.set(context.metadata().try_get::<Animation>());
        });
        value.set(1.0);
        assert_eq!(seen.get(), Some(Animation::Spring(Spring::smooth())));
    }
}
