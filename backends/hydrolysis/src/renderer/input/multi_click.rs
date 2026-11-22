//! Multi-click counting: one tracker for every interaction target that wants
//! to know which click of a run a press is.
//!
//! The native text targets and the embedded surfaces count through the same
//! [`MultiClickTracker`], with the platform's double-click interval and
//! distance ([`MultiClickConfig`]). Counting is uncapped; a consumer that has
//! a ceiling of its own — word and line selection stop at a triple click —
//! applies it to the count it reads.

use core::num::NonZeroU32;

use crate::platform::{InputConventions, MultiClickConfig};
use crate::time::Instant;

use super::{InteractionKey, PointerButton, SemanticCore};

/// The press a run continues from.
#[derive(Debug, Clone)]
struct Press {
    target: InteractionKey,
    button: PointerButton,
    point: kurbo::Point,
    at: Instant,
    count: NonZeroU32,
}

/// Counts presses into multi-click runs.
///
/// A press continues the previous press's run when it lands on the same
/// target with the same button within the configured interval and distance of
/// it; any other press starts a new run at one. The run is remembered by the
/// target's stable interaction identity, so it continues across a reflow and
/// never carries over to a different target that happens to land at the same
/// position.
#[derive(Debug, Default)]
pub struct MultiClickTracker {
    last: Option<Press>,
}

impl MultiClickTracker {
    /// Registers a press of `button` on `target` at the window-space `point`
    /// and answers which click of its run it is.
    pub(crate) fn press(
        &mut self,
        config: MultiClickConfig,
        target: &InteractionKey,
        button: PointerButton,
        point: kurbo::Point,
        at: Instant,
    ) -> NonZeroU32 {
        let count = match self.last.as_ref() {
            Some(previous)
                if previous.target == *target
                    && previous.button == button
                    && config.continues(
                        at.saturating_duration_since(previous.at),
                        point - previous.point,
                    ) =>
            {
                previous.count.saturating_add(1)
            }
            _ => NonZeroU32::MIN,
        };
        self.last = Some(Press {
            target: target.clone(),
            button,
            point,
            at,
            count,
        });
        count
    }
}

impl<E: crate::text::TextEngine> SemanticCore<E> {
    /// Registers a press on the interaction target `target` against the
    /// platform's current multi-click settings and answers which click of its
    /// run it is.
    pub(crate) fn next_click_count(
        &mut self,
        target: &InteractionKey,
        button: PointerButton,
        point: kurbo::Point,
        at: Instant,
    ) -> NonZeroU32 {
        let config = self.hit_test.input_conventions.multi_click;
        self.hit_test
            .multi_click
            .press(config, target, button, point, at)
    }

    /// The platform's current input conventions, carried by value from
    /// [`PlatformWindow::input_conventions`](crate::platform::PlatformWindow::input_conventions)
    /// each input dispatch so a setting the user changes applies to the next
    /// press.
    pub(crate) const fn set_input_conventions(&mut self, conventions: InputConventions) {
        self.hit_test.input_conventions = conventions;
    }
}
