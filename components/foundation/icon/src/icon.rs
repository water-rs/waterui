//! Unified [`Icon`] view and shared [`IconSize`] environment channel.

use waterui_accessibility::{
    AccessibilityChildren, AccessibilityHidden, AccessibilityLabel, AccessibilityRole, default_role,
};
use waterui_core::plugin::Plugin;
use waterui_core::{Computed, Environment, IntoSignalF32, Metadata, OneOf2, SignalExt as _, View};
#[cfg(feature = "svg")]
use waterui_core::{OneOf3, Str};
#[cfg(feature = "svg")]
use waterui_layout::frame::Frame;
#[cfg(feature = "svg")]
use waterui_svg::Svg;

use crate::IconGlyph;

/// Hides an unnamed icon's internal glyph/SVG geometry from assistive technology
/// (decorative by default), and exposes a single [`AccessibilityRole::Image`]
/// leaf with suppressed internal descendants when an explicit `.a11y_label(...)`
/// is present in `env`.
pub fn wrap_icon_accessibility<V: View>(view: V, env: &Environment) -> impl View {
    if env.get::<AccessibilityLabel>().is_some() {
        OneOf2::A(default_role(
            env,
            Metadata::new(view, AccessibilityChildren::ExcludeDescendants),
            AccessibilityRole::Image,
        ))
    } else {
        OneOf2::B(Metadata::new(view, AccessibilityHidden::new(true)))
    }
}

/// Default logical icon size in points when no [`IconSize`] plugin is installed.
pub const DEFAULT_ICON_SIZE: f32 = 24.0;

/// Shared icon-size channel installed in the [`Environment`].
///
/// Icons resolve their size from the nearest enclosing [`IconSize`] in the
/// environment, falling back to [`DEFAULT_ICON_SIZE`] (24 pt) when none is
/// installed.
#[derive(Debug, Clone)]
pub struct IconSize(Computed<f32>);

impl IconSize {
    /// Creates an [`IconSize`] channel from a constant or reactive `f32` signal.
    #[must_use]
    pub fn new(size: impl IntoSignalF32) -> Self {
        Self(size.into_signal_f32().computed())
    }

    /// Returns the reactive icon size signal in points.
    #[must_use]
    pub const fn size(&self) -> &Computed<f32> {
        &self.0
    }
}

impl Default for IconSize {
    fn default() -> Self {
        Self::new(DEFAULT_ICON_SIZE)
    }
}

impl Plugin for IconSize {}

#[derive(Debug, Clone)]
enum IconBacking {
    Glyph(IconGlyph),
    #[cfg(feature = "svg")]
    Svg {
        svg: Svg,
        viewbox: Option<(f32, f32)>,
    },
}

/// A portable UI icon returned by icon pack crates.
///
/// Every icon pack returns `Icon` from its `snake_case` constructor functions;
/// whether an icon draws from an SVG path or a webfont [`IconGlyph`] is
/// controlled by pack Cargo features rather than by distinct public types.
///
/// Colour comes from `.foreground(...)`, and size comes from [`IconSize`]
/// (or `.icon_size(...)`).
#[derive(Debug, Clone)]
#[must_use = "an `Icon` does nothing unless it is rendered as part of a view"]
pub struct Icon {
    backing: IconBacking,
}

impl Icon {
    /// Creates an [`Icon`] backed by a webfont [`IconGlyph`].
    pub const fn from_glyph(glyph: IconGlyph) -> Self {
        Self {
            backing: IconBacking::Glyph(glyph),
        }
    }

    /// Creates an [`Icon`] wrapping an [`Svg`].
    #[cfg(feature = "svg")]
    pub const fn from_svg(svg: Svg) -> Self {
        Self {
            backing: IconBacking::Svg { svg, viewbox: None },
        }
    }

    /// Creates an [`Icon`] from filled SVG path data and its viewbox dimensions.
    #[cfg(feature = "svg")]
    pub fn from_path(path_data: impl Into<Str>, width: f32, height: f32) -> Self {
        Self {
            backing: IconBacking::Svg {
                svg: Svg::from_path(path_data, width, height),
                viewbox: Some((width, height)),
            },
        }
    }

    /// Renders a path-backed [`Icon`] as strokes instead of filled shapes.
    #[cfg(feature = "svg")]
    pub fn stroked(self) -> Self {
        Self {
            backing: match self.backing {
                IconBacking::Glyph(glyph) => IconBacking::Glyph(glyph),
                IconBacking::Svg { svg, viewbox } => IconBacking::Svg {
                    svg: svg.stroked(),
                    viewbox,
                },
            },
        }
    }
}

impl From<IconGlyph> for Icon {
    fn from(glyph: IconGlyph) -> Self {
        Self::from_glyph(glyph)
    }
}

#[cfg(feature = "svg")]
impl From<Svg> for Icon {
    fn from(svg: Svg) -> Self {
        Self::from_svg(svg)
    }
}

#[cfg(feature = "svg")]
fn fit_viewbox(viewbox: Option<(f32, f32)>, size: f32) -> (f32, f32) {
    let Some((vw, vh)) = viewbox else {
        return (size, size);
    };
    let longest = vw.max(vh);
    if longest <= 0.0 || !longest.is_finite() {
        return (size, size);
    }
    let scale = size / longest;
    (vw * scale, vh * scale)
}

#[cfg(feature = "svg")]
impl View for Icon {
    fn body(self, env: &Environment) -> impl View {
        match self.backing {
            IconBacking::Glyph(glyph) => OneOf2::A(glyph),
            IconBacking::Svg { svg, viewbox } => {
                let svg_view = if let Some(icon_size) = env.get::<IconSize>() {
                    let width = icon_size.size().map(move |s| fit_viewbox(viewbox, s).0);
                    let height = icon_size.size().map(move |s| fit_viewbox(viewbox, s).1);
                    OneOf3::A(Frame::new(svg).width(width).height(height))
                } else if let Some((vw, vh)) = viewbox
                    && (vw > DEFAULT_ICON_SIZE || vh > DEFAULT_ICON_SIZE)
                {
                    let (w, h) = fit_viewbox(Some((vw, vh)), DEFAULT_ICON_SIZE);
                    OneOf3::B(Frame::new(svg).ideal_width(w).ideal_height(h))
                } else {
                    OneOf3::C(svg)
                };
                OneOf2::B(wrap_icon_accessibility(svg_view, env))
            }
        }
    }
}

#[cfg(not(feature = "svg"))]
impl View for Icon {
    fn body(self, _env: &Environment) -> impl View {
        match self.backing {
            IconBacking::Glyph(glyph) => glyph,
        }
    }
}

#[cfg(all(test, feature = "svg"))]
mod tests {
    use alloc::boxed::Box;
    use alloc::vec::Vec;
    use core::any::TypeId;

    use waterui_core::backend::{Backend, ChildSlot, cast};
    use waterui_core::metadata::MetadataKey;
    use waterui_core::plugin::Plugin;
    use waterui_core::view::hook::{HookList, HookableView};
    use waterui_core::{AnyView, Environment, Metadata, NativeView, View};
    use waterui_layout::container::FixedContainer;
    use waterui_svg::Svg;

    use crate::{Icon, IconGlyph, IconSize};

    type StaticDispatch = Box<dyn FnOnce(&mut StaticOnlyBackend, &Environment)>;

    struct RecordedSlot {
        type_id: TypeId,
        dispatch: StaticDispatch,
    }

    impl ChildSlot for RecordedSlot {
        fn from_view<V: View>(view: V) -> Self {
            Self {
                type_id: TypeId::of::<V>(),
                dispatch: Box::new(move |backend, env| {
                    view.dispatch(backend, env, ());
                }),
            }
        }
    }

    struct StaticOnlyBackend {
        reached: usize,
        child_types: Vec<TypeId>,
    }

    impl Backend for StaticOnlyBackend {
        type Output = ();
        type Ctx = ();
        type Slot = RecordedSlot;

        fn native<T: NativeView + 'static>(&mut self, native: T, _env: &Environment, (): ()) {
            self.reached += 1;
            if let Ok(container) = cast::<T, FixedContainer<Vec<RecordedSlot>>>(native) {
                let (_, children) = container.into_inner();
                self.child_types
                    .extend(children.into_iter().map(|slot| slot.type_id));
            }
        }

        fn metadata<K: MetadataKey>(
            &mut self,
            metadata: Metadata<K, Self::Slot>,
            env: &Environment,
            (): (),
        ) {
            self.reached += 1;
            (metadata.content.dispatch)(self, env);
        }

        fn erased(&mut self, _view: AnyView, _env: &Environment, (): ()) {
            panic!("Icon must dispatch statically without AnyView erasure");
        }

        fn scoped<H: HookList, V: View>(&mut self, _hooks: H, view: V, env: &Environment, (): ()) {
            view.dispatch(self, env, ());
        }

        fn hookable<T: HookableView>(&mut self, view: T, env: &Environment, (): ()) {
            self.native(view, env, ());
        }
    }

    #[test]
    fn icon_dispatches_glyph_and_framed_svg_without_anyview() {
        let plain_env = Environment::new();
        let mut sized_env = Environment::new();
        IconSize::new(32.0).install(&mut sized_env);

        let glyph = Icon::from_glyph(IconGlyph::new('\u{e5c4}', "TestIcons"));
        let mut backend = StaticOnlyBackend {
            reached: 0,
            child_types: Vec::new(),
        };
        glyph.dispatch(&mut backend, &plain_env, ());
        assert!(backend.reached > 0);

        let sized_svg = Icon::from_path("M0 0h24v24H0z", 24.0, 24.0);
        let mut backend = StaticOnlyBackend {
            reached: 0,
            child_types: Vec::new(),
        };
        sized_svg.dispatch(&mut backend, &sized_env, ());
        assert_eq!(backend.child_types, [TypeId::of::<Svg>()]);

        let oversized_svg = Icon::from_path("M0 0h48v48H0z", 48.0, 48.0);
        let mut backend = StaticOnlyBackend {
            reached: 0,
            child_types: Vec::new(),
        };
        oversized_svg.dispatch(&mut backend, &plain_env, ());
        assert_eq!(backend.child_types, [TypeId::of::<Svg>()]);
    }
}
