//! End-to-end accessibility-semantics tests for the `particle` component.

use waterui::ViewExt as _;
use waterui::accessibility::AccessibilityRole;
use waterui::graphics::color::Srgb;
use waterui_particle::ParticleSystem;
use waterui_testing::{Role, SemanticApp};

fn particle_system_view() -> impl waterui::View {
    ParticleSystem::new(128)
        .emit_from_rect(0.4, 0.2)
        .at(0.5, 0.2)
        .rate(48.0)
        .life(0.6, 1.0)
        .speed(0.15, 0.25)
        .angle(1.3, 1.8)
        .size(0.015, 0.03)
        .color(Srgb::WHITE, Srgb::new(1.0, 0.6, 0.1))
        .width(180.0)
        .height(120.0)
        .a11y_role(AccessibilityRole::Image)
        .a11y_label("Particle system")
}

#[waterui::test(particle_system_view)]
fn particle_system_exposes_accessibility_image(app: &mut SemanticApp) {
    app.query()
        .role(Role::IMAGE)
        .label("Particle system")
        .assert_exists();
}
