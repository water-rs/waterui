//! `.floating()` realization: the theme's [`FloatingLook`](crate::theme::FloatingLook)
//! (or a scoped [`FloatingOverride`](crate::theme::FloatingOverride)) rendered
//! as a live elevated surface behind clipped content.
//!
//! Elevation shadows, surface fill or backdrop material, and optional border
//! record once as live chrome operands beneath the content; interaction state
//! reported by the outermost interactive descendant via `InteractionReport`
//! updates those operands synchronously without rebuilding or patching the
//! render tree. Content flushes inside a live clip scope matching the surface's
//! silhouette so shadows extend freely outside the rounded surface while child
//! content stays clipped to it.

use std::any::Any;
use std::cell::{OnceCell, RefCell};
use std::rc::Rc;

use nami::{Binding, Signal as _, binding};
use waterui_core::Environment;
use waterui_graphics::color::Color;
use waterui_interaction::interaction::InteractionState;

use crate::renderer::chrome::LiveChromeParts;
use crate::renderer::{HydrolysisRenderer, RenderContext};
use crate::text::TextEngine;
use crate::theme::{
    Chrome, FloatingOverride, FocusIndicator, PressFeedback, StateLayer, WidgetTheme,
};

/// The node-owned state of a `.floating()` wrapper.
pub struct FloatingEffect {
    /// Scoped override captured at build time, if any; `None` queries
    /// [`WidgetTheme::floating`].
    override_surface: Option<FloatingOverride>,
    /// Interaction state reported by the outermost interactive control inside
    /// this floating surface.
    reported_state: Binding<InteractionState>,
    /// The surface-only chrome resolved on first draw.
    chrome: OnceCell<Chrome>,
    /// The chrome's live parts, updated in place as bounds or interaction
    /// state change.
    parts: RefCell<Option<LiveChromeParts>>,
    /// Guard keeping the synchronous `reported_state` -> `LiveChrome::sync`
    /// watcher alive for the lifetime of this wrapper node.
    state_guard: RefCell<Option<Box<dyn Any>>>,
    /// Unused report binding required by [`LiveChromeParts::ensure_following`].
    chrome_report: Binding<InteractionState>,
}

impl FloatingEffect {
    pub(crate) fn new(
        override_surface: Option<FloatingOverride>,
        reported_state: Binding<InteractionState>,
    ) -> Self {
        Self {
            override_surface,
            reported_state,
            chrome: OnceCell::new(),
            parts: RefCell::new(None),
            state_guard: RefCell::new(None),
            chrome_report: binding(InteractionState::empty()),
        }
    }

    fn resolve_chrome(&self, theme: &dyn WidgetTheme, env: &Environment) -> Chrome {
        let look = self.override_surface.as_ref().map_or_else(
            || theme.floating(env),
            |override_surface| override_surface.resolve(env),
        );
        Chrome::new(
            look.surface().clone(),
            StateLayer::new(Color::transparent()),
            PressFeedback::new(),
            FocusIndicator::Tint,
            look.motion().clone(),
        )
    }
}

impl<E: TextEngine> HydrolysisRenderer<E> {
    /// Draws the floating surface's shadows and fill behind the content, then
    /// flushes `render_content` clipped to the surface's live silhouette.
    pub(super) fn apply_floating(
        renderer: &mut Self,
        ctx: RenderContext,
        env: &Environment,
        effect: &FloatingEffect,
        render_content: impl FnOnce(&mut Self),
    ) {
        let theme = renderer.theme();
        let chrome = effect
            .chrome
            .get_or_init(|| effect.resolve_chrome(&*theme, env));
        let bounds = ctx.bounds();
        let state = effect.reported_state.snapshot();
        let parts = LiveChromeParts::ensure_following(
            &mut effect.parts.borrow_mut(),
            chrome,
            bounds,
            state,
            env,
            &effect.chrome_report,
        )
        .clone();
        let mut guard_slot = effect.state_guard.borrow_mut();
        if guard_slot.is_none() {
            let live = Rc::clone(parts.chrome());
            *guard_slot = Some(Box::new(effect.reported_state.watch(move |change| {
                live.sync(*change.value());
            })));
        }
        drop(guard_slot);
        parts.push_below(renderer, ctx);
        parts.wrap_clipped_content(renderer, ctx, render_content);
    }
}
