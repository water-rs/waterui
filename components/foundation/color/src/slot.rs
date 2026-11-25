//! Environment storage, scheme derivations, and [`Tint`] resolution for theme
//! colour tokens.

use core::any::type_name;
use core::fmt;
use core::marker::PhantomData;
use std::rc::Rc;

use nami::{
    Computed, Signal, SignalExt,
    signal::{IntoComputed, IntoSignal},
};
use waterui_core::{Environment, contract_violation, env::Store, resolve::Resolvable};

use crate::{Color, ColorScheme, WorkingColor, token};

pub trait Sealed {}

/// A theme colour slot: one of the tokens in [`token`].
///
/// Sealed: the slots are exactly the tokens [`install_color_scheme`]
/// re-derives when a colour scheme is installed.
pub trait ColorSlot: Sealed + 'static {}

/// How a colour slot follows the colour scheme.
///
/// A slot installed with [`install_color_derivation`] keeps its derivation,
/// so a colour scheme installed later on a subtree re-derives the slot from
/// that subtree's scheme, and a change of the scheme signal re-resolves the
/// slot in place.
#[derive(Clone)]
pub struct ColorDerivation(Rc<DeriveColor>);

/// A slot's colour signal under a colour-scheme signal.
type DeriveColor = dyn Fn(&Computed<ColorScheme>) -> Computed<WorkingColor>;

impl fmt::Debug for ColorDerivation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ColorDerivation").finish_non_exhaustive()
    }
}

impl ColorDerivation {
    /// The slot's colour for each colour scheme, as `derive` computes it.
    #[must_use]
    pub fn new(derive: impl Fn(ColorScheme) -> WorkingColor + 'static) -> Self {
        let derive = Rc::new(derive);
        Self(Rc::new(move |scheme| {
            let derive = Rc::clone(&derive);
            scheme.map(move |scheme| derive(scheme)).computed()
        }))
    }

    /// The slot's colour in the light and in the dark scheme, each its own
    /// signal: a host whose per-scheme colours change for reasons other than
    /// the scheme (an accent or contrast setting) feeds them through these.
    #[must_use]
    pub fn per_scheme(
        light: impl IntoComputed<WorkingColor>,
        dark: impl IntoComputed<WorkingColor>,
    ) -> Self {
        let light = light.into_computed();
        let dark = dark.into_computed();
        Self(Rc::new(move |scheme| {
            scheme
                .zip(&light)
                .zip(&dark)
                .map(|((scheme, light), dark)| match scheme {
                    ColorScheme::Light => light,
                    ColorScheme::Dark => dark,
                })
                .computed()
        }))
    }

    /// The slot's colour signal under `scheme`.
    fn derive(&self, scheme: &Computed<ColorScheme>) -> Computed<WorkingColor> {
        (self.0)(scheme)
    }
}

/// Internal storage for a color signal in the environment.
#[derive(Clone)]
struct ColorSlotValue<T> {
    signal: Computed<WorkingColor>,
    /// The derivation `signal` came from, re-run by [`install_color_scheme`];
    /// `None` for a colour that is the same in every scheme.
    derivation: Option<ColorDerivation>,
    _marker: PhantomData<T>,
}

impl<T> ColorSlotValue<T> {
    const fn new(signal: Computed<WorkingColor>, derivation: Option<ColorDerivation>) -> Self {
        Self {
            signal,
            derivation,
            _marker: PhantomData,
        }
    }
}

/// Resolves a color slot by looking up the signal installed by the backend or theme.
///
/// A slot the backend or theme never installed breaks the contract: a debug
/// build panics, and a release build logs the error and resolves opaque black.
pub fn resolve_color_slot<T: 'static>(env: &Environment) -> Computed<WorkingColor> {
    env.get::<ColorSlotValue<T>>().map_or_else(
        || {
            contract_violation!(
                "WaterUI color token `{}` is not installed in the environment, so it resolves opaque black",
                type_name::<T>()
            );
            Computed::constant(WorkingColor::BLACK)
        },
        |slot| slot.signal.clone(),
    )
}

/// Returns the current color scheme signal from the environment.
///
/// # Contract
///
/// A backend or theme that has not installed a color scheme breaks the
/// contract: a debug build panics, and a release build logs the error and
/// returns the light scheme.
#[must_use]
pub fn current_color_scheme(env: &Environment) -> Computed<ColorScheme> {
    installed_color_scheme(env).unwrap_or_else(|| {
        contract_violation!(
            "ColorScheme is not installed in the environment, so it is the light scheme"
        );
        Computed::constant(ColorScheme::default())
    })
}

/// Returns the color-scheme signal installed by the active backend or theme.
#[must_use]
pub fn installed_color_scheme(env: &Environment) -> Option<Computed<ColorScheme>> {
    env.query::<ColorScheme, Computed<ColorScheme>>().cloned()
}

/// Installs an explicit color signal for a specific slot.
///
/// The signal is the slot's colour in every scheme: a colour scheme installed
/// later leaves it as it is. A colour that follows the scheme is installed
/// with [`install_color_derivation`].
pub fn install_color_signal<T: 'static>(env: &mut Environment, signal: Computed<WorkingColor>) {
    insert_color_slot::<T>(env, signal, None);
}

/// Installs `derivation` for a colour slot, derived from the colour scheme
/// installed in `env`.
///
/// The slot keeps the derivation: [`install_color_scheme`] on a subtree
/// re-derives it from the subtree's scheme, and a change of the scheme
/// signal re-resolves it in place.
///
/// # Contract
///
/// The colour scheme is installed first; without one this is the contract
/// violation [`current_color_scheme`] reports.
pub fn install_color_derivation<T: ColorSlot>(env: &mut Environment, derivation: ColorDerivation) {
    let signal = derivation.derive(&current_color_scheme(env));
    insert_color_slot::<T>(env, signal, Some(derivation));
}

/// Re-derives slot `T` from `scheme` when it was installed as a derivation.
pub fn rederive_color_slot<T: ColorSlot>(env: &mut Environment, scheme: &Computed<ColorScheme>) {
    let Some(derivation) = env
        .get::<ColorSlotValue<T>>()
        .and_then(|slot| slot.derivation.clone())
    else {
        return;
    };
    insert_color_slot::<T>(env, derivation.derive(scheme), Some(derivation));
}

/// Stores slot `T`'s signal in the environment.
fn insert_color_slot<T: 'static>(
    env: &mut Environment,
    signal: Computed<WorkingColor>,
    derivation: Option<ColorDerivation>,
) {
    env.insert(ColorSlotValue::<T>::new(signal, derivation));
}

/// Returns an installed color signal for the requested slot when one exists.
#[must_use]
pub fn installed_color_signal<T: 'static>(env: &Environment) -> Option<Computed<WorkingColor>> {
    env.get::<ColorSlotValue<T>>()
        .map(|value| value.signal.clone())
}

/// Installs a color scheme signal and re-derives every colour slot installed as
/// a [`ColorDerivation`] from `signal`.
pub fn install_color_scheme(env: &mut Environment, signal: Computed<ColorScheme>) {
    token::rederive(env, &signal);
    env.insert(Store::<ColorScheme, Computed<ColorScheme>>::new(signal));
}

/// The tint the controls of a subtree draw with.
///
/// A control's tint is the nearest enclosing `Tint`, and the
/// [`Accent`](token::Accent) token when none is set; [`resolve_tint`] and
/// [`Color::tint`] resolve it. The colour may be a signal: a change of the
/// signal updates the controls in place, without rebuilding them.
///
/// Two tints are equal when they are the same installed tint, so a backend
/// asks "did the subtree's tint change" without comparing colours.
#[derive(Debug, Clone)]
pub struct Tint(Rc<Color>);

impl Tint {
    /// A tint of `color`.
    pub fn new(color: impl IntoSignal<Color> + 'static) -> Self {
        Self(Rc::new(Color::from_signal(color)))
    }

    /// The tint colour.
    #[must_use]
    pub fn color(&self) -> &Color {
        &self.0
    }

    /// The tint installed for the subtree `env` belongs to, if any.
    #[must_use]
    pub fn inherited(env: &Environment) -> Option<&Self> {
        env.get::<Self>()
    }

    /// The colour of `tint`, or the [`Accent`](token::Accent) token when there
    /// is none.
    #[must_use]
    pub fn color_or_accent(tint: Option<&Self>) -> Color {
        tint.map_or_else(Color::accent, |tint| tint.color().clone())
    }
}

impl PartialEq for Tint {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for Tint {}

/// The tint of the controls in `env`: the nearest [`Tint`], else the
/// [`Accent`](token::Accent) token.
#[must_use]
pub fn resolve_tint(env: &Environment) -> Color {
    Tint::color_or_accent(Tint::inherited(env))
}

#[derive(Debug, Clone, Copy, Default)]
struct ResolvedTint;

impl Resolvable for ResolvedTint {
    type Resolved = WorkingColor;

    fn resolve(&self, env: &Environment) -> impl Signal<Output = Self::Resolved> {
        resolve_tint(env).resolve(env)
    }
}

impl Color {
    /// The nearest enclosing [`Tint`], falling back to [`token::Accent`] when
    /// none is set in the environment.
    #[must_use]
    pub fn tint() -> Self {
        Self::new(ResolvedTint)
    }
}
