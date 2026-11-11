//! The box a canvas is laid out in.

use alloc::rc::Rc;
use core::cell::Cell;
use core::fmt;

use nami::{Computed, Signal as _};
use waterui_core::contract_violation;
use waterui_core::layout::Size;

use super::cell::SignalCell;

/// The box a canvas is laid out in: set by the mounted canvas, read by its
/// drawing ([`Cx::size`](super::Cx::size)) and by the views around it.
///
/// Create one before the [`Canvas`](super::Canvas) so the drawing and the
/// code around it derive from the same signal, then hand it over with
/// [`Canvas::sized_by`](super::Canvas::sized_by). Clones share one box. It
/// reads [`Size::zero`] until a mounted canvas is laid out, and a canvas
/// that is not given one holds its own.
///
/// One `CanvasSize` belongs to one mounted canvas at a time: mounting a
/// second canvas into it while the first is mounted is a contract violation.
#[derive(Clone)]
pub struct CanvasSize {
    cell: SignalCell<Size>,
    mounted: Rc<Cell<bool>>,
}

impl CanvasSize {
    /// A size source that reads [`Size::zero`] until a canvas is laid out
    /// into it.
    #[must_use]
    pub fn new() -> Self {
        Self {
            cell: SignalCell::new(Size::zero()),
            mounted: Rc::new(Cell::new(false)),
        }
    }

    /// The box, in points: a signal that follows layout.
    ///
    /// It is the signal [`Cx::size`](super::Cx::size) returns inside the
    /// drawing. The drawing closure runs at mount, before the first layout
    /// pass, and sees the zero size until the signal updates.
    #[must_use]
    pub fn signal(&self) -> Computed<Size> {
        Computed::new(self.cell.clone())
    }

    /// The cell the mounted canvas writes and its drawing reads.
    pub(crate) fn cell(&self) -> SignalCell<Size> {
        self.cell.clone()
    }

    /// Marks this size as taken by a mounting canvas, for as long as the
    /// returned claim lives.
    pub(crate) fn claim(&self) -> Claim {
        let owns = !self.mounted.replace(true);
        if !owns {
            contract_violation!(
                "a CanvasSize is given to a second canvas while another canvas laid out into it is still mounted"
            );
        }
        Claim {
            mounted: Rc::clone(&self.mounted),
            owns,
        }
    }
}

impl fmt::Debug for CanvasSize {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CanvasSize")
            .field("size", &self.cell.snapshot())
            .field("mounted", &self.mounted.get())
            .finish()
    }
}

impl Default for CanvasSize {
    fn default() -> Self {
        Self::new()
    }
}

/// A mounted canvas's hold on its [`CanvasSize`], released on unmount.
pub struct Claim {
    mounted: Rc<Cell<bool>>,
    /// Whether this claim set the flag; a second claim, reported as a
    /// violation, leaves the first mount's hold alone.
    owns: bool,
}

impl Drop for Claim {
    fn drop(&mut self) {
        if self.owns {
            self.mounted.set(false);
        }
    }
}
