//! `.background(Glass…)` (water-rs/waterui#4495): the theme's
//! [`GlassLook`](crate::theme::GlassLook) realized behind the content.
//!
//! The look's material, border and elevation are the surface of a chrome the
//! renderer realizes like any other — recorded once behind the content with
//! the shape, the effect, the border and the elevation as live operands, the
//! material joining the nearest enclosing `.material_group()` scope. The
//! chrome's outline is the glass's shape on the content's frame, and it
//! follows the frame as it changes with no re-recording. The glass registers
//! no hit region and no pointer target: it takes neither taps nor keyboard
//! focus, so the content and the views beneath it receive their input as if
//! there were no glass. An interactive glass registers one pointer observer
//! ([`PointerObserver`](crate::renderer::PointerObserver)) instead, which
//! hears the primary press without taking part in dispatch.

use std::cell::{OnceCell, RefCell};
use std::rc::Rc;

use kurbo::RoundedRectRadii;
use waterui::background::Glass;
use waterui_core::Environment;
use waterui_core::interaction::InteractionState;
use waterui_graphics::color::Color;
use waterui_shape::{ClipShape, ShapeKind};

use crate::renderer::chrome::LiveChromeParts;
use crate::renderer::{HydrolysisRenderer, InteractionKey, RenderContext, shape_kind_radii};
use crate::theme::{
    Chrome, CornerShape, Corners, Driven, FocusIndicator, GlassLook, Inputs, PressFeedback,
    StateLayer, Surface, WidgetTheme,
};

/// The node-owned state of a `.background(Glass…)` wrapper.
pub struct GlassEffect {
    glass: Glass,
    /// The theme's look of the glass as a chrome, requested the first time
    /// the wrapper draws; `None` once the theme answered it draws no glass.
    chrome: OnceCell<Option<Chrome>>,
    /// The chrome's live parts, kept across flushes while the chrome stays
    /// the same: a frame change moves them through their operands.
    parts: RefCell<Option<LiveChromeParts>>,
    /// The state the chrome reports; nothing reads it.
    state: nami::Binding<InteractionState>,
    /// The identity an interactive glass's interaction state is keyed by.
    identity: Rc<()>,
}

impl GlassEffect {
    pub(crate) fn new(glass: Glass) -> Self {
        Self {
            glass,
            chrome: OnceCell::new(),
            parts: RefCell::new(None),
            state: nami::binding(InteractionState::empty()),
            identity: Rc::new(()),
        }
    }

    /// The chrome `theme` makes of the glass in `env`; `None` when it draws
    /// no glass.
    fn resolve(&self, theme: &dyn WidgetTheme, env: &Environment) -> Option<Chrome> {
        theme
            .glass_background(&self.glass, env)
            .map(|look| chrome_of(&look, corner_shape(self.glass.outline())))
    }
}

/// The silhouette of `outline` on the chrome's frame, matching how
/// Hydrolysis resolves a shape kind everywhere else ([`shape_kind_radii`]);
/// a custom path resolves its commands against the frame the way a
/// `ClipShape` does (`path_commands_to_path`).
fn corner_shape(outline: &ClipShape) -> CornerShape {
    let kind = outline.kind();
    match kind {
        ShapeKind::Circle => CornerShape::circle(),
        ShapeKind::Ellipse => CornerShape::ellipse(),
        ShapeKind::CustomPath => CornerShape::path(outline.commands()),
        _ => CornerShape::new(Driven::new(move |inputs: &Inputs<'_, ()>| {
            let frame = inputs.frame;
            let min_side = crate::num_cast::f64_as_f32(frame.width().min(frame.height()).max(0.0));
            let [top_left, top_right, bottom_right, bottom_left] = shape_kind_radii(kind, min_side);
            Corners::Radii(RoundedRectRadii::new(
                f64::from(top_left),
                f64::from(top_right),
                f64::from(bottom_right),
                f64::from(bottom_left),
            ))
        })),
    }
}

/// The chrome whose surface is `look` on `shape`: no state layer, press
/// feedback or focus indicator, and no hit region — the glass takes no
/// input.
fn chrome_of(look: &GlassLook, shape: CornerShape) -> Chrome {
    let mut surface =
        Surface::new(shape, look.material().clone()).elevation(look.get_elevation().clone());
    if let Some(border) = look.get_border() {
        surface = surface.border(border.clone());
    }
    Chrome::new(
        surface,
        StateLayer::new(Color::transparent()),
        PressFeedback::new(),
        FocusIndicator::Tint,
        look.motion().clone(),
    )
}

impl<E: crate::text::TextEngine> HydrolysisRenderer<E> {
    /// Lists the theme's glass behind the content `render_content` flushes.
    pub(super) fn apply_glass(
        renderer: &mut Self,
        ctx: RenderContext,
        env: &Environment,
        effect: &GlassEffect,
        render_content: impl FnOnce(&mut Self),
    ) {
        let theme = renderer.theme();
        if let Some(chrome) = effect.chrome.get_or_init(|| effect.resolve(&*theme, env)) {
            let bounds = ctx.bounds();
            let (state, handles) = if effect.glass.is_interactive() {
                // The bind sees no ambient scopes: a glass is not the
                // outermost interactive control a `.interaction_state`
                // report or a `Selected` scope belongs to, and joins no
                // modal or focus scope — it never takes focus.
                let (state, slot, handles) = renderer.bind_live_chrome_target(
                    InteractionKey::for_rc(&effect.identity, 0),
                    bounds,
                    &Environment::new(),
                    false,
                    chrome,
                );
                renderer.register_pointer_observer(bounds, slot, Rc::clone(&handles));
                (state.state, Some(handles))
            } else {
                (InteractionState::empty(), None)
            };
            let parts = LiveChromeParts::ensure_following(
                &mut effect.parts.borrow_mut(),
                chrome,
                bounds,
                state,
                env,
                &effect.state,
            )
            .clone();
            if let Some(handles) = handles {
                handles.attach_live(Rc::clone(parts.chrome()));
            }
            parts.push_below(renderer, ctx);
        }
        render_content(renderer);
    }
}
