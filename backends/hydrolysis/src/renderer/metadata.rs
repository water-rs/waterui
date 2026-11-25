//! Metadata view handlers: styling, transforms, interaction, lifecycle
//! and accessibility metadata wrappers around content views.

// glob import of the module vocabulary — the renderer internals are designed to be used wholesale
#[allow(clippy::wildcard_imports)]
use super::*;
use crate::num_cast::f64_as_f32;
#[cfg(feature = "menu")]
use crate::renderer::chrome::record_surface_at_rest;
use crate::renderer::frame::ScopeClip;
#[cfg(feature = "menu")]
use crate::renderer::input::{anchored_overlay_scope, menu_look, presentation_content_transform};
#[cfg(feature = "menu")]
use crate::renderer::mount::ScopeKey;
#[cfg(feature = "menu")]
use crate::renderer::tree::transition::record_with_blur_radius;
#[cfg(feature = "menu")]
use crate::theme::{MenuLook, MenuPresentationSample};
#[cfg(feature = "menu")]
use cherenkov_record::PATH_TOLERANCE;
#[cfg(feature = "menu")]
use kurbo::{Affine, Rect, RoundedRect, Shape as _, Vec2};
use waterui_core::contract_violation;
use waterui_core::handler::ReportExtraction as _;
use waterui_core::layout::Point;
use waterui_interaction::gesture::{PointerButton, TapEvent};
use waterui_shape::CornerRadius;

#[cfg(feature = "menu")]
fn presentation_surface_clip(sample: &MenuPresentationSample) -> ScopeClip {
    if sample.surface_corner_radius > 0.0 {
        ScopeClip::RoundedRect {
            path: RoundedRect::from_rect(sample.surface, sample.surface_corner_radius)
                .to_path(PATH_TOLERANCE),
            rect: sample.surface,
        }
    } else {
        ScopeClip::Rect(sample.surface)
    }
}

/// The claim a tap gesture registered for the wrapped view: the node it
/// stands for, the environment it was registered under — carrying the naming
/// scope the claim drains — and whether the claim synthesized that scope
/// itself.
///
/// A synthesized claim is the gesture's own representative for content that
/// named nothing (a bare `.on_tap` over `text("T")`, water-rs/waterui#1070):
/// the wrapped node takes the gesture's actions and activation in the
/// post-walk single-child collapse. An explicit `a11y_role` claim is the
/// element the author declared, so it never collapses.
#[cfg(feature = "accessibility")]
#[cfg(hydrolysis_cherenkov)]
pub(super) struct GestureA11yClaim {
    /// The registered claim node.
    pub(super) node_id: AccessibilityNodeId,
    /// The environment the node was registered under — the view's own scope
    /// when it carried one, otherwise the claim's synthesized scope.
    pub(super) env: Environment,
    /// Whether the claim's naming scope was synthesized for a bare tap
    /// (`true`) or came from the view's own accessibility metadata
    /// (`false`).
    pub(super) synthesized: bool,
}

impl<E: crate::text::TextEngine> HydrolysisRenderer<E> {
    /// Apply a clip-shape prop on the recording node's own frame layer (§A.1):
    /// the wrapper's `frame` carries the clip, not a `("clip", 0)` scope layer.
    /// Callers must be layer nodes — a pass-through would clip the enclosing
    /// node's whole program.
    pub(super) fn apply_clip_shape(
        renderer: &mut Self,
        ctx: RenderContext,
        value: &ClipShape,
        render_content: impl FnOnce(&mut Self),
    ) {
        // Resolve from the structured kind, exactly as a fill of the same shape
        // does, and fall back to the unit-space commands only for a custom path.
        // The commands are normalized per axis, so resolving them against a
        // non-square rect makes every circular corner elliptical.
        let clip_path = shape_kind_path(value.kind(), ctx.bounds())
            .unwrap_or_else(|| path_commands_to_path(value.commands(), ctx.bounds()));
        let clip = if let Some(regular_clip) = kind_clip_shape(value.kind(), ctx.bounds())
            .or_else(|| regular_clip_shape(value.commands(), ctx.bounds()))
        {
            match regular_clip {
                RegularClipShape::Rect(rect) => ScopeClip::Rect(rect),
                RegularClipShape::RoundedRect { rect, .. } => ScopeClip::RoundedRect {
                    path: clip_path,
                    rect,
                },
            }
        } else {
            ScopeClip::Path(clip_path)
        };
        // The layer clip in the frame's content space — the node's own
        // bounds — with the conservative hit bound the placement scope
        // used to carry now on the node's placement itself.
        renderer.program().program_mut().clip = Some(clip.in_space(kurbo::Affine::IDENTITY));
        renderer
            .current_placement()
            .set_clip(Some(clip.hit_bounds()));
        render_content(renderer);
    }

    /// Render the given content then stroke the border over it, mirroring the
    /// historical order (content first, border on top). Shared by the dispatch
    /// handler and the retained `Wrapper` node. The border color resolves against
    /// `env`, so it is threaded through.
    pub(super) fn apply_border(
        renderer: &mut Self,
        ctx: RenderContext,
        env: &Environment,
        border: &Border,
        render_content: impl FnOnce(&mut Self),
    ) {
        render_content(renderer);

        if border.width() <= 0.0 {
            return;
        }

        let paint = || Paint::Solid(border.color().resolve(env).snapshot());
        let width = f64::from(border.width());

        if border.get_edges().all() && border.get_corner_radius() > 0.0 {
            let rounded =
                kurbo::RoundedRect::from_rect(ctx.bounds(), f64::from(border.get_corner_radius()));
            let stroke = kurbo::Stroke::new(width);
            renderer
                .scene_mut()
                .stroke_paint(&stroke, ctx.local(), paint(), &rounded);
            return;
        }

        if border.get_edges().top {
            let top = kurbo::Rect::new(
                ctx.bounds().x0,
                ctx.bounds().y0,
                ctx.bounds().x1,
                ctx.bounds().y0 + width,
            );
            renderer
                .scene_mut()
                .fill_paint(peniko::Fill::NonZero, ctx.local(), paint(), &top);
        }
        if border.get_edges().bottom {
            let bottom = kurbo::Rect::new(
                ctx.bounds().x0,
                ctx.bounds().y1 - width,
                ctx.bounds().x1,
                ctx.bounds().y1,
            );
            renderer
                .scene_mut()
                .fill_paint(peniko::Fill::NonZero, ctx.local(), paint(), &bottom);
        }
        if border.get_edges().leading {
            let leading = kurbo::Rect::new(
                ctx.bounds().x0,
                ctx.bounds().y0,
                ctx.bounds().x0 + width,
                ctx.bounds().y1,
            );
            renderer
                .scene_mut()
                .fill_paint(peniko::Fill::NonZero, ctx.local(), paint(), &leading);
        }
        if border.get_edges().trailing {
            let trailing = kurbo::Rect::new(
                ctx.bounds().x1 - width,
                ctx.bounds().y0,
                ctx.bounds().x1,
                ctx.bounds().y1,
            );
            renderer
                .scene_mut()
                .fill_paint(peniko::Fill::NonZero, ctx.local(), paint(), &trailing);
        }
    }

    /// Draw the shadow first, then render the given content over it (matching the
    /// historical order). Shared by the dispatch handler and the retained
    /// `Wrapper` node. The shadow color resolves against `env`.
    pub(super) fn apply_shadow(
        renderer: &mut Self,
        ctx: RenderContext,
        env: &Environment,
        shadow: &Shadow,
        render_content: impl FnOnce(&mut Self),
    ) {
        let blur = f64::from(shadow.radius().max(0.0));
        let offset_x = f64::from(shadow.offset().x);
        let offset_y = f64::from(shadow.offset().y);
        let shadow_rect = kurbo::Rect::new(
            ctx.bounds().x0 + offset_x,
            ctx.bounds().y0 + offset_y,
            ctx.bounds().x1 + offset_x,
            ctx.bounds().y1 + offset_y,
        );
        let shadow_color = shadow.color().resolve(env).snapshot();

        // The silhouette states the caster's shape. `kind_clip_shape` — the
        // same resolver a clip uses to decide between the uniform rounded-rect
        // fast path and the general path route — answers whether the shadow can
        // feed the engine's blurred-rounded-rect primitive directly.
        let silhouette = &shadow.silhouette();
        let uniform_radius = match kind_clip_shape(silhouette.kind(), shadow_rect) {
            Some(RegularClipShape::RoundedRect { corner_width, .. }) => Some(corner_width),
            Some(RegularClipShape::Rect(_)) => Some(0.0),
            None => None,
        };
        match uniform_radius {
            Some(corner_radius) => renderer.scene_mut().blurred_rounded_rect(
                ctx.local(),
                shadow_rect,
                shadow_color,
                corner_radius,
                blur,
            ),
            None => Self::draw_blurred_silhouette(
                renderer,
                ctx.local(),
                silhouette,
                shadow_rect,
                shadow_color,
                blur,
            ),
        }
        render_content(renderer);
    }

    /// Draw a non-rounded-rect silhouette's blurred shadow into `rect` — the
    /// general route for silhouettes `kind_clip_shape` cannot express as a
    /// uniform rounded rect (ellipse, non-square circle, uneven corners,
    /// custom path). The engine rasterizes and caches the blur; Hydrolysis
    /// keeps no pixmap cache of its own.
    fn draw_blurred_silhouette(
        renderer: &mut Self,
        transform: kurbo::Affine,
        silhouette: &ClipShape,
        rect: kurbo::Rect,
        color: WorkingColor,
        blur: f64,
    ) {
        // The same resolution `apply_clip_shape` performs — structured kind
        // first, unit-space commands only for a custom path — but against the
        // rect normalized to the origin: the rect's own position is applied
        // at draw time.
        let local_rect = kurbo::Rect::new(0.0, 0.0, rect.width(), rect.height());
        let local_path = shape_kind_path(silhouette.kind(), local_rect)
            .unwrap_or_else(|| path_commands_to_path(silhouette.commands(), local_rect));
        let placement = transform * kurbo::Affine::translate((rect.x0, rect.y0));

        if blur <= 0.0 {
            renderer.scene_mut().fill_paint(
                peniko::Fill::NonZero,
                placement,
                Paint::Solid(color),
                &local_path,
            );
            return;
        }

        // `blur` is already in device pixels; the engine scales `sigma` by
        // the transform's axis length, so the path arrives pre-transformed
        // and the op transform is identity.
        renderer.scene_mut().shadow(
            kurbo::Affine::IDENTITY,
            &(placement * &local_path),
            blur,
            color,
        );
    }

    /// Draw the theme's context-menu panel behind the wrapped menu rows,
    /// then render the content over it. This is the popup-window menu's
    /// surface ([`WrapperEffect::PopupMenuSurface`]): on Material 3 themes it
    /// is `md.comp.menu.container.color` (`surface-container`), the extra-small
    /// 4 dp `md.sys.shape.corner.extra-small` shape and
    /// `md.comp.menu.container.elevation` level 2 — drawn by the panel part
    /// of the theme's `MenuLook`, recorded as the surface it states, with the
    /// look's selected-row part over `selected_row` (a picker popup's
    /// selected row, in the panel's own space). The window leaves the
    /// panel's shadow room inside its own bounds: the panel's elevation
    /// extent (`popup_menu_panel_margin`) on every side.
    #[cfg(feature = "menu")]
    pub(super) fn apply_popup_menu_surface(
        renderer: &mut Self,
        ctx: RenderContext,
        env: &Environment,
        selected_row: Option<Rect>,
        render_content: impl FnOnce(&mut Self, RenderContext),
    ) {
        let look = menu_look(&renderer.theme(), env);
        let active = renderer.popup_menu.active_menu_presentation.take();
        let Some(active) = active else {
            let rest = MenuPresentationSample::at_rest(ctx.bounds(), ctx.bounds());
            renderer.draw_context(ctx, |draw| {
                if !look.record_presentation_panel(draw, &rest) {
                    record_surface_at_rest(look.panel(), draw, ctx.bounds());
                }
                if let Some(row) = selected_row {
                    let bounds = ctx.bounds();
                    look.record_selected_row(
                        draw,
                        row.with_origin((bounds.x0 + row.x0, bounds.y0 + row.y0)),
                    );
                }
            });
            render_content(renderer, ctx);
            return;
        };

        let panel_window = renderer.resolve_window_rect(ctx.bounds());
        let offset = Vec2::new(
            ctx.bounds().x0 - panel_window.x0,
            ctx.bounds().y0 - panel_window.y0,
        );
        let local_sample = active.sample.translated(offset);
        renderer.draw_context(ctx, |draw| {
            if !look.record_presentation_panel(draw, &local_sample) {
                record_surface_at_rest(look.panel(), draw, local_sample.surface);
            }
        });

        let at_rest = (local_sample.progress - 1.0).abs() <= 1e-4
            && (local_sample.vertical_progress - 1.0).abs() <= 1e-4
            && (local_sample.content_opacity - 1.0).abs() <= 1e-4
            && (local_sample.content_scale - 1.0).abs() <= 1e-4
            && local_sample.content_blur <= 0.01;
        if at_rest {
            if let Some(row) = selected_row {
                let bounds = ctx.bounds();
                renderer.draw_context(ctx, |draw| {
                    look.record_selected_row(
                        draw,
                        row.with_origin((bounds.x0 + row.x0, bounds.y0 + row.y0)),
                    );
                });
            }
            render_content(renderer, ctx);
            return;
        }

        Self::record_morphed_popup_menu_content(
            renderer,
            ctx,
            &look,
            &local_sample,
            &active,
            selected_row,
            render_content,
        );
    }

    #[cfg(feature = "menu")]
    fn record_morphed_popup_menu_content(
        renderer: &mut Self,
        ctx: RenderContext,
        look: &MenuLook,
        local_sample: &MenuPresentationSample,
        active: &ActiveMenuPresentation<E>,
        selected_row: Option<Rect>,
        render_content: impl FnOnce(&mut Self, RenderContext),
    ) {
        let content_transform = presentation_content_transform(
            local_sample.panel,
            local_sample.surface,
            local_sample.content_scale,
        );
        let content_ctx = ctx.child(content_transform, ctx.bounds());
        let paint_space = content_ctx.local();
        let layer = renderer.program().origin().inverse() * paint_space;
        let opacity = f64_as_f32(local_sample.content_opacity);
        let clip = presentation_surface_clip(local_sample);
        let key = ScopeKey {
            role: "menu_presentation_content",
            item: active.scope,
        };
        renderer.open_scope_with(
            key,
            layer,
            opacity,
            Some(&clip),
            ctx.local(),
            ScopeDelta {
                transform: content_transform,
                hit_alpha: opacity,
            },
            true,
        );
        let blur_radius = if local_sample.content_blur > 0.01 {
            f64_as_f32(local_sample.content_blur)
        } else {
            0.0
        };
        record_with_blur_radius(
            renderer,
            content_ctx,
            blur_radius,
            &active.blur,
            &active.owner,
            |renderer, inner_ctx| {
                if let Some(row) = selected_row {
                    let bounds = inner_ctx.bounds();
                    renderer.draw_context(inner_ctx, |draw| {
                        look.record_selected_row(
                            draw,
                            row.with_origin((bounds.x0 + row.x0, bounds.y0 + row.y0)),
                        );
                    });
                }
                render_content(renderer, inner_ctx);
            },
        );
        renderer.close_scope();
    }

    /// Render the wrapped content, then bind the single focusable target — a
    /// text input or an input surface — it registered to the
    /// `.focused(binding)` binding and reconcile focus state. Shared by the
    /// dispatch handler and the retained `Wrapper` node ([`WrapperEffect::Focused`]):
    /// the binding is read through `read_signal` so a change schedules a frame, and
    /// the target bookkeeping counts targets registered during the content render, so
    /// it works identically whether the content is dispatched or node-flushed.
    pub(super) fn apply_focused(
        renderer: &mut Self,
        value: &Focused,
        render_content: impl FnOnce(&mut Self),
    ) {
        let should_focus = renderer.read_signal(&value.0);
        let scope = renderer
            .reader_cell()
            .expect("hydrolysis .focused() applied outside a record");
        render_content(renderer);
        renderer.wire_focused_target(value, should_focus, &scope);
    }

    /// The semantic counterpart of [`Self::apply_focused`]: the same focus
    /// wiring over the text-input and surface targets the semantic walk
    /// registered, with no renderer in hand.
    #[cfg(feature = "accessibility")]
    #[cfg(hydrolysis_cherenkov)]
    pub(super) fn apply_focused_semantic(
        renderer: &mut SemanticCore<E>,
        value: &Focused,
        render_content: impl FnOnce(&mut SemanticCore<E>),
    ) {
        let should_focus = renderer.read_signal(&value.0);
        let scope = renderer
            .reader_cell()
            .expect("hydrolysis .focused() applied outside a record");
        render_content(renderer);
        renderer.wire_focused_target(value, should_focus, &scope);
    }

    /// Render the given content and, when hit-testing is disabled, truncate every
    /// interaction-target vector back to its pre-render length (and clear focus if
    /// the focused text input fell inside the wrapped range). Shared by the dispatch
    /// handler and the retained `Wrapper` node. The bookkeeping counts targets
    /// registered during the content render, so it works identically whether the
    /// content is dispatched or node-flushed.
    /// `.hittable(false)`: the subtree's retained registrations sit under a
    /// [`HitGate::Unhittable`](crate::renderer::HitGate::Unhittable) scope,
    /// which removes exactly the input kinds dev's per-frame truncation
    /// took (drop and context-menu targets, back targets, modal scopes and
    /// native-view occlusions stay). Materialization clears the hover of a
    /// target the gate removed, as the truncation did; focus needs no
    /// eager pass — `finish_rebuild_frame`'s absent-target sweep retires a
    /// focused target the materialized list no longer emits.
    pub(super) fn apply_hittable(
        renderer: &mut Self,
        value: &Hittable,
        render_content: impl FnOnce(&mut Self),
    ) {
        let enabled = renderer.read_signal(value.enabled());
        if enabled {
            render_content(renderer);
            return;
        }
        // Regions flushed inside carry the unhittable scope on their
        // chain, so materialization drops their input kinds; a stale
        // in-flight drag against the content clears in the materialization
        // signature check.
        renderer.push_hit_gate_scope(crate::renderer::HitGate::Unhittable);
        render_content(renderer);
        renderer.pop_placement_scope();
    }

    /// Register the cursor hit-target, then render the given content. Shared by
    /// the dispatch handler and the retained `Wrapper` node.
    pub(super) fn apply_cursor(
        renderer: &mut Self,
        ctx: RenderContext,
        value: &Cursor,
        render_content: impl FnOnce(&mut Self),
    ) {
        let style = renderer.read_signal(value.style());
        let bounds = ctx.bounds();
        renderer.register_cursor_target(bounds, style);
        render_content(renderer);
    }

    /// The `Click` → `Activate` wiring of a tap gesture's accessibility node:
    /// invoke the gesture's own action with the same layered environment the
    /// pointer path uses (`captured_env.layered_on(runtime_env)`).
    #[cfg(feature = "accessibility")]
    fn tap_accessibility_activation(
        env: &Environment,
        action: &Rc<RefCell<BoxedAction<()>>>,
    ) -> AccessibilityActivation<E> {
        let captured_env = env.clone();
        let action = Rc::clone(action);
        Rc::new(RefCell::new(
            move |_renderer: &mut crate::renderer::SemanticCore<E>, runtime_env: &Environment| {
                let tap_env = runtime_env.extending(State(TapEvent::new(
                    Point::new(0.0, 0.0),
                    1,
                    PointerButton::Primary,
                )));
                let action_env = captured_env.layered_on(&tap_env);
                action.borrow_mut()(&action_env).report_extraction();
                true
            },
        ))
    }

    /// Register the gesture target (and, for a tappable view with a role, its
    /// accessibility node), then render the given content under accessibility
    /// suppression when the role excludes descendants. Shared by the dispatch
    /// handler and the retained `Wrapper` node.
    ///
    /// The build-resolved state lives in [`GestureObserverEffect`] (the two pieces
    /// derived from `content` — the default a11y label and the gesture group
    /// identity — are resolved at build time, since a node has no `content` at
    /// flush). Everything else is re-resolved against `env` each call (role/label
    /// overrides, suppression), matching the dispatch path. The action is shared
    /// so the node can re-register the same action every flush.
    #[expect(
        clippy::too_many_lines,
        reason = "the function drives one continuous scenario through the renderer; splitting it would obscure the sequence"
    )]
    #[cfg_attr(
        not(feature = "accessibility"),
        expect(
            unused_variables,
            reason = "the content only names the synthesized accessibility claim"
        )
    )]
    pub(super) fn apply_gesture_observer(
        renderer: &mut Self,
        ctx: RenderContext,
        env: &Environment,
        effect: &GestureObserverEffect,
        content: &RenderNode<E>,
        render_content: impl FnOnce(&mut Self, &Environment),
    ) {
        // The action environment contract (water-rs/hydrolysis#177,
        // water-rs/waterui#1292): the handler resolves against `env` as seen
        // here — the environment the observer's *content* resolves in — layered
        // over the runtime env at dispatch. The caller already resolved the
        // content's leading `.state(&v)`/handler layers, so a `.state` install
        // reaches the handler whether it sits before or after `.gesture` in the
        // modifier chain. Every dispatch arm — a11y Activate, `layered_action`,
        // the press slot, keyboard activation, hover — applies this same
        // `captured_env.layered_on(runtime_env)` rule.
        let bounds = ctx.bounds();
        let disabled = env
            .get::<waterui_interaction::interaction::Disabled>()
            .is_some_and(|disabled| renderer.read_signal(disabled.signal()));
        #[cfg(feature = "accessibility")]
        let mut claimed_naming_node = None;
        #[cfg(feature = "accessibility")]
        let mut claim_env: Option<Environment> = None;
        #[cfg(feature = "accessibility")]
        let mut claim_synthesized = false;
        // A hidden subtree publishes nothing — not even the claim a bare tap
        // would synthesize, which would otherwise take focus ahead of the
        // visible controls (a dialog's pointer-only scrim).
        #[cfg(feature = "accessibility")]
        if matches!(effect.gesture, Gesture::Tap(_))
            && !env
                .get::<AccessibilityHidden>()
                .is_some_and(AccessibilityHidden::is_hidden)
        {
            let has_role = env.get::<AccessibilityRole>().is_some();
            let has_scope = env.get::<ScopedAccessibilitySemantics<E>>().is_some();
            if !renderer.accessibility_scope_is_claimed(env) && (has_role || !has_scope) {
                // A tap with no naming scope to claim is the wrapped view's
                // own representative: it synthesizes `Button`'s role and a
                // fresh `ScopedAccessibilitySemantics` so the activation it
                // publishes is reachable the way a `Button`'s is — a bare
                // `.on_tap` used to serialize identically to plain text
                // (water-rs/waterui#1070).
                let env_for_claim = if has_scope {
                    env.clone()
                } else {
                    let mut env_for_claim = env.clone();
                    // An explicit role wins the synthesized default.
                    if !has_role {
                        env_for_claim.insert(AccessibilityRole::Button);
                    }
                    env_for_claim.insert(ScopedAccessibilitySemantics::<E>::new());
                    env_for_claim
                };
                let mut node = AccessibilityNode::new(
                    crate::renderer::SemanticCore::<E>::resolve_accessibility_role(
                        &env_for_claim,
                        AccessibilityNodeRole::Button,
                    ),
                );
                let default_label = content.accessibility_label(renderer);
                if let Some(label) = renderer.resolve_accessibility_label(env, default_label) {
                    node.set_label(label);
                }
                if let Some(value) = renderer.resolve_accessibility_value(env, None) {
                    node.set_value(value);
                }
                node.add_action(AccessibilityAction::Focus);
                if renderer.control_selected(env, &InteractionKey::for_rc(&effect.action, 0)) {
                    node.set_selected(true);
                }
                let action_target = if disabled {
                    node.set_disabled();
                    None
                } else {
                    node.add_action(AccessibilityAction::Click);
                    // Direct semantic activation: invoke the gesture's own
                    // action with the same layered environment the pointer
                    // path uses.
                    Some(AccessibilityActionTarget::Activate {
                        action: Self::tap_accessibility_activation(env, &effect.action),
                    })
                };
                claim_synthesized = !has_scope;
                claimed_naming_node = renderer.register_accessibility_node(
                    node,
                    bounds,
                    &env_for_claim,
                    action_target,
                );
                if claimed_naming_node.is_some() {
                    claim_env = Some(env_for_claim);
                }
            } else if !disabled && let Some(scope) = env.get::<ScopedAccessibilitySemantics<E>>() {
                // The scope names this view's representative — registering a
                // second node would emit a silenced duplicate — so the tap
                // delegates its activation to the scope instead, which the
                // representative drains when its subtree has been walked. An
                // unclaimed scope (a `List` row's, whose node the row
                // registers itself) receives the same donation. The donation
                // carries the gesture's own hit region and clip — the
                // interaction owner whose geometry the activation-point
                // query projects (water-rs/waterui#1323 §5).
                scope.delegate_activation(
                    Self::tap_accessibility_activation(env, &effect.action),
                    Some(NodePlacement {
                        region: crate::renderer::mount::Region {
                            local: bounds,
                            placement: renderer.current_placement(),
                        },
                    }),
                );
            }
        }
        // A gesture node that claimed the naming scope represents the wrapped
        // view: its content walks under the same shielded environment a naming
        // container hands its children, so a leaf cannot repeat the claim's
        // role and label as a second node. A *synthesized* claim drops the
        // name-from-contents marker instead — the wrapped content keeps its
        // own nodes and the claim collapses onto a lone one after the drain,
        // so the action lands on the node the gesture wraps rather than on a
        // swallowed composite (water-rs/waterui#1070).
        #[cfg(feature = "accessibility")]
        let content_env = claim_env.as_ref().map_or_else(
            || env.clone(),
            |claim_env| {
                accessibility_container_child_environment(claim_env).map_or_else(
                    || claim_env.clone(),
                    |mut content_env| {
                        if claim_synthesized {
                            content_env.remove::<AccessibilityNameFromContents>();
                        }
                        content_env
                    },
                )
            },
        );
        #[cfg(feature = "accessibility")]
        let content_env = &content_env;
        #[cfg(not(feature = "accessibility"))]
        let content_env = env;
        #[cfg(feature = "accessibility")]
        let scope_claimed = claimed_naming_node.is_some();
        #[cfg(not(feature = "accessibility"))]
        let scope_claimed = false;
        let group_id = renderer.gesture_group_id_for_identity(effect.gesture_group_identity);
        let captured_env = env.clone();
        let action = Rc::clone(&effect.action);
        let mut layered_action: BoxedAction<()> = Box::new(move |runtime_env: &Environment| {
            let action_env = captured_env.layered_on(runtime_env);
            action.borrow_mut()(&action_env)
        });

        if matches!(effect.gesture, Gesture::Tap(_))
            && let Some(chrome) = env.get::<crate::theme::Chrome>().cloned()
        {
            let interaction_key = InteractionKey::for_rc(&effect.action, 0);
            let target = chrome.target(bounds);
            let (_, press_slot, handles) = renderer.bind_live_chrome_target(
                interaction_key.clone(),
                target,
                env,
                disabled,
                &chrome,
            );
            let state = renderer.reported_interaction_state(&interaction_key);
            if effect.chrome_state.snapshot() != state {
                effect.chrome_state.set(state);
            }
            let parts = crate::renderer::chrome::LiveChromeParts::ensure(
                &mut effect.chrome.borrow_mut(),
                &chrome,
                &crate::renderer::chrome::ChromeContent::default(),
                bounds,
                state,
                env,
                &effect.chrome_state,
            )
            .clone();
            handles.attach_live(Rc::clone(parts.chrome()));
            // The surface below the composed control's own content, the
            // optional content effect around it, and the interaction layers
            // above it.
            parts.push_below(renderer, ctx);
            let content_scoped = parts.open_content_effect(renderer, ctx);
            // The claim parents the wrapped content's nodes for the walk, the
            // way a naming container's node parents its children's — without
            // it the content registers beside the claim and the synthesized
            // claim's single-child collapse has nothing to merge into.
            #[cfg(feature = "accessibility")]
            if let Some(node_id) = claimed_naming_node {
                renderer.push_accessibility_parent(node_id);
            }
            Self::render_gesture_content(renderer, env, content_env, scope_claimed, render_content);
            #[cfg(feature = "accessibility")]
            if claimed_naming_node.is_some() {
                renderer.pop_accessibility_parent();
            }
            #[cfg(feature = "accessibility")]
            if let Some(node_id) = claimed_naming_node {
                // The node advertises `Focus`; without this link Tab can land
                // on it semantically while the interaction machinery never
                // sees the key — FOCUSED would never reach
                // `.interaction_state` reports or the focus ring.
                renderer.register_accessibility_focus_link(&interaction_key, node_id);
                renderer.drain_claim_scope(node_id, claim_env.as_ref().unwrap_or(env));
                if claim_synthesized {
                    renderer.collapse_synthesized_claim(node_id);
                }
            }

            parts.close_content_effect(renderer, content_scoped);
            parts.push_above(renderer, ctx);

            if !disabled {
                renderer.register_interactive_pointer_target_with_keyboard(
                    target,
                    press_slot,
                    chrome.keyboard_focusable(),
                    move |_renderer, point, runtime_env| {
                        let tap_env = runtime_env.extending(State(TapEvent::new(
                            Point::new(
                                f64_as_f32(point.x - bounds.x0),
                                f64_as_f32(point.y - bounds.y0),
                            ),
                            1,
                            PointerButton::Primary,
                        )));
                        layered_action(&tap_env).report_extraction();
                        false
                    },
                );
            }
            return;
        }

        if let Gesture::Tap(tap) = &effect.gesture
            && tap.count().get() == 1
            && tap.get_buttons() == waterui_interaction::gesture::PointerButtons::PRIMARY
            && env
                .get::<waterui_interaction::interaction::InteractionReport>()
                .is_some()
            && env.get::<crate::theme::Chrome>().is_none()
        {
            let interaction_key = InteractionKey::for_rc(&effect.action, 0);
            #[cfg(feature = "accessibility")]
            let focus_key = interaction_key.clone();
            let (_state, press_slot, _handles) =
                renderer.bind_reported_tap_target(interaction_key, bounds, env, disabled);
            #[cfg(feature = "accessibility")]
            if let Some(node_id) = claimed_naming_node {
                renderer.push_accessibility_parent(node_id);
            }
            Self::render_gesture_content(renderer, env, content_env, scope_claimed, render_content);
            #[cfg(feature = "accessibility")]
            if claimed_naming_node.is_some() {
                renderer.pop_accessibility_parent();
            }
            #[cfg(feature = "accessibility")]
            if let Some(node_id) = claimed_naming_node {
                renderer.register_accessibility_focus_link(&focus_key, node_id);
                renderer.drain_claim_scope(node_id, claim_env.as_ref().unwrap_or(env));
                if claim_synthesized {
                    renderer.collapse_synthesized_claim(node_id);
                }
            }
            if !disabled {
                renderer.register_interactive_pointer_target_with_keyboard(
                    bounds,
                    press_slot,
                    true,
                    move |_renderer, point, runtime_env| {
                        let tap_env = runtime_env.extending(State(TapEvent::new(
                            Point::new(
                                f64_as_f32(point.x - bounds.x0),
                                f64_as_f32(point.y - bounds.y0),
                            ),
                            1,
                            PointerButton::Primary,
                        )));
                        layered_action(&tap_env).report_extraction();
                        false
                    },
                );
            }
            return;
        }

        if let Some(target) = effect.gesture_target.take() {
            renderer.register_retained_gesture_target(&target, bounds, group_id);
            effect.gesture_target.set(Some(target));
        } else {
            effect
                .gesture_target
                .set(Some(renderer.register_gesture_target(
                    bounds,
                    group_id,
                    effect.gesture.clone(),
                    layered_action,
                )));
        }
        #[cfg(feature = "accessibility")]
        if let Some(node_id) = claimed_naming_node {
            renderer.push_accessibility_parent(node_id);
        }
        Self::render_gesture_content(renderer, env, content_env, scope_claimed, render_content);
        #[cfg(feature = "accessibility")]
        if claimed_naming_node.is_some() {
            renderer.pop_accessibility_parent();
        }
        #[cfg(feature = "accessibility")]
        if let Some(node_id) = claimed_naming_node {
            renderer.drain_claim_scope(node_id, claim_env.as_ref().unwrap_or(env));
            if claim_synthesized {
                renderer.collapse_synthesized_claim(node_id);
            }
        }
    }

    fn render_gesture_content(
        renderer: &mut Self,
        env: &Environment,
        content_env: &Environment,
        scope_claimed: bool,
        render_content: impl FnOnce(&mut Self, &Environment),
    ) {
        #[cfg(not(feature = "accessibility"))]
        let _ = (env, content_env, scope_claimed);
        // `ExcludeDescendants` belongs to the element that claims this naming
        // scope — the claim registers its node above and suppresses its own
        // descendants here. An observer that registers no node (a long-press,
        // or a silenced tap) must not consume the flag: doing so suppresses
        // the inner element the flag actually names (water-rs/hydrolysis#266).
        #[cfg(feature = "accessibility")]
        if scope_claimed
            && env
                .get::<AccessibilityChildren>()
                .is_some_and(AccessibilityChildren::excludes_descendants)
        {
            renderer.push_accessibility_suppression();
            render_content(renderer, content_env);
            renderer.pop_accessibility_suppression();
            return;
        }
        render_content(renderer, content_env);
    }

    /// The semantic counterpart of [`Self::apply_gesture_observer`]: the tap's
    /// accessibility node — role, label, `Click` → [`AccessibilityActionTarget::Activate`],
    /// or `disabled` — with no bounds, no pointer or gesture targets, and no
    /// interaction state layer.
    ///
    /// A tap claims a node when the environment names the wrapped view (an
    /// explicit `a11y_role` carries its own naming scope) *or* names nothing
    /// at all: a bare `.on_tap` sits in no naming scope, so the claim
    /// synthesizes `Button`'s role and a fresh [`ScopedAccessibilitySemantics`]
    /// — without one, a tappable `text("T")` published no action and its tree
    /// was identical to plain text's (water-rs/waterui#1070). The claim
    /// drains its scope after the content walk like any other claimer; a
    /// synthesized claim additionally collapses onto a lone wrapped node, so
    /// the wrapped element itself carries the action.
    ///
    /// When a scope exists that the tap did not claim, the activation is
    /// delegated to the scope's representative instead, and `None` returns.
    #[cfg(feature = "accessibility")]
    #[cfg(hydrolysis_cherenkov)]
    pub(super) fn emit_gesture_observer_accessibility(
        renderer: &mut SemanticCore<E>,
        env: &Environment,
        effect: &GestureObserverEffect,
        content: &RenderNode<E>,
    ) -> Option<GestureA11yClaim> {
        if !matches!(effect.gesture, Gesture::Tap(_)) {
            return None;
        }
        let disabled = env
            .get::<waterui_interaction::interaction::Disabled>()
            .is_some_and(|disabled| renderer.read_signal(disabled.signal()));
        // A hidden subtree publishes nothing, the claim included.
        if env
            .get::<AccessibilityHidden>()
            .is_some_and(AccessibilityHidden::is_hidden)
        {
            return None;
        }
        let has_role = env.get::<AccessibilityRole>().is_some();
        let has_scope = env.get::<ScopedAccessibilitySemantics<E>>().is_some();
        if !renderer.accessibility_scope_is_claimed(env) && (has_role || !has_scope) {
            // The claim's env is what the content walk and the scope drain
            // resolve against: the view's own scope when it carries one, a
            // synthesized scope-plus-Button-role for a bare tap.
            let (claim_env, synthesized) = if has_scope {
                (env.clone(), false)
            } else {
                let mut claim_env = env.clone();
                // An explicit role wins the synthesized default.
                if !has_role {
                    claim_env.insert(AccessibilityRole::Button);
                }
                claim_env.insert(ScopedAccessibilitySemantics::<E>::new());
                (claim_env, true)
            };
            let mut node = AccessibilityNode::new(
                crate::renderer::SemanticCore::<E>::resolve_accessibility_role(
                    &claim_env,
                    AccessibilityNodeRole::Button,
                ),
            );
            let default_label = content.accessibility_label(renderer);
            if let Some(label) = renderer.resolve_accessibility_label(env, default_label) {
                node.set_label(label);
            }
            node.add_action(AccessibilityAction::Focus);
            if renderer.control_selected(env, &InteractionKey::for_rc(&effect.action, 0)) {
                node.set_selected(true);
            }
            let action_target = if disabled {
                node.set_disabled();
                None
            } else {
                node.add_action(AccessibilityAction::Click);
                Some(AccessibilityActionTarget::Activate {
                    action: Self::tap_accessibility_activation(env, &effect.action),
                })
            };
            return renderer
                .register_accessibility_node_semantic(node, &claim_env, action_target)
                .map(|node_id| GestureA11yClaim {
                    node_id,
                    env: claim_env,
                    synthesized,
                });
        }
        if !disabled && let Some(scope) = env.get::<ScopedAccessibilitySemantics<E>>() {
            // Claimed or not, the scope's representative drains the donation
            // when its subtree ends — the same contract the rendered arm of
            // `apply_gesture_observer` holds. The semantic walk has no
            // geometry, so the donation carries no interaction region either.
            scope.delegate_activation(
                Self::tap_accessibility_activation(env, &effect.action),
                None,
            );
        }
        None
    }

    /// Register the hover-enter/move/exit target for `handler`, then render the
    /// given content. Shared by the dispatch handler and the retained `Wrapper`
    /// node. The handler is shared (`Rc<RefCell<OnEvent>>`) so the node can
    /// re-register the same handler every flush; the dispatch handler wraps its
    /// owned value once. The registered closure resolves the action environment
    /// against `env` exactly as before.
    pub(super) fn apply_on_event(
        renderer: &mut Self,
        ctx: RenderContext,
        env: &Environment,
        handler: Rc<RefCell<OnEvent>>,
        render_content: impl FnOnce(&mut Self),
    ) {
        let event = handler.borrow().event();
        let interaction_key = InteractionKey::for_rc(&handler, 0);
        let bounds = ctx.bounds();
        match event {
            Event::HoverEnter => {
                let captured_env = env.clone();
                renderer.register_hover_enter_target(interaction_key, bounds, move |env| {
                    let action_env = captured_env.layered_on(env);
                    handler.borrow_mut().handle(&action_env);
                    true
                });
            }
            Event::HoverMove => {
                let captured_env = env.clone();
                // The dispatcher hands `point` in the registration's local
                // space — the space `bounds` is in — so the view-relative
                // location is the offset from the view's own origin.
                renderer.register_hover_move_target(interaction_key, bounds, move |point, env| {
                    let hover_event = HoverEvent::new(waterui_core::layout::Point::new(
                        crate::num_cast::f64_as_f32(point.x)
                            - crate::num_cast::f64_as_f32(bounds.x0),
                        crate::num_cast::f64_as_f32(point.y)
                            - crate::num_cast::f64_as_f32(bounds.y0),
                    ));
                    let hover_env = captured_env.layered_on(&env.extending(State(hover_event)));
                    handler.borrow_mut().handle(&hover_env);
                    true
                });
            }
            Event::HoverExit => {
                let captured_env = env.clone();
                renderer.register_hover_exit_target(interaction_key, bounds, move |env| {
                    let action_env = captured_env.layered_on(env);
                    handler.borrow_mut().handle(&action_env);
                    true
                });
            }
            other => contract_violation!(
                "hydrolysis event variant {other:?} is not supported; its handler is not registered"
            ),
        }
        render_content(renderer);
    }

    /// Register the context-menu hit-target, then render the given content. Shared
    /// by the dispatch handler and the retained `Wrapper` node. The node owns the
    /// [`ContextMenuEffect`] by reference, so the menu items are cloned for
    /// registration and the preview/accessory slots travel with the target for
    /// the open presentation to mount. The node's environment travels with the
    /// target so the popup opens inside it (water-rs/hydrolysis#140).
    #[cfg(feature = "menu")]
    pub(super) fn apply_context_menu(
        renderer: &mut Self,
        ctx: RenderContext,
        env: &Environment,
        value: &ContextMenuEffect<E>,
        render_content: impl FnOnce(&mut Self),
    ) {
        let bounds = ctx.bounds();
        renderer.register_context_menu_target(bounds, value, env);
        render_content(renderer);
    }

    /// Register the anchor's live bounds and the overlay's handles for the
    /// post-flush render pass, then render the anchor content. Shared by the
    /// dispatch handler and the retained `Wrapper` node. The binding is read
    /// through [`HydrolysisRenderer::read_signal`], so a value change
    /// schedules the refresh that opens or closes the overlay; re-registering
    /// every frame is also what lets the render pass follow an anchor that
    /// moved or detect one that left the tree.
    pub(super) fn apply_anchored_overlay(
        renderer: &mut Self,
        ctx: RenderContext,
        env: &Environment,
        value: &AnchoredOverlayEffect<E>,
        render_content: impl FnOnce(&mut Self),
    ) {
        let bounds = renderer.resolve_window_rect(ctx.bounds());
        let presented = renderer.read_signal(&value.is_presented);
        #[cfg(feature = "menu")]
        let trigger_opacity = value
            .presentations
            .borrow()
            .trigger_opacity_at(renderer.frame_instant);
        renderer
            .popup_menu
            .anchored_overlays
            .push(RegisteredAnchoredOverlay {
                anchor: bounds,
                placement: value.placement,
                dismissal: value.dismissal,
                presented,
                is_presented: value.is_presented.clone(),
                placed_edge: value.placed_edge.clone(),
                env: env.clone(),
                builder: Rc::clone(&value.builder),
                presentations: Rc::clone(&value.presentations),
                marker: Rc::clone(&value.marker),
                cover_trigger: false,
            });
        #[cfg(feature = "menu")]
        if trigger_opacity < 1.0 - 1e-4 {
            let key = ScopeKey {
                role: "anchored_trigger",
                item: anchored_overlay_scope(&value.marker),
            };
            renderer.open_scope_with(
                key,
                Affine::IDENTITY,
                f64_as_f32(trigger_opacity),
                None,
                Affine::IDENTITY,
                ScopeDelta::RECORD_SPACE,
                true,
            );
            render_content(renderer);
            renderer.close_scope();
            return;
        }
        render_content(renderer);
    }

    /// Report the dialog's presentation to the window's dialog stack and,
    /// when it is the one on screen, register its layer for the post-flush
    /// render pass; then render the node's own content. The registration
    /// carries the modal environment — the node's environment plus the
    /// `ModalInteraction` scope that routes Escape to the dialog's cancel
    /// path — so the layer presents modal the moment it draws.
    #[cfg(waterui_feature = "dialog")]
    pub(super) fn apply_dialog(
        renderer: &mut Self,
        env: &Environment,
        value: &DialogEffect<E>,
        render_content: impl FnOnce(&mut Self),
    ) {
        let presented = renderer.read_signal(value.dialog.is_presented());
        let focus = renderer.hit_test.keyboard_focus.clone();
        if renderer
            .popup_menu
            .dialog_stack
            .report(&value.marker, &value.dialog, presented, focus)
        {
            renderer.popup_menu.front_dialog = Some(RegisteredDialog {
                env: dialog_modal_environment(env, &value.dialog),
                content: Rc::clone(&value.content),
            });
        }
        render_content(renderer);
    }

    /// Register the draggable hit-target, then render the given content. Shared by
    /// the dispatch handler and the retained `Wrapper` node. The node owns the
    /// [`Draggable`] in an `Rc`, so the registration clones the handle and the
    /// payload reads live at the moment the drag begins.
    pub(super) fn apply_draggable(
        renderer: &mut Self,
        ctx: RenderContext,
        value: &Rc<Draggable>,
        render_content: impl FnOnce(&mut Self),
    ) {
        let bounds = ctx.bounds();
        renderer.register_draggable_target(bounds, Rc::clone(value));
        render_content(renderer);
    }

    /// Register the drop-destination hit-target from pre-wrapped handler handles,
    /// then render the given content. Shared by the dispatch handler and the
    /// retained `Wrapper` node (which holds the handles by value and re-registers
    /// the same `Rc`s every flush).
    pub(super) fn apply_drop_destination(
        renderer: &mut Self,
        ctx: RenderContext,
        env: &Environment,
        handles: &DropDestinationHandles,
        render_content: impl FnOnce(&mut Self),
    ) {
        let bounds = ctx.bounds();
        renderer.register_drop_destination_handles(bounds, handles, env);
        render_content(renderer);
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum RegularClipShape {
    Rect(kurbo::Rect),
    RoundedRect {
        rect: kurbo::Rect,
        corner_width: f64,
        corner_height: f64,
    },
}

/// The fast rounded-rect/rect clip for a structured shape kind.
///
/// A normalized radius resolves against the shorter side, so corners stay
/// circular and a fully-rounded shape is a stadium rather than an ellipse.
#[expect(
    clippy::float_cmp,
    reason = "the comparison is exact by design — the value originates from a literal fixture, not accumulated arithmetic"
)]
fn kind_clip_shape(kind: ShapeKind, bounds: kurbo::Rect) -> Option<RegularClipShape> {
    let min_side = bounds.width().min(bounds.height()).max(0.0);
    let rounded = |corner: f64| {
        Some(RegularClipShape::RoundedRect {
            rect: bounds,
            corner_width: corner,
            corner_height: corner,
        })
    };
    let resolve_corner = |radius: CornerRadius| {
        let limit = min_side / 2.0;
        match (radius.as_points(), radius.as_fraction()) {
            (Some(points), None) => f64::from(points).min(limit),
            (None, Some(fraction)) => (f64::from(fraction) * min_side).min(limit),
            _ => unreachable!("CornerRadius is either points or fraction"),
        }
    };
    match kind {
        ShapeKind::Rect => Some(RegularClipShape::Rect(bounds)),
        ShapeKind::RoundedRect { radii } => {
            radii.as_uniform().and_then(|r| rounded(resolve_corner(r)))
        }
        ShapeKind::Capsule => rounded(min_side / 2.0),
        // A circle is *inscribed* in the bounds, so only a square one is a
        // rounded rect: elsewhere `min_side / 2.0` describes a stadium filling
        // the bounds, which is what a capsule is and what a circle is not. The
        // fill path builds a real `kurbo::Circle`, and a clip that disagreed
        // with its own fill is the bug this guard closes.
        ShapeKind::Circle if bounds.width() == bounds.height() => rounded(min_side / 2.0),
        // An ellipse is not a rounded rect, a non-square circle is not either,
        // and uneven corners need the path mask; all stay on the general route.
        ShapeKind::Circle | ShapeKind::Ellipse | ShapeKind::CustomPath => None,
    }
}

#[cfg(test)]
mod clip_shape_tests {
    use super::{RegularClipShape, ShapeKind, kind_clip_shape};
    use kurbo::Rect;

    /// A square circle is exactly a rounded rect whose corner is half the
    /// side, so the fast clip is allowed to take it.
    #[test]
    fn a_square_circle_takes_the_rounded_rect_fast_path() {
        let bounds = Rect::new(0.0, 0.0, 100.0, 100.0);
        let clip = kind_clip_shape(ShapeKind::Circle, bounds);
        assert!(
            matches!(
                clip,
                Some(RegularClipShape::RoundedRect {
                    corner_width,
                    corner_height,
                    ..
                }) if (corner_width - 50.0).abs() < f64::EPSILON
                    && (corner_height - 50.0).abs() < f64::EPSILON
            ),
            "a square circle should clip as a rounded rect with a half-side corner, got {clip:?}"
        );
    }

    /// On a wider-than-tall rect the same shortcut would describe a stadium,
    /// which is a capsule and not the inscribed circle the fill draws.
    #[test]
    fn a_non_square_circle_does_not_take_the_fast_path() {
        let bounds = Rect::new(0.0, 0.0, 200.0, 100.0);
        assert!(
            kind_clip_shape(ShapeKind::Circle, bounds).is_none(),
            "a non-square circle must fall through to the path mask so the clip \
             matches the inscribed circle the fill builds"
        );
    }

    /// A capsule *is* the stadium, on any aspect ratio.
    #[test]
    fn a_capsule_takes_the_fast_path_at_any_aspect_ratio() {
        let bounds = Rect::new(0.0, 0.0, 200.0, 100.0);
        assert!(matches!(
            kind_clip_shape(ShapeKind::Capsule, bounds),
            Some(RegularClipShape::RoundedRect { .. })
        ));
    }

    /// A point radius is a length in points: 12 stays 12 on a wide bar, and
    /// only the half-shorter-side ceiling cuts it down.
    #[test]
    fn a_fixed_radius_clips_at_its_own_length_up_to_the_ceiling() {
        use waterui_shape::{CornerRadii, CornerRadius};

        let wide = Rect::new(0.0, 0.0, 200.0, 50.0);
        assert!(matches!(
            kind_clip_shape(
                ShapeKind::RoundedRect {
                    radii: CornerRadii::uniform(CornerRadius::points(12.0)),
                },
                wide
            ),
            Some(RegularClipShape::RoundedRect {
                corner_width,
                corner_height,
                ..
            }) if (corner_width - 12.0).abs() < f64::EPSILON
                && (corner_height - 12.0).abs() < f64::EPSILON
        ));
        assert!(matches!(
            kind_clip_shape(
                ShapeKind::RoundedRect {
                    radii: CornerRadii::uniform(CornerRadius::points(40.0)),
                },
                wide
            ),
            Some(RegularClipShape::RoundedRect {
                corner_width,
                corner_height,
                ..
            }) if (corner_width - 25.0).abs() < f64::EPSILON
                && (corner_height - 25.0).abs() < f64::EPSILON
        ));
    }

    /// Per-corner radii cannot be a uniform `RoundedRect` clip.
    #[test]
    fn a_fixed_uneven_kind_stays_on_the_path_mask() {
        use waterui_shape::CornerRadii;

        let bounds = Rect::new(0.0, 0.0, 200.0, 100.0);
        assert!(
            kind_clip_shape(
                ShapeKind::RoundedRect {
                    radii: CornerRadii::new(0.0, 16.0, 0.0, 16.0),
                },
                bounds
            )
            .is_none()
        );
    }
}

fn regular_clip_shape(commands: &[PathCommand], bounds: kurbo::Rect) -> Option<RegularClipShape> {
    regular_rect(commands, bounds).or_else(|| regular_rounded_rect(commands, bounds))
}

fn regular_rect(commands: &[PathCommand], bounds: kurbo::Rect) -> Option<RegularClipShape> {
    let [
        PathCommand::MoveTo { x: x0, y: y0 },
        PathCommand::LineTo { x: x1, y: top_y },
        PathCommand::LineTo { x: right_x, y: y1 },
        PathCommand::LineTo {
            x: left_x,
            y: bottom_y,
        },
        PathCommand::Close,
    ] = commands
    else {
        return None;
    };
    if !approx_eq(*y0, *top_y)
        || !approx_eq(*x1, *right_x)
        || !approx_eq(*y1, *bottom_y)
        || !approx_eq(*x0, *left_x)
        || !valid_rect(*x0, *y0, *x1, *y1)
    {
        return None;
    }
    Some(RegularClipShape::Rect(resolve_normalized_rect(
        *x0, *y0, *x1, *y1, bounds,
    )))
}

#[allow(clippy::too_many_lines)]
#[expect(
    clippy::similar_names,
    reason = "the names follow the fixture domain vocabulary; renaming would obscure rather than clarify"
)]
fn regular_rounded_rect(commands: &[PathCommand], bounds: kurbo::Rect) -> Option<RegularClipShape> {
    let [
        PathCommand::MoveTo { x: start_x, y: y0 },
        PathCommand::LineTo {
            x: top_end_x,
            y: top_y,
        },
        PathCommand::Arc {
            cx: top_right_cx,
            cy: top_right_cy,
            rx,
            ry,
            start: top_right_start,
            sweep: top_right_sweep,
        },
        PathCommand::LineTo {
            x: x1,
            y: right_end_y,
        },
        PathCommand::Arc {
            cx: bottom_right_cx,
            cy: bottom_right_cy,
            rx: bottom_right_rx,
            ry: bottom_right_ry,
            start: bottom_right_start,
            sweep: bottom_right_sweep,
        },
        PathCommand::LineTo {
            x: bottom_end_x,
            y: y1,
        },
        PathCommand::Arc {
            cx: bottom_left_cx,
            cy: bottom_left_cy,
            rx: bottom_left_rx,
            ry: bottom_left_ry,
            start: bottom_left_start,
            sweep: bottom_left_sweep,
        },
        PathCommand::LineTo {
            x: x0,
            y: left_end_y,
        },
        PathCommand::Arc {
            cx: top_left_cx,
            cy: top_left_cy,
            rx: top_left_rx,
            ry: top_left_ry,
            start: top_left_start,
            sweep: top_left_sweep,
        },
        PathCommand::Close,
    ] = commands
    else {
        return None;
    };

    let quarter_turn = core::f32::consts::FRAC_PI_2;
    let uniform_radii = [*bottom_right_rx, *bottom_left_rx, *top_left_rx]
        .into_iter()
        .all(|radius| approx_eq(radius, *rx))
        && [*bottom_right_ry, *bottom_left_ry, *top_left_ry]
            .into_iter()
            .all(|radius| approx_eq(radius, *ry));
    let geometry_matches = approx_eq(*top_y, *y0)
        && approx_eq(*start_x, *x0 + *rx)
        && approx_eq(*top_end_x, *x1 - *rx)
        && approx_eq(*top_right_cx, *x1 - *rx)
        && approx_eq(*top_right_cy, *y0 + *ry)
        && approx_eq(*right_end_y, *y1 - *ry)
        && approx_eq(*bottom_right_cx, *x1 - *rx)
        && approx_eq(*bottom_right_cy, *y1 - *ry)
        && approx_eq(*bottom_end_x, *x0 + *rx)
        && approx_eq(*bottom_left_cx, *x0 + *rx)
        && approx_eq(*bottom_left_cy, *y1 - *ry)
        && approx_eq(*left_end_y, *y0 + *ry)
        && approx_eq(*top_left_cx, *x0 + *rx)
        && approx_eq(*top_left_cy, *y0 + *ry);
    let angles_match = approx_eq(*top_right_start, -quarter_turn)
        && approx_eq(*top_right_sweep, quarter_turn)
        && approx_eq(*bottom_right_start, 0.0)
        && approx_eq(*bottom_right_sweep, quarter_turn)
        && approx_eq(*bottom_left_start, quarter_turn)
        && approx_eq(*bottom_left_sweep, quarter_turn)
        && approx_eq(*top_left_start, core::f32::consts::PI)
        && approx_eq(*top_left_sweep, quarter_turn);
    if !uniform_radii
        || !geometry_matches
        || !angles_match
        || !valid_rect(*x0, *y0, *x1, *y1)
        || !rx.is_finite()
        || !ry.is_finite()
        || *rx < 0.0
        || *ry < 0.0
    {
        return None;
    }

    // A normalized corner radius resolves against the shorter side, so the corner
    // stays circular on a non-square rect. Scaling each axis by its own extent
    // instead turns every rounded-rect *clip* into an ellipse while the identical
    // shape *fills* as a rounded rect, because the fill route (`rounded_rect_path`)
    // already resolves against `min_side`. The two must agree.
    let min_side = bounds.width().min(bounds.height()).max(0.0);
    Some(RegularClipShape::RoundedRect {
        rect: resolve_normalized_rect(*x0, *y0, *x1, *y1, bounds),
        corner_width: f64::from(*rx) * min_side,
        corner_height: f64::from(*ry) * min_side,
    })
}

fn resolve_normalized_rect(x0: f32, y0: f32, x1: f32, y1: f32, bounds: kurbo::Rect) -> kurbo::Rect {
    kurbo::Rect::new(
        f64::from(x0) * bounds.width(),
        f64::from(y0) * bounds.height(),
        f64::from(x1) * bounds.width(),
        f64::from(y1) * bounds.height(),
    )
}

fn valid_rect(x0: f32, y0: f32, x1: f32, y1: f32) -> bool {
    [x0, y0, x1, y1].into_iter().all(f32::is_finite) && x0 <= x1 && y0 <= y1
}

fn approx_eq(left: f32, right: f32) -> bool {
    (left - right).abs() <= f32::EPSILON * 64.0
}

#[cfg(test)]
mod regular_clip_tests {
    use waterui_shape::{CornerRadii, CornerRadius, Path, Rectangle, RoundedRectangle, Shape as _};

    use super::*;

    const BOUNDS: kurbo::Rect = kurbo::Rect::new(0.0, 0.0, 200.0, 100.0);

    #[test]
    fn recognizes_axis_aligned_rectangle() {
        assert_eq!(
            regular_clip_shape(&Rectangle.path(), BOUNDS),
            Some(RegularClipShape::Rect(BOUNDS))
        );
    }

    /// A normalized corner radius resolves against the shorter side, so the
    /// corners stay circular on a non-square rect and a clip matches the fill of
    /// the same shape. Resolving each axis against its own extent produced
    /// elliptical corners — a fully-rounded clip came out as an ellipse instead
    /// of a pill.
    #[test]
    fn uniform_rounded_rectangle_clip_keeps_circular_corners() {
        let Some(RegularClipShape::RoundedRect {
            rect,
            corner_width,
            corner_height,
        }) = regular_clip_shape(
            &RoundedRectangle::new(CornerRadius::fraction(0.1)).path(),
            BOUNDS,
        )
        else {
            panic!("uniform rounded rectangle must use the regular clip route");
        };
        let min_side = BOUNDS.width().min(BOUNDS.height());
        assert_eq!(rect, BOUNDS);
        assert!(0.1f64.mul_add(-min_side, corner_width).abs() < 1.0e-5);
        assert!(
            (corner_width - corner_height).abs() < 1.0e-5,
            "a uniform rounded rectangle must clip with circular corners, got \
             {corner_width}x{corner_height} on a {}x{} rect",
            BOUNDS.width(),
            BOUNDS.height()
        );
    }

    /// The fully-rounded case: a clip at the maximum normalized radius is a
    /// stadium whose caps are half the shorter side, not an ellipse.
    #[test]
    fn fully_rounded_clip_is_a_stadium_not_an_ellipse() {
        let Some(RegularClipShape::RoundedRect {
            corner_width,
            corner_height,
            ..
        }) = regular_clip_shape(
            &RoundedRectangle::new(CornerRadius::fraction(0.5)).path(),
            BOUNDS,
        )
        else {
            panic!("a fully-rounded rectangle must use the regular clip route");
        };
        let cap = BOUNDS.width().min(BOUNDS.height()) / 2.0;
        assert!((corner_width - cap).abs() < 1.0e-5);
        assert!((corner_height - cap).abs() < 1.0e-5);
    }

    #[test]
    fn leaves_uneven_and_custom_paths_on_the_path_mask_route() {
        assert_eq!(
            regular_clip_shape(
                &RoundedRectangle::new(CornerRadii::new(
                    CornerRadius::fraction(0.1),
                    CornerRadius::fraction(0.2),
                    CornerRadius::fraction(0.3),
                    CornerRadius::fraction(0.4),
                ))
                .path(),
                BOUNDS,
            ),
            None
        );
        let triangle = Path::new()
            .move_to(0.5, 0.0)
            .line_to(1.0, 1.0)
            .line_to(0.0, 1.0)
            .close();
        let triangle_commands: Vec<_> = triangle.path().collect();
        assert_eq!(regular_clip_shape(&triangle_commands, BOUNDS), None);
    }
}
