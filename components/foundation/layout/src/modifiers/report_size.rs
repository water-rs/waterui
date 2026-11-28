//! Placement size reporting for layout views.

use nami::Binding;
use waterui_core::layout::Size;
use waterui_core::metadata::MetadataKey;

/// Metadata key carrying the [`Binding<Size>`] a backend writes this view's
/// committed placed size into after each layout pass.
///
/// Reporting observes the result of layout and adds no layout rule: the
/// backend writes into the binding only when the committed size changes, and
/// never reads it back into layout proposals.
#[derive(Clone, Debug)]
pub struct ReportSize {
    size: Binding<Size>,
}

impl MetadataKey for ReportSize {}

impl ReportSize {
    /// Creates a new `ReportSize` that writes this view's committed placed size
    /// into `size`.
    #[must_use]
    pub fn new(size: &Binding<Size>) -> Self {
        Self { size: size.clone() }
    }

    /// Returns the binding that receives this view's committed placed size.
    #[must_use]
    pub const fn binding(&self) -> &Binding<Size> {
        &self.size
    }

    /// Writes `placed` into the bound [`Binding<Size>`] if it differs from the
    /// binding's current value, avoiding spurious watcher notifications on
    /// unchanged frames.
    pub fn report(&self, placed: Size) {
        if nami::Signal::snapshot(&self.size) != placed {
            self.size.set(placed);
        }
    }
}

#[cfg(test)]
mod tests {
    extern crate std;

    use core::cell::Cell;
    use std::rc::Rc;

    use nami::Signal;

    use super::*;

    #[test]
    fn report_writes_only_when_size_changes() {
        let writes = Rc::new(Cell::new(0usize));
        let size = Binding::new(Size::zero()).filter({
            let writes = Rc::clone(&writes);
            move |_| {
                writes.set(writes.get() + 1);
                true
            }
        });
        let reporter = ReportSize::new(&size);

        reporter.report(Size::zero());
        assert_eq!(writes.get(), 0, "unchanged initial zero must not write");

        let first = Size::new(320.0, 200.0);
        reporter.report(first);
        assert_eq!(size.snapshot(), first);
        assert_eq!(writes.get(), 1);

        reporter.report(first);
        assert_eq!(writes.get(), 1, "identical placed size must not re-write");

        let resized = Size::new(640.0, 200.0);
        reporter.report(resized);
        assert_eq!(size.snapshot(), resized);
        assert_eq!(writes.get(), 2);
    }
}
