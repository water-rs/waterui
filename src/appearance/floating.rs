//! Floating-surface presentation for elevated interactive views.

use waterui_core::{Environment, View, plugin::Plugin};
use waterui_shape::{RoundedRectangle, ShapeExt as _};

use crate::{
    ViewExt as _,
    style::{FloatingStyle, Shadow, Vector},
};

/// Marks the content of a [`Floating`] surface, carrying the style that surface
/// resolved.
///
/// This is what tells a descendant that it is *inside* an elevated surface, as
/// opposed to merely being in an app whose theme defines what such a surface
/// would look like. A [`FloatingStyle`] in the environment answers the second
/// question only: it is the ambient token set a bare
/// [`floating`](crate::ViewExt::floating) resolves against, and every themed app
/// has one. A backend that treats its presence as "I am inside a floating
/// surface" concludes that of every view in the app — which is exactly how
/// Material buttons lost their containers.
///
/// Backends should read this instead. It answers both questions at once: whether
/// there is an enclosing floating surface, and with which style.
#[derive(Debug, Clone)]
pub struct FloatingScope(pub FloatingStyle);

impl Plugin for FloatingScope {}

/// A view promoted to the floating interaction layer.
#[derive(Debug)]
pub struct Floating<Content> {
    content: Content,
    style: Option<FloatingStyle>,
}

impl<Content: View> Floating<Content> {
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
        // A theme that styles floating surfaces supplies these tokens. The
        // framework default is itself expressed in theme tokens (`Surface`,
        // `Accent`, a 44pt minimum target), so a backend that installs no
        // floating tokens still gets a correct surface rather than a panic.
        let style = self
            .style
            .or_else(|| env.get::<FloatingStyle>().cloned())
            .unwrap_or_default();
        let shape = RoundedRectangle::new(style.clip_radius);
        let ambient_shadow = Shadow::new(
            style.ambient_shadow_color.clone(),
            Vector::new(0.0, style.ambient_shadow_offset_y),
            style.ambient_shadow_radius,
            shape,
        );
        let key_shadow = Shadow::new(
            style.key_shadow_color.clone(),
            Vector::new(0.0, style.key_shadow_offset_y),
            style.key_shadow_radius,
            shape,
        );

        self.content
            .install(FloatingScope(style.clone()))
            .background(shape.fill(style.container_color))
            .clip(shape)
            .shadow(ambient_shadow)
            .shadow(key_shadow)
    }
}

#[cfg(test)]
mod tests {
    use waterui_core::{AnyView, Environment, Metadata, View};
    use waterui_shape::{ClipShape, ShapeKind};
    use waterui_text::Text;

    use super::{Floating, FloatingStyle};
    use crate::style::Shadow;

    /// A floating surface fills, clips, and shadows with one shape, so each
    /// shadow's silhouette must carry the clip's `ShapeKind` — a normalized
    /// `clip_radius` can no longer be read back as an absolute shadow radius.
    #[test]
    fn shadows_carry_the_clip_silhouette() {
        let style = FloatingStyle {
            clip_radius: 0.4,
            ..FloatingStyle::default()
        };
        let view = Floating::with_style(Text::new("label"), style).body(&Environment::new());
        let key = *AnyView::new(view)
            .downcast::<Metadata<Shadow>>()
            .expect("the outermost floating layer is the key shadow");
        let key_kind = key.value.silhouette.kind();
        let ambient = *key
            .content
            .downcast::<Metadata<Shadow>>()
            .expect("inside the key shadow is the ambient shadow");
        let ambient_kind = ambient.value.silhouette.kind();
        let clip = *ambient
            .content
            .downcast::<Metadata<ClipShape>>()
            .expect("inside the ambient shadow is the clip shape");
        let clip_kind = clip.value.kind();
        assert!(
            matches!(clip_kind, ShapeKind::RoundedRect { corner_radius } if corner_radius == 0.4),
            "Floating clips with RoundedRectangle::new(clip_radius), got {clip_kind:?}"
        );
        for (name, kind) in [("key", key_kind), ("ambient", ambient_kind)] {
            assert_eq!(
                format!("{kind:?}"),
                format!("{clip_kind:?}"),
                "the {name} shadow's silhouette must be the clip's shape"
            );
        }
    }
}
