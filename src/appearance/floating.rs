//! Floating-surface presentation for elevated interactive views.

use waterui_core::{Environment, View};
use waterui_shape::{RoundedRectangle, ShapeExt as _};

use crate::{
    ViewExt as _,
    style::{FloatingStyle, Shadow, Vector},
};

/// A view promoted to the floating interaction layer.
#[derive(Debug)]
pub struct Floating<Content> {
    content: Content,
    style: Option<FloatingStyle>,
}

impl<Content> Floating<Content> {
    /// Creates a floating view that reads its style from the environment.
    #[must_use]
    pub const fn new(content: Content) -> Self {
        Self {
            content,
            style: None,
        }
    }

    /// Creates a floating view with an explicit style.
    #[must_use]
    pub const fn with_style(content: Content, style: FloatingStyle) -> Self {
        Self {
            content,
            style: Some(style),
        }
    }
}

impl<Content> View for Floating<Content>
where
    Content: View,
{
    fn body(self, env: &Environment) -> impl View {
        let style = self
            .style
            .or_else(|| env.get::<FloatingStyle>().cloned())
            .expect("WaterUI `.floating()` requires FloatingStyle theme tokens");
        let shape = RoundedRectangle::new(style.clip_radius);
        let ambient_shadow = Shadow::new(
            style.ambient_shadow_color.clone(),
            Vector::new(0.0, style.ambient_shadow_offset_y),
            style.ambient_shadow_radius,
        );
        let key_shadow = Shadow::new(
            style.key_shadow_color.clone(),
            Vector::new(0.0, style.key_shadow_offset_y),
            style.key_shadow_radius,
        );

        self.content
            .install(style.clone())
            .background(shape.fill(style.container_color))
            .clip(shape)
            .shadow(ambient_shadow)
            .shadow(key_shadow)
    }
}
