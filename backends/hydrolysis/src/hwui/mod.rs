//! The HWUI render target: Hydrolysis's retained layer tree recorded into
//! Android `RenderNode` display lists (water-rs/waterui#1811).
//!
//! [`HwuiTarget`] is a [`cherenkov_record`](waterui_graphics::draw) target.
//! Its [`Encoder`] mirrors a [`SurfaceTree`](waterui_graphics::draw::SurfaceTree)
//! from committed [`ChangeSet`]s and writes each frame into one Rust-owned
//! command buffer, which the Kotlin `CommandDecoder` (`android/hwui`)
//! replays onto `RenderNode`s: one per layer, two for a scrolled layer.
//!
//! Colour differs from Cherenkov on purpose: HWUI composites in the
//! window's encoded colour space, not Cherenkov's extended-linear working
//! space (see `color`), so blends and alpha composites are encoded-space
//! operations here.

/// Declares a module of wire codes and, under test, the table the
/// generated Kotlin mirrors (`hwui::kotlin`).
macro_rules! codes {
    (
        $(#[doc = $doc:literal])*
        $module:ident: $ty:ty => $object:literal {
            $( $(#[doc = $code_doc:literal])* $name:ident = $value:expr; )*
        }
    ) => {
        $(#[doc = $doc])*
        pub mod $module {
            $( $(#[doc = $code_doc])* pub const $name: $ty = $value; )*

            /// The codes, for the generated Kotlin.
            #[cfg(test)]
            pub const CODES: crate::hwui::kotlin::Codes<$ty> = crate::hwui::kotlin::Codes {
                object: $object,
                doc: &[$($doc),*],
                entries: &[$( (stringify!($name), $name, &[$($code_doc),*]) ),*],
            };
        }
    };
}

mod buffer;
mod color;
#[cfg(test)]
mod contract;
mod encoder;
mod fonts;
mod geometry;
mod ids;
#[cfg(hydrolysis_hwui)]
mod jni;
#[cfg(hydrolysis_hwui)]
mod jni_resources;
#[cfg(test)]
mod kotlin;
mod lower;
mod platform;
mod protocol;
mod resources;
pub mod text;

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use waterui_graphics::draw::{ChangeSet, ProjectiveError, ProjectiveLayers, Queue, Target};

pub use encoder::{Encoder, Frame, HostEntry, Recycle};
pub use platform::HwuiResources;

/// The HWUI render target. It samples projective layers
/// ([`ProjectiveLayers`]); it does not sample backdrops and installs no GPU
/// producers, so neither edit exists for it.
#[derive(Debug)]
pub struct HwuiTarget;

impl Target for HwuiTarget {
    type Queue = HwuiQueue;
    // No `GpuInstalls`: an install can never be constructed.
    type Install = std::convert::Infallible;
}

impl ProjectiveLayers for HwuiTarget {}

/// The HWUI target's queue endpoint. A visible surface schedules a frame
/// through its waker; a hidden one keeps each drained change set for the
/// encoder's next [`Encoder::commit`].
#[derive(Clone)]
pub struct HwuiQueue {
    state: Rc<QueueState>,
}

struct QueueState {
    inline: Cell<bool>,
    applied: RefCell<Vec<ChangeSet<HwuiTarget>>>,
    wake: Box<dyn Fn()>,
}

impl std::fmt::Debug for HwuiQueue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HwuiQueue")
            .field("inline", &self.state.inline.get())
            .field("applied", &self.state.applied.borrow().len())
            .finish_non_exhaustive()
    }
}

impl HwuiQueue {
    /// A visible queue whose [`Queue::wake`] calls `wake`, typically asking
    /// the host's `Choreographer` for a frame.
    pub fn new(wake: impl Fn() + 'static) -> Self {
        Self {
            state: Rc::new(QueueState {
                inline: Cell::new(false),
                applied: RefCell::new(Vec::new()),
                wake: Box::new(wake),
            }),
        }
    }

    /// Whether drains apply inline (a hidden surface) rather than waking.
    pub fn set_inline(&self, inline: bool) {
        self.state.inline.set(inline);
    }

    /// The change sets applied inline since the last call, in order.
    #[must_use]
    pub fn take_applied(&self) -> Vec<ChangeSet<HwuiTarget>> {
        std::mem::take(&mut *self.state.applied.borrow_mut())
    }
}

impl Queue<HwuiTarget> for HwuiQueue {
    fn drains_inline(&self) -> bool {
        self.state.inline.get()
    }

    fn apply(&self, changes: ChangeSet<HwuiTarget>) {
        self.state.applied.borrow_mut().push(changes);
    }

    fn wake(&self) {
        (self.state.wake)();
    }
}

/// Why the HWUI target cannot encode a frame. Every case the design does
/// not cover is one of these; nothing degrades silently.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum HwuiError {
    /// A record broke the wire format: an internal invariant.
    #[error("HWUI command buffer: {op} {reason}")]
    Encoding {
        /// The op being written.
        op: &'static str,
        /// What is wrong.
        reason: String,
    },
    /// A kind's dense ids ran out.
    #[error("HWUI target: every {kind} id is live")]
    IdsExhausted {
        /// The resource kind.
        kind: &'static str,
    },
    /// The layer asks for something the HWUI target does not render.
    #[error("HWUI target: layer {layer} {what}")]
    Unsupported {
        /// The layer.
        layer: u64,
        /// What it asks for.
        what: String,
    },
    /// The layer needs a newer Android than the device runs.
    #[error(
        "HWUI target: layer {layer} draws {what}, which needs API {needs}; the device runs API {device}"
    )]
    RequiresApi {
        /// The layer.
        layer: u64,
        /// The feature.
        what: &'static str,
        /// The API level it needs.
        needs: u32,
        /// The device's API level.
        device: u32,
    },
    /// The layer names a resource not registered with this target.
    #[error(
        "HWUI target: layer {layer} draws {kind} {id}, which is not registered with the HWUI target"
    )]
    Unregistered {
        /// The layer.
        layer: u64,
        /// The resource kind.
        kind: &'static str,
        /// The resource's plain id.
        id: u64,
    },
    /// A text layout request or reply broke the text wire, or the platform
    /// text provider refused it.
    #[error("HWUI text: {reason}")]
    Text {
        /// What is wrong.
        reason: String,
    },
    /// The layer's projective pose is degenerate.
    #[error("HWUI target: layer {layer}'s projective transform: {source}")]
    Projective {
        /// The layer.
        layer: u64,
        /// Why.
        source: ProjectiveError,
    },
}
