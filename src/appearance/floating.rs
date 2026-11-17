//! Floating-surface presentation for elevated interactive views.

use nami::binding;
use waterui_core::{Dynamic, Environment, View, interaction::InteractionState, plugin::Plugin};
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
        // The surface rises and settles with the control on it, so its shadows
        // follow the control's interaction state. They are cast by a layer of
        // their own beneath the clipped surface: clipping the surface must not
        // cut them off, and a state change rebuilds only that layer.
        let state = binding(InteractionState::empty());
        let container = style.container_color.clone();
        let elevation = style.elevation.clone();
        let shadows = Dynamic::watch(state.clone(), move |state| {
            let elevation = elevation.resolve(state);
            shape
                .fill(container.clone())
                .shadow(Shadow::new(
                    elevation.ambient_shadow_color.clone(),
                    Vector::new(0.0, elevation.ambient_shadow_offset_y),
                    elevation.ambient_shadow_radius,
                    shape,
                ))
                .shadow(Shadow::new(
                    elevation.key_shadow_color.clone(),
                    Vector::new(0.0, elevation.key_shadow_offset_y),
                    elevation.key_shadow_radius,
                    shape,
                ))
        });

        self.content
            .install(FloatingScope(style.clone()))
            .interaction_state(&state)
            .background(shape.fill(style.container_color))
            .clip(shape)
            .background(shadows)
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::vec::Vec;

    use nami::Binding;
    use waterui_core::{
        AnyView, Dynamic, Environment, Retain, View,
        backend::{peel_metadata, peel_native},
        interaction::{InteractionReport, InteractionState, StateValue},
    };
    use waterui_layout::container::FixedContainer;
    use waterui_shape::{ClipShape, ShapeKind};
    use waterui_text::Text;

    use super::{Floating, FloatingStyle};
    use crate::style::{FloatingElevation, Shadow};

    /// The children of a fixed container, read from its erased form: a typed
    /// `FixedContainer` expands to a native container of erased children.
    fn container_children(container: impl View, env: &Environment, role: &str) -> Vec<AnyView> {
        peel_native::<FixedContainer>(container, env)
            .unwrap_or_else(|| panic!("{role} resolves to a fixed container"))
            .into_inner()
            .1
    }

    /// Explodes a `Floating` body into the pieces a backend sees: the binding
    /// `.interaction_state` asks the backend to report into, the surface's clip
    /// silhouette, and every shadow layer the background `Dynamic` produces.
    fn floating_parts(
        style: FloatingStyle,
    ) -> (
        Binding<InteractionState>,
        ShapeKind,
        Rc<RefCell<Vec<AnyView>>>,
        Retain,
    ) {
        let env = Environment::new();
        let surface = Floating::with_style(Text::new("label"), style);
        let mut children = container_children(surface, &env, "a background modifier").into_iter();
        let shadows = children.next().expect("the shadow layer is the background");
        let clipped = children.next().expect("the clipped surface is the content");

        let clip =
            peel_metadata::<ClipShape>(clipped, &env).expect("the surface is clipped to its shape");
        let clip_kind = clip.value.kind();

        // Inside the clip is the fill background over the content; the
        // environment modifier `.interaction_state` installs carries the
        // InteractionReport plugin.
        let mut inner_children =
            container_children(clip.content, &env, "the surface fill").into_iter();
        let _fill = inner_children.next();
        let env_meta =
            peel_metadata::<Environment>(inner_children.next().expect("the styled content"), &env)
                .expect(".interaction_state installs an environment");
        let report = env_meta
            .value
            .get::<InteractionReport>()
            .expect(".interaction_state must install an InteractionReport")
            .0
            .clone();

        // The background is a watched Dynamic; unroll one step to reach it,
        // then connect a receiver as a renderer would. The Retain keeps the
        // watcher guard alive, so it is handed back to the caller.
        let retained =
            peel_metadata::<Retain>(shadows, &env).expect("the shadow layer is a watched Dynamic");
        let dynamic = peel_native::<Dynamic>(retained.content, &env)
            .expect("the retained view is the Dynamic");
        let layers = Rc::new(RefCell::new(Vec::new()));
        let sink = Rc::clone(&layers);
        dynamic.connect(move |ctx| sink.borrow_mut().push(ctx.into_value()));

        (report, clip_kind, layers, retained.value)
    }

    /// The ambient and key shadow radii of one shadow layer, innermost first.
    fn shadow_radii(layer: AnyView) -> (f32, f32) {
        let (key, ambient) = shadow_pair(layer);
        (ambient.radius(), key.radius())
    }

    /// The key shadow of one shadow layer and the ambient shadow inside it.
    fn shadow_pair(layer: AnyView) -> (Shadow, Shadow) {
        let env = Environment::new();
        let key = peel_metadata::<Shadow>(layer, &env)
            .expect("the outermost shadow layer is the key shadow");
        let ambient = peel_metadata::<Shadow>(key.content, &env)
            .expect("inside the key shadow is the ambient shadow");
        (key.value, ambient.value)
    }

    /// A floating surface fills, clips, and shadows with one shape, so each
    /// shadow's silhouette must carry the clip's `ShapeKind` — a normalized
    /// `clip_radius` can no longer be read back as an absolute shadow radius.
    #[test]
    fn shadows_carry_the_clip_silhouette() {
        let (_report, clip_kind, layers, _live) = floating_parts(FloatingStyle {
            clip_radius: 0.4,
            ..FloatingStyle::default()
        });
        assert_eq!(
            clip_kind,
            ShapeKind::RoundedRect { corner_radius: 0.4 },
            "Floating clips with RoundedRectangle::new(clip_radius)"
        );
        let layer = layers
            .take()
            .into_iter()
            .next()
            .expect("connecting delivers the resting shadow layer");
        let (key, ambient) = shadow_pair(layer);
        for (name, kind) in [
            ("key", key.silhouette().kind()),
            ("ambient", ambient.silhouette().kind()),
        ] {
            assert_eq!(
                kind, clip_kind,
                "the {name} shadow's silhouette must be the clip's shape"
            );
        }
    }

    /// The shadow layer is driven by the state `.interaction_state` reports:
    /// writing HOVERED into the binding rebuilds it at the hovered elevation.
    #[test]
    fn shadow_layer_follows_the_reported_state() {
        let mut style = FloatingStyle::default();
        let resting = style.elevation.resting().clone();
        style.elevation = StateValue::new(resting.clone()).when(
            InteractionState::HOVERED,
            FloatingElevation {
                ambient_shadow_radius: resting.ambient_shadow_radius * 2.0,
                key_shadow_radius: resting.key_shadow_radius * 2.0,
                ..resting
            },
        );
        let (report, _clip, layers, _live) = floating_parts(style);

        let resting = layers.take();
        assert_eq!(resting.len(), 1, "connect delivers the resting layer");
        let (ambient_rest, key_rest) = shadow_radii(resting.into_iter().next().expect("one layer"));

        report.set(InteractionState::HOVERED);

        let hovered = layers.take();
        assert_eq!(hovered.len(), 1, "a reported state rebuilds the layer");
        let (ambient_hover, key_hover) =
            shadow_radii(hovered.into_iter().next().expect("one layer"));
        assert!(
            ambient_hover > ambient_rest && key_hover > key_rest,
            "a reported HOVERED state must raise the shadows: \
             ({ambient_rest}, {key_rest}) -> ({ambient_hover}, {key_hover})"
        );
    }
}
