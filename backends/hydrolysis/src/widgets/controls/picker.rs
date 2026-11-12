#[cfg(feature = "accessibility")]
use crate::renderer::AccessibilityActionTarget;
use crate::renderer::chrome::{ChromeContent, LabelPaint, LiveChromeParts};
use crate::renderer::live_segmented::{LiveSegmented, LiveSegmentedParts, SegmentedFrames};
use crate::renderer::live_toggle::LiveToggleParts;
use crate::renderer::{
    HydroNativeView, HydroState, HydrolysisRenderer, PickerMenuEntry, PickerMenuRequest,
    RenderContext, RetainedSubview, WidgetRenderContext, measure_picker_intrinsic,
    measure_picker_intrinsic_with_label_size,
};
use crate::renderer::{InteractionKey, Track, TrackAction, measure_segmented_intrinsic, menu_look};
#[cfg(feature = "accessibility")]
use crate::renderer::{picker_menu_panel_height, picker_menu_row_rect};
use crate::theme::Label as LookLabel;
use crate::theme::{
    Chrome, Inputs, MenuLayout, MenuPickerLayout, MenuPickerLook, PickerSpec, RadioGroupLook,
    RadioLayout, Segment, SegmentedLayout, SegmentedLook, SegmentedSpec, SpecTint, WidgetTheme,
};
use crate::widgets::controls::button::styled_with_default_foreground;
use crate::widgets::util::mirrored_in;
#[cfg(feature = "accessibility")]
use accesskit::{
    Action as AccessibilityAction, Node as AccessibilityNode, Role as AccessibilityNodeRole,
    Toggled as AccessibilityToggled,
};
use kurbo::{Point, Rect, Vec2};
use nami::{Binding, Computed, Signal, SignalExt as _};
use std::any::Any;
use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::rc::Rc;
use waterui::ViewExt as _;
use waterui_controls::label::Label;
use waterui_core::AnyView;
use waterui_core::Environment;
use waterui_core::Native;
use waterui_core::contract_violation;
use waterui_core::id::Id;
use waterui_core::interaction::InteractionState;
use waterui_core::layout::{
    HorizontalAlignment, LayoutDirection, ProposalSize, Size as LayoutSize, StretchAxis,
    ViewDimensions, layout_direction,
};
use waterui_form::picker::PickerItem;
use waterui_form::picker::{PickerConfig, PickerStyle};
use waterui_text::IntoText as _;
use waterui_text::font::Font;
use waterui_text::styled::StyledStr;

use crate::widgets::util::{label_beside_control_bounds, widget_disabled};

impl<E: crate::text::TextEngine> HydroNativeView<E> for Native<PickerConfig> {
    fn intrinsic(
        state: &mut HydroState<E>,
        view: &Self,
        env: &Environment,
        theme: &Rc<dyn crate::theme::WidgetTheme>,
    ) -> LayoutSize {
        measure_picker_intrinsic(view.as_inner(), state, env, theme)
    }
}

/// State shared by the retained picker node and its popup callbacks: the
/// cloneable [`PickerConfig`] drives the field chrome + accessibility, and its
/// label is held as a [`RetainedSubview`] built once and re-flushed each frame
/// so reactive label content stays live.
pub struct PickerRenderState<E: crate::text::TextEngine> {
    pub(crate) config: PickerConfig,
    label_view: RetainedSubview<E>,
    menu_open: Rc<Cell<bool>>,
    /// The menu picker's field look, held while its spec is.
    menu: Option<(PickerSpec, MenuPickerLook)>,
    /// The menu picker field's live chrome.
    menu_parts: Option<LiveChromeParts>,
    /// The state the field last reported, which feeds the chrome.
    menu_report: Binding<InteractionState>,
    /// The radio group's look, held while its spec is.
    radio: Option<(PickerSpec, RadioGroupLook)>,
    /// Each radio item's live indicator, keyed by the item.
    radio_parts: BTreeMap<Id, Option<LiveToggleParts>>,
    segmented: SegmentedState<E>,
}

/// The look a theme gave for a segmented picker's spec, held while the spec
/// is, with each segment's chrome it gives.
struct HeldSegmentedLook {
    spec: SegmentedSpec,
    look: SegmentedLook,
    /// Each segment's live selected flag, leading first: the one signal its
    /// label's font and its chrome both follow.
    selected: Rc<[Computed<bool>]>,
    /// Each segment's chrome, leading first: evaluated once for the spec.
    chromes: Rc<[Chrome]>,
}

/// The look a segmented picker is drawn with, as [`HeldSegmentedLook`]
/// holds it.
struct SegmentedLookParts {
    look: SegmentedLook,
    selected: Rc<[Computed<bool>]>,
    chromes: Rc<[Chrome]>,
}

/// A segmented picker's retained state: the look the theme gave for its
/// spec, the parts recorded from it, and each segment's label.
struct SegmentedState<E: crate::text::TextEngine> {
    look: Option<HeldSegmentedLook>,
    parts: Option<LiveSegmentedParts>,
    /// Each segment's label, leading first, built for the held look.
    labels: Vec<RetainedSubview<E>>,
    /// Raised by the items' watcher: the labels are built again.
    items_changed: Rc<Cell<bool>>,
    /// Raised by the selection's watcher for the accessibility publish.
    selection_changed: Rc<Cell<bool>>,
    watches: Vec<Box<dyn Any>>,
}

impl<E: crate::text::TextEngine> Default for SegmentedState<E> {
    fn default() -> Self {
        Self {
            look: None,
            parts: None,
            labels: Vec::new(),
            items_changed: Rc::new(Cell::new(false)),
            selection_changed: Rc::new(Cell::new(false)),
            watches: Vec::new(),
        }
    }
}

impl<E: crate::text::TextEngine> PickerRenderState<E> {
    pub(crate) fn from_config(config: PickerConfig) -> Self {
        Self {
            label_view: RetainedSubview::new(menu_picker_label_view(config.label())),
            config,
            menu_open: Rc::new(Cell::new(false)),
            menu: None,
            menu_parts: None,
            menu_report: Binding::container(InteractionState::empty()),
            radio: None,
            radio_parts: BTreeMap::new(),
            segmented: SegmentedState::default(),
        }
    }

    /// The menu picker's look for its spec in `env`: the one already held
    /// when the spec is unchanged, otherwise requested again.
    fn menu_picker_look(
        &mut self,
        theme: &Rc<dyn WidgetTheme>,
        env: &Environment,
    ) -> MenuPickerLook {
        let spec = picker_spec(&self.config, env);
        if self.menu.as_ref().is_none_or(|(held, _)| *held != spec) {
            let look = theme.menu_picker(&spec, env);
            self.menu = Some((spec, look));
            self.menu_parts = None;
        }
        let (_, look) = self.menu.as_ref().expect("the look was just ensured");
        look.clone()
    }

    /// The radio group's look for its spec in `env`: the one already held
    /// when the spec is unchanged, otherwise requested again, which drops
    /// every item's indicator built from the previous one.
    fn radio_group_look(
        &mut self,
        theme: &Rc<dyn WidgetTheme>,
        env: &Environment,
    ) -> RadioGroupLook {
        let spec = picker_spec(&self.config, env);
        if self.radio.as_ref().is_none_or(|(held, _)| *held != spec) {
            let look = theme.radio_group(&spec, env);
            self.radio = Some((spec, look));
            self.radio_parts.clear();
        }
        let (_, look) = self.radio.as_ref().expect("the look was just ensured");
        look.clone()
    }

    /// The segmented look for `spec` in `env`, each segment's live selected
    /// flag and each segment's chrome: the ones already held when the spec
    /// is unchanged, otherwise requested again, which drops the parts and
    /// the labels built from the previous one.
    fn segmented_look(
        &mut self,
        theme: &Rc<dyn WidgetTheme>,
        spec: SegmentedSpec,
        env: &Environment,
    ) -> SegmentedLookParts {
        if let Some(held) = self.held(&spec) {
            return SegmentedLookParts {
                look: held.look.clone(),
                selected: Rc::clone(&held.selected),
                chromes: Rc::clone(&held.chromes),
            };
        }
        let look = theme.segmented_picker(&spec, env);
        let selected = segment_selected_flags(&self.config, spec.segment_count());
        let chromes: Rc<[Chrome]> = look.segment_chromes(&selected).into();
        self.segmented.look = Some(HeldSegmentedLook {
            spec,
            look: look.clone(),
            selected: Rc::clone(&selected),
            chromes: Rc::clone(&chromes),
        });
        self.segmented.parts = None;
        self.segmented.labels.clear();
        SegmentedLookParts {
            look,
            selected,
            chromes,
        }
    }

    fn held(&self, spec: &SegmentedSpec) -> Option<&HeldSegmentedLook> {
        self.segmented
            .look
            .as_ref()
            .filter(|held| held.spec == *spec)
    }

    fn held_segmented_look(&self, spec: &SegmentedSpec) -> Option<&SegmentedLook> {
        self.held(spec).map(|held| &held.look)
    }

    /// The segmented look for `spec` in `env` without holding it: the held
    /// one, or a fresh one for a measurement or an accessibility emission
    /// ahead of the render that holds it.
    fn peek_segmented_look(
        &self,
        theme: &Rc<dyn WidgetTheme>,
        spec: &SegmentedSpec,
        env: &Environment,
    ) -> SegmentedLook {
        self.held_segmented_look(spec)
            .cloned()
            .unwrap_or_else(|| theme.segmented_picker(spec, env))
    }

    /// Eagerly build the label sub-view (the measure path has only
    /// `&mut HydroState`, no renderer, so it must be built before then).
    pub(crate) fn prebuild(
        &mut self,
        renderer: &mut crate::renderer::SemanticCore<E>,
        env: &Environment,
    ) {
        self.label_view.ensure_built(renderer, env);
    }

    /// Consume the label sub-view's layout-dirty marks: its measured size
    /// feeds the picker's, so an enclosing retained sub-view must see them
    /// (water-rs/waterui#1957).
    pub(crate) fn take_layout_dirty(&mut self) -> bool {
        self.label_view.take_layout_dirty_if_built()
    }
}

/// The label view a menu picker draws inside its field: the picker's own label
/// in the platform's field-label chrome — the `Caption` font slot and the
/// `MutedForeground` colour token the theme provides for chrome labels, never a
/// named size or colour. A hidden label keeps its zero-size body: it draws
/// nothing and takes no space. A custom-content label owns its own styling, so
/// only the colour token is applied to it.
pub fn menu_picker_label_view(label: &Label) -> AnyView {
    let styled = if label.has_custom_content() {
        label.clone()
    } else {
        label.clone().font(waterui_text::font::Caption)
    };
    AnyView::new(styled.muted())
}

/// Emits a picker's accessibility tree from its retained state. The rendered
/// `Widget`-node path passes its [`RenderContext`] and theme (option rows get
/// bounds); the semantic emission walk passes `None` for both — every action
/// target is semantic (selection bindings, direct menu activation), so no
/// geometry or style is needed.
#[expect(
    clippy::too_many_lines,
    reason = "the function drives one continuous scenario through the renderer; splitting it would obscure the sequence"
)]
pub fn picker_accessibility<E: crate::text::TextEngine>(
    renderer: &mut crate::renderer::SemanticCore<E>,
    ctx: Option<RenderContext>,
    theme: Option<&Rc<dyn crate::theme::WidgetTheme>>,
    state: &Rc<RefCell<PickerRenderState<E>>>,
    env: &Environment,
) {
    #[cfg(feature = "accessibility")]
    {
        let owner = state;
        let state = state.borrow();
        let picker = &state.config;
        let menu_open = Rc::clone(&state.menu_open);
        let disabled = renderer.read_signal(&widget_disabled(env));
        let items = renderer.read_signal(picker.items());
        if !has_items(&items) {
            return;
        }
        match picker.style() {
            PickerStyle::Automatic | PickerStyle::Menu => {
                let selected = renderer.read_signal(picker.selection());
                let selected_index = selected_item_index(&items, selected);
                let mut option_labels = Vec::with_capacity(items.len());
                let mut max_item_text_height: f64 = 0.0;
                for item in &items {
                    let label = renderer
                        .read_resolved_text_styled(&item.content, env)
                        .to_plain();
                    let label_size = HydrolysisRenderer::measure_text_intrinsic_size(
                        renderer.state_mut(),
                        StyledStr::plain(label.clone()),
                        env,
                    );
                    max_item_text_height = max_item_text_height.max(f64::from(label_size.height));
                    option_labels.push(label);
                }
                let selected_text = selected_option_text(&option_labels, selected_index);
                let mut node = AccessibilityNode::new(
                    crate::renderer::SemanticCore::<E>::resolve_accessibility_role(
                        env,
                        AccessibilityNodeRole::ComboBox,
                    ),
                );
                let default_label =
                    crate::renderer::SemanticCore::<E>::accessibility_label_from_label(
                        picker.label(),
                        env,
                    );
                let label = renderer.resolve_accessibility_label(env, default_label);
                if let Some(label) = label {
                    node.set_label(label);
                }
                // The selected option's text is the default value; an explicit
                // `.a11y_value` wins the same way `.a11y_label` wins the name.
                if let Some(value) = renderer
                    .resolve_accessibility_value(env, Some(selected_text.as_str().to_owned()))
                {
                    node.set_value(value);
                }
                node.add_action(AccessibilityAction::Focus);
                if disabled {
                    node.set_disabled();
                } else {
                    node.add_action(AccessibilityAction::Click);
                }
                // Option row bounds exist only in the rendered runtime, where
                // the popup rect is computable from the trigger's bounds and
                // the theme's field and menu looks: the same functions the
                // popup window is opened and drawn with.
                let option_geometry = ctx.as_ref().zip(theme).map(|(ctx, theme)| {
                    let (field, menu) = menu_picker_layouts(theme, picker, env);
                    let row_height = menu_picker_row_height(max_item_text_height, field, menu);
                    let popup_rect =
                        menu_picker_popup_rect(ctx.bounds(), row_height, items.len(), field, menu);
                    (ctx, row_height, popup_rect, menu)
                });
                for (index, item) in items.iter().enumerate() {
                    let mut option = AccessibilityNode::new(
                        crate::renderer::SemanticCore::<E>::resolve_accessibility_role(
                            env,
                            AccessibilityNodeRole::ListBoxOption,
                        ),
                    );
                    option.set_label(option_labels[index].as_str().to_owned());
                    let is_selected = item.tag == selected;
                    option.set_selected(is_selected);
                    option.set_toggled(AccessibilityToggled::from(is_selected));
                    option.add_action(AccessibilityAction::Focus);
                    if disabled {
                        option.set_disabled();
                    } else {
                        option.add_action(AccessibilityAction::Click);
                    }
                    let option_target =
                        (!disabled).then(|| AccessibilityActionTarget::PickerSelect {
                            selection: picker.selection().clone(),
                            target: item.tag,
                        });
                    let option_id = match option_geometry {
                        Some((_ctx, row_height, popup_rect, menu)) => {
                            let option_bounds =
                                menu_picker_option_rect(popup_rect, row_height, index, menu);
                            renderer.register_accessibility_child_node_with_key(
                                i64::from(i32::from(item.tag)),
                                option,
                                option_bounds,
                                env,
                                option_target,
                            )
                        }
                        None => renderer.register_accessibility_child_node_with_key_semantic(
                            i64::from(i32::from(item.tag)),
                            option,
                            env,
                            option_target,
                        ),
                    };
                    if let Some(option_id) = option_id {
                        node.push_child(option_id);
                    }
                }
                // Direct activation: `Click` does exactly what the field's
                // pointer target does — toggle the popup. The rendered runtime
                // shows the real window under the trigger; the semantic runtime
                // has no window manager, so it records the open state and
                // active menu group — placement is presentation detail only.
                let action_target = (!disabled).then(|| {
                    let menu_entries: Vec<PickerMenuEntry> = items
                        .iter()
                        .zip(option_labels.iter())
                        .map(|(item, label)| PickerMenuEntry {
                            label: label.to_string(),
                            tag: item.tag,
                        })
                        .collect();
                    let selection = picker.selection().clone();
                    let open = Rc::clone(&menu_open);
                    let request = ctx.as_ref().zip(theme).map(|(ctx, theme)| {
                        let (field, menu) = menu_picker_layouts(theme, picker, env);
                        let bounds = renderer.resolve_window_rect(ctx.bounds());
                        (
                            menu_picker_popup_origin(bounds, field),
                            bounds.width(),
                            menu_picker_row_height(max_item_text_height, field, menu),
                            menu_look(theme, env),
                        )
                    });
                    // The popup opens in the picker's environment layered over
                    // the dispatch's (water-rs/hydrolysis#140).
                    let picker_env = env.clone();
                    AccessibilityActionTarget::Activate {
                        action: Rc::new(RefCell::new(
                            move |renderer: &mut crate::renderer::SemanticCore<E>,
                                  env: &Environment| {
                                // `Click` toggles the popup — opening or
                                // dismissing is a handled activation either
                                // way, and so is a menu with nothing to show.
                                if open.get() {
                                    renderer.dismiss_active_popup_menu();
                                } else {
                                    let env = picker_env.layered_on(env);
                                    match &request {
                                        Some((origin, width, row_height, menu)) => {
                                            renderer.show_picker_menu(
                                                PickerMenuRequest {
                                                    entries: menu_entries.clone(),
                                                    selection: selection.clone(),
                                                    open: Rc::clone(&open),
                                                    origin: *origin,
                                                    width: *width,
                                                    row_height: *row_height,
                                                    selected,
                                                    menu: menu.clone(),
                                                },
                                                &env,
                                            );
                                        }
                                        None => {
                                            renderer.activate_picker_menu(
                                                menu_entries.clone(),
                                                selection.clone(),
                                                &open,
                                                &env,
                                            );
                                        }
                                    }
                                }
                                true
                            },
                        )),
                    }
                });
                let trigger_id = match ctx {
                    Some(ctx) => {
                        let bounds = ctx.bounds();
                        renderer.register_accessibility_node(node, bounds, env, action_target)
                    }
                    None => renderer.register_accessibility_node_semantic(node, env, action_target),
                };
                if let Some(trigger_id) = trigger_id {
                    renderer.register_accessibility_focus_link(
                        &crate::renderer::InteractionKey::for_rc(owner, 0),
                        trigger_id,
                    );
                }
            }
            PickerStyle::Segmented => {
                let mut group = option_group::<E>(renderer, picker, env);
                // The rendered tree follows the selection live: the render
                // path reads it unwatched, and the watcher's flag publishes
                // each change. The semantic walk has no render to skip.
                let selected = if ctx.is_some() {
                    picker.selection().snapshot()
                } else {
                    renderer.read_signal(picker.selection())
                };
                let labels: Vec<_> = items
                    .iter()
                    .map(|item| renderer.read_resolved_text_styled(&item.content, env))
                    .collect();
                // The segments' rectangles come from the placement the
                // parts and labels are drawn by.
                let segment_rects = ctx.as_ref().zip(theme).map(|(ctx, theme)| {
                    let spec = segmented_spec(picker, items.len(), env);
                    let look = state.peek_segmented_look(theme, &spec, env);
                    let sizes: Vec<_> = labels
                        .iter()
                        .enumerate()
                        .map(|(index, styled)| {
                            let segment = Segment {
                                index,
                                count: items.len(),
                            };
                            segmented_label_size(&look, styled, segment, renderer.state_mut(), env)
                        })
                        .collect();
                    let heading = state
                        .label_view
                        .measure_built(renderer.state_mut(), env, theme);
                    place_segmented(
                        look.layout(),
                        &sizes,
                        f64::from(heading.height),
                        ctx.bounds(),
                        spec.direction(),
                    )
                    .drawn
                });
                let mut options = Vec::with_capacity(items.len());
                for (index, (item, label)) in items.iter().zip(&labels).enumerate() {
                    let rect = segment_rects.as_ref().map(|rects| rects[index]);
                    let key = option_key(owner, item.tag);
                    register_option(
                        renderer,
                        &mut group,
                        &key,
                        picker,
                        item.tag,
                        label.to_plain().to_string(),
                        item.tag == selected,
                        disabled,
                        rect,
                        env,
                    );
                    options.push((key, item.tag));
                }
                if ctx.is_some() {
                    // An option's discriminator is its tag's 32 bits, so
                    // `usize::MAX` names the group alone.
                    renderer.register_live_selection(
                        InteractionKey::for_rc(owner, usize::MAX),
                        Rc::clone(&state.segmented.selection_changed),
                        picker.selection().clone(),
                        options,
                    );
                }
                match ctx {
                    Some(ctx) => {
                        let _ =
                            renderer.register_accessibility_node(group, ctx.bounds(), env, None);
                    }
                    None => {
                        let _ = renderer.register_accessibility_node_semantic(group, env, None);
                    }
                }
            }
            PickerStyle::Radio => {
                let mut group = option_group::<E>(renderer, picker, env);
                let geometry = ctx
                    .as_ref()
                    .zip(theme)
                    .map(|(ctx, theme)| (ctx, theme.radio_group(&picker_spec(picker, env), env)));
                // The group heading offsets the option rows exactly as the
                // render path lays them out, so emitted row bounds overlay the
                // drawn rows — the same presence rule: measured height > 0.
                let group_label_height = theme.map_or(0.0, |theme| {
                    f64::from(
                        state
                            .label_view
                            .measure_built(renderer.state_mut(), env, theme)
                            .height,
                    )
                });
                let selected = renderer.read_signal(picker.selection());
                let mut row_y = geometry.as_ref().map(|(ctx, look)| {
                    ctx.bounds().y0
                        + look.layout().vertical_inset
                        + if group_label_height > 0.0 {
                            group_label_height + look.layout().label_spacing
                        } else {
                            0.0
                        }
                });
                // Each option's label, measured as the render path measures
                // it, so the rows it sizes match the drawn ones.
                let labels: Vec<_> = items
                    .iter()
                    .map(|item| {
                        let styled = renderer.read_resolved_text_styled(&item.content, env);
                        let measured = match &geometry {
                            Some((_, look)) => label_styled(look.get_label(), styled.clone()),
                            None => styled.clone(),
                        };
                        let size = HydrolysisRenderer::measure_text_intrinsic_size(
                            renderer.state_mut(),
                            measured,
                            env,
                        );
                        (styled.to_plain().to_string(), size)
                    })
                    .collect();
                for (item, (label, label_size)) in items.iter().zip(labels) {
                    let row_rect = geometry.as_ref().map(|(ctx, look)| {
                        let y = row_y.unwrap_or_else(|| ctx.bounds().y0);
                        let rect =
                            radio_row_rect(look, ctx.bounds(), y, f64::from(label_size.height));
                        row_y = Some(rect.y1 + look.layout().row_spacing);
                        rect
                    });
                    if let Some(rect) = row_rect
                        && rect.height() <= 0.0
                    {
                        break;
                    }
                    register_option(
                        renderer,
                        &mut group,
                        &option_key(owner, item.tag),
                        picker,
                        item.tag,
                        label,
                        item.tag == selected,
                        disabled,
                        row_rect,
                        env,
                    );
                }
                match ctx {
                    Some(ctx) => {
                        let group_bounds = ctx.bounds();
                        let _ =
                            renderer.register_accessibility_node(group, group_bounds, env, None);
                    }
                    None => {
                        let _ = renderer.register_accessibility_node_semantic(group, env, None);
                    }
                }
            }
            style => contract_violation!(
                "hydrolysis PickerStyle variant {style:?} is not implemented; the picker exposes no accessibility node"
            ),
        }
    }
    #[cfg(not(feature = "accessibility"))]
    {
        let _ = (renderer, ctx, theme, state, env);
    }
}

/// The interaction identity of the option tagged `tag`: the key its
/// segment or row binds its interaction by and links its node to.
fn option_key<E: crate::text::TextEngine>(
    owner: &Rc<RefCell<PickerRenderState<E>>>,
    tag: Id,
) -> InteractionKey {
    InteractionKey::for_rc(owner, crate::num_cast::i32_as_u32(i32::from(tag)) as usize)
}

/// The group node a radio or segmented picker's options hang under.
#[cfg(feature = "accessibility")]
fn option_group<E: crate::text::TextEngine>(
    renderer: &mut crate::renderer::SemanticCore<E>,
    picker: &PickerConfig,
    env: &Environment,
) -> AccessibilityNode {
    let mut group = AccessibilityNode::new(
        crate::renderer::SemanticCore::<E>::resolve_accessibility_role(
            env,
            AccessibilityNodeRole::Group,
        ),
    );
    let default_label =
        crate::renderer::SemanticCore::<E>::accessibility_label_from_label(picker.label(), env);
    if let Some(label) = renderer.resolve_accessibility_label(env, default_label) {
        group.set_label(label);
    }
    if let Some(value) = renderer.resolve_accessibility_value(env, None) {
        group.set_value(value);
    }
    group
}

/// Emits the radio-button node of the option tagged `tag` under `group`, at
/// `rect` in the rendered tree (`None` in the semantic one), linked to the
/// interaction `key` binds.
#[cfg(feature = "accessibility")]
#[expect(
    clippy::too_many_arguments,
    reason = "one option's independent facts: its group, identity, selection, label, state and place"
)]
fn register_option<E: crate::text::TextEngine>(
    renderer: &mut crate::renderer::SemanticCore<E>,
    group: &mut AccessibilityNode,
    key: &InteractionKey,
    picker: &PickerConfig,
    tag: Id,
    label: String,
    is_selected: bool,
    disabled: bool,
    rect: Option<kurbo::Rect>,
    env: &Environment,
) {
    let mut option = AccessibilityNode::new(
        crate::renderer::SemanticCore::<E>::resolve_accessibility_role(
            env,
            AccessibilityNodeRole::RadioButton,
        ),
    );
    option.set_label(label);
    option.set_toggled(AccessibilityToggled::from(is_selected));
    option.set_selected(is_selected);
    option.add_action(AccessibilityAction::Focus);
    if disabled {
        option.set_disabled();
    } else {
        option.add_action(AccessibilityAction::Click);
    }
    let target = (!disabled).then(|| AccessibilityActionTarget::PickerSelect {
        selection: picker.selection().clone(),
        target: tag,
    });
    let child_key = i64::from(i32::from(tag));
    let child_id = match rect {
        Some(rect) => renderer
            .register_accessibility_child_node_with_key(child_key, option, rect, env, target),
        None => renderer
            .register_accessibility_child_node_with_key_semantic(child_key, option, env, target),
    };
    if let Some(child_id) = child_id {
        group.push_child(child_id);
        renderer.register_accessibility_focus_link(key, child_id);
    }
}

/// Measures a retained picker leaf from its [`PickerRenderState`], reading the
/// label size from its already-built [`RetainedSubview`] so layout and the
/// in-field label render agree (mirrors [`measure_picker_intrinsic`]).
///
/// A segmented picker (`StretchAxis::Horizontal`) answers a finite width
/// proposal, never below its intrinsic width; every other picker answers its
/// intrinsic size (`docs/layout-spec.md` §6).
pub fn measure_picker_node<E: crate::text::TextEngine>(
    state: &PickerRenderState<E>,
    proposal: ProposalSize,
    hydro: &mut HydroState<E>,
    env: &Environment,
    theme: &Rc<dyn crate::theme::WidgetTheme>,
) -> ViewDimensions {
    let label_size = state.label_view.measure_built(hydro, env, theme);
    let intrinsic = if state.config.style() == PickerStyle::Segmented {
        // The held look, so the measurement and the drawing agree.
        let items = hydro.measure_signal(state.config.items());
        let spec = segmented_spec(&state.config, items.len(), env);
        let look = state.peek_segmented_look(theme, &spec, env);
        measure_segmented_intrinsic(&look, &items, label_size, hydro, env)
    } else {
        measure_picker_intrinsic_with_label_size(&state.config, label_size, hydro, env, theme)
    };
    let width = match (
        waterui_core::NativeView::stretch_axis(&state.config),
        proposal.width,
    ) {
        (StretchAxis::Horizontal, Some(proposed)) if proposed.is_finite() => {
            proposed.max(intrinsic.width)
        }
        _ => intrinsic.width,
    };
    ViewDimensions::new(LayoutSize::new(width, intrinsic.height))
}

/// Renders a retained picker leaf every flush: emits a11y (unless hidden) then the
/// style-specific chrome + options, reading the items/selection signals each frame.
pub fn render_picker_node<E: crate::text::TextEngine>(
    ctx: &mut WidgetRenderContext<'_, E>,
    state: &Rc<RefCell<PickerRenderState<E>>>,
    env: &Environment,
) {
    let hidden = env
        .get::<waterui::accessibility::AccessibilityHidden>()
        .is_some_and(waterui::accessibility::AccessibilityHidden::is_hidden);
    if !hidden {
        let theme = ctx.theme();
        let render_ctx = ctx.render_context();
        picker_accessibility(
            ctx.renderer_mut(),
            Some(render_ctx),
            Some(&theme),
            state,
            env,
        );
    }
    render_picker_parts(ctx, state, env);
}

pub fn render_picker_parts<E: crate::text::TextEngine>(
    ctx: &mut WidgetRenderContext<'_, E>,
    state: &Rc<RefCell<PickerRenderState<E>>>,
    env: &Environment,
) {
    // The items/selection signals are read through `read_signal` so a membership or
    // selection change schedules a frame and this persistent node re-renders.
    let (items_signal, selection, style) = {
        let picker = state.borrow();
        (
            picker.config.items().clone(),
            picker.config.selection().clone(),
            picker.config.style(),
        )
    };
    let items = ctx.renderer_mut().read_signal(&items_signal);
    if !has_items(&items) {
        return;
    }
    match style {
        PickerStyle::Automatic | PickerStyle::Menu => {
            render_menu_picker(ctx, state, selection, items, env);
        }
        PickerStyle::Radio => {
            render_radio_picker(ctx, state, selection, items, env);
        }
        PickerStyle::Segmented => {
            render_segmented_picker(ctx, state, selection, items, env);
        }
        style => contract_violation!(
            "hydrolysis PickerStyle variant {style:?} is not implemented; the picker is not drawn"
        ),
    }
}

/// Whether the picker has an item to show. A picker without items is a
/// contract violation by the app, reported; the picker then draws and exposes
/// nothing.
fn has_items(items: &[PickerItem<Id>]) -> bool {
    let has_items = !items.is_empty();
    if !has_items {
        contract_violation!(
            "hydrolysis picker requires at least one item; the picker is not drawn"
        );
    }
    has_items
}

/// The index of the item `selected` tags. A selection that tags no item is a
/// contract violation by the app, reported; the picker then shows no
/// selected text.
fn selected_item_index(items: &[PickerItem<Id>], selected: Id) -> Option<usize> {
    let index = items.iter().position(|item| item.tag == selected);
    if index.is_none() {
        contract_violation!(
            "hydrolysis picker selection is not present in picker items; the picker shows no selected text"
        );
    }
    index
}

/// The text of the selected option, empty when nothing is selected.
fn selected_option_text<T: Clone + Default>(texts: &[T], selected: Option<usize>) -> T {
    selected
        .and_then(|index| texts.get(index))
        .cloned()
        .unwrap_or_default()
}

/// The picker request `config` makes of the theme: its control size, the
/// layout direction of `env` and its tint.
pub fn picker_spec(config: &PickerConfig, env: &Environment) -> PickerSpec {
    PickerSpec::new(
        config.size(),
        layout_direction(env).snapshot(),
        SpecTint::resolve(env),
    )
}

/// The measures of the menu picker `config` is laid out by and of the menu
/// its popup draws, requested from `theme` for `env`.
fn menu_picker_layouts(
    theme: &Rc<dyn WidgetTheme>,
    config: &PickerConfig,
    env: &Environment,
) -> (MenuPickerLayout, MenuLayout) {
    (
        theme.menu_picker(&picker_spec(config, env), env).layout(),
        menu_look(theme, env).layout(),
    )
}

/// `styled` in the font and colour `label` gives its text at rest: the
/// styling of a menu picker's selected value and of a radio row's label,
/// which has no state of its own.
pub fn label_styled(label: &LookLabel, styled: StyledStr) -> StyledStr {
    let inputs = Inputs::at_rest(Rect::ZERO, InteractionState::empty(), &());
    let styled = match label.get_font() {
        Some(font) => styled.font(font.target(&inputs)),
        None => styled,
    };
    match label.get_color() {
        Some(color) => styled_with_default_foreground(styled, color.target(&inputs)),
        None => styled,
    }
}

/// The height of one row of a menu picker's popup: the menu's row height,
/// grown to fit the tallest option text inside the field's vertical inset.
pub const fn menu_picker_row_height(
    max_item_text_height: f64,
    field: MenuPickerLayout,
    menu: MenuLayout,
) -> f64 {
    menu.row_height
        .max(field.vertical_inset.mul_add(2.0, max_item_text_height))
}

/// Where a menu picker's popup opens for a field at `field_bounds`: its
/// leading edge at the field's, `popup_gap` below the field's bottom edge.
pub fn menu_picker_popup_origin(
    field_bounds: kurbo::Rect,
    field: MenuPickerLayout,
) -> waterui_core::layout::Point {
    let anchor = crate::widgets::util::popup_anchor(field_bounds);
    waterui_core::layout::Point::new(
        anchor.x,
        anchor.y + crate::num_cast::f64_as_f32(field.popup_gap),
    )
}

/// The panel of a menu picker's popup for a field at `field_bounds`: the
/// rect the popup window is opened at and the accessibility option
/// rectangles are laid inside.
#[cfg(feature = "accessibility")]
pub fn menu_picker_popup_rect(
    field_bounds: kurbo::Rect,
    row_height: f64,
    item_count: usize,
    field: MenuPickerLayout,
    menu: MenuLayout,
) -> kurbo::Rect {
    let y0 = field_bounds.y1 + field.popup_gap;
    let y1 = y0 + picker_menu_panel_height(menu, row_height, item_count);
    kurbo::Rect::new(field_bounds.x0, y0, field_bounds.x1, y1)
}

/// Option `index`'s rect inside `popup_rect`: the row the popup window lays
/// the option's button in.
#[cfg(feature = "accessibility")]
pub fn menu_picker_option_rect(
    popup_rect: kurbo::Rect,
    row_height: f64,
    index: usize,
    menu: MenuLayout,
) -> kurbo::Rect {
    let row = picker_menu_row_rect(menu, row_height, popup_rect.width(), index);
    row.with_origin((popup_rect.x0 + row.x0, popup_rect.y0 + row.y0))
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "the parameter is a small Copy value taken by value for a uniform call-site signature"
)]
#[expect(
    clippy::too_many_lines,
    reason = "one continuous render pass over the picker's parts; splitting it would only mirror the steps artificially"
)]
pub fn render_menu_picker<E: crate::text::TextEngine>(
    ctx: &mut WidgetRenderContext<'_, E>,
    owner: &Rc<RefCell<PickerRenderState<E>>>,
    selection: Binding<Id>,
    items: Vec<PickerItem<Id>>,
    env: &Environment,
) {
    let interaction_key = crate::renderer::InteractionKey::for_rc(owner, 0);
    let theme = ctx.theme();
    let look = owner.borrow_mut().menu_picker_look(&theme, env);
    let metrics = look.layout();
    let menu = menu_look(&theme, env).layout();
    let selected = ctx.renderer_mut().read_signal(&selection);
    // Register this node-owned open handle so an outside click can dismiss it; the
    // registry is Rc-pruned, so re-registering the same handle each frame is a no-op.
    let menu_open = Rc::clone(&owner.borrow().menu_open);
    ctx.renderer_mut().register_picker_menu(&menu_open);
    let selected_index = selected_item_index(&items, selected);
    let mut option_texts = Vec::with_capacity(items.len());
    let mut max_item_text_height: f64 = 0.0;
    for item in &items {
        let styled = ctx
            .renderer_mut()
            .read_resolved_text_styled(&item.content, env);
        let plain = styled.to_plain();
        let size = HydrolysisRenderer::measure_text_intrinsic_size(
            ctx.state_mut(),
            StyledStr::plain(plain.clone()),
            env,
        );
        max_item_text_height = max_item_text_height.max(f64::from(size.height));
        option_texts.push(plain);
    }
    let selected_text = selected_option_text(&option_texts, selected_index);
    let row_height = menu_picker_row_height(max_item_text_height, metrics, menu);
    let entries = items
        .iter()
        .zip(option_texts.iter())
        .map(|(item, label)| PickerMenuEntry {
            label: label.to_string(),
            tag: item.tag,
        })
        .collect::<Vec<_>>();

    let bounds = ctx.bounds;
    let chrome = look.chrome().clone();
    let (parts, press_slot, hit_bounds) = {
        let target = chrome.target(bounds);
        let (_, press_slot, handles) = ctx.renderer_mut().bind_live_chrome_target(
            interaction_key.clone(),
            target,
            env,
            false,
            &chrome,
        );
        let flags = ctx
            .renderer_mut()
            .reported_interaction_state(&interaction_key);
        let mut state = owner.borrow_mut();
        if state.menu_report.snapshot() != flags {
            state.menu_report.set(flags);
        }
        let report = state.menu_report.clone();
        let parts = LiveChromeParts::ensure(
            &mut state.menu_parts,
            &chrome,
            &ChromeContent::default(),
            bounds,
            flags,
            env,
            &report,
        )
        .clone();
        handles.attach_live(Rc::clone(parts.chrome()));
        (parts, press_slot, target)
    };
    let render_ctx = ctx.render_context();
    parts.push_below(ctx.renderer_mut(), render_ctx);
    {
        let field_open_state = Rc::clone(&menu_open);
        let picker_selection = selection;
        let menu_entries = entries;
        // The popup opens in window space `popup_gap` below the field's
        // bottom edge: the one panel the accessibility option rectangles lie
        // in too.
        let window_bounds = ctx.renderer_mut().resolve_window_rect(bounds);
        let menu_origin = menu_picker_popup_origin(window_bounds, metrics);
        let menu_width = window_bounds.width();
        let popup_menu = menu_look(&theme, env);
        // The popup opens in the picker's environment layered over the
        // dispatch's (water-rs/hydrolysis#140).
        let picker_env = env.clone();
        ctx.renderer_mut().register_interactive_pointer_target(
            hit_bounds,
            press_slot,
            move |renderer, _point, env| {
                if field_open_state.get() {
                    renderer.dismiss_active_popup_menu();
                    false
                } else {
                    let env = picker_env.layered_on(env);
                    renderer.show_picker_menu(
                        PickerMenuRequest {
                            entries: menu_entries.clone(),
                            selection: picker_selection.clone(),
                            open: Rc::clone(&field_open_state),
                            origin: menu_origin,
                            width: menu_width,
                            row_height,
                            selected,
                            menu: popup_menu.clone(),
                        },
                        &env,
                    )
                }
            },
        );
    }

    let label_size = owner
        .borrow_mut()
        .label_view
        .measure_intrinsic(ctx.renderer_mut(), env);
    // One rule decides the label's presence in both measure and render: it is
    // drawn iff its measured height is nonzero.
    let right_to_left = layout_direction(env).snapshot().is_right_to_left();
    let (label_bounds, text_bounds) =
        menu_picker_content_rects(ctx.bounds, metrics, label_size, right_to_left);
    if let Some(label_bounds) = label_bounds {
        flush_picker_label(ctx, &mut owner.borrow_mut().label_view, env, label_bounds);
    }
    let styled = label_styled(look.get_value(), StyledStr::plain(selected_text));
    ctx.render_styled_text(styled, HorizontalAlignment::Leading, env, text_bounds);
    let render_ctx = ctx.render_context();
    parts.push_above(ctx.renderer_mut(), render_ctx);
}

/// The menu picker's in-field content layout: the field label drawn above the
/// selected value, both inset by the metrics' horizontal inset and kept clear
/// of the dropdown indicator, with the theme's label spacing between them.
/// A label that measures empty (a hidden or content-free label) draws nothing
/// and takes no space — the value keeps its full-height inset. The label is
/// placed at its measured width, flush with the content's leading edge — the
/// left in left-to-right layout, the right in right-to-left — so it lines up
/// with the value, never centred over the field.
///
/// The look draws the field left to right, with the dropdown indicator at
/// its right end; in a right-to-left layout the content is that layout
/// reflected about the field, so the indicator's space lies at the left end
/// and the content is flush with the right.
pub fn menu_picker_content_rects(
    bounds: kurbo::Rect,
    metrics: MenuPickerLayout,
    label_size: LayoutSize,
    right_to_left: bool,
) -> (Option<kurbo::Rect>, kurbo::Rect) {
    let label_height = f64::from(label_size.height);
    let label_width = f64::from(label_size.width);
    let text_x0 = bounds.x0 + metrics.horizontal_inset;
    let text_x1 = (bounds.x1 - metrics.horizontal_inset - metrics.indicator_space).max(text_x0);
    let text_bottom = bounds.y1 - metrics.vertical_inset;
    let (label_x0, label_x1) = (text_x0, (text_x0 + label_width).min(text_x1));
    let (label, value) = if label_height > 0.0 {
        let label_rect = kurbo::Rect::new(
            label_x0,
            bounds.y0 + metrics.vertical_inset,
            label_x1,
            bounds.y0 + metrics.vertical_inset + label_height,
        );
        let value_rect = kurbo::Rect::new(
            text_x0,
            label_rect.y1 + metrics.label_spacing,
            text_x1,
            text_bottom,
        );
        (Some(label_rect), value_rect)
    } else {
        (
            None,
            kurbo::Rect::new(
                text_x0,
                bounds.y0 + metrics.vertical_inset,
                text_x1,
                text_bottom,
            ),
        )
    };
    if right_to_left {
        (
            label.map(|rect| mirrored_in(rect, bounds)),
            mirrored_in(value, bounds),
        )
    } else {
        (label, value)
    }
}

/// The radio group's label layout: the heading sits in the top inset band
/// inside the horizontal insets — aligned with the option rows' leading edge —
/// and the rows begin below it with the metrics' label spacing between them.
/// The label is present iff its measured height is nonzero — the same rule the
/// measure path uses. A label measuring zero height (a hidden one) draws
/// nothing and takes no space: the rows begin at `content_y_without_label`.
pub fn radio_group_label_area(
    bounds: kurbo::Rect,
    metrics: RadioLayout,
    label_height: f64,
    content_y_without_label: f64,
) -> (Option<kurbo::Rect>, f64) {
    if label_height > 0.0 {
        let heading = kurbo::Rect::new(
            bounds.x0 + metrics.horizontal_inset,
            bounds.y0 + metrics.vertical_inset,
            bounds.x1 - metrics.horizontal_inset,
            bounds.y0 + metrics.vertical_inset + label_height,
        );
        (Some(heading), heading.y1 + metrics.label_spacing)
    } else {
        (None, content_y_without_label)
    }
}

/// The spec of the segmented picker `config` describes, with `count`
/// segments, in `env`.
pub fn segmented_spec(config: &PickerConfig, count: usize, env: &Environment) -> SegmentedSpec {
    SegmentedSpec::new(
        config.size(),
        count,
        layout_direction(env).snapshot(),
        SpecTint::resolve(env),
    )
}

/// The font `look` draws `segment`'s label in, selected or not.
fn segment_font(look: &SegmentedLook, segment: Segment, selected: bool) -> Font {
    let state = if selected {
        InteractionState::SELECTED
    } else {
        InteractionState::empty()
    };
    look.label()
        .get_font()
        .target(&Inputs::at_rest(Rect::ZERO, state, &segment))
}

/// The size `segment`'s label `styled` is laid out at under `look`: the
/// larger of its selected and unselected fonts' in each direction, so a
/// selection change never changes the picker's size.
pub fn segmented_label_size<E: crate::text::TextEngine>(
    look: &SegmentedLook,
    styled: &StyledStr,
    segment: Segment,
    state: &mut HydroState<E>,
    env: &Environment,
) -> LayoutSize {
    let unselected = HydrolysisRenderer::measure_text_intrinsic_size(
        state,
        styled.clone().font(segment_font(look, segment, false)),
        env,
    );
    let selected = HydrolysisRenderer::measure_text_intrinsic_size(
        state,
        styled.clone().font(segment_font(look, segment, true)),
        env,
    );
    LayoutSize::new(
        unselected.width.max(selected.width),
        unselected.height.max(selected.height),
    )
}

/// The intrinsic size of a segmented picker laid out by `layout` whose
/// segments' labels measure `sizes` under a heading of `label_size`: each
/// segment as wide as its label and padding, at least the least segment
/// width, the row as tall as [`segmented_row_height`], and a visible heading
/// above it, spaced by the layout's label spacing.
pub fn segmented_intrinsic(
    layout: &SegmentedLayout,
    sizes: &[LayoutSize],
    label_size: LayoutSize,
) -> LayoutSize {
    let total_width: f64 = sizes
        .iter()
        .map(|size| {
            layout
                .label_padding
                .x
                .mul_add(2.0, f64::from(size.width))
                .max(layout.minimum_segment_width)
        })
        .sum();
    // The heading spans the row edge to edge: its width competes with the
    // row's with no padding term. A zero-height (hidden) heading adds
    // nothing.
    let width = total_width
        .max(f64::from(label_size.width))
        .max(layout.minimum_width);
    let row_height = segmented_row_height(layout, sizes.iter().map(|size| size.height));
    let height = row_height
        + if label_size.height > 0.0 {
            f64::from(label_size.height) + layout.label_spacing
        } else {
            0.0
        };
    LayoutSize::new(
        crate::num_cast::f64_as_f32(width),
        crate::num_cast::f64_as_f32(height),
    )
}

/// The segment row's height from its segments' label heights: the tallest
/// label plus its vertical padding, floored at the layout's row height.
///
/// Layout carries extents as `f32`, so the row height is quantised to `f32`
/// here, once. The measured size adds the heading on top of this value, and
/// the drawn row and the emitted segment bounds take it as is. A row derived
/// as the bounds minus the heading would inherit the rounding of the `f32`
/// total instead, and the labelled row would differ from the unlabelled one.
pub fn segmented_row_height(
    layout: &SegmentedLayout,
    item_heights: impl IntoIterator<Item = f32>,
) -> f64 {
    let max_item_height = item_heights.into_iter().map(f64::from).fold(0.0, f64::max);
    f64::from(crate::num_cast::f64_as_f32(
        layout
            .label_padding
            .y
            .mul_add(2.0, max_item_height)
            .max(layout.row_height),
    ))
}

/// Where a segmented picker's heading, parts and labels lie in its bounds.
#[derive(Debug, Clone)]
pub struct SegmentedPlacement {
    /// The heading's rect, when the heading is visible.
    pub heading: Option<Rect>,
    /// The frames the parts record against, left to right.
    pub frames: SegmentedFrames,
    /// Each segment's rect as drawn, leading first: the frames mirrored
    /// about the row in a right-to-left layout.
    pub drawn: Vec<Rect>,
}

/// Places a segmented picker laid out by `layout` in `bounds`: the heading of
/// `label_height` at the top, the row of equal segments on the bottom edge,
/// mirrored about the row in a right-to-left `direction`. Every consumer —
/// the parts, the labels, the touch targets and the accessibility tree —
/// reads this one placement.
pub fn place_segmented(
    layout: &SegmentedLayout,
    sizes: &[LayoutSize],
    label_height: f64,
    bounds: Rect,
    direction: waterui_core::layout::LayoutDirection,
) -> SegmentedPlacement {
    let row_height = segmented_row_height(layout, sizes.iter().map(|size| size.height));
    let (heading, row) = segmented_label_area(bounds, label_height, row_height);
    let count = crate::num_cast::usize_as_f64(sizes.len());
    let width = row.width() / count;
    let segments: Vec<Rect> = (0..sizes.len())
        .map(|index| {
            let x0 = width.mul_add(crate::num_cast::usize_as_f64(index), row.x0);
            Rect::new(x0, row.y0, x0 + width, row.y1)
        })
        .collect();
    let mirrored = direction.is_right_to_left();
    let drawn = segments
        .iter()
        .map(|segment| {
            if mirrored {
                Rect::new(
                    row.x0 + row.x1 - segment.x1,
                    segment.y0,
                    row.x0 + row.x1 - segment.x0,
                    segment.y1,
                )
            } else {
                *segment
            }
        })
        .collect();
    SegmentedPlacement {
        heading,
        frames: SegmentedFrames {
            row,
            segments,
            mirrored,
        },
        drawn,
    }
}

/// The segmented picker's label layout: the segment row is flush with the
/// control's sides and bottom edge at its own [`segmented_row_height`], so its
/// height is the same with or without a heading. The heading spans the full
/// width at the top edge, edge to edge like the row below it, and the
/// layout's label spacing separates the two. Same presence rule: a label
/// measuring zero height (a hidden one) draws nothing and takes no space.
pub fn segmented_label_area(
    bounds: kurbo::Rect,
    label_height: f64,
    row_height: f64,
) -> (Option<kurbo::Rect>, kurbo::Rect) {
    let row = kurbo::Rect::new(bounds.x0, bounds.y1 - row_height, bounds.x1, bounds.y1);
    let heading = (label_height > 0.0)
        .then(|| kurbo::Rect::new(bounds.x0, bounds.y0, bounds.x1, bounds.y0 + label_height));
    (heading, row)
}

/// Flushes the picker's retained label sub-view at `rect` with accessibility
/// suppressed: the label is a retained node re-flushed each frame so reactive
/// content stays live, and its semantics are merged into the picker's own node
/// by `picker_accessibility`, so the sub-view emits no node of its own.
fn flush_picker_label<E: crate::text::TextEngine>(
    ctx: &mut WidgetRenderContext<'_, E>,
    label_view: &mut RetainedSubview<E>,
    env: &Environment,
    rect: kurbo::Rect,
) {
    let render_ctx = ctx.render_context();
    let label_area = ctx.safe_area_for(rect);
    ctx.renderer_mut()
        .with_suppressed_accessibility(|renderer| {
            label_view.place(
                renderer,
                render_ctx,
                env,
                ProposalSize::UNSPECIFIED,
                rect,
                label_area,
            );
        });
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "the parameter is a small Copy value taken by value for a uniform call-site signature"
)]
pub fn render_radio_picker<E: crate::text::TextEngine>(
    ctx: &mut WidgetRenderContext<'_, E>,
    owner: &Rc<RefCell<PickerRenderState<E>>>,
    selection: Binding<Id>,
    items: Vec<PickerItem<Id>>,
    env: &Environment,
) {
    let theme = ctx.theme();
    let look = owner.borrow_mut().radio_group_look(&theme, env);
    let layout = look.layout();
    let indicator = look.indicator().clone();
    let direction = layout_direction(env).snapshot();
    let bounds = ctx.bounds;
    // Reading the disabled signal watches it, so a change schedules a frame
    // and this persistent node re-renders (and re-registers input) with the
    // new state.
    let disabled = {
        let signal = widget_disabled(env);
        ctx.renderer_mut().read_signal(&signal)
    };
    // The group heading sits in the top inset band; the option rows begin
    // below it with the layout's label spacing, at the same y the
    // accessibility rows and the measured height agree on.
    let label_size = owner
        .borrow_mut()
        .label_view
        .measure_intrinsic(ctx.renderer_mut(), env);
    let (heading, mut row_y) = radio_group_label_area(
        bounds,
        layout,
        f64::from(label_size.height),
        bounds.y0 + layout.vertical_inset,
    );
    if let Some(heading) = heading {
        flush_picker_label(ctx, &mut owner.borrow_mut().label_view, env, heading);
    }
    // An indicator is kept per item, so an item that leaves the picker takes
    // its indicator with it and the others keep theirs.
    let tags: Vec<Id> = items.iter().map(|item| item.tag).collect();
    owner
        .borrow_mut()
        .radio_parts
        .retain(|tag, _| tags.contains(tag));
    for item in items {
        let label = label_styled(
            look.get_label(),
            ctx.renderer_mut()
                .read_resolved_text_styled(&item.content, env),
        );
        let label_size =
            HydrolysisRenderer::measure_text_intrinsic_size(ctx.state_mut(), label.clone(), env);
        let row_rect = radio_row_rect(&look, bounds, row_y, f64::from(label_size.height));
        if row_rect.height() <= 0.0 {
            break;
        }
        row_y = row_rect.y1 + layout.row_spacing;

        let tag = item.tag;
        let indicator_frame = radio_indicator_frame(&look, row_rect, direction);
        let hit_rect = indicator.target(indicator_frame).union(row_rect);
        let discriminator = crate::num_cast::i32_as_u32(i32::from(tag)) as usize;
        let key = InteractionKey::for_rc(owner, discriminator);
        let (_, press_slot, handles) = ctx.renderer_mut().bind_live_toggle_target(
            key.clone(),
            hit_rect,
            env,
            disabled,
            &indicator,
        );
        let flags = ctx.renderer_mut().reported_interaction_state(&key);
        // The indicator reads whether its item is the selected one through the
        // selection: choosing an item moves the operands of the two
        // indicators it touches and records nothing.
        let is_selected = Computed::new(selection.map(move |selected: Id| selected == tag));
        let parts = {
            let mut state = owner.borrow_mut();
            LiveToggleParts::ensure(
                state.radio_parts.entry(tag).or_default(),
                &indicator,
                indicator_frame,
                flags,
                &is_selected,
            )
            .clone()
        };
        handles.attach_live_toggle(Rc::clone(parts.toggle()));
        let render_ctx = ctx.render_context();
        parts.push(ctx.renderer_mut(), render_ctx);

        let label_rect = radio_label_rect(
            &look,
            row_rect,
            indicator_frame,
            f64::from(label_size.height),
            direction,
        );
        ctx.render_styled_text(label, HorizontalAlignment::Leading, env, label_rect);

        // A disabled radio group registers no tap target.
        if disabled {
            continue;
        }
        ctx.renderer_mut()
            .register_interactive_pointer_target(hit_rect, press_slot, {
                let selection = selection.clone();
                move |_renderer, _point, _env| {
                    if selection.snapshot() == tag {
                        return false;
                    }
                    selection.set(tag);
                    true
                }
            });
    }
}

/// The rect of a radio row whose label is `label_height` tall, laid by
/// `look` across `bounds` at `y`: as tall as the taller of its label and
/// its indicator, no lower than the bounds' bottom edge. The drawn rows and
/// the accessibility rows are this one rect.
pub fn radio_row_rect(look: &RadioGroupLook, bounds: Rect, y: f64, label_height: f64) -> Rect {
    let row_height = label_height.max(look.indicator().size().height);
    Rect::new(bounds.x0, y, bounds.x1, (y + row_height).min(bounds.y1))
}

/// Where a radio row's indicator lies in `row`: centred on the row's height,
/// inset from the row's leading edge. The look draws left to right; a
/// right-to-left layout reflects the indicator about the row.
fn radio_indicator_frame(look: &RadioGroupLook, row: Rect, direction: LayoutDirection) -> Rect {
    let size = look.indicator().size();
    let drawn = Rect::from_origin_size(
        Point::new(
            row.x0 + look.layout().horizontal_inset,
            row.y0 + (row.height() - size.height) / 2.0,
        ),
        size,
    );
    if direction.is_right_to_left() {
        mirrored_in(drawn, row)
    } else {
        drawn
    }
}

/// Where a radio row's label lies: beside the indicator, away from the row's
/// leading edge, to the row's inset trailing edge.
fn radio_label_rect(
    look: &RadioGroupLook,
    row: Rect,
    indicator_frame: Rect,
    label_height: f64,
    direction: LayoutDirection,
) -> Rect {
    let inset = look.layout().horizontal_inset;
    let spacing = look.indicator().get_label_spacing();
    let (x0, x1) = if direction.is_right_to_left() {
        (row.x0 + inset, indicator_frame.x0 - spacing)
    } else {
        (indicator_frame.x1 + spacing, row.x1 - inset)
    };
    // The label is text drawn leading-aligned and wrapped across the whole
    // room, so it takes all of it.
    label_beside_control_bounds(
        x0,
        x1,
        row,
        indicator_frame,
        kurbo::Size::new(x1 - x0, label_height),
    )
}

/// Binds a segmented picker's segments, lays its look out and lists it: the
/// parts below the labels, the labels, the parts over them, and each
/// segment's touch target.
#[expect(
    clippy::needless_pass_by_value,
    reason = "the parameter is a small Copy value taken by value for a uniform call-site signature"
)]
#[expect(
    clippy::too_many_lines,
    reason = "the function drives one continuous scenario through the renderer; splitting it would obscure the sequence"
)]
pub fn render_segmented_picker<E: crate::text::TextEngine>(
    ctx: &mut WidgetRenderContext<'_, E>,
    owner: &Rc<RefCell<PickerRenderState<E>>>,
    selection: Binding<Id>,
    items: Vec<PickerItem<Id>>,
    env: &Environment,
) {
    let theme = ctx.theme();
    // Reading the disabled signal watches it, so a change schedules a frame
    // and this persistent node re-renders (and re-registers input) with the
    // new state. The selection is read unwatched: a change moves the parts'
    // operands and the two labels it changes, and re-renders nothing.
    let disabled = {
        let signal = widget_disabled(env);
        ctx.renderer_mut().read_signal(&signal)
    };
    let mut state = owner.borrow_mut();
    let spec = segmented_spec(&state.config, items.len(), env);
    let direction = spec.direction();
    let SegmentedLookParts {
        look,
        selected: flags,
        chromes,
    } = state.segmented_look(&theme, spec, env);
    if state.segmented.watches.is_empty() {
        let frames = ctx.renderer_mut().frame_signals();
        let watches = segmented_watches(&state.config, &state.segmented, frames);
        state.segmented.watches = watches;
    }
    let count = items.len();
    if state.segmented.items_changed.replace(false) || state.segmented.labels.len() != count {
        state.segmented.labels = items
            .iter()
            .enumerate()
            .zip(flags.iter())
            .map(|((index, item), is_selected)| {
                let segment = Segment { index, count };
                let mut label = RetainedSubview::new(segment_label_view(
                    &look,
                    segment,
                    item,
                    is_selected,
                    env,
                ));
                label.ensure_built(ctx.renderer_mut(), env);
                label
            })
            .collect();
    }
    // Each label is laid out at the larger of its two fonts, so the row is
    // as layout measured it whichever segment is selected.
    let sizes: Vec<LayoutSize> = items
        .iter()
        .enumerate()
        .map(|(index, item)| {
            let styled = ctx
                .renderer_mut()
                .read_resolved_text_styled(&item.content, env);
            segmented_label_size(
                &look,
                &styled,
                Segment { index, count },
                ctx.state_mut(),
                env,
            )
        })
        .collect();
    let heading = state.label_view.measure_intrinsic(ctx.renderer_mut(), env);
    let placed = place_segmented(
        look.layout(),
        &sizes,
        f64::from(heading.height),
        ctx.bounds,
        direction,
    );
    if let Some(rect) = placed.heading {
        flush_picker_label(ctx, &mut state.label_view, env, rect);
    }

    let tags: Vec<Id> = items.iter().map(|item| item.tag).collect();
    // A selection naming no item is reported; the thumb then rests on the
    // leading segment until the selection names one.
    let selected = selected_item_index(&items, selection.snapshot()).unwrap_or(0);
    // Each segment binds its own interaction on its own touch target, so the
    // look's hit region applies per segment.
    let bound: Vec<_> = placed
        .drawn
        .iter()
        .zip(&tags)
        .enumerate()
        .map(|(index, (frame, tag))| {
            let key = option_key(owner, *tag);
            let target = look.target(*frame);
            let (_, press_slot, handles) = ctx.renderer_mut().bind_live_segment_target(
                key.clone(),
                target,
                env,
                disabled,
                &look,
                chromes.get(index),
            );
            let flags = ctx.renderer_mut().reported_interaction_state(&key);
            (target, press_slot, handles, flags)
        })
        .collect();
    let parts = LiveSegmentedParts::ensure(
        &mut state.segmented.parts,
        &look,
        &chromes,
        placed.frames.clone(),
        &tags,
        selected,
        disabled,
        &selection,
        env,
    )
    .clone();
    let picker = Rc::clone(parts.picker());
    for (index, (_, _, handles, flags)) in bound.iter().enumerate() {
        picker.sync(index, *flags);
        handles.attach_live_segment(Rc::clone(&picker), index);
    }

    // The picker as one group at the look's group opacity: the parts below
    // the labels, the labels in their drawn segments, then the parts over
    // them, so a backdrop material over the labels captures them.
    let padding = look.layout().label_padding;
    let labels = &mut state.segmented.labels;
    let reach = picker.visual_bounds();
    let body = |ctx: &mut WidgetRenderContext<'_, E>| {
        let render_ctx = ctx.render_context();
        parts.push_below(ctx.renderer_mut(), render_ctx);
        for (index, label) in labels.iter_mut().enumerate() {
            let rect = segmented_label_rect(placed.drawn[index], sizes[index], padding.x);
            let env = env.extending(LabelPaint(picker.label_paint(index)));
            flush_picker_label(ctx, label, &env, rect);
        }
        let render_ctx = ctx.render_context();
        parts.push_above(ctx.renderer_mut(), render_ctx);
    };
    let key = crate::renderer::mount::ScopeKey {
        role: "segmented-group",
        item: 0,
    };
    if let Some(opacity) = picker.live_group_opacity() {
        let operands = crate::renderer::ScopeOperands {
            opacity: Some(opacity),
            transform: None,
        };
        ctx.with_live_scope(key, operands, reach, body);
    } else {
        let opacity = picker.fixed_group_opacity();
        ctx.with_scope_if(opacity < 1.0, key, opacity, reach, body);
    }

    // A disabled segmented picker registers no touch target: the pointer
    // neither presses nor selects a segment. Targets are re-registered
    // every flush, so re-enabling restores interactivity on the next frame.
    if disabled {
        return;
    }
    for (index, ((target, press_slot, ..), tag)) in bound.into_iter().zip(tags).enumerate() {
        let track = segment_track(
            Rc::clone(&picker),
            index,
            tag,
            selection.clone(),
            press_slot.slop,
        );
        let keyboard = selection.clone();
        ctx.renderer_mut().register_tracked_pointer_target(
            target,
            press_slot,
            track,
            move |_renderer, _point, _env| select(&keyboard, tag),
        );
    }
}

/// Selects `tag` through `selection`; whether that changed the selection.
fn select(selection: &Binding<Id>, tag: Id) -> bool {
    if selection.snapshot() == tag {
        return false;
    }
    selection.set(tag);
    true
}

/// The watchers a segmented picker keeps: the items', which has the labels
/// built again, and the selection's, which has the accessibility tree
/// publish the change. Each skips the emission its registration makes.
fn segmented_watches<E: crate::text::TextEngine>(
    config: &PickerConfig,
    state: &SegmentedState<E>,
    frames: crate::renderer::FrameSignals,
) -> Vec<Box<dyn Any>> {
    let armed = Rc::new(Cell::new(false));
    let items_changed = Rc::clone(&state.items_changed);
    let items_armed = Rc::clone(&armed);
    let items = config.items().watch(move |_| {
        if items_armed.get() {
            items_changed.set(true);
        }
    });
    let selection_changed = Rc::clone(&state.selection_changed);
    let selection_armed = Rc::clone(&armed);
    let selection = config.selection().watch(move |_| {
        if !selection_armed.get() {
            return;
        }
        selection_changed.set(true);
        // Wakes the pump for the accessibility publish. A redraw is the
        // cheapest request: it marks nothing and re-encodes nothing.
        frames.request_redraw();
    });
    armed.set(true);
    vec![Box::new(items), Box::new(selection)]
}

/// Each of `count` segments' live selected flag, leading first: `true`
/// while `config`'s selection names the item at the segment's index. Each
/// notifies only when its value changes, so a selection change reaches the
/// two segments it changes and no other.
fn segment_selected_flags(config: &PickerConfig, count: usize) -> Rc<[Computed<bool>]> {
    let selection = config.selection();
    let items = config.items();
    (0..count)
        .map(|index| {
            selection
                .zip(items)
                .map(move |(current, items): (Id, Vec<PickerItem<Id>>)| {
                    items.get(index).is_some_and(|item| item.tag == current)
                })
                .distinct()
                .computed()
        })
        .collect()
}

/// The label view of `segment`, showing `item`: the item's text in the
/// look's font for the segment's selected state, `is_selected`. Its content
/// changes only when that state does, so a selection change lays out again
/// the two labels it changes and no other.
fn segment_label_view(
    look: &SegmentedLook,
    segment: Segment,
    item: &PickerItem<Id>,
    is_selected: &Computed<bool>,
    env: &Environment,
) -> AnyView {
    let unselected = segment_font(look, segment, false);
    let selected = segment_font(look, segment, true);
    let content = item.content.resolve(env).content();
    let styled: Computed<StyledStr> = Computed::new(content.zip(is_selected).map(
        move |(styled, is_selected): (StyledStr, bool)| {
            styled.font(if is_selected {
                selected.clone()
            } else {
                unselected.clone()
            })
        },
    ));
    AnyView::new(styled.into_text())
}

/// The handler of a segment's press: a press on the selected segment that
/// moves farther than `slop` horizontally drags the thumb, which follows the
/// touch and settles on the nearest segment on release; any other press
/// released within reach selects the segment.
fn segment_track(
    picker: Rc<LiveSegmented>,
    index: usize,
    tag: Id,
    selection: Binding<Id>,
    slop: f64,
) -> TrackAction {
    let start = Cell::new(None::<Point>);
    let dragging = Cell::new(false);
    Rc::new(RefCell::new(move |step: Track| match step {
        Track::Begin(point) => {
            start.set(Some(point));
            dragging.set(false);
        }
        Track::Move(point) => {
            if !picker.drags() || picker.selected() != index {
                return;
            }
            let Some(from) = start.get() else {
                return;
            };
            let offset = Vec2::new(point.x - from.x, 0.0);
            if !dragging.get() && offset.x.abs() > slop {
                dragging.set(true);
                picker.begin_drag();
            }
            if dragging.get() {
                picker.drag_to(offset);
            }
        }
        Track::Release { within_reach, .. } => {
            start.set(None);
            if dragging.replace(false) {
                picker.end_drag(&selection);
            } else if within_reach {
                select(&selection, tag);
            }
        }
        Track::Cancel => {
            start.set(None);
            if dragging.replace(false) {
                picker.cancel_drag();
            }
        }
    }))
}

/// Where a label of `label_size` lies in its drawn segment: centred, no
/// wider than the segment less the look's horizontal label padding.
fn segmented_label_rect(
    segment_rect: kurbo::Rect,
    label_size: waterui_core::layout::Size,
    padding: f64,
) -> kurbo::Rect {
    let max_width = padding.mul_add(-2.0, segment_rect.width()).max(0.0);
    let width = f64::from(label_size.width).min(max_width);
    let height = f64::from(label_size.height).min(segment_rect.height());
    let x0 = (segment_rect.width() - width).mul_add(0.5, segment_rect.x0);
    let y0 = (segment_rect.height() - height).mul_add(0.5, segment_rect.y0);
    kurbo::Rect::new(x0, y0, x0 + width, y0 + height)
}

/// Emits a retained picker's accessibility tree for the semantic walk — the
/// same nodes `picker_accessibility` registers, with no bounds.
#[cfg(feature = "accessibility")]
pub fn emit_picker_accessibility<E: crate::text::TextEngine>(
    renderer: &mut crate::renderer::SemanticCore<E>,
    state: &Rc<RefCell<PickerRenderState<E>>>,
    env: &Environment,
) {
    picker_accessibility(renderer, None, None, state, env);
}

#[cfg(test)]
mod tests {
    use super::{
        has_items, menu_picker_content_rects, radio_group_label_area, segmented_label_area,
        selected_item_index, selected_option_text,
    };
    use crate::theme::{MenuPickerLayout, RadioLayout};
    use kurbo::Rect;
    use waterui_core::id::Id;
    use waterui_core::layout::Size;
    use waterui_form::picker::PickerItem;

    fn metrics() -> MenuPickerLayout {
        MenuPickerLayout {
            min_width: 280.0,
            min_height: 56.0,
            horizontal_inset: 16.0,
            vertical_inset: 8.0,
            label_spacing: 8.0,
            indicator_space: 18.0,
            popup_gap: 4.0,
        }
    }

    fn radio_metrics() -> RadioLayout {
        RadioLayout {
            min_width: 280.0,
            min_height: 56.0,
            horizontal_inset: 16.0,
            vertical_inset: 8.0,
            label_spacing: 8.0,
            row_spacing: 8.0,
        }
    }

    /// The M3 exposed-dropdown label sits inside the field, directly above the
    /// selected value with the metrics' label spacing between them.
    #[test]
    fn labelled_content_places_label_above_value_inside_the_field() {
        let bounds = Rect::new(0.0, 0.0, 320.0, 56.0);
        let (label, value) =
            menu_picker_content_rects(bounds, metrics(), Size::new(40.0, 12.0), false);
        let label = label.expect("a drawn label must be placed");

        assert_eq!(label, Rect::new(16.0, 8.0, 56.0, 20.0));
        assert_eq!(value, Rect::new(16.0, 28.0, 286.0, 48.0));
        assert!(bounds.contains_rect(label) && bounds.contains_rect(value));
        approx::assert_relative_eq!(value.y0, label.y1 + metrics().label_spacing);
    }

    /// The label starts at the content's leading edge at its measured width,
    /// aligned with the value, and a label wider than the content is clamped to
    /// the content's far edge. In right-to-left layout the leading edge is the
    /// right one.
    #[test]
    fn label_aligns_with_the_value_at_the_leading_edge() {
        let bounds = Rect::new(0.0, 0.0, 320.0, 56.0);
        let rects = |width, right_to_left| {
            menu_picker_content_rects(bounds, metrics(), Size::new(width, 12.0), right_to_left)
        };
        let (label, value) = rects(40.0, false);
        assert_eq!(label.expect("drawn").x0, value.x0);
        assert_eq!(rects(900.0, false).0.expect("drawn").x1, value.x1);
        let (label, mirrored) = rects(40.0, true);
        let label = label.expect("drawn");
        assert_eq!((label.x0, label.x1), (264.0, mirrored.x1));
        assert_eq!(rects(900.0, true).0.expect("drawn").x0, mirrored.x0);
    }

    /// A right-to-left field is the left-to-right one reflected about it: the
    /// dropdown indicator's space lies at the left end and the content is flush
    /// with the right.
    #[test]
    fn a_right_to_left_field_reserves_the_indicator_space_on_the_left() {
        let bounds = Rect::new(0.0, 0.0, 320.0, 56.0);
        let (_, value) = menu_picker_content_rects(bounds, metrics(), Size::new(40.0, 0.0), false);
        let (_, mirrored) =
            menu_picker_content_rects(bounds, metrics(), Size::new(40.0, 0.0), true);

        assert_eq!(value, Rect::new(16.0, 8.0, 286.0, 48.0));
        assert_eq!(mirrored, Rect::new(34.0, 8.0, 304.0, 48.0));
    }

    /// A hidden label measures empty: it draws nothing, takes no space, and the
    /// value keeps the field's full inset height.
    #[test]
    fn hidden_label_leaves_the_value_alone_in_the_field() {
        let bounds = Rect::new(0.0, 0.0, 320.0, 56.0);
        let (label, value) =
            menu_picker_content_rects(bounds, metrics(), Size::new(40.0, 0.0), false);

        assert!(label.is_none());
        assert_eq!(value, Rect::new(16.0, 8.0, 286.0, 48.0));
    }

    /// The radio group's heading sits in the top inset band inside the
    /// horizontal insets, and the first option row begins below it with the
    /// metrics' label spacing.
    #[test]
    fn radio_label_heading_sits_above_the_rows_with_label_spacing() {
        let bounds = Rect::new(0.0, 0.0, 320.0, 96.0);
        let default_row_y = bounds.y0 + metrics().vertical_inset;
        let (heading, row_y) = radio_group_label_area(bounds, radio_metrics(), 12.0, default_row_y);
        let heading = heading.expect("a drawn group label must be placed");

        assert_eq!(heading, Rect::new(16.0, 8.0, 304.0, 20.0));
        approx::assert_relative_eq!(row_y, heading.y1 + radio_metrics().label_spacing);
        assert!(bounds.contains_rect(heading));
    }

    /// The segmented group's heading spans the full control width at the top
    /// edge — edge to edge like the row it heads — and the segment row keeps
    /// its own height on the bottom edge, the label spacing below the heading.
    #[test]
    fn segmented_label_heading_spans_edge_to_edge_above_the_row() {
        let bounds = Rect::new(0.0, 0.0, 320.0, 64.0);
        let (heading, row) = segmented_label_area(bounds, 12.0, 44.0);
        let heading = heading.expect("a drawn group label must be placed");

        assert_eq!(heading, Rect::new(0.0, 0.0, 320.0, 12.0));
        assert_eq!(row, Rect::new(0.0, 20.0, 320.0, 64.0));
        approx::assert_relative_eq!(row.y0, heading.y1 + metrics().label_spacing);
    }

    /// A hidden group label draws nothing and takes no space: no heading rect
    /// and the content keeps its unlabelled span for either group style.
    #[test]
    fn hidden_group_label_leaves_the_content_start_untouched() {
        let bounds = Rect::new(0.0, 0.0, 320.0, 96.0);

        assert_eq!(
            radio_group_label_area(bounds, radio_metrics(), 0.0, 14.0),
            (None, 14.0)
        );
        assert_eq!(segmented_label_area(bounds, 0.0, 96.0), (None, bounds));
    }

    fn item(tag: i32, text: &'static str) -> PickerItem<Id> {
        PickerItem::new(Id::try_from(tag).unwrap(), waterui_text::Text::new(text))
    }

    #[test]
    #[cfg_attr(
        debug_assertions,
        should_panic(expected = "picker requires at least one item")
    )]
    fn a_picker_without_items_is_reported_and_draws_nothing() {
        assert!(!has_items(&[]));
    }

    #[test]
    fn a_picker_with_items_is_not_reported() {
        assert!(has_items(&[item(1, "a")]));
    }

    #[test]
    #[cfg_attr(
        debug_assertions,
        should_panic(expected = "selection is not present in picker items")
    )]
    fn a_selection_naming_no_item_is_reported_and_shows_no_text() {
        let items = [item(1, "a"), item(2, "b")];
        let index = selected_item_index(&items, Id::try_from(3).unwrap());
        assert_eq!(index, None);
        assert_eq!(
            selected_option_text(&["a".to_owned(), "b".to_owned()], index),
            ""
        );
    }

    #[test]
    fn a_selection_naming_an_item_shows_its_text() {
        let items = [item(1, "a"), item(2, "b")];
        let index = selected_item_index(&items, Id::try_from(2).unwrap());
        assert_eq!(index, Some(1));
        assert_eq!(
            selected_option_text(&["a".to_owned(), "b".to_owned()], index),
            "b"
        );
    }
}
