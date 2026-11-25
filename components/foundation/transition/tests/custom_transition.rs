//! Integration tests verifying `TransitionProperties` can be constructed and
//! returned by a custom `Transition` outside `waterui-structural-transition`.

use core::time::Duration;

use waterui_core::animation::Animation;
use waterui_core::layout::{LayoutDirection, Size};
use waterui_structural_transition::{
    PropertyTransition, Transition, TransitionGeometry, TransitionPhase, TransitionProperties,
    TransitionSpec,
};

struct Rise {
    distance: f32,
}

impl Transition for Rise {
    fn properties(
        &self,
        _phase: TransitionPhase,
        progress: f32,
        _geometry: TransitionGeometry,
    ) -> TransitionProperties {
        TransitionProperties {
            translation: [0.0, self.distance * progress],
            ..TransitionProperties::IDENTITY
        }
    }
}

fn near(actual: f32, expected: f32) {
    assert!((actual - expected).abs() < 0.001, "{actual} != {expected}");
}

const fn geometry() -> TransitionGeometry {
    TransitionGeometry::new(Size::new(200.0, 80.0), LayoutDirection::LeftToRight)
}

#[test]
fn custom_translation_only_transition_returns_non_identity_properties() {
    let spec = TransitionSpec::new(Rise { distance: 24.0 })
        .with_animation(Animation::linear(Duration::from_millis(100)));

    let mut incoming = spec.start(TransitionPhase::Insertion, false);
    near(incoming.properties(geometry()).translation[1], 24.0);
    incoming.advance(Duration::from_millis(50));
    let mid = incoming.properties(geometry());
    near(mid.opacity, 1.0);
    near(mid.scale[0], 1.0);
    near(mid.scale[1], 1.0);
    near(mid.translation[0], 0.0);
    near(mid.translation[1], 12.0);
    near(mid.rotation, 0.0);
    near(mid.blur, 0.0);

    let mut outgoing = spec.start(TransitionPhase::Removal, false);
    outgoing.advance(Duration::from_millis(75));
    let out = outgoing.properties(geometry());
    near(out.translation[0], 0.0);
    near(out.translation[1], 18.0);
    near(out.opacity, 1.0);
}

#[test]
fn property_transition_offset_translates_from_outside_crate() {
    let spec = TransitionSpec::new(PropertyTransition::offset(-30.0, 16.0))
        .with_animation(Animation::linear(Duration::from_millis(100)));
    let mut state = spec.start(TransitionPhase::Insertion, false);
    state.advance(Duration::from_millis(25));
    let properties = state.properties(geometry());
    near(properties.translation[0], -22.5);
    near(properties.translation[1], 12.0);
    near(properties.opacity, 1.0);
    near(properties.scale[0], 1.0);
    near(properties.scale[1], 1.0);
    near(properties.rotation, 0.0);
    near(properties.blur, 0.0);
}
