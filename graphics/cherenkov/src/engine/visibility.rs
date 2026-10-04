//! A surface's announced visibility, readable wherever a wake for the
//! surface can start.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use arc_swap::ArcSwap;

use crate::backend::Visibility;

/// The [`Visibility`] the host last announced for one surface, shared with
/// everything that wakes the host on the surface's behalf.
///
/// It flips on the UI thread the moment the host announces a change, ahead
/// of the render loop, which learns it in order with every other message.
/// A wake checks it at the moment it fires, so a hidden surface wakes
/// nothing from the moment the host hid it — whichever thread the wake
/// starts on and whatever the render thread has applied by then.
#[derive(Clone, Debug)]
pub struct SurfaceVisibility(Arc<AtomicBool>);

impl SurfaceVisibility {
    /// A visible surface's flag; surfaces start visible.
    pub(crate) fn new() -> Self {
        Self(Arc::new(AtomicBool::new(true)))
    }

    /// The visibility the host last announced.
    #[must_use]
    pub fn get(&self) -> Visibility {
        if self.is_visible() {
            Visibility::Visible
        } else {
            Visibility::Hidden
        }
    }

    /// Whether the host last announced the surface visible.
    #[must_use]
    pub fn is_visible(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }

    /// Records the host's announcement.
    pub(crate) fn set(&self, visibility: Visibility) {
        self.0
            .store(visibility == Visibility::Visible, Ordering::Release);
    }
}

/// Two handles are equal when they are the same surface's flag.
impl PartialEq for SurfaceVisibility {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for SurfaceVisibility {}

/// Gates the host wakes of a source a backend drives on its own — a GPU
/// producer's or a filter's redraw request — on the surfaces it draws
/// into: the gate is open while one of them is visible.
///
/// The render loop sets the surfaces from its frames, so the membership
/// can lag a frame behind the content; each surface's
/// [`SurfaceVisibility`] is read when the wake fires, so the visibility
/// never lags. A gate starts closed.
#[derive(Debug)]
pub struct WakeGate(ArcSwap<Vec<SurfaceVisibility>>);

impl Default for WakeGate {
    fn default() -> Self {
        Self(ArcSwap::from_pointee(Vec::new()))
    }
}

impl WakeGate {
    /// Whether a wake may reach the host: one of the gate's surfaces is
    /// visible.
    #[must_use]
    pub fn is_open(&self) -> bool {
        self.0.load().iter().any(SurfaceVisibility::is_visible)
    }

    /// Replaces the surfaces the source draws into. Setting the surfaces it
    /// already has allocates nothing.
    pub fn set(&self, surfaces: &[SurfaceVisibility]) {
        if self.0.load().as_slice() != surfaces {
            self.0.store(Arc::new(surfaces.to_vec()));
        }
    }

    /// Closes the gate: the source draws into no surface.
    pub fn close(&self) {
        self.set(&[]);
    }
}
