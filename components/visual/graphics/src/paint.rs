//! Unified shape paint: a solid [`Color`] or a [`Gradient`] resolved in the
//! view's [`Environment`].
//!
//! [`Paint`] lets shape fills and strokes accept every colour form (concrete
//! [`Color`], theme tokens such as `Accent` or `Surface`, colour-space values,
//! and reactive colour signals) alongside [`Gradient`] values (and reactive
//! gradient signals) through one ergonomic [`IntoPaint`] bound.

extern crate alloc;

use nami::{Computed, SignalExt as _, signal::IntoSignal};
use waterui_color::Color;
use waterui_core::{Environment, flatten_signal};

use crate::draw;
use crate::gradient::Gradient;

/// Marker for [`IntoPaint`] conversions from any colour or colour signal.
#[derive(Debug, Clone, Copy)]
pub struct ColorPaint;

/// Marker for [`IntoPaint`] conversions from any [`Gradient`] or gradient signal.
#[derive(Debug, Clone, Copy)]
pub struct GradientPaint;

/// Marker for [`IntoPaint`] conversions from [`Paint`] itself.
#[derive(Debug, Clone, Copy)]
pub struct DirectPaint;

#[derive(Debug, Clone)]
enum PaintKind {
    Color(Color),
    Gradient(Gradient),
    GradientSignal(Computed<Gradient>),
}

/// A paint that fills or strokes a shape: either a [`Color`] or a [`Gradient`].
///
/// Both variants resolve against the view's [`Environment`] into a reactive
/// unit-space [`draw::Paint`], updating in place when theme tokens, colour
/// schemes, or reactive signals change.
#[derive(Debug, Clone)]
pub struct Paint {
    kind: PaintKind,
}

impl Default for Paint {
    fn default() -> Self {
        Self::color(Color::default())
    }
}

impl Paint {
    /// Creates a solid-colour paint from any colour, theme token, or colour signal.
    #[must_use]
    pub fn color(color: impl IntoSignal<Color> + 'static) -> Self {
        Self {
            kind: PaintKind::Color(Color::from_signal(color)),
        }
    }

    /// Creates a gradient paint from a [`Gradient`].
    #[must_use]
    pub const fn gradient(gradient: Gradient) -> Self {
        Self {
            kind: PaintKind::Gradient(gradient),
        }
    }

    /// Creates a reactive gradient paint from a signal of [`Gradient`] values.
    #[must_use]
    pub fn from_gradient_signal(signal: impl IntoSignal<Gradient> + 'static) -> Self {
        Self {
            kind: PaintKind::GradientSignal(signal.into_signal().computed()),
        }
    }

    /// Returns the underlying [`Color`] if this paint is a solid colour.
    #[must_use]
    pub const fn as_color(&self) -> Option<&Color> {
        match &self.kind {
            PaintKind::Color(color) => Some(color),
            PaintKind::Gradient(_) | PaintKind::GradientSignal(_) => None,
        }
    }

    /// Returns the underlying [`Gradient`] if this paint is a static gradient.
    #[must_use]
    pub const fn as_gradient(&self) -> Option<&Gradient> {
        match &self.kind {
            PaintKind::Gradient(gradient) => Some(gradient),
            PaintKind::Color(_) | PaintKind::GradientSignal(_) => None,
        }
    }

    /// Resolves this paint in `env` into a reactive unit-space [`draw::Paint`].
    ///
    /// Solid colours resolve to [`draw::Paint::Solid`]; gradients resolve to
    /// unit-space linear, radial, or sweep paints matching [`Gradient::resolve`].
    #[must_use]
    pub fn resolve(&self, env: &Environment) -> Computed<draw::Paint> {
        match &self.kind {
            PaintKind::Color(color) => color.resolve(env).map(draw::Paint::Solid).computed(),
            PaintKind::Gradient(gradient) => gradient.resolve(env),
            PaintKind::GradientSignal(signal) => {
                let env = env.clone();
                flatten_signal(signal.clone().map(move |gradient| gradient.resolve(&env)))
            }
        }
    }
}

impl From<Color> for Paint {
    fn from(color: Color) -> Self {
        Self {
            kind: PaintKind::Color(color),
        }
    }
}

impl From<Gradient> for Paint {
    fn from(gradient: Gradient) -> Self {
        Self::gradient(gradient)
    }
}

/// Converts a colour, colour signal, [`Gradient`], gradient signal, or [`Paint`]
/// into a [`Paint`] for shape fills and strokes.
pub trait IntoPaint<Marker = ColorPaint> {
    /// Converts this value into a [`Paint`].
    fn into_paint(self) -> Paint;
}

impl<T: IntoSignal<Color> + 'static> IntoPaint<ColorPaint> for T {
    fn into_paint(self) -> Paint {
        Paint::color(self)
    }
}

impl<T: IntoSignal<Gradient> + 'static> IntoPaint<GradientPaint> for T {
    fn into_paint(self) -> Paint {
        Paint::from_gradient_signal(self)
    }
}

impl IntoPaint<DirectPaint> for Paint {
    fn into_paint(self) -> Paint {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nami::{Binding, Signal as _};
    use waterui_color::{Srgb, WorkingColor, install_color_signal, token::Accent};
    use waterui_core::layout::UnitPoint;

    fn accept_paint<M>(paint: impl IntoPaint<M>) -> Paint {
        paint.into_paint()
    }

    #[test]
    fn color_token_signal_and_gradient_all_convert_into_paint() {
        let mut env = Environment::new();
        let accent = Srgb::new(0.2, 0.4, 0.8).resolve();
        install_color_signal::<Accent>(&mut env, Computed::constant(accent));

        let from_token = accept_paint(Accent).resolve(&env).snapshot();
        assert_eq!(from_token, draw::Paint::Solid(accent));

        let reactive = Binding::new(Color::srgb(255, 0, 0));
        let resolved_reactive = accept_paint(reactive.clone()).resolve(&env);
        assert!(matches!(
            resolved_reactive.snapshot(),
            draw::Paint::Solid(_)
        ));
        reactive.set(Color::srgb(0, 0, 255));
        assert!(matches!(
            resolved_reactive.snapshot(),
            draw::Paint::Solid(_)
        ));

        let gradient = Gradient::linear(
            [Color::srgb(255, 0, 0), Color::srgb(0, 0, 255)],
            UnitPoint::TOP_LEADING,
            UnitPoint::BOTTOM_TRAILING,
        );
        let from_gradient = accept_paint(gradient.clone()).resolve(&env).snapshot();
        assert!(matches!(from_gradient, draw::Paint::Linear(_)));

        let from_paint = accept_paint(Paint::gradient(gradient))
            .resolve(&env)
            .snapshot();
        assert!(matches!(from_paint, draw::Paint::Linear(_)));

        let reactive_grad = Binding::new(Gradient::linear(
            [Color::srgb(255, 0, 0), Color::srgb(0, 255, 0)],
            UnitPoint::LEADING,
            UnitPoint::TRAILING,
        ));
        let resolved_grad = accept_paint(reactive_grad.clone()).resolve(&env);
        assert!(matches!(resolved_grad.snapshot(), draw::Paint::Linear(_)));
        reactive_grad.set(Gradient::radial(
            [Color::srgb(255, 255, 255), Color::srgb(0, 0, 0)],
            UnitPoint::CENTER,
            0.0,
            0.5,
        ));
        assert!(matches!(resolved_grad.snapshot(), draw::Paint::Radial(_)));
        let _ = WorkingColor::WHITE;
    }
}
