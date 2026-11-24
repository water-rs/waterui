use std::cell::Cell;
use std::rc::Rc;
use waterui_core::extract::{ExtractionState, Extractor, State};
use waterui_core::{Environment, Error};

/// A window's identity: the runner assigns one when it
/// creates the window and hands it to the window's renderer, so the shared
/// menu shortcut registry can scope a mounted `Menu`'s chords to the window
/// they mounted in. Unlike a pointer address, it cannot drift when the
/// renderer moves nor alias a recycled allocation (water-rs/hydrolysis#247).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowId {
    /// A winit window — its native `winit::window::WindowId`.
    #[cfg(hydrolysis_winit)]
    Winit(winit::window::WindowId),
    /// A window on a runner without a native window id — an ordinal the
    /// registry hands out in window-creation order.
    Runner(u64),
    /// A renderer no runner claimed — embedded GPU hosts and renderers built
    /// directly in tests. Their mounted sources answer only to their own
    /// dispatch.
    Orphan,
}

impl WindowId {
    /// The winit window this id names, when it names one.
    #[cfg(all(hydrolysis_closable_windows, feature = "menu"))]
    pub(crate) const fn as_winit(self) -> Option<winit::window::WindowId> {
        match self {
            Self::Winit(id) => Some(id),
            Self::Runner(_) | Self::Orphan => None,
        }
    }
}

impl Extractor for WindowId {
    fn extract(env: &Environment) -> Result<Self, Error> {
        State::<Self>::extract(env).map(|State(id)| id)
    }

    fn extract_from_action(env: &Environment, state: &mut ExtractionState) -> Result<Self, Error> {
        State::<Self>::extract_from_action(env, state).map(|State(id)| id)
    }
}

/// Window identities shared by every window belonging to one runner.
#[derive(Clone, Default)]
pub struct WindowIdentities(Rc<Cell<u64>>);

impl WindowIdentities {
    pub(crate) fn mint_window_id(&self) -> WindowId {
        let ordinal = self.0.get();
        self.0.set(ordinal + 1);
        WindowId::Runner(ordinal)
    }
}
