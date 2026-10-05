//! The drawn `.context_menu` presentation that wraps the popup menu when the
//! resolved menu carries an accessory: the window dims, the preview (or the
//! source view) lifts at the source's rect, and the interactive accessory
//! anchors to the preview's edge, kept clear of the menu
//! (water-rs/hydrolysis#200, water-rs/waterui#1245).
//!
//! The presentation is encoded into the *source* window's scene, once a frame
//! from the window's flush path — the popup menu is a separate window floating
//! over it. The lifted views are the declaring node's own `RetainedSubview`s,
//! lent to the presentation on open and handed back on close, so their state
//! and identity survive repeated presentations.

// glob import of the module vocabulary — the renderer internals are designed to be used wholesale
#[allow(clippy::wildcard_imports)]
use super::*;

/// Gap between the lifted preview and its accessory, in logical points.
pub const CONTEXT_MENU_ACCESSORY_GAP: f64 = 8.0;
/// Corner radius of the lift shadow, in logical points.
const CONTEXT_MENU_LIFT_RADIUS: f64 = 12.0;
/// Blur radius of the lift shadow, in logical points.
const CONTEXT_MENU_LIFT_SHADOW_RADIUS: f64 = 24.0;
/// Opacity of the dim backdrop the lifted content floats over.
const CONTEXT_MENU_DIM_OPACITY: f32 = 0.4;
/// Opacity of the lift shadow.
const CONTEXT_MENU_SHADOW_OPACITY: f32 = 0.35;

/// The live state of an open `.context_menu` accessory presentation. Stored on
/// [`PopupMenuState`] for the same lifetime as the menu window; when the menu
/// closes — by item choice, outside press, or `dismiss_requests` — the
/// presentation drops and the sub-views return to their owning node's slots.
pub struct ContextMenuPresentation {
    /// The context-menu target's rect in hit space: where the preview is
    /// lifted and what the accessory anchors to.
    pub(crate) source_bounds: kurbo::Rect,
    /// The menu's rows, drawn into this window's scene at `menu_frame` every
    /// frame — no borderless popup window exists in this presentation, so
    /// nothing transparent can stray over the dimmed backdrop.
    pub(crate) menu: RetainedSubview,
    /// The custom preview being lifted over `source_bounds`. `None` keeps the
    /// source view lit through a hole in the dim backdrop — the source is
    /// still drawn by the owning tree, so it cannot be re-flushed here.
    pub(crate) preview: Option<RetainedSubview>,
    /// The mounted interactive accessory. `None` for a preview-only or
    /// hold-opened presentation — those mount the dim and the lift without
    /// an accessory panel.
    pub(crate) accessory: Option<RetainedSubview>,
    /// The owning node's slots the sub-views return to on drop.
    pub(crate) preview_slot: Rc<RefCell<Option<RetainedSubview>>>,
    pub(crate) accessory_slot: Rc<RefCell<Option<RetainedSubview>>>,
    /// The accessory's dismiss-request counter; every change closes the menu.
    pub(crate) dismiss_requests: nami::Computed<i32>,
    /// The counter value last observed by the presentation's frame render.
    pub(crate) last_dismiss_requests: i32,
    /// The popup's own open/closed handle. An item choice closes it through
    /// `PopupMenuStateGroup::close_all` without clearing
    /// `active_popup_menu_group`, so this binding is the only signal that still
    /// reports the menu gone.
    pub(crate) menu_state: Binding<WindowState>,
    /// The popup's frame in hit space — the accessory is kept clear of it.
    /// Recomputed every frame by [`context_menu_presentation_layout`].
    pub(crate) menu_frame: kurbo::Rect,
    /// The menu's measured size in hit space, kept for the per-frame layout.
    pub(crate) menu_size: (f64, f64),
    /// The lifted preview's frame as last laid out — the source's rect until
    /// fitting the whole stack pushes it.
    pub(crate) lift_frame: kurbo::Rect,
    /// `true` when the stack was taller than the window and `lift_frame` is
    /// shorter than the source: the preview draws cropped to that height.
    pub(crate) lift_cropped: bool,
    /// The accessory's frame as last laid out, in hit space — outside-press
    /// and context-menu-open handling read it to let presses through to the
    /// accessory's own targets.
    pub(crate) accessory_frame: Option<kurbo::Rect>,
    /// The declaring view's environment — the presentation's sub-views are
    /// built, measured and flushed inside it.
    pub(crate) env: Environment,
}

impl Drop for ContextMenuPresentation {
    fn drop(&mut self) {
        // Return the borrowed sub-views to the node's slots so a later open
        // mounts them again, still built and still holding their state.
        if let Some(subview) = self.preview.take() {
            *self.preview_slot.borrow_mut() = Some(subview);
        }
        if let Some(subview) = self.accessory.take() {
            *self.accessory_slot.borrow_mut() = Some(subview);
        }
    }
}

/// Where the presentation's pieces land for a frame, in hit space: the
/// lifted preview, the menu panel and the accessory panel.
#[derive(Clone, Copy)]
struct ContextMenuLayout {
    /// The lifted copy's rect — the source's rect until fitting pushes it.
    lift: kurbo::Rect,
    /// `true` when the stack was taller than the window: `lift` is then
    /// shorter than the source, and the preview draws cropped to it.
    lift_cropped: bool,
    /// The menu panel's frame.
    menu: kurbo::Rect,
    /// The accessory panel's frame, when one mounts.
    accessory: Option<kurbo::Rect>,
}

/// Lay out `accessory`, `gap`, `lift`, `gap`, `menu` inside the window
/// (water-rs/hydrolysis#200): the accessory and the menu take opposite sides
/// of the lift — accessory above and menu below by default, swapped when the
/// window has no room for the default. When neither order fits around the
/// source the lifted copy moves (the source view itself never does) so the
/// whole stack sits inside the window; only when the stack is taller than
/// the window does the preview shrink — it then draws cropped to what
/// remains after the accessory and the menu. A lift that cannot move (the
/// source stays lit through the dim) instead packs accessory and menu
/// contiguously on the roomier side, the accessory closest to the source.
#[expect(
    clippy::similar_names,
    reason = "the names follow the fixture domain vocabulary; renaming would obscure rather than clarify"
)]
fn context_menu_presentation_layout(
    source: kurbo::Rect,
    lift_movable: bool,
    menu_size: (f64, f64),
    accessory_size: Option<(f64, f64)>,
    window: kurbo::Rect,
) -> ContextMenuLayout {
    let gap = CONTEXT_MENU_ACCESSORY_GAP;
    let (menu_w, menu_h) = menu_size;
    let (acc_w, acc_h) = accessory_size.unwrap_or((0.0, 0.0));
    let acc_need = if accessory_size.is_some() {
        acc_h + gap
    } else {
        0.0
    };
    let menu_need = menu_h + gap;

    // A panel's leading edge is the lift's leading edge — its trailing edge
    // when leading would overflow — clamped inside the window.
    let edge_x = |width: f64| {
        let mut x0 = source.x0;
        if x0 + width > window.x1 {
            x0 = source.x1 - width;
        }
        x0.clamp(window.x0, (window.x1 - width).max(window.x0))
    };

    // `menu_above` selects the order: `false` = accessory above, menu below
    // (the default); `true` = menu above, accessory below.
    let place = |lift: kurbo::Rect, menu_above: bool| -> ContextMenuLayout {
        let (menu_y0, acc_y0) = if menu_above {
            (lift.y0 - gap - menu_h, lift.y1 + gap)
        } else {
            (lift.y1 + gap, lift.y0 - gap - acc_h)
        };
        let menu_x0 = edge_x(menu_w);
        let menu = kurbo::Rect::new(menu_x0, menu_y0, menu_x0 + menu_w, menu_y0 + menu_h);
        let accessory = accessory_size.map(|_| {
            let x0 = edge_x(acc_w);
            kurbo::Rect::new(x0, acc_y0, x0 + acc_w, acc_y0 + acc_h)
        });
        ContextMenuLayout {
            lift,
            lift_cropped: lift.height() < source.height() - 0.5,
            menu,
            accessory,
        }
    };

    // Order A fits iff the lift has `acc_need` above and `menu_need` below;
    // order B swaps them. Try both at the source's own position first.
    if source.y0 - acc_need >= window.y0 && source.y1 + menu_need <= window.y1 {
        return place(source, false);
    }
    if source.y0 - menu_need >= window.y0 && source.y1 + acc_need <= window.y1 {
        return place(source, true);
    }

    if lift_movable {
        // Move the lifted copy into a slot where an order fits — order A's
        // lift y range is [window.y0 + acc_need, window.y1 - menu_need - h].
        let h = source.height();
        for (above, below, menu_above) in
            [(acc_need, menu_need, false), (menu_need, acc_need, true)]
        {
            let lo = window.y0 + above;
            let hi = window.y1 - below - h;
            if lo <= hi {
                let y0 = source.y0.clamp(lo, hi);
                let lift = kurbo::Rect::new(source.x0, y0, source.x1, y0 + h);
                return place(lift, menu_above);
            }
        }
        // The stack is taller than the window: the accessory and the menu
        // keep full size, and the preview draws cropped to what remains.
        let capped = (window.y1 - window.y0 - acc_need - menu_need).max(0.0);
        let y0 = window.y0 + acc_need;
        let lift = kurbo::Rect::new(source.x0, y0, source.x1, y0 + capped);
        return place(lift, false);
    }

    // The lift is the source itself and cannot move: pack the accessory and
    // the menu contiguously on the roomier side, the accessory closest to
    // the source, each clamped inside the window.
    let clamp_y = |y0: f64, h: f64| y0.clamp(window.y0, (window.y1 - h).max(window.y0));
    let below_space = window.y1 - source.y1;
    let above_space = source.y0 - window.y0;
    let (menu_y0, acc_y0) = if below_space >= above_space {
        (source.y1 + gap + acc_need, source.y1 + gap)
    } else {
        (source.y0 - gap - menu_h - acc_need, source.y0 - gap - acc_h)
    };
    let menu_x0 = edge_x(menu_w);
    let menu_y0 = clamp_y(menu_y0, menu_h);
    let menu = kurbo::Rect::new(menu_x0, menu_y0, menu_x0 + menu_w, menu_y0 + menu_h);
    let accessory = accessory_size.map(|_| {
        let x0 = edge_x(acc_w);
        let y0 = clamp_y(acc_y0, acc_h);
        kurbo::Rect::new(x0, y0, x0 + acc_w, y0 + acc_h)
    });
    ContextMenuLayout {
        lift: source,
        lift_cropped: false,
        menu,
        accessory,
    }
}

impl SemanticCore {
    /// Whether `point` (hit space) lands inside the open context-menu
    /// presentation — its drawn menu or its accessory. Presses there belong
    /// to the presentation's own targets: they must not dismiss the menu,
    /// open a new one, or arm a press-and-hold (water-rs/waterui#1245 —
    /// accessory actions don't close the menu by themselves).
    pub(crate) fn context_menu_presentation_contains(&self, point: kurbo::Point) -> bool {
        self.popup_menu
            .context_menu_presentation
            .as_ref()
            .is_some_and(|presentation| {
                presentation.menu_frame.contains(point)
                    || presentation
                        .accessory_frame
                        .is_some_and(|frame| frame.contains(point))
            })
    }

    /// The open drawn `.context_menu` presentation's `(menu, accessory)`
    /// frames in hit space — `None` when none is open.
    #[cfg(test)]
    pub(crate) fn context_menu_presentation_frames(
        &self,
    ) -> Option<(kurbo::Rect, Option<kurbo::Rect>)> {
        self.popup_menu
            .context_menu_presentation
            .as_ref()
            .map(|presentation| (presentation.menu_frame, presentation.accessory_frame))
    }

    /// The lifted preview's frame as last laid out, in hit space — the
    /// source's rect unless fitting the stack moved or cropped it.
    #[cfg(test)]
    pub(crate) fn context_menu_lift_frame(&self) -> Option<kurbo::Rect> {
        self.popup_menu
            .context_menu_presentation
            .as_ref()
            .map(|presentation| presentation.lift_frame)
    }

    /// The drawn menu's row frames — the pointer targets inside `menu_frame`
    /// in hit order. The panel's swallow is the only target as tall as the
    /// menu itself, so the row targets are the shorter ones.
    #[cfg(test)]
    pub(crate) fn context_menu_row_frames(&self) -> Vec<kurbo::Rect> {
        let Some(presentation) = self.popup_menu.context_menu_presentation.as_ref() else {
            return Vec::new();
        };
        let menu_frame = presentation.menu_frame;
        self.hit_test
            .pointer_targets
            .iter()
            .map(|target| target.bounds)
            .filter(|bounds| {
                menu_frame.contains(bounds.center()) && bounds.height() < menu_frame.height() - 1.0
            })
            .collect()
    }
}

impl HydrolysisRenderer {
    /// Open a `.context_menu` popup at `origin` (hit space) — the drawn menu a
    /// secondary press or press-and-hold resolves. The drawn presentation
    /// mounts when the declaring node carries a preview or an accessory, or
    /// when the menu was opened by a touch or pen press-and-hold
    /// (water-rs/hydrolysis#191): the window dims, the preview — or the source
    /// itself — lifts at the source's rect, the menu sits beside it and the
    /// accessory anchors to its edge. A secondary click on a menu with
    /// neither keeps the plain popup at the pointer, as desktop menus do
    /// (water-rs/waterui#1245).
    pub(crate) fn show_context_menu(
        &mut self,
        nodes: Vec<PopupMenuNode>,
        target: Option<&ContextMenuTarget>,
        origin: LayoutPoint,
        metrics: TextContextMenuMetrics,
        env: &Environment,
        opened_by_hold: bool,
    ) -> bool {
        if nodes.is_empty() {
            return false;
        }
        // Reopening while a menu is up returns the previous presentation's
        // borrowed sub-views to their nodes first, so the new presentation
        // takes them as before.
        self.dismiss_active_popup_menu();
        let group = PopupMenuStateGroup::new();
        // The menu's chords publish to the app's shared registry — live while
        // any menu in the group is open — so a popup window that took
        // keyboard focus still resolves them (water-rs/hydrolysis#247).
        env.get::<MenuShortcutRegistry>()
            .expect(MISSING_MENU_SHORTCUT_REGISTRY)
            .register_popup(&nodes, &group, env);
        let theme = self.theme();
        let text = self.popup_menu_text_metrics(&nodes, metrics, env, &theme);
        let (menu_width, menu_height) = popup_menu_size(&nodes, metrics, &text);

        let presentation = opened_by_hold
            || target.is_some_and(|target| {
                target.preview.borrow().is_some() || target.accessory.borrow().is_some()
            });
        let source_bounds = target.map_or_else(
            || {
                kurbo::Rect::from_origin_size(
                    kurbo::Point::new(f64::from(origin.x), f64::from(origin.y)),
                    kurbo::Size::ZERO,
                )
            },
            |target| target.bounds,
        );
        let menu_origin = if presentation {
            // The accessory's size is unknown until its first measure; the
            // per-frame layout refines the frame once it is.
            let acc_probe =
                target.and_then(|target| target.accessory.borrow().is_some().then_some((0.0, 0.0)));
            let preview_present = target.is_some_and(|target| target.preview.borrow().is_some());
            let origin = context_menu_presentation_layout(
                source_bounds,
                preview_present,
                (menu_width, menu_height),
                acc_probe,
                self.window_bounds,
            )
            .menu;
            LayoutPoint::new(
                crate::num_cast::f64_as_f32(origin.x0),
                crate::num_cast::f64_as_f32(origin.y0),
            )
        } else {
            origin
        };
        let popup_origin = popup_window_origin(menu_origin, env);

        if presentation {
            // The drawn menu: no borderless window, so no transparent corner
            // wedges can stray over the dim. Its surface and rows are encoded
            // into the source window's scene by
            // `render_context_menu_presentation`; `menu_state` is the group's
            // close handle exactly as a popup window's would be.
            let menu_state = Binding::container(WindowState::Normal);
            group.push(menu_state.clone());
            let menu = RetainedSubview::new(AnyView::new(
                popup_menu_content(nodes, 0, metrics, text, popup_origin, menu_width, &theme)
                    .with(group.clone()),
            ));
            self.popup_menu.context_menu_presentation = Some(ContextMenuPresentation {
                source_bounds,
                menu,
                preview: target.and_then(|target| target.preview.borrow_mut().take()),
                accessory: target.and_then(|target| target.accessory.borrow_mut().take()),
                preview_slot: target.map_or_else(
                    || Rc::new(RefCell::new(None)),
                    |target| Rc::clone(&target.preview),
                ),
                accessory_slot: target.map_or_else(
                    || Rc::new(RefCell::new(None)),
                    |target| Rc::clone(&target.accessory),
                ),
                dismiss_requests: target.map_or_else(
                    || nami::Computed::constant(0),
                    |target| target.dismiss_requests.clone(),
                ),
                last_dismiss_requests: target
                    .map_or(0, |target| target.dismiss_requests.snapshot()),
                menu_state,
                menu_frame: kurbo::Rect::new(
                    f64::from(menu_origin.x),
                    f64::from(menu_origin.y),
                    f64::from(menu_origin.x) + menu_width,
                    f64::from(menu_origin.y) + menu_height,
                ),
                menu_size: (menu_width, menu_height),
                lift_frame: source_bounds,
                lift_cropped: false,
                accessory_frame: None,
                // `env` is the menu's layered environment (the declaring
                // view's over the dispatch's) — the same env the popup opens
                // inside is the one its sub-views build and flush in.
                env: env.clone(),
            });
            self.popup_menu.active_popup_menu_group = Some(group);
            self.request_refresh();
        } else {
            let (window, state) =
                popup_menu_window(nodes, popup_origin, group.clone(), 0, metrics, text, &theme);
            group.push(state);
            env.get::<PopupWindowManager>()
                .expect("hydrolysis popup menus require PopupWindowManager in environment")
                .show(window, env);
            self.popup_menu.active_popup_menu_group = Some(group);
        }
        true
    }
}

impl HydrolysisRenderer {
    /// Snapshot every hit-test registration the overlay can produce, run `f`
    /// (a sub-view flush), then truncate them back — the lifted preview is a
    /// snapshot of what the menu acts on: it renders, but it does not receive
    /// input (the source's own press target is what dismisses on press). The
    /// accessory flushes unsuppressed — it is the interactive half of the
    /// presentation.
    fn with_preview_targets_suppressed(&mut self, f: impl FnOnce(&mut Self)) {
        let pointer_start = self.hit_test.pointer_targets.len();
        let gesture_start = self.gesture_engine.target_count();
        let gesture_region_start = self.hit_test.gesture_regions.len();
        let cursor_start = self.hit_test.cursor_targets.len();
        let hover_start = self.hit_test.hover_targets.len();
        let drop_start = self.hit_test.drop_targets.len();
        let scroll_start = self.hit_test.scroll_targets.len();
        let menu_start = self.hit_test.context_menu_targets.len();
        let text_start = self.text_editing.text_input_targets.len();
        let embedded_start = self.hit_test.embedded_input_targets.len();
        let occlusion_start = self.hit_test.native_view_occlusions.len();

        f(self);

        self.hit_test.pointer_targets.truncate(pointer_start);
        self.ensure_active_pointer_drag_target_is_live();
        self.gesture_engine.truncate_targets(gesture_start);
        self.hit_test.gesture_regions.truncate(gesture_region_start);
        self.hit_test.cursor_targets.truncate(cursor_start);
        let removed_hover: Vec<_> = self.hit_test.hover_targets[hover_start..]
            .iter()
            .map(|target| (target.slot.clone(), target.handles.clone()))
            .collect();
        let now = self.frame_instant();
        for (slot, handles) in removed_hover {
            self.hit_test.interaction.set_hovering(&slot, false);
            if let Some(handles) = handles {
                handles.set_hovering(false, now);
            }
        }
        self.hit_test.hover_targets.truncate(hover_start);
        self.hit_test.drop_targets.truncate(drop_start);
        self.hit_test.scroll_targets.truncate(scroll_start);
        self.hit_test.context_menu_targets.truncate(menu_start);
        self.text_editing.text_input_targets.truncate(text_start);
        self.hit_test
            .embedded_input_targets
            .truncate(embedded_start);
        self.hit_test
            .native_view_occlusions
            .truncate(occlusion_start);
    }

    /// Encode the open `.context_menu` presentation into this window's scene —
    /// dim backdrop, lifted preview at the source's rect, and the interactive
    /// accessory anchored to the preview's edge — then tear it down when the
    /// menu is gone. Runs once a frame after the tree's flush, in the same
    /// position as the text context menu overlay, so the drawn presentation
    /// re-encodes with the frame it floats over.
    ///
    /// `transform` is the window's encode transform; the sub-views flush under
    /// an identity hit transform because their frames are already in hit
    /// space, the convention the text context menu overlay follows.
    #[expect(
        clippy::option_if_let_else,
        reason = "the if-let/else mirrors the control flow more clearly than the combinator chain here"
    )]
    #[expect(
        clippy::too_many_lines,
        reason = "the function drives one continuous scenario through the renderer; splitting it would obscure the sequence"
    )]
    pub(crate) fn render_context_menu_presentation(
        &mut self,
        transform: kurbo::Affine,
        safe_area: &SafeAreaLayout,
    ) {
        let Some(mut presentation) = self.popup_menu.context_menu_presentation.take() else {
            return;
        };

        // The menu's own handle is the truth: an item choice closes the popup
        // through the state group without clearing `active_popup_menu_group`.
        if self.read_signal(&presentation.menu_state) == WindowState::Closed {
            return; // drop(presentation) returns the sub-views to their node.
        }

        // Every `dismiss_requests` change asks the menu to close; the counter
        // wraps, so only a difference matters.
        let dismiss_requests = self.read_signal(&presentation.dismiss_requests);
        if dismiss_requests != presentation.last_dismiss_requests {
            self.dismiss_active_popup_menu();
            return;
        }
        presentation.last_dismiss_requests = dismiss_requests;

        let window = self.window_bounds;
        let source = presentation.source_bounds;
        let theme = self.theme();
        let metrics = theme.text_context_menu_metrics();
        let dim = peniko::Color::new([0.0, 0.0, 0.0, CONTEXT_MENU_DIM_OPACITY]);
        let shadow = peniko::Color::new([0.0, 0.0, 0.0, CONTEXT_MENU_SHADOW_OPACITY]);

        let presentation_env = presentation.env.clone();

        // Re-measure every frame: the accessory re-lays out whenever its
        // measured size changes (an expanding picker reflows while open). Its
        // frame is its panel — the content plus the theme's padding — so
        // presses on the panel's padding still belong to the accessory.
        let accessory_size = if let Some(accessory) = presentation.accessory.as_mut() {
            let (size, _stretch) =
                accessory.patch_and_measure(self, &presentation_env, ProposalSize::UNSPECIFIED);
            Some((
                metrics
                    .horizontal_padding
                    .mul_add(2.0, f64::from(size.width)),
                metrics
                    .vertical_padding
                    .mul_add(2.0, f64::from(size.height)),
            ))
        } else {
            None
        };

        // The stack — accessory, gap, lift, gap, menu — lays out around the
        // source each frame; when it cannot fit there the lifted preview
        // (never the source view) moves so the whole stack stays inside the
        // window, and only a stack taller than the window caps the preview's
        // height — it then draws cropped.
        let layout = context_menu_presentation_layout(
            source,
            presentation.preview.is_some(),
            presentation.menu_size,
            accessory_size,
            window,
        );
        presentation.menu_frame = layout.menu;
        presentation.accessory_frame = layout.accessory;
        presentation.lift_frame = layout.lift;
        presentation.lift_cropped = layout.lift_cropped;

        // The dim and the lift shadow draw inside an isolated layer, then a
        // destination-out fill cuts the lit hole back out for the source
        // view when no custom preview replaces it (the source keeps drawing
        // itself). The menu and accessory panels draw opaque over the dim —
        // their corner wedges stay scrim and their elevation shadows land on
        // it, so no square hole is punched for them.
        self.scene.with_group(
            peniko::Fill::NonZero,
            peniko::BlendMode::default(),
            1.0,
            transform,
            &window,
            |scene| {
                scene.fill(
                    peniko::Fill::NonZero,
                    transform,
                    &peniko::Brush::Solid(dim),
                    None,
                    &window,
                );
                scene.blurred_rounded_rect(
                    transform,
                    layout.lift,
                    working_color(shadow),
                    CONTEXT_MENU_LIFT_RADIUS,
                    CONTEXT_MENU_LIFT_SHADOW_RADIUS,
                );
                if presentation.preview.is_none() {
                    scene.with_group(
                        peniko::Fill::NonZero,
                        peniko::BlendMode {
                            mix: peniko::Mix::Normal,
                            compose: peniko::Compose::DestOut,
                        },
                        1.0,
                        transform,
                        &window,
                        |scene| {
                            let hole = kurbo::RoundedRect::from_rect(
                                layout.lift,
                                CONTEXT_MENU_LIFT_RADIUS,
                            );
                            scene.fill(
                                peniko::Fill::NonZero,
                                transform,
                                &peniko::Brush::Solid(peniko::Color::WHITE),
                                None,
                                &hole,
                            );
                        },
                    );
                }
            },
        );

        // Menu and accessory sit on the theme's context-menu surface —
        // container colour, radius and elevation — the same Material surface
        // the drawn text context menu gets.
        {
            self.scene.record_picture(transform, |draw| {
                theme.draw_text_context_menu_panel(&mut *draw, presentation.menu_frame);
                if let Some(accessory_frame) = presentation.accessory_frame {
                    theme.draw_text_context_menu_panel(&mut *draw, accessory_frame);
                }
            });
        }

        // Every press inside either painted panel belongs to the overlay —
        // rows, padding, and the gesture regions the dimmed content carries
        // beneath them alike. The occluders go in before the row/accessory
        // flushes so the real controls outrank them in hit order.
        for frame in std::iter::once(presentation.menu_frame).chain(presentation.accessory_frame) {
            self.register_hit_test_occluder(frame);
        }

        presentation.menu.flush_in_rect(
            self,
            RenderContext::with_transforms(window, transform, kurbo::Affine::IDENTITY),
            &presentation_env,
            bounded_proposal(presentation.menu_frame),
            presentation.menu_frame,
            // §7.1: the presentation is chrome of its own placement.
            Some(safe_area.for_subtree(presentation.menu_frame)),
        );

        if let Some(preview) = presentation.preview.as_mut() {
            self.with_suppressed_accessibility(|renderer| {
                renderer.with_preview_targets_suppressed(|renderer| {
                    preview.flush_in_rect(
                        renderer,
                        RenderContext::with_transforms(window, transform, kurbo::Affine::IDENTITY),
                        &presentation_env,
                        bounded_proposal(layout.lift),
                        layout.lift,
                        Some(safe_area.for_subtree(layout.lift)),
                    );
                });
            });
        }

        if let Some(accessory) = presentation.accessory.as_mut() {
            let frame = presentation.accessory_frame.unwrap_or(layout.lift);
            let content = inset_rect(frame, metrics.horizontal_padding, metrics.vertical_padding);
            accessory.flush_in_rect(
                self,
                RenderContext::with_transforms(window, transform, kurbo::Affine::IDENTITY),
                &presentation_env,
                bounded_proposal(content),
                content,
                Some(safe_area.for_subtree(content)),
            );
        }

        self.popup_menu.context_menu_presentation = Some(presentation);
    }
}
