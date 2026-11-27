//! The `link` leaf: `Native<LinkConfig>` realized with `ButtonStyle::Link`
//! appearance, pointer-hand cursor, `AccessibilityRole::Link`, and URL
//! activation through `waterkit-deeplink`.

use crate::renderer::{
    HeldChild, HydroState, HydrolysisRenderer, InteractionKey, RetainedSubview, WidgetRenderContext,
};
#[cfg(feature = "accessibility")]
use crate::renderer::{RenderContext, SemanticCore};
use crate::text::TextEngine;
use crate::theme::WidgetTheme;
#[cfg(feature = "accessibility")]
use crate::widgets::controls::button::control_accessibility;
use crate::widgets::controls::button::{
    ButtonChromeState, ControlPresentation, ensure_control_label_built, init_control_label,
    measure_control_node, render_control_parts,
};
#[cfg(feature = "accessibility")]
use accesskit::Role as AccessibilityNodeRole;
use executor_core::spawn_local;
use std::cell::RefCell;
use std::rc::Rc;
use waterui_controls::ControlSize;
use waterui_controls::button::ButtonStyle;
use waterui_controls::link::LinkConfig;
use waterui_core::Environment;
use waterui_core::Signal as _;
use waterui_core::layout::{ProposalSize, ViewDimensions};
use waterui_interaction::cursor::CursorStyle;

const LINK_PRESENTATION: ControlPresentation = ControlPresentation {
    style: ButtonStyle::Link,
    size: ControlSize::Small,
    role: None,
};

/// Opens a URL in the system's default handler.
fn open_url(url: &str) {
    let url = url.to_owned();
    spawn_local(async move {
        if let Err(error) = waterkit_deeplink::open_url(&url).await {
            tracing::error!("Failed to open URL '{}': {:?}", url, error);
        }
    })
    .detach();
}

/// Retained render state for a native [`LinkConfig`] leaf.
pub struct LinkRenderState<E: TextEngine> {
    config: LinkConfig<HeldChild<E>>,
    label_view: Option<RetainedSubview<E>>,
    chrome: ButtonChromeState,
}

impl<E: TextEngine> LinkRenderState<E> {
    #[cfg(feature = "accessibility")]
    pub(crate) fn contributed_label(
        &self,
        renderer: &mut SemanticCore<E>,
        env: &Environment,
    ) -> Option<String> {
        renderer.accessibility_label_from_label(self.config.label(), env)
    }

    pub(crate) fn from_config(config: LinkConfig<HeldChild<E>>) -> Self {
        Self {
            config,
            label_view: None,
            chrome: ButtonChromeState::new(),
        }
    }

    pub(crate) fn take_layout_dirty(&mut self) -> bool {
        self.label_view
            .as_mut()
            .is_some_and(RetainedSubview::take_layout_dirty_if_built)
    }

    pub(crate) fn init_label(&mut self) {
        self.label_view = init_control_label(self.config.label());
    }

    pub(crate) fn ensure_label_built(
        &mut self,
        renderer: &mut HydrolysisRenderer<E>,
        env: &Environment,
    ) {
        ensure_control_label_built(
            renderer,
            &mut self.label_view,
            &mut self.chrome,
            self.config.label(),
            LINK_PRESENTATION,
            env,
        );
    }
}

/// Emits a link's accessibility node from its retained state.
#[cfg(feature = "accessibility")]
pub fn link_accessibility<E: TextEngine>(
    renderer: &mut SemanticCore<E>,
    ctx: Option<RenderContext>,
    state: &Rc<RefCell<LinkRenderState<E>>>,
    env: &Environment,
) {
    let label = state.borrow().config.label().clone();
    let activate_state = Rc::clone(state);
    control_accessibility(
        renderer,
        ctx,
        state,
        &label,
        (AccessibilityNodeRole::Link, None),
        None,
        env,
        move |_renderer, _env| {
            let target_url = activate_state.borrow().config.url().snapshot();
            open_url(target_url.as_str());
            true
        },
    );
}

/// Measures a retained link leaf from its [`LinkRenderState`].
pub fn measure_link_node<E: TextEngine>(
    render_state: &LinkRenderState<E>,
    proposal: ProposalSize,
    state: &mut HydroState<E>,
    env: &Environment,
    theme: &Rc<dyn WidgetTheme>,
) -> ViewDimensions {
    measure_control_node(
        render_state.config.label(),
        render_state.label_view.as_ref(),
        &render_state.chrome,
        LINK_PRESENTATION,
        (proposal, state, env, theme),
    )
}

/// Renders a retained link leaf every flush: emits accessibility, paints the link
/// chrome and label, and registers pointing-hand cursor and URL activation targets.
pub fn render_link_node<E: TextEngine>(
    ctx: &mut WidgetRenderContext<'_, E>,
    state: &Rc<RefCell<LinkRenderState<E>>>,
    env: &Environment,
) {
    #[cfg(feature = "accessibility")]
    if !ctx.renderer_mut().is_accessibility_hidden(env) {
        let render_ctx = ctx.render_context();
        link_accessibility(ctx.renderer_mut(), Some(render_ctx), state, env);
    }
    let key = InteractionKey::for_rc(state, 0);
    let active_target = {
        let mut state_mut = state.borrow_mut();
        let state_mut = &mut *state_mut;
        render_control_parts(
            ctx,
            &mut state_mut.chrome,
            &mut state_mut.label_view,
            state_mut.config.label(),
            LINK_PRESENTATION,
            key,
            None,
            env,
        )
    };
    let Some((target, press_slot, keyboard_focusable)) = active_target else {
        return;
    };
    ctx.renderer_mut()
        .register_cursor_target(target, CursorStyle::PointingHand);
    let state = Rc::clone(state);
    ctx.renderer_mut()
        .register_interactive_pointer_target_with_keyboard(
            target,
            press_slot,
            keyboard_focusable,
            move |_renderer, _point, _env| {
                let target_url = state.borrow().config.url().snapshot();
                open_url(target_url.as_str());
                true
            },
        );
}

/// Emits a retained link's accessibility node for the semantic walk.
#[cfg(feature = "accessibility")]
#[cfg(hydrolysis_cherenkov)]
pub fn emit_link_accessibility<E: TextEngine>(
    renderer: &mut SemanticCore<E>,
    state: &Rc<RefCell<LinkRenderState<E>>>,
    env: &Environment,
) {
    if renderer.is_accessibility_hidden(env) {
        return;
    }
    link_accessibility(renderer, None, state, env);
}
