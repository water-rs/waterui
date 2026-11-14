#[cfg(feature = "accessibility")]
use crate::renderer::AccessibilityActionTarget;
use crate::renderer::chrome::{ChromeContent, LiveChromeParts};
use crate::renderer::{
    GroupOpacity, HydroNativeView, HydroState, HydrolysisRenderer, RenderContext, RetainedSubview,
    WidgetRenderContext, measure_label_intrinsic, measure_view_intrinsic, popup_menu_nodes,
};
use crate::theme::{
    ButtonLook, ButtonSpec, Chrome, CornerShape, Corners, Inputs, Label as LookLabel, SpecTint,
    StateLayer, Surface, WidgetTheme,
};
#[cfg(feature = "accessibility")]
use accesskit::{
    Action as AccessibilityAction, Node as AccessibilityNode, Role as AccessibilityNodeRole,
};
use kurbo::Rect;
use nami::{Signal, SignalExt};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use waterui::ViewExt as _;
use waterui::floating::FloatingScope;
use waterui::style::FloatingStyle;
use waterui_controls::ControlSize;
use waterui_controls::button::{ButtonConfig, ButtonStyle};
use waterui_controls::label::{Label, LabelDisplayMode};
use waterui_controls::menu::ResolvedMenu;
use waterui_core::handler::ReportExtraction as _;
use waterui_core::interaction::InteractionState;
use waterui_core::layout::Size as LayoutSize;
use waterui_core::layout::{ProposalSize, ViewDimensions};
use waterui_core::{AnyView, Environment, Native};
use waterui_graphics::color::Color;
use waterui_text::font::Font;
use waterui_text::styled::StyledStr;

use crate::widgets::util::{centered_label_rect, inset_rect, widget_disabled};

/// The interaction states that change without a render: their changes move
/// live operands.
pub const LIVE_STATES: InteractionState = InteractionState::HOVERED
    .union(InteractionState::PRESSED)
    .union(InteractionState::FOCUSED)
    .union(InteractionState::DRAGGED);

/// What a button-like control keeps across renders to realize its look:
/// the look for the spec it was requested for, the live chrome, the
/// reported-state binding the label's colour follows, and the chrome frame
/// the label's [`Inputs`] see.
struct ButtonChromeState {
    look: Option<(ButtonSpec, ButtonLook)>,
    parts: Option<LiveChromeParts>,
    /// The last [`InteractionState`] the control reported, feeding the
    /// retained label's reactive colour.
    label_state: nami::Binding<InteractionState>,
    frame: Rc<Cell<Rect>>,
}

impl ButtonChromeState {
    fn new() -> Self {
        Self {
            look: None,
            parts: None,
            label_state: nami::Binding::container(InteractionState::empty()),
            frame: Rc::new(Cell::new(Rect::ZERO)),
        }
    }

    /// The look for `spec` in `env`: the one already held when the spec and
    /// the environment's presentation are unchanged, otherwise requested
    /// again.
    fn look(
        &mut self,
        theme: &Rc<dyn WidgetTheme>,
        spec: &ButtonSpec,
        env: &Environment,
    ) -> &ButtonLook {
        if self.held(spec, env).is_none() {
            self.look = Some((spec.clone(), resolve_look(theme, spec, env)));
        }
        &self.look.as_ref().expect("the look was just ensured").1
    }

    /// The held look when it is the one `spec` in `env` takes.
    fn held(&self, spec: &ButtonSpec, env: &Environment) -> Option<&ButtonLook> {
        let plugin = env.get::<Chrome>();
        self.look
            .as_ref()
            .filter(|(held, look)| {
                held == spec && plugin.is_none_or(|plugin| plugin.same(look.chrome()))
            })
            .map(|(_, look)| look)
    }

    /// The look for `spec` in `env` without holding it: the held one, or a
    /// fresh one for a measurement ahead of the render that holds it.
    fn peek(
        &self,
        theme: &Rc<dyn WidgetTheme>,
        spec: &ButtonSpec,
        env: &Environment,
    ) -> ButtonLook {
        self.held(spec, env)
            .cloned()
            .unwrap_or_else(|| resolve_look(theme, spec, env))
    }
}

/// The look a button of `spec` takes in `env`: a composed control's
/// [`Chrome`] plugin with no padding, minimum or label styling; otherwise
/// the theme's look, lowered onto the enclosing floating surface's style
/// when there is one.
fn resolve_look(theme: &Rc<dyn WidgetTheme>, spec: &ButtonSpec, env: &Environment) -> ButtonLook {
    if let Some(chrome) = env.get::<Chrome>() {
        // The composed control's content draws its own container; a text
        // label keeps the theme's button font.
        let mut label = LookLabel::new();
        if let Some(font) = theme.button(spec, env).label().get_font() {
            label = label.font(font.clone());
        }
        return ButtonLook::new(chrome.clone(), label);
    }
    let look = theme.button(spec, env);
    if env.get::<MenuRowChrome>().is_some() {
        return lower_menu_row(
            &look,
            crate::renderer::menu_look(theme, env)
                .layout()
                .horizontal_padding,
        );
    }
    match env.get::<FloatingScope>() {
        Some(FloatingScope(style)) => lower_floating(&look, style),
        None => look,
    }
}

/// Marks the rows of a popup menu: each is a button the renderer draws as a
/// menu item rather than a button.
#[derive(Debug, Clone, Copy)]
pub struct MenuRowChrome;

/// A popup menu row: the theme look's state-layer tokens, press feedback,
/// motion, hit region and font over a square, containerless surface whose
/// state layer is the foreground colour, inset by the theme's menu-item
/// inset. A row shows keyboard focus through its state layer alone.
fn lower_menu_row(look: &ButtonLook, horizontal_inset: f64) -> ButtonLook {
    let theme_chrome = look.chrome();
    let layer = theme_chrome.state_layer();
    let chrome = Chrome::new(
        Surface::clear(CornerShape::radius(0.0)),
        StateLayer::new(Color::new(waterui::theme::color::Foreground)).opacities(
            layer.hover,
            layer.focus,
            layer.pressed,
            layer.dragged,
        ),
        theme_chrome.press().clone(),
        crate::theme::FocusIndicator::Tint,
        theme_chrome.motion().clone(),
    )
    .hit(*theme_chrome.get_hit());
    let mut label = LookLabel::new();
    if let Some(font) = look.label().get_font() {
        label = label.font(font.clone());
    }
    ButtonLook::new(chrome, label).padding(horizontal_inset, 0.0)
}

/// The font a popup menu row's label takes in `env`: the borderless button
/// look's.
pub fn menu_row_font(theme: &Rc<dyn WidgetTheme>, env: &Environment) -> Option<Font> {
    let spec = button_spec(ButtonStyle::Borderless, ControlSize::Small, false, env);
    look_font(
        &theme.button(&spec, env),
        InteractionState::empty(),
        Rect::ZERO,
    )
}

/// A button on a floating surface: the surface draws its own container and
/// shadows, so the button keeps the theme look's state-layer tokens, press
/// feedback, focus indicator, motion and hit region, and takes the floating
/// style's silhouette, state-layer colour, content colour and metrics.
fn lower_floating(look: &ButtonLook, style: &FloatingStyle) -> ButtonLook {
    let theme_chrome = look.chrome();
    let layer = theme_chrome.state_layer();
    let chrome = Chrome::new(
        Surface::clear(CornerShape::new(Corners::Fraction(f64::from(
            style.clip_radius,
        )))),
        StateLayer::new(style.state_layer_color.clone()).opacities(
            layer.hover,
            layer.focus,
            layer.pressed,
            layer.dragged,
        ),
        theme_chrome.press().clone(),
        theme_chrome.focus().clone(),
        theme_chrome.motion().clone(),
    )
    .hit(*theme_chrome.get_hit());
    let content = style.content_color.clone();
    let disabled = style
        .content_color
        .clone()
        .with_opacity(style.disabled_content_opacity);
    let mut label = LookLabel::new().color(crate::theme::Driven::new(move |inputs| {
        if inputs.state.contains(InteractionState::DISABLED) {
            disabled.clone()
        } else {
            content.clone()
        }
    }));
    if let Some(font) = look.label().get_font() {
        label = label.font(font.clone());
    }
    let group_opacity = look.get_group_opacity().clone();
    let built = ButtonLook::new(chrome, label)
        .padding(style.content_inset_x, style.content_inset_y)
        .minimum_size(kurbo::Size::new(style.minimum_width, style.minimum_height))
        .group_opacity(group_opacity);
    match look.get_group_blend_space() {
        Some(space) => built.group_blend_space(space),
        None => built,
    }
}

/// The spec of a `style` button of `size` in `env`.
fn button_spec(
    style: ButtonStyle,
    size: ControlSize,
    icon_only: bool,
    env: &Environment,
) -> ButtonSpec {
    ButtonSpec::new(
        style,
        size,
        icon_only,
        waterui_core::layout::layout_direction(env).snapshot(),
        SpecTint::resolve(env),
    )
}

/// The label font of `look`. A font is laid out, so it follows the spec
/// and the disabled and selected states, never the live states.
fn look_font(look: &ButtonLook, state: InteractionState, frame: Rect) -> Option<Font> {
    look.label()
        .get_font()
        .map(|font| font.target(&Inputs::at_rest(frame, state.difference(LIVE_STATES), &())))
}

/// The label colour of `look` in `state`.
fn look_color(look: &ButtonLook, state: InteractionState, frame: Rect) -> Option<Color> {
    look.label()
        .get_color()
        .map(|color| color.target(&Inputs::at_rest(frame, state, &())))
}

/// The retained label's colour: the look's, following the reported state
/// reactively so the retained sub-view recolours without a rebuild.
fn retained_label_color(
    look: &ButtonLook,
    chrome: &ButtonChromeState,
    env: &Environment,
) -> Option<Color> {
    if env.get::<ListRowChrome>().is_some() {
        return Some(Color::new(waterui::theme::color::Foreground));
    }
    let color = look.label().get_color()?.clone();
    let frame = Rc::clone(&chrome.frame);
    Some(waterui_graphics::color::signal_color(
        chrome
            .label_state
            .clone()
            .map(move |state| color.target(&Inputs::at_rest(frame.get(), state, &()))),
    ))
}

/// The retained render state of a button. A `TitleOnly` label is rendered as
/// centered styled text fresh each frame (so its reactive title stays live); any
/// other label is a move-only composite, so it is held as a [`RetainedSubview`]
/// built once (with the look's label styling applied) and re-flushed each
/// frame. The `config` is kept for the value/style/action and for
/// accessibility resolution.
pub struct ButtonRenderState<E: crate::text::TextEngine> {
    config: ButtonConfig,
    /// `Some` for a non-title (general) label held as a retained sub-view; `None`
    /// for a `TitleOnly` label rendered as styled text from `config.label`.
    label_view: Option<RetainedSubview<E>>,
    chrome: ButtonChromeState,
}

impl<E: crate::text::TextEngine> ButtonRenderState<E> {
    pub(crate) fn from_config(config: ButtonConfig) -> Self {
        Self {
            config,
            label_view: None,
            chrome: ButtonChromeState::new(),
        }
    }

    /// Consume the label sub-view's layout-dirty marks: its measured size
    /// feeds the button's, so an enclosing retained sub-view must see them
    /// (water-rs/waterui#1957).
    pub(crate) fn take_layout_dirty(&mut self) -> bool {
        self.label_view
            .as_mut()
            .is_some_and(RetainedSubview::take_layout_dirty_if_built)
    }

    /// Create the general (non-title) label sub-view at tree-build time. A plain
    /// `TitleOnly` label stays `None` and is rendered as styled text each frame.
    /// The sub-view is created unbuilt and unpainted: the look supplies the
    /// label font and foreground, so styling is applied in
    /// [`Self::ensure_label_built`] during the layout-time prepare pass, the
    /// first point a theme exists.
    ///
    /// A label carrying custom content also resolves to `TitleOnly`, because it
    /// has no icon — but its content is a view, and rendering its semantic text
    /// instead would throw that view away. Content kind decides here, not
    /// display mode.
    pub(crate) fn init_label(&mut self) {
        if renders_as_plain_title(self.config.label()) {
            return;
        }
        self.label_view = Some(RetainedSubview::new(AnyView::new(
            self.config.label().clone(),
        )));
    }

    fn spec(&self, env: &Environment) -> ButtonSpec {
        button_spec(
            self.config.style(),
            self.config.size(),
            label_resolves_icon_only(self.config.label(), env),
            env,
        )
    }

    /// Apply the look's label styling and build the label sub-view. Called from
    /// the layout-time prepare pass (`WidgetBehavior::prepare`) — the measure
    /// path has no renderer, so the sub-view must be built before then. A
    /// semantic runtime never prepares, so the label view is never styled or
    /// built there: accessibility reads the label config directly.
    pub(crate) fn ensure_label_built(
        &mut self,
        renderer: &mut HydrolysisRenderer<E>,
        env: &Environment,
    ) {
        if self
            .label_view
            .as_ref()
            .is_none_or(RetainedSubview::is_built)
        {
            return;
        }
        let theme = renderer.theme();
        let spec = self.spec(env);
        let look = self.chrome.look(&theme, &spec, env).clone();
        let color = retained_label_color(&look, &self.chrome, env);
        let font = look_font(
            &look,
            self.chrome.label_state.snapshot(),
            self.chrome.frame.get(),
        );
        let styled = styled_button_label(font, self.config.label().clone());
        let subview = self
            .label_view
            .as_mut()
            .expect("the label sub-view was checked above");
        subview.map_source(|_| button_label_view(color, AnyView::new(styled)));
        subview.ensure_built(renderer, env);
    }
}

impl<E: crate::text::TextEngine> HydroNativeView<E> for Native<ButtonConfig> {
    fn intrinsic(
        state: &mut crate::renderer::HydroState<E>,
        view: &Self,
        env: &Environment,
        theme: &Rc<dyn crate::theme::WidgetTheme>,
    ) -> LayoutSize {
        measure_button_intrinsic(view.as_inner(), state, env, theme)
    }
}

/// Emits a button's accessibility node from its retained state. Shared by the
/// rendered `Widget`-node flush and the semantic emission walk so both produce
/// the same a11y tree.
// empty when the accessibility feature is off
#[cfg_attr(not(feature = "accessibility"), allow(clippy::missing_const_for_fn))]
pub fn button_accessibility<E: crate::text::TextEngine>(
    renderer: &mut crate::renderer::SemanticCore<E>,
    ctx: Option<RenderContext>,
    state: &Rc<RefCell<ButtonRenderState<E>>>,
    env: &Environment,
) {
    #[cfg(feature = "accessibility")]
    {
        let button = &state.borrow().config;
        let mut node = AccessibilityNode::new(
            crate::renderer::SemanticCore::<E>::resolve_accessibility_role(
                env,
                match button.style() {
                    ButtonStyle::Link => AccessibilityNodeRole::Link,
                    _ => AccessibilityNodeRole::Button,
                },
            ),
        );
        let default_label =
            crate::renderer::SemanticCore::<E>::accessibility_label_from_label(button.label(), env);
        let label = renderer.resolve_accessibility_label(env, default_label);
        if let Some(label) = label {
            node.set_label(label);
        }
        if let Some(value) = renderer.resolve_accessibility_value(env, None) {
            node.set_value(value);
        }
        node.add_action(AccessibilityAction::Focus);
        // A button under a `Selected` scope it owns announces the state.
        if renderer.control_selected(env, &crate::renderer::InteractionKey::for_rc(state, 0)) {
            node.set_selected(true);
        }
        // A disabled button stays in the tree (focusable, announced as
        // disabled) but exposes no click action and no action target.
        let disabled = renderer.read_signal(&widget_disabled(env));
        let action_target = if disabled {
            node.set_disabled();
            None
        } else {
            node.add_action(AccessibilityAction::Click);
            Some(AccessibilityActionTarget::Activate {
                action: button_activation(state, env),
            })
        };
        let node_id = match ctx {
            Some(ctx) => {
                let bounds = ctx.bounds();
                renderer.register_accessibility_node(node, bounds, env, action_target)
            }
            None => renderer.register_accessibility_node_semantic(node, env, action_target),
        };
        if let Some(node_id) = node_id {
            renderer.register_accessibility_focus_link(
                &crate::renderer::InteractionKey::for_rc(state, 0),
                node_id,
            );
        }
    }
    #[cfg(not(feature = "accessibility"))]
    {
        let _ = (renderer, ctx, state, env);
    }
}

/// The activation closure a button's pointer target and its accessibility
/// `Click` target share: invokes `config.action` through the retained state
/// cell so both hit the live config.
#[cfg(feature = "accessibility")]
pub fn button_activation<E: crate::text::TextEngine>(
    state: &Rc<RefCell<ButtonRenderState<E>>>,
    env: &Environment,
) -> crate::renderer::AccessibilityActivation<E> {
    let state = Rc::clone(state);
    let action_env = env.clone();
    Rc::new(RefCell::new(
        move |_renderer: &mut crate::renderer::SemanticCore<E>, _env: &Environment| {
            (state.borrow_mut().config.action_mut())(&action_env).report_extraction();
            true
        },
    ))
}

/// Resolves a button's size from the label size and the look's metrics. The
/// minimum is a floor for unconstrained layout; a tighter explicit proposal
/// caps it, so a fixed-size parent (e.g. a 40pt calendar day cell) receives
/// the size it proposed instead of chrome and hit bounds overflowing the
/// assigned slot. Padded label content keeps its intrinsic footprint — the
/// parent decides how any remaining overflow is aligned.
fn button_size(label_size: LayoutSize, look: &ButtonLook, proposal: ProposalSize) -> LayoutSize {
    let padding = look.get_padding();
    let minimum = look.get_minimum_size();
    let content_width = padding.x.mul_add(2.0, f64::from(label_size.width));
    let content_height = padding.y.mul_add(2.0, f64::from(label_size.height));
    let min_width = proposal
        .width
        .map_or(minimum.width, |width| minimum.width.min(f64::from(width)));
    let min_height = proposal.height.map_or(minimum.height, |height| {
        minimum.height.min(f64::from(height))
    });
    LayoutSize::new(
        crate::num_cast::f64_as_f32(content_width.max(min_width)),
        crate::num_cast::f64_as_f32(content_height.max(min_height)),
    )
}

/// Measures a retained button leaf from its [`ButtonRenderState`]: a general label
/// is measured from its built [`RetainedSubview`], a title from its styled text —
/// mirroring the render path so layout and render agree.
pub fn measure_button_node<E: crate::text::TextEngine>(
    render_state: &ButtonRenderState<E>,
    proposal: ProposalSize,
    state: &mut HydroState<E>,
    env: &Environment,
    theme: &Rc<dyn crate::theme::WidgetTheme>,
) -> ViewDimensions {
    let spec = render_state.spec(env);
    let look = render_state.chrome.peek(theme, &spec, env);
    let label_size = if let Some(subview) = &render_state.label_view {
        subview.measure_built(state, env, theme)
    } else {
        let font = look_font(
            &look,
            render_state.chrome.label_state.snapshot(),
            render_state.chrome.frame.get(),
        );
        let styled = styled_button_title(state, font, render_state.config.label(), env);
        HydrolysisRenderer::measure_text_intrinsic_size(state, styled, env)
    };
    ViewDimensions::new(button_size(label_size, &look, proposal))
}

/// Renders a retained button leaf every flush: emits a11y (unless hidden) then the
/// chrome + label + tap target, reading the config's live signals each frame.
pub fn render_button_node<E: crate::text::TextEngine>(
    ctx: &mut WidgetRenderContext<'_, E>,
    state: &Rc<RefCell<ButtonRenderState<E>>>,
    env: &Environment,
) {
    let hidden = env
        .get::<waterui::accessibility::AccessibilityHidden>()
        .is_some_and(waterui::accessibility::AccessibilityHidden::is_hidden);
    if !hidden {
        let render_ctx = ctx.render_context();
        button_accessibility(ctx.renderer_mut(), Some(render_ctx), state, env);
    }
    render_button_parts(ctx, state, env);
}

/// The retained render state of a menu trigger. `ResolvedMenu::label` is a
/// move-only `AnyView`, so the persistent `Widget` node holds it as a
/// [`RetainedSubview`] built once and re-flushed each frame (unless it is a
/// `Label`/`TitleOnly`, which is rendered as styled text fresh each frame like a
/// button); the `items`/`accessibility_label` signals are kept and read through
/// `read_signal`.
pub struct MenuRenderState<E: crate::text::TextEngine> {
    /// The build-time decision of how to render the label.
    label: MenuLabel<E>,
    /// Whether the label resolves to an icon-only presentation — decided
    /// once at build from the label's own display-mode resolution, so the
    /// trigger takes the icon-button look like a button.
    icon_only: bool,
    /// The trigger button's style and size, from the menu.
    style: ButtonStyle,
    size: ControlSize,
    items: nami::Computed<Vec<waterui_controls::menu::ResolvedMenuItem>>,
    #[cfg(feature = "accessibility")]
    accessibility_label: nami::Computed<StyledStr>,
    chrome: ButtonChromeState,
}

/// How a menu trigger renders its label, decided once at build time from the
/// label `AnyView`.
enum MenuLabel<E: crate::text::TextEngine> {
    /// A `Label` with `TitleOnly` display: rendered as centered styled text fresh
    /// each frame (cloneable, like a button title).
    Title(Label),
    /// Any other view: re-flushed from a retained sub-view each frame.
    View(Box<RetainedSubview<E>>),
}

impl<E: crate::text::TextEngine> MenuRenderState<E> {
    pub(crate) fn from_resolved(menu: ResolvedMenu, env: &Environment) -> Self {
        let ResolvedMenu {
            label,
            items,
            #[cfg(feature = "accessibility")]
            accessibility_label,
            #[cfg(not(feature = "accessibility"))]
                accessibility_label: _,
            style,
            size,
        } = menu;
        let icon_only = label
            .downcast_ref::<Label>()
            .is_some_and(|label| label_resolves_icon_only(label, env));
        let label = match label.downcast::<Label>() {
            Ok(label) if renders_as_plain_title(&label) => MenuLabel::Title(*label),
            Ok(label) => MenuLabel::View(Box::new(RetainedSubview::new(AnyView::new(*label)))),
            Err(view) => MenuLabel::View(Box::new(RetainedSubview::new(view))),
        };
        Self {
            label,
            icon_only,
            style,
            size,
            items,
            #[cfg(feature = "accessibility")]
            accessibility_label,
            chrome: ButtonChromeState::new(),
        }
    }

    fn spec(&self, env: &Environment) -> ButtonSpec {
        button_spec(self.style, self.size, self.icon_only, env)
    }

    /// Apply the look's label styling and build the label sub-view. Called
    /// from the layout-time prepare pass (`WidgetBehavior::prepare`) — the
    /// measure path has no renderer, so the sub-view must be built before
    /// then; a semantic runtime never prepares and never needs the painted
    /// label.
    pub(crate) fn ensure_label_built(
        &mut self,
        renderer: &mut HydrolysisRenderer<E>,
        env: &Environment,
    ) {
        let theme = renderer.theme();
        let spec = self.spec(env);
        let MenuLabel::View(subview) = &mut self.label else {
            return;
        };
        if subview.is_built() {
            return;
        }
        let look = self.chrome.look(&theme, &spec, env).clone();
        let color = retained_label_color(&look, &self.chrome, env);
        subview.map_source(|view| button_label_view(color, view));
        subview.ensure_built(renderer, env);
    }

    /// Consume the label sub-view's layout-dirty marks: its measured size
    /// feeds the trigger's, so an enclosing retained sub-view must see them
    /// (water-rs/waterui#1957).
    pub(crate) fn take_layout_dirty(&mut self) -> bool {
        match &mut self.label {
            MenuLabel::View(subview) => subview.take_layout_dirty_if_built(),
            MenuLabel::Title(_) => false,
        }
    }
}

impl<E: crate::text::TextEngine> HydroNativeView<E> for Native<ResolvedMenu> {
    fn intrinsic(
        state: &mut crate::renderer::HydroState<E>,
        view: &Self,
        env: &Environment,
        theme: &Rc<dyn crate::theme::WidgetTheme>,
    ) -> LayoutSize {
        measure_menu_intrinsic(view.as_inner(), state, env, theme)
    }
}

/// Emits a menu trigger's accessibility node from its accessibility-label signal.
/// Shared by the rendered `Widget`-node flush (which passes its [`RenderContext`]
/// and theme for the popup anchor and metrics) and the semantic emission walk
/// (which passes `None` for both — activation only marks a menu group active).
pub fn menu_accessibility<E: crate::text::TextEngine>(
    renderer: &mut crate::renderer::SemanticCore<E>,
    ctx: Option<RenderContext>,
    theme: Option<&Rc<dyn crate::theme::WidgetTheme>>,
    state: &Rc<RefCell<MenuRenderState<E>>>,
    env: &Environment,
) {
    #[cfg(feature = "accessibility")]
    {
        let accessibility_label = state.borrow().accessibility_label.clone();
        let mut node = AccessibilityNode::new(
            crate::renderer::SemanticCore::<E>::resolve_accessibility_role(
                env,
                AccessibilityNodeRole::Button,
            ),
        );
        let default_label = Some(
            renderer
                .read_signal(&accessibility_label)
                .to_plain()
                .to_string(),
        );
        let label = renderer.resolve_accessibility_label(env, default_label);
        if let Some(label) = label {
            node.set_label(label);
        }
        if let Some(value) = renderer.resolve_accessibility_value(env, None) {
            node.set_value(value);
        }
        node.add_action(AccessibilityAction::Focus);
        node.add_action(AccessibilityAction::Click);
        // Direct activation: show the popup under the trigger's own anchor. The
        // semantic runtime has no trigger rect, so its popup mounts at the
        // origin through `activate_popup_menu_nodes` — the items land in the
        // merged accessibility tree exactly as the rendered popup's do.
        let request = ctx.as_ref().zip(theme).map(|(ctx, theme)| {
            (
                crate::widgets::util::popup_anchor(renderer.resolve_window_rect(ctx.bounds())),
                theme.clone(),
            )
        });
        let items = state.borrow().items.clone();
        // The popup opens in the trigger node's environment layered over the
        // dispatch's, so `.state(&value)` overlays reach the item actions
        // (water-rs/hydrolysis#140).
        let menu_env = env.clone();
        let activation = AccessibilityActionTarget::Activate {
            action: Rc::new(RefCell::new(
                move |renderer: &mut crate::renderer::SemanticCore<E>, env: &Environment| {
                    let env = menu_env.layered_on(env);
                    let nodes =
                        popup_menu_nodes(&items.snapshot(), &env, renderer.window_closable());
                    match &request {
                        Some((anchor, theme)) => {
                            renderer.show_popup_menu_nodes(nodes, *anchor, &env, theme);
                        }
                        None => {
                            renderer.activate_popup_menu_nodes(nodes, &env);
                        }
                    }
                    true
                },
            )),
        };
        let node_id = match ctx {
            Some(ctx) => {
                let bounds = ctx.bounds();
                renderer.register_accessibility_node(node, bounds, env, Some(activation))
            }
            None => renderer.register_accessibility_node_semantic(node, env, Some(activation)),
        };
        if let Some(node_id) = node_id {
            renderer.register_accessibility_focus_link(
                &crate::renderer::InteractionKey::for_rc(state, 0),
                node_id,
            );
        }
    }
    #[cfg(not(feature = "accessibility"))]
    {
        let _ = (renderer, ctx, theme, state, env);
    }
}

/// Measures a retained menu leaf from its [`MenuRenderState`].
pub fn measure_menu_node<E: crate::text::TextEngine>(
    state: &MenuRenderState<E>,
    proposal: ProposalSize,
    hydro: &mut HydroState<E>,
    env: &Environment,
    theme: &Rc<dyn crate::theme::WidgetTheme>,
) -> ViewDimensions {
    let spec = state.spec(env);
    let look = state.chrome.peek(theme, &spec, env);
    let label_size = match &state.label {
        MenuLabel::Title(label) => {
            let font = look_font(
                &look,
                state.chrome.label_state.snapshot(),
                state.chrome.frame.get(),
            );
            let styled = styled_button_title(hydro, font, label, env);
            HydrolysisRenderer::measure_text_intrinsic_size(hydro, styled, env)
        }
        MenuLabel::View(subview) => subview.measure_built(hydro, env, theme),
    };
    ViewDimensions::new(button_size(label_size, &look, proposal))
}

/// Renders a retained menu leaf every flush: emits a11y (unless hidden) then the
/// chrome + label + tap target, reading the accessibility-label/items signals.
pub fn render_menu_node<E: crate::text::TextEngine>(
    ctx: &mut WidgetRenderContext<'_, E>,
    state: &Rc<RefCell<MenuRenderState<E>>>,
    env: &Environment,
) {
    let hidden = env
        .get::<waterui::accessibility::AccessibilityHidden>()
        .is_some_and(waterui::accessibility::AccessibilityHidden::is_hidden);
    if !hidden {
        let theme = ctx.theme();
        let render_ctx = ctx.render_context();
        menu_accessibility(
            ctx.renderer_mut(),
            Some(render_ctx),
            Some(&theme),
            state,
            env,
        );
    }
    render_menu_parts(ctx, state, env);
}

/// What one render of a button-like control resolved: the look, the live
/// chrome parts, the reported state, the touch target and where the label
/// goes.
struct RenderedChrome {
    look: ButtonLook,
    parts: LiveChromeParts,
    flags: InteractionState,
    target: Rect,
    press_slot: crate::renderer::PressSlot,
    label_target: Rect,
}

/// Binds a button-like control's interaction target, realizes its look's
/// chrome and reports its state: everything a render needs before the
/// chrome's layers and the label are listed. A `title` label takes its
/// colour from the chrome's live operand; a label laid out from a view
/// follows the reported state through its own colour signal.
fn bind_button_chrome<E: crate::text::TextEngine>(
    ctx: &mut WidgetRenderContext<'_, E>,
    chrome_state: &mut ButtonChromeState,
    key: crate::renderer::InteractionKey,
    spec: &ButtonSpec,
    disabled: bool,
    title: bool,
    env: &Environment,
) -> RenderedChrome {
    let theme = ctx.theme();
    let look = chrome_state.look(&theme, spec, env).clone();
    let bounds = ctx.bounds;
    let chrome = look.chrome().clone();
    let frame = chrome.frame(bounds);
    chrome_state.frame.set(frame);
    let target = chrome.target(bounds);
    let report_key = key.clone();
    let (_, press_slot, handles) = ctx
        .renderer_mut()
        .bind_live_chrome_target(key, target, env, disabled, &chrome);
    let flags = ctx.renderer_mut().reported_interaction_state(&report_key);
    // Feed the reported state to the retained label's colour signal so it
    // resolves the current colour without a rebuild.
    if chrome_state.label_state.snapshot() != flags {
        chrome_state.label_state.set(flags);
    }
    let content = ChromeContent {
        group_opacity: Some(look.get_group_opacity().clone()),
        label_color: (title && env.get::<ListRowChrome>().is_none())
            .then(|| look.label().get_color().cloned())
            .flatten(),
        slider: None,
    };
    let parts = LiveChromeParts::ensure(
        &mut chrome_state.parts,
        &chrome,
        &content,
        bounds,
        flags,
        env,
        &chrome_state.label_state,
    )
    .clone();
    handles.attach_live(Rc::clone(parts.chrome()));
    let padding = look.get_padding();
    let label_bounds = inset_rect(bounds, padding.x, padding.y);
    // A degenerate padded rect (a button smaller than its padding) falls back to the
    // full bounds so the label still shows.
    let label_target = if label_bounds.width() > 0.0 && label_bounds.height() > 0.0 {
        label_bounds
    } else {
        bounds
    };
    RenderedChrome {
        look,
        parts,
        flags,
        target,
        press_slot,
        label_target,
    }
}

/// Lists a button-like control's chrome around its label: the group scope
/// at the look's group opacity, the surface below, the label `render_label`
/// records, the interaction layers above.
fn record_button_chrome<E: crate::text::TextEngine>(
    ctx: &mut WidgetRenderContext<'_, E>,
    rendered: &RenderedChrome,
    render_label: impl FnOnce(&mut WidgetRenderContext<'_, E>),
) {
    let frame = rendered.parts.chrome().visual_bounds().union(ctx.bounds);
    let key = crate::renderer::mount::ScopeKey {
        role: "button-group",
        item: 0,
    };
    let body = |ctx: &mut WidgetRenderContext<'_, E>| {
        let render_ctx = ctx.render_context();
        rendered.parts.push_below(ctx.renderer_mut(), render_ctx);
        render_label(ctx);
        let render_ctx = ctx.render_context();
        rendered.parts.push_above(ctx.renderer_mut(), render_ctx);
    };
    // An opacity that varies between renders draws the button as one
    // group following the chrome's operand; a fixed one groups it only
    // when it fades.
    let opacity = rendered.parts.chrome().group_opacity().map_or_else(
        || {
            GroupOpacity::Fixed(rendered.look.get_group_opacity().target(&Inputs::at_rest(
                rendered.look.chrome().frame(ctx.bounds),
                rendered.flags,
                &(),
            )))
        },
        GroupOpacity::Live,
    );
    let space = rendered.look.get_group_blend_space();
    ctx.with_group_scope(key, opacity, space, frame, body);
}

/// Places a retained label sub-view centred in `label_target`. Its
/// semantics are merged into the control's own node, so it flushes
/// visual-only.
fn place_label_view<E: crate::text::TextEngine>(
    ctx: &mut WidgetRenderContext<'_, E>,
    rendered: &RenderedChrome,
    subview: &mut RetainedSubview<E>,
    env: &Environment,
) {
    let label_target = rendered.label_target;
    let proposal = ProposalSize::new(
        Some(crate::num_cast::f64_as_f32(label_target.width())),
        Some(crate::num_cast::f64_as_f32(label_target.height())),
    );
    let (label_size, _) = subview.patch_and_measure(ctx.renderer_mut(), env, proposal);
    let label_target = centered_label_rect(label_target, label_size);
    let mut place = |ctx: &mut WidgetRenderContext<'_, E>| {
        let render_ctx = ctx.render_context();
        let label_area = ctx.safe_area_for(label_target);
        ctx.renderer_mut()
            .with_suppressed_accessibility(|renderer| {
                subview.place(
                    renderer,
                    render_ctx,
                    env,
                    ProposalSize::UNSPECIFIED,
                    label_target,
                    label_area,
                );
            });
    };
    // A label that follows the press scale is placed at rest inside a
    // layer whose transform follows the chrome's scale operand.
    match label_scale(rendered) {
        Some(scale) => {
            let operands = crate::renderer::ScopeOperands {
                opacity: None,
                transform: Some(scale),
            };
            let clip = rendered.parts.chrome().visual_bounds().union(ctx.bounds);
            ctx.with_live_scope(
                crate::renderer::mount::ScopeKey {
                    role: "button-label",
                    item: 0,
                },
                operands,
                clip,
                place,
            );
        }
        None => place(ctx),
    }
}

/// The chrome's press scale when the look's label follows it and the
/// chrome scales on press.
fn label_scale(rendered: &RenderedChrome) -> Option<crate::theme::live::Operand<kurbo::Affine>> {
    let chrome = rendered.parts.chrome();
    (rendered.look.label().get_follows_press_scale() && chrome.scales_content())
        .then(|| chrome.content_scale_operand())
}

/// Renders a title label: centred styled text in the look's font, over the
/// look's colour for the reported state — live, following the chrome's
/// label colour operand, when that colour varies between renders.
fn render_title<E: crate::text::TextEngine>(
    ctx: &mut WidgetRenderContext<'_, E>,
    rendered: &RenderedChrome,
    label: &Label,
    env: &Environment,
) {
    let target = rendered.label_target;
    if target.width() <= 0.0 || target.height() <= 0.0 {
        return;
    }
    let frame = rendered.look.chrome().frame(ctx.bounds);
    let font = look_font(&rendered.look, rendered.flags, frame);
    let mut styled = styled_button_title(ctx.state_mut(), font, label, env);
    let paint = rendered.parts.chrome().label_paint();
    if paint.is_none() {
        let color = if env.get::<ListRowChrome>().is_some() {
            Some(Color::new(waterui::theme::color::Foreground))
        } else {
            look_color(&rendered.look, rendered.flags, frame)
        };
        if let Some(color) = color {
            styled = styled_with_default_foreground(styled, color);
        }
    }
    // A label that follows the press scale grows with the chrome.
    let scale = rendered
        .look
        .label()
        .get_follows_press_scale()
        .then(|| rendered.parts.chrome());
    ctx.render_styled_text_single_line_scaled(styled, env, target, scale, paint.as_ref());
}

pub fn render_button_parts<E: crate::text::TextEngine>(
    ctx: &mut WidgetRenderContext<'_, E>,
    state: &Rc<RefCell<ButtonRenderState<E>>>,
    env: &Environment,
) {
    // Subscribe to the (possibly reactive) title so a label change schedules a frame
    // and this persistent node re-renders the new text.
    {
        let label = state.borrow().config.label().clone();
        watch_button_title(ctx.renderer_mut(), &label, env);
    }
    // Reading the disabled signal watches it, so a change schedules a frame
    // and this persistent node re-renders (and re-registers input) with the
    // new state.
    let disabled = {
        let signal = widget_disabled(env);
        ctx.renderer_mut().read_signal(&signal)
    };
    let key = crate::renderer::InteractionKey::for_rc(state, 0);
    let rendered = {
        let mut state_mut = state.borrow_mut();
        let spec = state_mut.spec(env);
        let title = state_mut.label_view.is_none();
        bind_button_chrome(ctx, &mut state_mut.chrome, key, &spec, disabled, title, env)
    };
    record_button_chrome(ctx, &rendered, |ctx| {
        let mut state_mut = state.borrow_mut();
        let state_mut = &mut *state_mut;
        if let Some(subview) = &mut state_mut.label_view {
            place_label_view(ctx, &rendered, subview, env);
        } else {
            render_title(ctx, &rendered, state_mut.config.label(), env);
        }
    });

    // A disabled button registers no tap target: the pointer neither presses
    // it nor invokes its action. Targets are re-registered every flush, so
    // re-enabling restores interactivity on the next frame.
    if disabled {
        return;
    }
    // Invoke the action through the shared state cell so the retained node and the
    // dispatch path register equivalent tap targets without moving the action out.
    let state = Rc::clone(state);
    let action_env = env.clone();
    let keyboard_focusable = rendered.look.chrome().keyboard_focusable();
    ctx.renderer_mut()
        .register_interactive_pointer_target_with_keyboard(
            rendered.target,
            rendered.press_slot,
            keyboard_focusable,
            move |_renderer, _point, _env| {
                (state.borrow_mut().config.action_mut())(&action_env).report_extraction();
                true
            },
        );
}

pub fn render_menu_parts<E: crate::text::TextEngine>(
    ctx: &mut WidgetRenderContext<'_, E>,
    state: &Rc<RefCell<MenuRenderState<E>>>,
    env: &Environment,
) {
    let theme = ctx.theme();
    let key = crate::renderer::InteractionKey::for_rc(state, 0);
    let rendered = {
        let mut state_mut = state.borrow_mut();
        let spec = state_mut.spec(env);
        let title = matches!(state_mut.label, MenuLabel::Title(_));
        bind_button_chrome(ctx, &mut state_mut.chrome, key, &spec, false, title, env)
    };
    record_button_chrome(ctx, &rendered, |ctx| {
        let mut state_mut = state.borrow_mut();
        match &mut state_mut.label {
            MenuLabel::Title(label) => {
                render_title(ctx, &rendered, label, env);
            }
            MenuLabel::View(subview) => {
                place_label_view(ctx, &rendered, subview, env);
            }
        }
    });

    let items = state.borrow().items.clone();
    // Watch the items so a change schedules a frame (the popup re-reads live items
    // on open, but a change still re-presents the trigger).
    let _ = ctx.renderer_mut().read_signal(&items);
    // A mounted `Menu` arms its commands' chords on the window's scope while
    // mounted (water-rs/hydrolysis#247); unmount drops the render state and
    // the registration with it.
    ctx.renderer_mut()
        .register_menu_shortcuts(Rc::downgrade(state), items.clone(), env.clone());
    // The popup anchors in window space at the trigger's bottom edge.
    let bounds = ctx.bounds;
    let anchor = crate::widgets::util::popup_anchor(ctx.renderer_mut().resolve_window_rect(bounds));
    let menu_theme = theme.clone();
    // The popup opens in the trigger node's environment layered over the
    // dispatch's (water-rs/hydrolysis#140).
    let menu_env = env.clone();
    let keyboard_focusable = rendered.look.chrome().keyboard_focusable();
    ctx.renderer_mut()
        .register_interactive_pointer_target_with_keyboard(
            rendered.target,
            rendered.press_slot,
            keyboard_focusable,
            move |renderer, _point, env| {
                let env = menu_env.layered_on(env);
                renderer.show_popup_menu_nodes(
                    popup_menu_nodes(&items.snapshot(), &env, renderer.window_closable()),
                    anchor,
                    &env,
                    &menu_theme,
                )
            },
        );
}

/// The intrinsic size of a button from its look, measured without a
/// retained node.
pub fn measure_button_intrinsic<E: crate::text::TextEngine>(
    button: &ButtonConfig,
    state: &mut HydroState<E>,
    env: &Environment,
    theme: &Rc<dyn crate::theme::WidgetTheme>,
) -> LayoutSize {
    measure_button_look_intrinsic(
        button.style(),
        button.size(),
        button.label(),
        state,
        env,
        theme,
    )
}

/// The intrinsic size of a `style` button of `size` showing `label`, from
/// its look — shared by the configured and the generic `Button` paths.
pub fn measure_button_look_intrinsic<E: crate::text::TextEngine>(
    style: ButtonStyle,
    size: ControlSize,
    label: &Label,
    state: &mut HydroState<E>,
    env: &Environment,
    theme: &Rc<dyn crate::theme::WidgetTheme>,
) -> LayoutSize {
    let spec = button_spec(style, size, label_resolves_icon_only(label, env), env);
    let look = resolve_look(theme, &spec, env);
    let font = look_font(&look, InteractionState::empty(), Rect::ZERO);
    let label_size = if renders_as_plain_title(label) {
        let styled = styled_button_title(state, font, label, env);
        HydrolysisRenderer::measure_text_intrinsic_size(state, styled, env)
    } else {
        let label = styled_button_label(font, label.clone());
        measure_label_intrinsic(&label, state, env, theme)
    };
    button_size(label_size, &look, ProposalSize::UNSPECIFIED)
}

pub fn measure_menu_intrinsic<E: crate::text::TextEngine>(
    menu: &ResolvedMenu,
    state: &mut HydroState<E>,
    env: &Environment,
    theme: &Rc<dyn crate::theme::WidgetTheme>,
) -> LayoutSize {
    let icon_only = menu
        .label
        .downcast_ref::<Label>()
        .is_some_and(|label| label_resolves_icon_only(label, env));
    let spec = button_spec(menu.style, menu.size, icon_only, env);
    let look = resolve_look(theme, &spec, env);
    let label_size = if let Some(label) = menu.label.downcast_ref::<Label>()
        && renders_as_plain_title(label)
    {
        let font = look_font(&look, InteractionState::empty(), Rect::ZERO);
        let styled = styled_button_title(state, font, label, env);
        HydrolysisRenderer::measure_text_intrinsic_size(state, styled, env)
    } else {
        measure_view_intrinsic(&menu.label, state, env, theme)
    };
    button_size(label_size, &look, ProposalSize::UNSPECIFIED)
}

fn button_label_view(color: Option<Color>, label: AnyView) -> AnyView {
    match color {
        Some(color) => AnyView::new(label.foreground(color)),
        None => label,
    }
}

/// Marks the subtree of a list row.
///
/// A row's content is body text that happens to be tappable, so it keeps the
/// ordinary foreground colour. Themes paint a container-less button in their
/// accent colour, which is right for a text button on a screen and wrong for
/// every row of a list.
#[derive(Debug, Clone, Copy)]
pub struct ListRowChrome;

/// Whether a label is nothing but its own title, and can therefore take the
/// cheap path that draws styled text instead of retaining a sub-view.
///
/// A label built from caller-supplied views also reports `TitleOnly` once
/// resolved — it has no icon — so the display mode alone is not the question;
/// answering it with the mode alone drew the spoken text in place of the
/// content.
const fn renders_as_plain_title(label: &Label) -> bool {
    matches!(label.display_mode_preference(), LabelDisplayMode::TitleOnly)
        && !label.has_custom_content()
}

/// Whether the label's own configuration resolves to an icon-only
/// presentation. The label resolves its effective display mode — its
/// preference, the `LabelDisplayMode` it inherits from the environment, and
/// whether it carries an icon — so the chrome agrees with what the label
/// draws without inspecting rendered children.
pub fn label_resolves_icon_only(label: &Label, env: &Environment) -> bool {
    matches!(
        label.effective_display_mode(env),
        LabelDisplayMode::IconOnly
    )
}

fn styled_button_title<E: crate::text::TextEngine>(
    state: &mut HydroState<E>,
    font: Option<Font>,
    label: &Label,
    env: &Environment,
) -> StyledStr {
    let title = label.semantic_text().clone();
    let title = if let Some(font) = font {
        title.font(font)
    } else {
        title
    };
    let resolved = title.resolve(env);
    state.measure_signal(&resolved.content())
}

/// Subscribes to a label's reactive title content so a change schedules a frame
/// (the persistent `Widget` node then re-renders it live). The returned value is
/// ignored; the side effect is the watch registration.
fn watch_button_title<E: crate::text::TextEngine>(
    renderer: &mut HydrolysisRenderer<E>,
    label: &Label,
    env: &Environment,
) {
    let _ = renderer.read_signal(&label.semantic_text().resolve(env).content());
}

/// Fills `color` into chunks that have no explicit foreground: the look's
/// label colour is a default, never an override of caller styling.
#[expect(
    clippy::needless_pass_by_value,
    reason = "the parameter is a small Copy value taken by value for a uniform call-site signature"
)]
pub fn styled_with_default_foreground(styled: StyledStr, color: Color) -> StyledStr {
    let mut out = StyledStr::empty();
    for (chunk, style) in styled.chunks() {
        let mut style = style.clone();
        if style.get_foreground().is_none() {
            style.set_foreground(Some(color.clone()));
        }
        out.push(chunk.clone(), style);
    }
    out
}

/// Applies the look's label font to a label that owns its text. A label
/// built from caller-supplied views carries its own typography, and
/// `Label::font` rejects it outright.
pub fn styled_button_label(font: Option<Font>, label: Label) -> Label {
    if label.has_custom_content() {
        return label;
    }
    if let Some(font) = font {
        label.font(font)
    } else {
        label
    }
}

/// Emits a retained button's accessibility node for the semantic walk — the
/// same node `button_accessibility` registers, with no bounds. The label
/// sub-view flushes visual-only (its semantics are merged into the button's
/// node), so there is nothing else to emit.
#[cfg(feature = "accessibility")]
pub fn emit_button_accessibility<E: crate::text::TextEngine>(
    renderer: &mut crate::renderer::SemanticCore<E>,
    state: &Rc<RefCell<ButtonRenderState<E>>>,
    env: &Environment,
) {
    button_accessibility(renderer, None, state, env);
}

/// Emits a retained menu trigger's accessibility node for the semantic walk —
/// the same node `menu_accessibility` registers, with no bounds.
#[cfg(feature = "accessibility")]
pub fn emit_menu_accessibility<E: crate::text::TextEngine>(
    renderer: &mut crate::renderer::SemanticCore<E>,
    state: &Rc<RefCell<MenuRenderState<E>>>,
    env: &Environment,
) {
    menu_accessibility(renderer, None, None, state, env);
}

#[cfg(test)]
mod tests {
    use super::{
        ButtonChromeState, MenuRenderState, button_size, look_color, retained_label_color,
    };
    use crate::theme::{
        ButtonLook, Chrome, CornerShape, FocusIndicator, Label, Motion, PressFeedback, StateLayer,
        Surface,
    };
    use cherenkov_record::WorkingColor;
    use kurbo::Rect;
    use nami::{Signal as _, SignalExt as _};
    use waterui::Color;
    use waterui::{AnyView, View};
    use waterui_controls::ControlSize;
    use waterui_controls::button::ButtonStyle;
    use waterui_controls::menu::{Menu, ResolvedMenu};
    use waterui_core::Environment;
    use waterui_core::animation::Animation;
    use waterui_core::interaction::{InteractionState, StateValue};
    use waterui_core::layout::{ProposalSize, Size};

    /// The trigger spec of `menu` resolved in `env`.
    fn trigger_spec(menu: Menu, env: &Environment) -> crate::theme::ButtonSpec {
        let resolved = AnyView::new(View::body(menu, env))
            .downcast::<ResolvedMenu>()
            .expect("a menu resolves to its native config");
        MenuRenderState::<crate::text::ParleyEngine>::from_resolved(*resolved, env).spec(env)
    }

    #[test]
    fn menu_trigger_takes_the_menus_style_and_size() {
        let env = Environment::new();
        let plain = trigger_spec(Menu::new("Sort", ()), &env);
        assert_eq!(
            (plain.style(), plain.size()),
            (ButtonStyle::Automatic, ControlSize::Small),
            "a menu defaults to the default button"
        );

        let styled = trigger_spec(
            Menu::new("Sort", ())
                .style(ButtonStyle::BorderedProminent)
                .size(ControlSize::Large),
            &env,
        );
        assert_eq!(
            (styled.style(), styled.size()),
            (ButtonStyle::BorderedProminent, ControlSize::Large)
        );

        let mut ambient = Environment::new();
        ambient.install(ButtonStyle::Bordered);
        assert_eq!(
            trigger_spec(Menu::new("Sort", ()), &ambient).style(),
            ButtonStyle::Bordered,
            "an automatic trigger takes the ambient button style"
        );
    }

    fn tint_of(spec: &crate::theme::ButtonSpec, env: &Environment) -> [f32; 4] {
        spec.tint().resolve(env).snapshot().components
    }

    #[test]
    fn a_tinted_subtree_carries_its_tint_and_an_untinted_one_the_accent() {
        let accent = WorkingColor::new([0.0, 0.0, 1.0, 1.0]);
        let red = WorkingColor::new([1.0, 0.0, 0.0, 1.0]);
        let mut env = Environment::new();
        waterui::theme::install_color_signal::<waterui::theme::color::Accent>(
            &mut env,
            nami::constant(accent).computed(),
        );
        let spec = |env: &Environment| {
            super::button_spec(ButtonStyle::Automatic, ControlSize::Small, false, env)
        };
        assert_eq!(tint_of(&spec(&env), &env), accent.components);

        let tinted = env.extending(waterui::theme::Tint::new(Color::new(
            waterui_graphics::color::Working(red),
        )));
        assert_eq!(tint_of(&spec(&tinted), &tinted), red.components);
        assert_ne!(
            spec(&tinted),
            spec(&env),
            "a re-tint requests the look again"
        );
        assert_eq!(
            spec(&tinted),
            spec(&tinted),
            "the same tint is the same spec"
        );
    }

    #[test]
    fn a_tint_signal_change_reaches_the_look_without_requesting_it_again() {
        let source = nami::Binding::container(WorkingColor::new([1.0, 0.0, 0.0, 1.0]));
        let tint = waterui::theme::Tint::new(waterui_graphics::color::signal_color(
            source.map(|color| Color::new(waterui_graphics::color::Working(color))),
        ));
        let env = Environment::new().extending(tint);
        let spec = super::button_spec(ButtonStyle::Automatic, ControlSize::Small, false, &env);
        let color = spec.tint().resolve(&env);
        assert_eq!(color.snapshot().components, [1.0, 0.0, 0.0, 1.0]);

        source.set(WorkingColor::new([0.0, 1.0, 0.0, 1.0]));

        assert_eq!(color.snapshot().components, [0.0, 1.0, 0.0, 1.0]);
        let again = super::button_spec(ButtonStyle::Automatic, ControlSize::Small, false, &env);
        assert_eq!(again, spec, "the held look stays: no rebuild");
    }

    #[test]
    fn button_measurement_uses_the_look_and_the_proposal() {
        let label = Size::new(11.0, 13.0);
        let chrome = Chrome::new(
            Surface::clear(CornerShape::radius(0.0)),
            StateLayer::new(Color::srgb(0, 0, 0)),
            PressFeedback::new(),
            FocusIndicator::Tint,
            Motion::uniform(Animation::linear(core::time::Duration::ZERO)),
        );
        let look = ButtonLook::new(chrome, Label::new())
            .padding(3.0, 5.0)
            .minimum_size(kurbo::Size::new(37.0, 41.0));

        assert_eq!(
            button_size(label, &look, ProposalSize::UNSPECIFIED),
            Size::new(37.0, 41.0)
        );
        assert_eq!(
            button_size(label, &look, ProposalSize::new(Some(31.0), Some(29.0))),
            Size::new(31.0, 29.0)
        );
    }

    fn rgb(color: WorkingColor) -> (u8, u8, u8) {
        let srgb = waterui_graphics::color::working::to_srgb(color);
        let channel = |v: f32| crate::num_cast::f32_as_u8((v * 255.0).round());
        (channel(srgb.red), channel(srgb.green), channel(srgb.blue))
    }

    fn coloured_look() -> ButtonLook {
        let chrome = Chrome::new(
            Surface::clear(CornerShape::radius(0.0)),
            StateLayer::new(Color::srgb(0, 0, 0)),
            PressFeedback::new(),
            FocusIndicator::Tint,
            Motion::uniform(Animation::linear(core::time::Duration::ZERO)),
        );
        ButtonLook::new(
            chrome,
            Label::new().color(
                StateValue::new(Color::srgb(0, 0, 255))
                    .when(InteractionState::DISABLED, Color::srgb(128, 128, 128))
                    .when(InteractionState::HOVERED, Color::srgb(255, 0, 0)),
            ),
        )
    }

    /// The look's label colour resolves against the state: a disabled
    /// label takes the disabled value, an enabled one the resting value.
    #[test]
    fn look_label_colour_resolves_disabled() {
        let env = crate::renderer::tests::pumped_test_environment();
        let look = coloured_look();
        let resolve = |state| {
            rgb(look_color(&look, state, Rect::ZERO)
                .expect("the look styles the label colour")
                .resolve(&env)
                .snapshot())
        };
        assert_eq!(resolve(InteractionState::empty()), (0, 0, 255), "enabled");
        assert_eq!(
            resolve(InteractionState::DISABLED),
            (128, 128, 128),
            "disabled"
        );
    }

    /// The retained label's colour follows the reported state reactively,
    /// not just DISABLED: writing HOVERED recolours it with no rebuild.
    #[test]
    fn retained_label_colour_follows_hover() {
        let env = crate::renderer::tests::pumped_test_environment();
        let chrome = ButtonChromeState::new();
        let resolved = retained_label_color(&coloured_look(), &chrome, &env)
            .expect("the look styles the label colour")
            .resolve(&env);

        assert_eq!(rgb(resolved.snapshot()), (0, 0, 255), "resting");
        chrome.label_state.set(InteractionState::HOVERED);
        assert_eq!(rgb(resolved.snapshot()), (255, 0, 0), "hovered");
        chrome.label_state.set(InteractionState::empty());
        assert_eq!(rgb(resolved.snapshot()), (0, 0, 255), "back to resting");
    }
}
