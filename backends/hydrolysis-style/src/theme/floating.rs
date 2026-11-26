//! The floating-surface look of the theme contract: what a theme returns for
//! `.floating()` ([`FloatingLook`]), the token bundle for solid-fill floating
//! surfaces ([`FloatingStyle`] / [`FloatingElevation`]), the scoped override
//! plugin ([`FloatingOverride`]), and the descendant marker ([`FloatingScope`]).

use core::time::Duration;

use cherenkov_record::{Paint, Shadow, WorkingColor};
use kurbo::Vec2;
use nami::Signal as _;
use waterui_core::{Environment, animation::Animation, plugin::Plugin};
use waterui_graphics::color::{
    Color, ColorScheme, current_color_scheme,
    token::{Accent, Foreground, Surface as SurfaceToken},
};
use waterui_interaction::interaction::StateValue;

use super::{CornerShape, Corners, Driven, Elevation, Inputs, Motion, Surface, WidgetTheme};

/// The two shadows that lift a floating surface: a soft ambient shadow and a
/// tighter key shadow.
#[derive(Debug, Clone)]
pub struct FloatingElevation {
    /// Ambient shadow color.
    pub ambient_shadow_color: Color,
    /// Ambient shadow blur radius.
    pub ambient_shadow_radius: f32,
    /// Ambient shadow vertical offset.
    pub ambient_shadow_offset_y: f32,
    /// Key shadow color.
    pub key_shadow_color: Color,
    /// Key shadow blur radius.
    pub key_shadow_radius: f32,
    /// Key shadow vertical offset.
    pub key_shadow_offset_y: f32,
}

/// Token bundle for a solid-fill floating surface (such as a Material 3 FAB or
/// elevated floating control group).
///
/// Themes and style-provided floating controls build a [`FloatingLook`] from
/// this via [`FloatingLook::from_style`], or construct a [`FloatingLook`]
/// directly with any [`Surface`] (including a [`Backdrop`](super::Backdrop)
/// material for glass themes).
#[derive(Debug, Clone)]
pub struct FloatingStyle {
    /// Floating surface fill.
    pub container_color: Color,
    /// Foreground used by semantic controls inside the floating surface.
    pub content_color: Color,
    /// Hover, focus, press, and ripple color.
    pub state_layer_color: Color,
    /// Normalized corner radius used by the surface and its state layer.
    pub clip_radius: f32,
    /// Horizontal content inset used when measuring a semantic button.
    pub content_inset_x: f64,
    /// Vertical content inset used when measuring a semantic button.
    pub content_inset_y: f64,
    /// Theme-provided minimum width for a floating semantic button.
    pub minimum_width: f64,
    /// Theme-provided minimum height for a floating semantic button.
    pub minimum_height: f64,
    /// Opacity applied to floating-button content while disabled.
    pub disabled_content_opacity: f32,
    /// The surface's elevation per interaction state — a floating action
    /// button rises one level while hovered.
    pub elevation: StateValue<FloatingElevation>,
}

impl Default for FloatingStyle {
    fn default() -> Self {
        Self {
            container_color: SurfaceToken.into(),
            content_color: Accent.into(),
            state_layer_color: Accent.into(),
            clip_radius: 0.25,
            content_inset_x: 0.0,
            content_inset_y: 0.0,
            minimum_width: 44.0,
            minimum_height: 44.0,
            disabled_content_opacity: 0.38,
            elevation: StateValue::new(FloatingElevation {
                ambient_shadow_color: Color::new(Foreground).with_opacity(0.08),
                ambient_shadow_radius: 6.0,
                ambient_shadow_offset_y: 2.0,
                key_shadow_color: Color::new(Foreground).with_opacity(0.12),
                key_shadow_radius: 3.0,
                key_shadow_offset_y: 1.0,
            }),
        }
    }
}

impl FloatingElevation {
    fn to_elevation(&self, env: &Environment) -> Elevation {
        Elevation::new(
            Shadow {
                sigma: f64::from(self.key_shadow_radius.max(0.0)),
                offset: Vec2::new(0.0, f64::from(self.key_shadow_offset_y)),
                spread: 0.0,
                color: self.key_shadow_color.resolve(env).snapshot(),
            },
            Shadow {
                sigma: f64::from(self.ambient_shadow_radius.max(0.0)),
                offset: Vec2::new(0.0, f64::from(self.ambient_shadow_offset_y)),
                spread: 0.0,
                color: self.ambient_shadow_color.resolve(env).snapshot(),
            },
        )
    }
}

/// The declarative look of a `.floating()` surface: its container [`Surface`],
/// interaction [`Motion`], and the tokens semantic controls (such as buttons)
/// adopt when placed inside it.
#[derive(Debug, Clone)]
pub struct FloatingLook {
    surface: Surface,
    motion: Motion,
    content_color: Color,
    state_layer_color: Color,
    content_inset_x: f64,
    content_inset_y: f64,
    minimum_width: f64,
    minimum_height: f64,
    disabled_content_opacity: f32,
}

impl FloatingLook {
    /// Creates a floating look from its container `surface`, interaction
    /// `motion`, and button foreground / state-layer colours.
    #[must_use]
    pub fn new(
        surface: Surface,
        motion: Motion,
        content_color: impl Into<Color>,
        state_layer_color: impl Into<Color>,
    ) -> Self {
        Self {
            surface,
            motion,
            content_color: content_color.into(),
            state_layer_color: state_layer_color.into(),
            content_inset_x: 0.0,
            content_inset_y: 0.0,
            minimum_width: 44.0,
            minimum_height: 44.0,
            disabled_content_opacity: 0.38,
        }
    }

    /// Builds a [`FloatingLook`] from a solid-fill [`FloatingStyle`] resolved
    /// against `env`.
    #[must_use]
    pub fn from_style(style: &FloatingStyle, env: &Environment) -> Self {
        let shape = CornerShape::new(Corners::Fraction(f64::from(style.clip_radius)));
        let fill = Driven::follow(
            style.container_color.resolve(env),
            |color: &WorkingColor, _| Paint::from(*color),
        );
        let style_elevation = style.elevation.clone();
        let elevation_env = env.clone();
        let elevation = Driven::follow(
            current_color_scheme(env),
            move |_scheme: &ColorScheme, inputs: &Inputs<'_, ()>| {
                style_elevation
                    .resolve(inputs.state)
                    .to_elevation(&elevation_env)
            },
        );
        let surface = Surface::new(shape, fill).elevation(elevation);
        let motion = Motion::uniform(Animation::linear(Duration::ZERO));
        Self::new(
            surface,
            motion,
            style.content_color.clone(),
            style.state_layer_color.clone(),
        )
        .content_inset(style.content_inset_x, style.content_inset_y)
        .minimum_size(style.minimum_width, style.minimum_height)
        .disabled_content_opacity(style.disabled_content_opacity)
    }

    /// Sets horizontal and vertical content inset for semantic buttons inside
    /// this floating surface.
    #[must_use]
    pub const fn content_inset(mut self, x: f64, y: f64) -> Self {
        self.content_inset_x = x;
        self.content_inset_y = y;
        self
    }

    /// Sets the minimum size for semantic buttons inside this floating surface.
    #[must_use]
    pub const fn minimum_size(mut self, width: f64, height: f64) -> Self {
        self.minimum_width = width;
        self.minimum_height = height;
        self
    }

    /// Sets the disabled content opacity for semantic buttons inside this
    /// floating surface.
    #[must_use]
    pub const fn disabled_content_opacity(mut self, opacity: f32) -> Self {
        self.disabled_content_opacity = opacity;
        self
    }

    /// The floating surface's container (silhouette, fill/backdrop, border,
    /// elevation, optional content effect).
    #[must_use]
    pub const fn surface(&self) -> &Surface {
        &self.surface
    }

    /// Motion for interaction-driven surface changes.
    #[must_use]
    pub const fn motion(&self) -> &Motion {
        &self.motion
    }

    /// Foreground colour used by semantic controls inside the surface.
    #[must_use]
    pub const fn content_color(&self) -> &Color {
        &self.content_color
    }

    /// State-layer colour used by semantic controls inside the surface.
    #[must_use]
    pub const fn state_layer_color(&self) -> &Color {
        &self.state_layer_color
    }

    /// Horizontal content inset for floating semantic buttons.
    #[must_use]
    pub const fn content_inset_x(&self) -> f64 {
        self.content_inset_x
    }

    /// Vertical content inset for floating semantic buttons.
    #[must_use]
    pub const fn content_inset_y(&self) -> f64 {
        self.content_inset_y
    }

    /// Minimum width for floating semantic buttons.
    #[must_use]
    pub const fn minimum_width(&self) -> f64 {
        self.minimum_width
    }

    /// Minimum height for floating semantic buttons.
    #[must_use]
    pub const fn minimum_height(&self) -> f64 {
        self.minimum_height
    }

    /// Opacity applied to floating-button content while disabled.
    #[must_use]
    pub const fn disabled_content_opacity_value(&self) -> f32 {
        self.disabled_content_opacity
    }
}

#[derive(Debug, Clone)]
enum FloatingOverrideInner {
    Look(Box<FloatingLook>),
    Style(FloatingStyle),
}

/// Overrides the look of the immediately enclosed `.floating()` surface.
///
/// Style-provided floating components (such as Material 3 FAB size/color
/// variants and FAB menus) install this plugin on a `.floating()` view so
/// Hydrolysis realizes that surface with the variant's look instead of the
/// theme's default [`WidgetTheme::floating`]. Hydrolysis consumes the override
/// at the enclosing `.floating()` boundary so nested floating surfaces do not
/// inherit it.
#[derive(Debug, Clone)]
pub struct FloatingOverride(FloatingOverrideInner);

impl FloatingOverride {
    /// Creates an override from a declarative [`FloatingLook`].
    #[must_use]
    pub fn look(look: FloatingLook) -> Self {
        Self(FloatingOverrideInner::Look(Box::new(look)))
    }

    /// Creates an override from solid-fill [`FloatingStyle`] tokens.
    #[must_use]
    pub const fn style(style: FloatingStyle) -> Self {
        Self(FloatingOverrideInner::Style(style))
    }

    /// Resolves this override into a [`FloatingLook`] in `env`.
    #[must_use]
    pub fn resolve(&self, env: &Environment) -> FloatingLook {
        match &self.0 {
            FloatingOverrideInner::Look(look) => (**look).clone(),
            FloatingOverrideInner::Style(style) => FloatingLook::from_style(style, env),
        }
    }
}

impl From<FloatingLook> for FloatingOverride {
    fn from(look: FloatingLook) -> Self {
        Self::look(look)
    }
}

impl From<FloatingStyle> for FloatingOverride {
    fn from(style: FloatingStyle) -> Self {
        Self::style(style)
    }
}

impl Plugin for FloatingOverride {}

/// Marks the content inside an enclosing `.floating()` surface.
///
/// Hydrolysis installs this into the child environment when realizing
/// `Metadata<Floating>` so semantic controls inside (such as buttons) adopt
/// the enclosing floating surface's geometry, foreground colour, and minimum
/// target metrics instead of drawing their own opaque container.
#[derive(Debug, Clone)]
pub struct FloatingScope {
    override_surface: Option<FloatingOverride>,
}

impl FloatingScope {
    /// Marks a floating surface backed by the active theme's
    /// [`WidgetTheme::floating`].
    #[must_use]
    pub const fn theme() -> Self {
        Self {
            override_surface: None,
        }
    }

    /// Marks a floating surface backed by an explicit [`FloatingOverride`], or
    /// the active theme when `override_surface` is `None`.
    #[must_use]
    pub const fn new(override_surface: Option<FloatingOverride>) -> Self {
        Self { override_surface }
    }

    /// Resolves the enclosing floating surface's [`FloatingLook`] against
    /// `theme` and `env`.
    #[must_use]
    pub fn resolve(&self, theme: &dyn WidgetTheme, env: &Environment) -> FloatingLook {
        self.override_surface.as_ref().map_or_else(
            || theme.floating(env),
            |override_surface| override_surface.resolve(env),
        )
    }
}

impl Plugin for FloatingScope {}

/// Default framework floating look used until a style package overrides
/// [`WidgetTheme::floating`].
#[must_use]
pub fn default_floating_look(env: &Environment) -> FloatingLook {
    FloatingLook::from_style(&FloatingStyle::default(), env)
}
