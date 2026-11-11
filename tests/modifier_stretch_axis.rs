//! Environment-modifying wrappers are transparent for layout: each reports its
//! content's stretch axis, which callers read before `body` runs.

use waterui::interaction::InteractionState;
use waterui::layout::StretchAxis;
use waterui::prelude::*;

fn spacer_axis() -> StretchAxis {
    let axis = spacer().stretch_axis();
    assert_ne!(axis, StretchAxis::None, "a spacer stretches");
    axis
}

#[test]
fn disabled_keeps_content_stretch_axis() {
    assert_eq!(spacer().disabled(true).stretch_axis(), spacer_axis());
}

#[test]
fn foreground_keeps_content_stretch_axis() {
    assert_eq!(
        spacer().foreground(Color::red()).stretch_axis(),
        spacer_axis()
    );
}

#[test]
fn interaction_state_keeps_content_stretch_axis() {
    let state = Binding::container(InteractionState::empty());
    assert_eq!(
        spacer().interaction_state(&state).stretch_axis(),
        spacer_axis()
    );
}
