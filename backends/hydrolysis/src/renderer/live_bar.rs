//! Hydrolysis's realization of a navigation bar's surface and separator
//! from a [`NavigationBarLook`]: the parts recorded once, while the bar's
//! frame and colour hold.

use std::cell::RefCell;
use std::rc::Rc;

use cherenkov_record::{Recorder, WorkingColor};
use kurbo::Rect;
use waterui_core::interaction::InteractionState;

use crate::renderer::chrome::{LayeredParts, PartsBody};
use crate::renderer::{HydrolysisRenderer, RenderContext};
use crate::theme::{BarSurface, Inputs, NavigationBarLook, RealizedCustom};

/// Where a navigation bar's parts lie, in the widget's record space.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BarFrames {
    /// The bar's surface, extended through the safe-area regions of every
    /// edge it touches.
    pub surface: Rect,
    /// The line under the bar, when the bar has one.
    pub separator: Option<Rect>,
}

/// One bar's parts: the surface and, when the look draws one and the bar has
/// a line, the separator.
struct LiveBar {
    frames: BarFrames,
    color: WorkingColor,
    surface: RealizedCustom<BarSurface>,
    separator: Option<RealizedCustom<BarSurface>>,
}

impl LiveBar {
    fn new(look: &NavigationBarLook, frames: BarFrames, color: WorkingColor) -> Self {
        let value = BarSurface { color };
        let at_rest = |frame: Rect| Inputs::at_rest(frame, InteractionState::empty(), &value);
        Self {
            surface: look.bar().realize(&at_rest(frames.surface)),
            separator: look
                .separator_part()
                .zip(frames.separator)
                .map(|(part, line)| part.realize(&at_rest(line))),
            frames,
            color,
        }
    }

    fn record(&self, recorder: &mut Recorder) {
        self.surface.record(recorder, self.frames.surface);
        if let Some((separator, line)) = self.separator.as_ref().zip(self.frames.separator) {
            separator.record(recorder, line);
        }
    }
}

/// A navigation bar's parts and the recording its program lists: the
/// identity the mount keeps them by.
#[derive(Clone)]
pub struct LiveBarParts {
    bar: Rc<LiveBar>,
    layers: Rc<RefCell<Option<LayeredParts>>>,
}

impl LiveBarParts {
    /// The bar of `look` on `frames` in `color`: the parts in `slot` when
    /// they already realize it, otherwise fresh parts, replacing them.
    /// `slot` is emptied by the caller whenever the look is requested again.
    pub(crate) fn ensure<'a>(
        slot: &'a mut Option<Self>,
        look: &NavigationBarLook,
        frames: BarFrames,
        color: WorkingColor,
    ) -> &'a Self {
        if !slot
            .as_ref()
            .is_some_and(|parts| parts.bar.frames == frames && parts.bar.color == color)
        {
            *slot = Some(Self {
                bar: Rc::new(LiveBar::new(look, frames, color)),
                layers: Rc::default(),
            });
        }
        slot.as_ref().expect("the bar parts were just ensured")
    }

    /// Lists the bar's surface and separator in `ctx`.
    pub(crate) fn push<E: crate::text::TextEngine>(
        &self,
        renderer: &mut HydrolysisRenderer<E>,
        ctx: RenderContext,
    ) {
        let scope = renderer.material_scope();
        let mut layers = self.layers.borrow_mut();
        let bar = Rc::clone(&self.bar);
        LayeredParts::ensure(&mut layers, scope, move || -> PartsBody {
            Rc::new(move |recorder: &mut Recorder| bar.record(recorder))
        })
        .push(renderer, ctx);
    }
}
